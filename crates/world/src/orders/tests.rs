use super::*;
use crate::entity::{TargetIdentity, ZoomTarget};
use crate::player::Controller;
use crate::{AiScripts, Rules};

/// A file of the original game.
fn read(f: &str) -> String {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    let bytes = std::fs::read(dir.join(f))
        .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// jl1 with a human, `me`, against an easy computer.
fn game() -> (World, PlayerId, PlayerId) {
    let rules = Rules::load(&[read("res/rules.txt")]);
    let ai = AiScripts::load(&[read("res/ai.txt")]);
    let mut w = World::new(rules, ai, Vec::new(), 0);
    w.load_and_build_map(&read("res/multi/jl1.txt")).unwrap();
    let players = w
        .populate_map(
            &[Controller::human("me", true), Controller::computer("easy")],
            false,
            true,
        )
        .unwrap();
    (w, players[0], players[1])
}

fn home(w: &World, p: PlayerId) -> SquareId {
    match w.e(w.pl(p).units[0]).place {
        Some(Place::Square(s)) => s,
        _ => panic!("on a square"),
    }
}

fn add(w: &mut World, type_name: &str, p: PlayerId, s: SquareId) -> EntityId {
    let t = w.rules.type_id(type_name).unwrap();
    let (x, y) = (w.square(s).x, w.square(s).y);
    let u = w.create_creature(t, Some(p), Place::Square(s), x, y);
    u.unwrap()
}

fn find(w: &World, p: PlayerId, type_name: &str) -> EntityId {
    let mut units = w.pl(p).units.iter().copied();
    units.find(|&u| &*w.e(u).type_name == type_name).unwrap()
}

/// The orders of a unit as the words they store: keyword, type, target.
fn orders(w: &World, u: EntityId) -> Vec<Vec<String>> {
    w.e(u).orders().iter().map(|o| o.words(w)).collect()
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|&s| s.to_owned()).collect()
}

// Targets

#[test]
fn a_target_is_written_as_the_original_wrote_it_and_read_back() {
    let (w, _, _) = game();
    let a1 = w.square_by_name("a1").unwrap();
    let id = w.square(a1).id;
    let cases = [
        (TargetRef::Square(a1), id.to_string()),
        (TargetRef::SquareNamed(a1), "a1".to_owned()),
        (
            TargetRef::Point {
                square: a1,
                x: 2000,
                y: 10000,
            },
            format!("zoom-{id}-2000-10000"),
        ),
        (TargetRef::Entity(EntityId(999_999)), "999999".to_owned()),
        (TargetRef::Nothing, "None".to_owned()),
    ];
    for (t, text) in cases {
        assert_eq!(t.text(&w), text);
        assert_eq!(TargetRef::parse(&w, &text), Ok(t));
    }
}

#[test]
fn a_word_names_a_target_as_the_original_read_it() {
    let (w, _, _) = game();
    let a1 = w.square_by_name("a1").unwrap();
    let id = w.square(a1).id;
    let parse = |s: &str| TargetRef::parse(&w, s);
    // A point may name its square by name.
    assert_eq!(
        parse("zoom-a1-1-2"),
        Ok(TargetRef::Point {
            square: a1,
            x: 1,
            y: 2
        })
    );
    // A number is a square's protocol id or an entity's id; any other word
    // names nothing.
    assert_eq!(parse(&id.to_string()), Ok(TargetRef::Square(a1)));
    for nothing in ["007", "+7", "", "None", "foo", "-1", "4294967296"] {
        assert_eq!(parse(nothing), Ok(TargetRef::Nothing), "{nothing:?}");
    }
    let negative = format!("zoom-{id}--1-0");
    let refused = [
        ("zoom", "\"zoom\" is not zoom-<square>-<x>-<y>".to_owned()),
        (
            "zoom-a1-1",
            "\"zoom-a1-1\" is not zoom-<square>-<x>-<y>".to_owned(),
        ),
        (
            &negative,
            format!("{negative:?} is not zoom-<square>-<x>-<y>"),
        ),
        (
            "zoom-999999-1-2",
            "\"zoom-999999-1-2\" names no square".to_owned(),
        ),
        (
            "zoom-a1-x-2",
            "\"x\" in \"zoom-a1-x-2\" is not a number".to_owned(),
        ),
    ];
    for (word, error) in refused {
        assert_eq!(parse(word), Err(error), "{word:?}");
    }
}

