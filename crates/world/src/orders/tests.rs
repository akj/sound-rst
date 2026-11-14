use super::*;
use crate::entity::ZoomTarget;
use crate::player::Controller;
use crate::{AiScripts, Rules};

/// jl1 with a human, `me`, against an easy computer.
fn game() -> (World, PlayerId, PlayerId) {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    let read = |f: &str| {
        let bytes = std::fs::read(dir.join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
        String::from_utf8_lossy(&bytes).into_owned()
    };
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
    w.e(u)
        .orders
        .iter()
        .map(|o| {
            let mut words = vec![o.keyword.as_str().to_owned()];
            words.extend(o.type_name.iter().cloned());
            words.extend(o.args.iter().cloned());
            words
        })
        .collect()
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
    let refused = [
        ("zoom", "not enough values to unpack"),
        ("zoom-a1-1", "not enough values to unpack"),
        (&format!("zoom-{id}--1-0"), "not enough values to unpack"),
        ("zoom-999999-1-2", "object has no attribute 'title'"),
        ("zoom-a1-x-2", "invalid literal for int()"),
    ];
    for (word, error) in refused {
        assert_eq!(parse(word), Err(Abort::new(error)), "{word:?}");
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
    let ok = |t: Option<Target>| format!("{:?}", Ok::<_, Abort>(t));
    let zoom = |w: &World, (x, y)| {
        Target::Zoom(ZoomTarget {
            place: s,
            x,
            y,
            id: Some(format!("zoom-{}-{x}-{y}", w.square(s).id)),
        })
    };
    let square = ok(Some(Target::Square(a1)));
    assert_eq!(resolve(&w, TargetRef::Square(a1)), square);
    assert_eq!(resolve(&w, TargetRef::SquareNamed(a1)), square);
    let (x, y) = inside;
    let point = TargetRef::Point { square: s, x, y };
    assert_eq!(resolve(&w, point), ok(Some(zoom(&w, inside))));
    let (x, y) = outside;
    let point = TargetRef::Point { square: s, x, y };
    assert_eq!(
        w.resolve_target(me, &point).unwrap_err(),
        Abort::new("the zoom is outside its square")
    );
    assert_eq!(
        resolve(&w, TargetRef::Entity(peasant)),
        ok(Some(Target::Entity(peasant)))
    );
    match w.resolve_target(me, &TargetRef::Entity(seen)).unwrap() {
        Some(Target::Memory(m)) => assert_eq!((m.owner, m.snapshot.id), (me, seen)),
        other => panic!("{other:?}"),
    }
    assert_eq!(resolve(&w, TargetRef::Entity(far_enemy)), ok(None));
    assert_eq!(resolve(&w, TargetRef::Entity(EntityId(999_999))), ok(None));
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
        let by_text = format!("{:?}", w.get_object_by_id(me, &t.text(&w)));
        assert_eq!(by_text, resolve(&w, t), "{t:?}");
    }
    let by_text = w.get_object_by_id(me, &seen.to_string()).unwrap().unwrap();
    let typed = w.resolve_target(me, &TargetRef::Entity(seen)).unwrap();
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
            Request::build_phase_two(e),
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
        let stored = r.stored(1, &w);
        assert_eq!(stored.keyword.as_str(), expected[0]);
        let type_name = r.keyword().is_complex().then(|| expected[1].clone());
        assert_eq!(stored.type_name, type_name);
        let args = &expected[1 + usize::from(type_name.is_some())..];
        assert_eq!(stored.args, args);
    }
    for &k in Keyword::ALL {
        assert_eq!(Request::plain(k).is_some(), !k.is_complex(), "{k:?}");
        assert_eq!(Request::typed(k, "x").is_some(), k.is_complex(), "{k:?}");
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
    assert_eq!(w.e(townhall).rallying_point, Some(b2_id.clone()));
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
    w.em(peasant).auto_repair = Some(true);
    w.em(townhall).hp -= 10;
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
    assert!(w.resolve_target(me, &t).unwrap().is_none());
}