#[test]
fn a_target_means_what_the_player_perceives_or_remembers_now() {
    let (mut w, me, you) = game();
    let s = home(&w, me);
    let a1 = w.square_by_name("a1").unwrap();
    let peasant = find(&w, me, "peasant");
    let far_enemy = w.pl(you).units[0];
    // An enemy the player saw, then lost sight of.
    let seen = add(&mut w, "footman", you, s);
    w.update();
    assert!(w.pl(me).perception.contains(&seen));
    let far = w.square(w.e(far_enemy).place.map_or(a1, |p| match p {
        Place::Square(f) => f,
        Place::Inside(_) => a1,
    }));
    let (fx, fy) = (far.x, far.y);
    let far_place = w.e(far_enemy).place;
    let _ = w.move_to(seen, far_place, Some((fx, fy)), 0);
    w.update();
    assert!(!w.pl(me).perception.contains(&seen));
    let h = w.square(s);
    let inside = (h.x, h.y);
    let outside = (h.xmax, h.y);
    // A target has no equality but identity: compare what it shows.
    let resolve = |w: &World, t| format!("{:?}", w.resolve_target(me, &t));
    let ok = |t: Option<Target>| format!("{t:?}");
    let zoom = |(x, y)| {
        Target::Zoom(ZoomTarget {
            place: s,
            x,
            y,
            given: true,
        })
    };
    let square = ok(Some(Target::Square(a1)));
    assert_eq!(resolve(&w, TargetRef::Square(a1)), square);
    assert_eq!(resolve(&w, TargetRef::SquareNamed(a1)), square);
    let (x, y) = inside;
    let point = TargetRef::Point { square: s, x, y };
    assert_eq!(resolve(&w, point), ok(Some(zoom(inside))));
    let (x, y) = outside;
    let point = TargetRef::Point { square: s, x, y };
    // A point outside its square means nothing.
    assert!(w.resolve_target(me, &point).is_none());
    assert_eq!(
        resolve(&w, TargetRef::Entity(peasant)),
        ok(Some(Target::Entity(peasant)))
    );
    match w.resolve_target(me, &TargetRef::Entity(seen)) {
        Some(Target::Memory(m)) => assert_eq!((m.owner, m.snapshot.id), (me, seen)),
        other => panic!("{other:?}"),
    }
    assert_eq!(resolve(&w, TargetRef::Entity(far_enemy)), ok(None));
    assert_eq!(resolve(&w, TargetRef::Entity(EntityId(999_999))), ok(None));
    // Orders compare targets by identity: a remembered entity is the
    // entity, a point the player gave is its square and coordinates, and a
    // subsquare the simulation chose has none.
    let remembered = w.resolve_target(me, &TargetRef::Entity(seen));
    assert_eq!(
        remembered.and_then(|t| t.identity()),
        Some(TargetIdentity::Entity(seen))
    );
    assert_eq!(
        zoom(inside).identity(),
        Some(TargetIdentity::Point(s, inside.0, inside.1))
    );
    let subsquare = ZoomTarget {
        place: s,
        x: inside.0,
        y: inside.1,
        given: false,
    };
    assert_eq!(Target::Zoom(subsquare).identity(), None);
    // An id that is a square's names the square.
    let a1_id = EntityId(w.square(a1).id);
    assert_eq!(resolve(&w, TargetRef::Entity(a1_id)), square);
    assert_eq!(resolve(&w, TargetRef::Nothing), ok(None));
    // The text of each target means the same.
    let all = [
        TargetRef::Square(a1),
        TargetRef::Entity(peasant),
        TargetRef::Entity(far_enemy),
        TargetRef::Entity(a1_id),
        TargetRef::Nothing,
        TargetRef::Point {
            square: s,
            x: inside.0,
            y: inside.1,
        },
    ];
    for t in all {
        let read_back = TargetRef::parse(&w, &t.text(&w)).unwrap();
        let by_text = format!("{:?}", w.resolve_target(me, &read_back));
        assert_eq!(by_text, resolve(&w, t), "{t:?}");
    }
    let read_back = TargetRef::parse(&w, &seen.to_string()).unwrap();
    let by_text = w.resolve_target(me, &read_back).unwrap();
    let typed = w.resolve_target(me, &TargetRef::Entity(seen));
    assert!(by_text.same(&typed.unwrap()));
}

// Requests

#[test]
fn every_request_has_the_words_the_original_wrote() {
    let (w, _, _) = game();
    let a1 = w.square_by_name("a1").unwrap();
    let id = w.square(a1).id.to_string();
    let e = TargetRef::Entity(EntityId(7));
    let sq = TargetRef::Square(a1);
    let cases = [
        (Request::stop(), words(&["stop"])),
        (Request::auto_explore(), words(&["auto_explore"])),
        (Request::go(sq), words(&["go", &id])),
        (Request::go(TargetRef::Nothing), words(&["go", "None"])),
        (
            Request::go(TargetRef::Point {
                square: a1,
                x: 1,
                y: 2,
            }),
            words(&["go", &format!("zoom-{id}-1-2")]),
        ),
        (Request::gather(e), words(&["gather", "7"])),
        (Request::repair(e), words(&["repair", "7"])),
        (Request::load(e), words(&["load", "7"])),
        (
            Request::plain(Keyword::BuildPhaseTwo).unwrap().at(e),
            words(&["build_phase_two", "7"]),
        ),
        (Request::train("peasant"), words(&["train", "peasant"])),
        (
            Request::research("melee_weapon"),
            words(&["research", "melee_weapon"]),
        ),
        (
            Request::upgrade_to("knight"),
            words(&["upgrade_to", "knight"]),
        ),
        (Request::build("farm", e), words(&["build", "farm", "7"])),
        (
            Request::use_ability("a_meteors"),
            words(&["use", "a_meteors"]),
        ),
        (
            Request::use_ability("a_meteors").at(sq),
            words(&["use", "a_meteors", &id]),
        ),
        (Request::join_group("1"), words(&["join_group", "1"])),
        (
            Request::plain(Keyword::RallyingPoint).unwrap().at(sq),
            words(&["rallying_point", &id]),
        ),
    ];
    for (r, expected) in cases {
        assert_eq!(r.words(&w), expected);
        // An order done at once is never stored; a queued one keeps the
        // words it was given.
        match r.taken(&w).unwrap() {
            Taken::Now(_) => assert!(r.keyword().is_immediate(), "{expected:?}"),
            Taken::Queued(kind) => {
                assert!(!r.keyword().is_immediate(), "{expected:?}");
                assert_eq!(Order::new(1, kind).words(&w), expected);
            }
        }
    }
    for &k in Keyword::ALL {
        assert_eq!(Request::plain(k).is_some(), !k.is_complex(), "{k:?}");
        assert_eq!(Request::typed(k, "x").is_some(), k.is_complex(), "{k:?}");
    }
}

#[test]
fn only_the_orders_done_at_once_are_never_queued() {
    let (w, _, _) = game();
    let a1 = w.square_by_name("a1").unwrap();
    for &k in Keyword::ALL {
        let r = match k.menu_attribute() {
            Some(_) => Request::typed(k, "footman").unwrap(),
            None => Request::plain(k).unwrap(),
        }
        .at(TargetRef::Square(a1));
        let taken = r.taken(&w).unwrap();
        assert_eq!(matches!(taken, Taken::Now(_)), k.is_immediate(), "{k:?}");
        if let Taken::Queued(kind) = taken {
            assert_eq!(kind.keyword(), k);
        }
    }
}

#[test]
fn an_order_of_the_map_language_is_read_once_or_refused() {
    let (w, _, _) = game();
    let b1 = w.square_by_name("b1").unwrap();
    let parse = |s: &str| {
        let words: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
        Request::parse(&w, &words)
    };
    assert_eq!(parse("go b1"), Ok(Request::go(TargetRef::SquareNamed(b1))));
    assert_eq!(parse("go b1").unwrap().words(&w), words(&["go", "b1"]));
    assert_eq!(
        parse("auto_attack"),
        Request::plain(Keyword::AutoAttack).ok_or(String::new())
    );
    assert_eq!(parse("train footman"), Ok(Request::train("footman")));
    assert_eq!(
        parse("build farm b1"),
        Ok(Request::build("farm", TargetRef::SquareNamed(b1)))
    );
    // A group is a name, not a target.
    assert_eq!(parse("join_group b1"), Ok(Request::join_group("b1")));
    for refused in ["", "dance", "train", "go b1 b2", "go zoom-b1"] {
        assert!(parse(refused).is_err(), "{refused:?}");
    }
}

// The orders units give without a player, where the recorded games do not
// reach.

#[test]
fn a_unit_that_enters_goes_to_its_transport_which_loads_it() {
    let (mut w, me, _) = game();
    let s = home(&w, me);
    let footman = add(&mut w, "footman", me, s);
    let transport = add(&mut w, "new_flyingmachine", me, s);
    let enter = Request::plain(Keyword::Enter).unwrap();
    w.take_order(
        footman,
        enter.at(TargetRef::Entity(transport)),
        true,
        false,
        None,
    )
    .unwrap();
    assert_eq!(
        orders(&w, transport),
        [words(&["load", &footman.to_string()])]
    );
    assert_eq!(
        orders(&w, footman),
        [words(&["go", &transport.to_string()])]
    );
}

#[test]
fn a_unit_trained_goes_to_the_rallying_point() {
    let (mut w, me, _) = game();
    let townhall = find(&w, me, "townhall");
    let b2 = w.square_by_name("b2").unwrap();
    let b2_id = w.square(b2).id.to_string();
    let rally = Request::plain(Keyword::RallyingPoint).unwrap();
    w.take_order(townhall, rally.at(TargetRef::Square(b2)), true, false, None)
        .unwrap();
    assert_eq!(
        w.c(townhall).unwrap().rallying_point,
        Some(TargetRef::Square(b2))
    );
    let before = w.pl(me).units.clone();
    w.take_order(townhall, Request::train("peasant"), false, false, None)
        .unwrap();
    for _ in 0..500 {
        w.update();
        if w.pl(me).units.len() > before.len() {
            break;
        }
    }
    let new = *w.pl(me).units.iter().find(|u| !before.contains(u)).unwrap();
    assert_eq!(orders(&w, new).first(), Some(&words(&["go", &b2_id])));
}

#[test]
fn an_idle_worker_repairs_a_damaged_building_near_it() {
    let (mut w, me, _) = game();
    let townhall = find(&w, me, "townhall");
    let peasant = find(&w, me, "peasant");
    w.cm(peasant).unwrap().work_mut().unwrap().auto_repair = true;
    w.cm(townhall).unwrap().hp -= 10;
    w.decide(peasant).unwrap();
    assert_eq!(
        orders(&w, peasant),
        [words(&["repair", &townhall.to_string()])]
    );
}

#[test]
fn a_place_inside_a_unit_is_no_target() {
    let (mut w, me, _) = game();
    let s = home(&w, me);
    let transport = add(&mut w, "new_flyingmachine", me, s);
    let t = w.place_target(Place::Inside(transport));
    assert_eq!(t, TargetRef::Nothing);
    assert_eq!(Request::go(t).words(&w), words(&["go", "None"]));
    assert!(w.resolve_target(me, &t).is_none());
}

// Units that change hands or change type, given orders as a player does.

/// Two humans with no units, `a` (whose events are recorded) on a1 and
/// `b` on b1, in a game that allows cheats, with the base rules and then
/// a mod's `extra` rules.
fn two_humans(extra: &str) -> (World, PlayerId, PlayerId) {
    let rules = Rules::load(&[read("res/rules.txt"), extra.to_owned()]);
    let ai = AiScripts::load(&[read("res/ai.txt")]);
    let mut w = World::new(rules, ai, Vec::new(), 0);
    w.allow_cheatmode = true;
    let map = "square_width 12\nnb_columns 2\nnb_lines 1\nnb_meadows_by_square 0\n\
               nb_players_min 2\nnb_players_max 2\nstarting_squares a1 b1\n";
    w.load_and_build_map(map).unwrap();
    let humans = [Controller::human("a", true), Controller::human("b", false)];
    let ps = w.populate_map(&humans, false, false).unwrap();
    for &p in &ps {
        w.plm(p).resources = vec![1000 * crate::nofloat::PRECISION; 2];
    }
    (w, ps[0], ps[1])
}

/// Gives `units` of `p` an order written as a game on a server sends it:
/// `use a_conversion 12`.
fn order(w: &mut World, p: PlayerId, units: &[EntityId], words: &str) {
    w.queue(p, crate::Command::Control(units.to_vec()));
    w.queue_command(p, &format!("order 0 0 {words}"));
}

/// The units of `p` of this type, in the player's order.
fn all(w: &World, p: PlayerId, type_name: &str) -> Vec<EntityId> {
    let units = w.pl(p).units.iter().copied();
    units.filter(|&u| &*w.e(u).type_name == type_name).collect()
}

/// Runs turns until `done`, at least one and at most 1000: the commands
/// queued run in the next.
fn update_until(w: &mut World, done: impl Fn(&World) -> bool) {
    for _ in 0..1000 {
        w.update();
        if done(w) {
            return;
        }
    }
    panic!("not done after 1000 turns");
}

/// Damage and speed, in thousandths, and the damage upgrade level.
fn bonuses(w: &World, u: EntityId) -> (i64, i64, i64) {
    let e = w.e(u);
    let damage_level = e.creature().unwrap().damage_level;
    (e.stats.damage, e.stats.speed, damage_level)
}

#[test]
fn a_resurrected_unit_has_its_owners_upgrades_once() {
    let (mut w, a, _) = two_humans("");
    let line = "cmd add_units a1 melee_weapon horse_speed u_resurrection knight priest";
    w.queue_command(a, line);
    w.update();
    let knight = find(&w, a, "knight");
    // A knight does 6 damage and walks at 2.5: the weapon adds its damage
    // bonus of 6, the horses 2.5.
    // Like the original, the weapon upgrade counts twice in the level.
    assert_eq!(bonuses(&w, knight), (12_000, 5_000, 2));
    w.die(knight, None).unwrap();
    let priest = find(&w, a, "priest");
    order(&mut w, a, &[priest], "use a_resurrection a1");
    update_until(&mut w, |w| !all(w, a, "knight").is_empty());
    let knight = find(&w, a, "knight");
    assert_eq!(bonuses(&w, knight), (12_000, 5_000, 2));
}

#[test]
fn a_converted_unit_has_its_new_owners_upgrades_and_none_of_its_old_owners() {
    let (mut w, a, b) = two_humans("");
    w.queue_command(a, "cmd add_units a1 melee_weapon u_conversion priest");
    w.queue_command(b, "cmd add_units a1 melee_weapon horse_speed knight");
    w.update();
    let knight = find(&w, b, "knight");
    assert_eq!(bonuses(&w, knight), (12_000, 5_000, 2));
    let priest = find(&w, a, "priest");
    order(&mut w, a, &[priest], &format!("use a_conversion {knight}"));
    update_until(&mut w, |w| w.e(knight).player() == Some(a));
    assert_eq!(bonuses(&w, knight), (12_000, 2_500, 2));
}

/// Two archers of `a` in a scout tower that is then upgraded to a guard
/// tower, with a mod's `extra` rules: the archers and the guard tower.
fn upgrade_an_occupied_tower(extra: &str) -> (World, PlayerId, Vec<EntityId>, EntityId) {
    let (mut w, a, _) = two_humans(extra);
    w.queue_command(a, "cmd add_units a1 lumbermill scouttower 2 archer");
    w.update();
    let tower = find(&w, a, "scouttower");
    let archers = all(&w, a, "archer");
    order(&mut w, a, &archers, &format!("enter {tower}"));
    // A building with an order cannot be upgraded: the tower has loaded
    // both archers.
    update_until(&mut w, |w| {
        w.e(tower).inside().unwrap().len() == 2 && w.e(tower).orders().is_empty()
    });
    order(&mut w, a, &[tower], "upgrade_to guardtower");
    update_until(&mut w, |w| !all(w, a, "guardtower").is_empty());
    let guardtower = find(&w, a, "guardtower");
    (w, a, archers, guardtower)
}

#[test]
fn units_inside_a_building_upgraded_are_inside_the_new_building() {
    let (w, a, archers, guardtower) = upgrade_an_occupied_tower("");
    assert_eq!(w.e(guardtower).inside(), Some(&archers[..]));
    for &u in &archers {
        assert_eq!(w.e(u).player(), Some(a));
        assert_eq!(w.e(u).place, Some(Place::Inside(guardtower)));
    }
    assert_eq!(all(&w, a, "archer"), archers);
}

#[test]
fn units_that_no_longer_fit_in_a_building_upgraded_are_beside_it() {
    // A mod whose guard tower holds one archer.
    let (w, a, archers, guardtower) =
        upgrade_an_occupied_tower("def guardtower\ntransport_capacity 1\n");
    assert_eq!(w.e(guardtower).inside(), Some(&archers[..1]));
    assert_eq!(w.e(archers[1]).player(), Some(a));
    assert_eq!(w.e(archers[1]).place, w.e(guardtower).place);
}

#[test]
fn load_all_never_loads_the_transport_into_itself() {
    // A mod whose flying machine fits in a transport.
    let (mut w, a, _) = two_humans("def new_flyingmachine\ntransport_volume 1\n");
    w.queue_command(a, "cmd add_units a1 new_flyingmachine footman");
    w.update();
    let transport = find(&w, a, "new_flyingmachine");
    let footman = find(&w, a, "footman");
    let a1 = w.square_by_name("a1").unwrap();
    let here = format!("load_all {}", w.square(a1).id);
    order(&mut w, a, &[transport], &here);
    update_until(&mut w, |w| w.e(transport).orders().is_empty());
    assert_eq!(w.e(transport).place, Some(Place::Square(a1)));
    assert_eq!(w.e(transport).inside(), Some(&[footman][..]));
}

#[test]
fn units_upgraded_take_the_place_of_the_old_ones_where_the_player_named_them() {
    let (mut w, a, _) = two_humans("");
    w.queue_command(a, "cmd add_units a1 lumbermill magestower 2 archer");
    w.update();
    let archers = all(&w, a, "archer");
    order(&mut w, a, &archers, "reset_group 6");
    order(&mut w, a, &archers, "join_group 6");
    order(&mut w, a, &archers, "upgrade_to darkarcher");
    w.drain_events(a);
    update_until(&mut w, |w| all(w, a, "darkarcher").len() == 2);
    let dark = all(&w, a, "darkarcher");
    let replaced: Vec<crate::Event> = w
        .drain_events(a)
        .into_iter()
        .filter(|e| matches!(e, crate::Event::Replaced { .. }))
        .collect();
    // The second archer is upgraded first.
    let (first, second) = (dark[1], dark[0]);
    let told = |old, new| crate::Event::Replaced { old, new };
    assert_eq!(
        replaced,
        [told(archers[1], second), told(archers[0], first)]
    );
    let groups = w.turn_message(a, Vec::new()).state.groups;
    assert_eq!(groups, [("6".to_owned(), vec![first, second])]);
    assert_eq!(w.pl(a).group, Some(vec![first, second]));
    assert_eq!(w.c(first).unwrap().group.as_deref(), Some("6"));
}

#[test]
fn a_building_upgraded_keeps_its_rallying_point() {
    let (mut w, a, _) = two_humans("");
    w.queue_command(
        a,
        "cmd add_units a1 barracks lumbermill blacksmith stables keep",
    );
    w.update();
    let keep = find(&w, a, "keep");
    let b1 = w.square_by_name("b1").unwrap();
    let b1_id = w.square(b1).id.to_string();
    order(&mut w, a, &[keep], &format!("rallying_point {b1_id}"));
    order(&mut w, a, &[keep], "upgrade_to castle");
    // The upgrade takes 360 seconds: 1200 turns.
    for _ in 0..1300 {
        w.update();
    }
    let castle = find(&w, a, "castle");
    assert_eq!(
        w.c(castle).unwrap().rallying_point,
        Some(TargetRef::Square(b1))
    );
    // A peasant trained in the castle goes there.
    order(&mut w, a, &[castle], "train peasant");
    update_until(&mut w, |w| !all(w, a, "peasant").is_empty());
    let peasant = find(&w, a, "peasant");
    assert_eq!(orders(&w, peasant).first(), Some(&words(&["go", &b1_id])));
}

#[test]
fn a_converted_unit_has_no_more_hit_points_or_mana_than_its_new_maximum() {
    // A mod whose upgrades raise a knight's hit points and a priest's mana.
    let extra = "def toughness\nclass upgrade\neffect bonus hp_max 10\n\
                 def wisdom\nclass upgrade\neffect bonus mana_max 50\n\
                 def knight\ncan_use toughness\ndef priest\ncan_use a_conversion wisdom\n";
    let (mut w, a, b) = two_humans(extra);
    w.queue_command(a, "cmd add_units a1 u_conversion priest");
    w.queue_command(b, "cmd add_units a1 toughness wisdom knight priest");
    w.update();
    let knight = find(&w, b, "knight");
    let enemy_priest = find(&w, b, "priest");
    for u in [knight, enemy_priest] {
        let e = w.em(u);
        let (hp_max, mana_max) = (e.stats.hp_max, e.stats.mana_max);
        let c = e.creature_mut().unwrap();
        c.hp = hp_max;
        c.mana = mana_max;
    }
    let vitals = |w: &World, u| {
        let e = w.e(u);
        let c = e.creature().unwrap();
        (c.hp, e.stats.hp_max, c.mana, e.stats.mana_max)
    };
    assert_eq!(vitals(&w, knight), (55_000, 55_000, 0, 0));
    assert_eq!(vitals(&w, enemy_priest), (30_000, 30_000, 250_000, 250_000));
    let priest = find(&w, a, "priest");
    for target in [knight, enemy_priest] {
        w.cm(priest).unwrap().mana = w.e(priest).stats.mana_max;
        order(&mut w, a, &[priest], &format!("use a_conversion {target}"));
        update_until(&mut w, |w| w.e(target).player() == Some(a));
    }
    assert_eq!(vitals(&w, knight), (45_000, 45_000, 0, 0));
    assert_eq!(vitals(&w, enemy_priest), (30_000, 30_000, 200_000, 200_000));
}

#[test]
fn a_deposit_listed_as_a_product_is_never_trained_built_or_upgraded_to() {
    let extra = "def townhall\ncan_train peasant goldmine\ncan_upgrade_to keep goldmine\n\
                 def peasant\ncan_build farm goldmine\n";
    let (mut w, a, _) = two_humans(extra);
    w.queue_command(a, "cmd add_units a1 townhall barracks peasant");
    w.update();
    let townhall = find(&w, a, "townhall");
    let peasant = find(&w, a, "peasant");
    let allowed = |w: &World, u, kw, t| w.is_order_allowed(u, kw, Some(t));
    assert!(allowed(&w, townhall, Keyword::Train, "peasant"));
    assert!(!allowed(&w, townhall, Keyword::Train, "goldmine"));
    assert!(allowed(&w, townhall, Keyword::UpgradeTo, "keep"));
    assert!(!allowed(&w, townhall, Keyword::UpgradeTo, "goldmine"));
    assert!(allowed(&w, peasant, Keyword::Build, "farm"));
    assert!(!allowed(&w, peasant, Keyword::Build, "goldmine"));
    let resources = w.pl(a).resources.clone();
    order(&mut w, a, &[townhall], "train goldmine");
    w.update();
    assert!(w.e(townhall).orders().is_empty());
    assert_eq!(w.pl(a).resources, resources);
}

/// The events of the entities `p` perceives, since the last call.
fn entity_events(w: &mut World, p: PlayerId) -> Vec<EntityEvent> {
    w.drain_events(p)
        .into_iter()
        .filter_map(|e| match e {
            crate::Event::Entity { event, .. } => Some(event),
            _ => None,
        })
        .collect()
}

#[test]
fn a_recall_with_no_room_for_its_units_is_impossible_says_why_once_and_costs_nothing() {
    let (mut w, a, _) = two_humans("");
    // a1 is full: 143 footmen and the mage are as many ground units as a
    // square holds.
    w.queue_command(a, "cmd add_units a1 u_recall mage 143 footman");
    w.queue_command(a, "cmd add_units b1 footman");
    w.update();
    let mage = find(&w, a, "mage");
    w.cm(mage).unwrap().mana = 200_000;
    let b1 = w.square_by_name("b1").unwrap();
    let recall = format!("use a_recall {}", w.square(b1).id);
    order(&mut w, a, &[mage], &recall);
    entity_events(&mut w, a);
    update_until(&mut w, |w| {
        w.e(mage).orders().first().is_some_and(Order::is_final)
    });
    let o = &w.e(mage).orders()[0];
    assert_eq!(o.progress(), Progress::Impossible);
    assert_eq!(w.c(mage).unwrap().distance_to_goal, crate::INF);
    assert_eq!(w.c(mage).unwrap().mana, 200_000);
    w.update();
    assert!(w.e(mage).orders().is_empty());
    let events = entity_events(&mut w, a);
    let refused: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                EntityEvent::OrderImpossible(_) | EntityEvent::UseComplete(_)
            )
        })
        .collect();
    assert_eq!(
        refused,
        [&EntityEvent::OrderImpossible(Some(Reason::NotEnoughSpace))]
    );
}

#[test]
fn an_upgrade_that_finds_no_meadow_is_refunded_and_says_why() {
    // A mod whose guard tower needs a meadow, on a map that has none.
    let (mut w, a, _) = two_humans("def guardtower\nis_buildable_anywhere 0\n");
    w.queue_command(a, "cmd add_units a1 lumbermill scouttower");
    w.update();
    let tower = find(&w, a, "scouttower");
    let before = (w.pl(a).resources.clone(), w.pl(a).used_food);
    order(&mut w, a, &[tower], "upgrade_to guardtower");
    w.update();
    // Paid: 11 - 6 gold and 4 - 2 wood.
    let paid = vec![
        995 * crate::nofloat::PRECISION,
        998 * crate::nofloat::PRECISION,
    ];
    assert_eq!(w.pl(a).resources, paid);
    entity_events(&mut w, a);
    update_until(&mut w, |w| w.e(tower).orders().is_empty());
    let refused: Vec<_> = entity_events(&mut w, a)
        .into_iter()
        .filter(|e| matches!(e, EntityEvent::OrderImpossible(_)))
        .collect();
    assert_eq!(
        refused,
        [EntityEvent::OrderImpossible(Some(Reason::CannotBuildHere))]
    );
    assert_eq!((w.pl(a).resources.clone(), w.pl(a).used_food), before);
    assert_eq!(all(&w, a, "scouttower"), [tower]);
    assert!(all(&w, a, "guardtower").is_empty());
}

#[test]
fn a_ship_trained_with_no_water_near_is_refunded_and_impossible() {
    let (mut w, a, _) = two_humans("");
    w.queue_command(a, "cmd add_units a1 townhall lumbermill farm shipyard");
    w.update();
    let shipyard = find(&w, a, "shipyard");
    let before = (w.pl(a).resources.clone(), w.pl(a).used_food);
    order(&mut w, a, &[shipyard], "train boat");
    entity_events(&mut w, a);
    update_until(&mut w, |w| {
        w.e(shipyard).orders().first().is_some_and(Order::is_final)
    });
    assert_eq!(w.e(shipyard).orders()[0].progress(), Progress::Impossible);
    assert_eq!((w.pl(a).resources.clone(), w.pl(a).used_food), before);
    let refused: Vec<_> = entity_events(&mut w, a)
        .into_iter()
        .filter(|e| matches!(e, EntityEvent::OrderImpossible(_)))
        .collect();
    assert_eq!(
        refused,
        [EntityEvent::OrderImpossible(Some(Reason::NotEnoughSpace))]
    );
    assert!(all(&w, a, "boat").is_empty());
}

/// What a unit says in the turn after an order, and its orders then.
fn after_order(
    w: &mut World,
    p: PlayerId,
    u: EntityId,
    words: &str,
) -> (Vec<EntityEvent>, Vec<Keyword>) {
    entity_events(w, p);
    w.queue(p, crate::Command::Control(vec![u]));
    w.queue_command(p, &format!("order 0 1 {words}"));
    w.update();
    let events = entity_events(w, p)
        .into_iter()
        .filter(|e| matches!(e, EntityEvent::OrderImpossible(_) | EntityEvent::OrderOk))
        .collect();
    (events, w.e(u).orders().iter().map(Order::keyword).collect())
}

#[test]
fn an_order_without_its_target_is_impossible_at_once_and_one_aimed_off_its_square_aims_at_nothing()
{
    let (mut w, a, _) = two_humans("");
    w.queue_command(a, "cmd add_units a1 footman");
    w.update();
    let footman = find(&w, a, "footman");
    let a1 = w.square(w.square_by_name("a1").unwrap());
    let (id, corner) = (a1.id, (a1.xmin + 500, a1.ymin + 500));
    let near = format!("go zoom-{id}-{}-{}", corner.0, corner.1);
    let (events, kept) = after_order(&mut w, a, footman, &near);
    assert_eq!(
        (events, kept.clone()),
        (vec![EntityEvent::OrderOk], vec![Keyword::Go])
    );
    let impossible = vec![EntityEvent::OrderImpossible(None)];
    // Refused before anything changes: the order in the queue stays.
    for words in ["go", "attack", "patrol", "join_group"] {
        let refused = after_order(&mut w, a, footman, words);
        assert_eq!(refused, (impossible.clone(), kept.clone()), "{words}");
    }
    // A point off its square aims at nothing, as a target that died: the
    // order replaces the queue and is impossible.
    let far = format!("go zoom-{id}-{}-0", i64::MAX);
    assert_eq!(after_order(&mut w, a, footman, &far), (impossible, vec![]));
    assert_eq!(w.drain_warnings(), Vec::<String>::new());
}

#[test]
fn entering_a_square_or_a_deposit_is_impossible() {
    let (mut w, a, _) = two_humans("");
    w.queue_command(a, "cmd add_units a1 footman");
    w.update();
    let footman = find(&w, a, "footman");
    let a1 = w.square_by_name("a1").unwrap();
    let gold = w.rules.type_id("goldmine").unwrap();
    let mine = w.create_deposit(gold, a1, 75_000).unwrap();
    w.update();
    let impossible = (vec![EntityEvent::OrderImpossible(None)], vec![]);
    let square = format!("enter {}", w.square(a1).id);
    assert_eq!(after_order(&mut w, a, footman, &square), impossible);
    assert_eq!(
        after_order(&mut w, a, footman, &format!("enter {mine}")),
        impossible
    );
    assert_eq!(w.drain_warnings(), Vec::<String>::new());
}

#[test]
fn a_unit_killed_by_or_dying_without_an_owner_is_removed() {
    let (mut w, a, _) = two_humans("");
    w.queue_command(a, "cmd add_units a1 footman");
    w.update();
    let footman = find(&w, a, "footman");
    let a1 = w.square_by_name("a1").unwrap();
    let (x, y) = (w.square(a1).x, w.square(a1).y);
    let t = w.rules.type_id("footman").unwrap();
    let ownerless = w.create_creature(t, None, Place::Square(a1), x, y).unwrap();
    w.cm(footman).unwrap().hp = 1;
    w.hit(ownerless, footman);
    assert_eq!(w.e(footman).place, None);
    assert!(!w.pl(a).units.contains(&footman));
    // Its time is up: it dies at the next slow update.
    w.cm(ownerless).unwrap().time_limit = Some(w.time);
    for _ in 0..4 {
        w.update();
    }
    assert_eq!(w.e(ownerless).place, None);
    assert_eq!(w.drain_warnings(), Vec::<String>::new());
}

#[test]
fn a_way_that_avoids_danger_finds_no_path_to_a_very_dangerous_square() {
    let (w, me, _) = game();
    let mut w = w;
    let s = home(&w, me);
    let (d, plain) = (0..w.squares.len())
        .map(|i| SquareId(i as u32))
        .filter(|&d| d != s)
        .find_map(|d| {
            let plain = w
                .shortest_path_distance_to(s, d, Some(me), AirGround::Ground, false)
                .unwrap();
            (plain < crate::INF).then_some((d, plain))
        })
        .unwrap();
    assert_eq!(
        w.shortest_path_distance_to(s, d, Some(me), AirGround::Ground, true),
        Ok(plain)
    );
    w.plm(me).enemy_menace.insert(d, 1);
    w.plm(me).enemy_presence.push(d);
    // A new turn: the paths found this turn are kept.
    w.time += crate::VIRTUAL_TIME_INTERVAL;
    assert_eq!(
        w.shortest_path_distance_to(s, d, Some(me), AirGround::Ground, true),
        Ok(crate::INF)
    );
    assert_eq!(
        w.shortest_path_to(s, d, Some(me), AirGround::Ground, true),
        Ok(None)
    );
}
