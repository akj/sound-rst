//! What a seat knows, question by question, and the proof that it knows
//! all the world tells it ([`proof`]).

use super::*;
use crate::player::{Controller, Remembrance};
use crate::setup::GameSetup;
use crate::world::{World, multiplayer_default_triggers};

mod proof;

pub(crate) fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/soundrts-upstream")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A game on jl1 between the local human and another human, who gives no
/// order unless a test does, after a few turns. Returns the world, the
/// local human and the other.
pub(crate) fn game() -> (World, PlayerId, PlayerId) {
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    let mut them = Controller::human("them", false);
    them.alliance = Some("2".into());
    let setup = GameSetup {
        rules: read("res/rules.txt"),
        ai: read("res/ai.txt"),
        map: read("res/multi/jl1.txt"),
        seed: 3,
        controllers: vec![me, them],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: true,
        random_starts: true,
        equivalents: true,
    };
    let (mut w, seats) = setup.build_world().unwrap();
    for _ in 0..5 {
        w.update();
    }
    (w, seats[0], seats[1])
}

/// The first unit of `p` of this type.
pub(crate) fn unit(w: &World, p: PlayerId, type_name: &str) -> EntityId {
    w.player(p)
        .units
        .iter()
        .copied()
        .find(|&u| &*w.entity(u).unwrap().type_name == type_name)
        .unwrap_or_else(|| panic!("no {type_name}"))
}

fn square_of(w: &World, id: EntityId) -> SquareId {
    w.entity(id).unwrap().square().unwrap()
}

/// Makes `p` remember an entity as it is now.
pub(crate) fn remember(w: &mut World, p: PlayerId, id: EntityId) {
    let r = Remembrance {
        time_stamp: w.time,
        snapshot: Arc::new(w.entity(id).unwrap().clone()),
    };
    w.player_mut(p).memory.insert(id, r);
}

/// What `p` is told of the world as it is now, with the terrain and the
/// upgrades it sees noted.
pub(crate) fn seat(w: &World, p: PlayerId) -> SeatView {
    let mut s = SeatView::new(Arc::new(GameStart::of(w)), w.turn_message(p, Vec::new()));
    s.remember();
    s
}

/// Runs the commands of `p` in the next turn.
pub(crate) fn command(w: &mut World, p: PlayerId, lines: &[String]) {
    for l in lines {
        w.queue_command(p, l);
    }
    w.update();
}

#[test]
fn a_seat_knows_no_entity_it_neither_perceives_nor_remembers() {
    let (mut w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    let peasant = unit(&w, me, "peasant");
    assert!(!w.player(me).perception.contains(&townhall));
    let v = seat(&w, me);
    assert!(v.perceived(townhall).is_none());
    assert!(v.remembered(townhall).is_none());
    assert_eq!(v.perceived(peasant).map(|e| e.id), Some(peasant));
    assert!(v.remembered(peasant).is_none());

    remember(&mut w, me, townhall);
    let v = seat(&w, me);
    assert!(v.perceived(townhall).is_none());
    assert_eq!(
        v.remembered(townhall).map(|r| (r.id, r.last_seen)),
        Some((townhall, Some(1500)))
    );
}

#[test]
fn an_entity_is_known_as_perceived_else_as_remembered() {
    let (mut w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    let peasant = unit(&w, me, "peasant");
    // Neither perceived nor remembered: unknown, though it is there.
    assert!(w.entity(townhall).is_some());
    assert!(seat(&w, me).known(townhall).is_none());

    // Remembered: the copy as last seen, not the town hall now.
    remember(&mut w, me, townhall);
    let seen_hp = w.entity(townhall).unwrap().creature().unwrap().hp;
    w.entity_mut(townhall).unwrap().creature_mut().unwrap().hp = 1000;
    assert_ne!(seen_hp, 1000);
    let v = seat(&w, me);
    let e = v.known(townhall).unwrap();
    assert_eq!((e.id, e.hp, e.last_seen), (townhall, seen_hp, Some(1500)));

    // Perceived: as it is now.
    let e = v.known(peasant).unwrap();
    assert_eq!((e.id, e.last_seen), (peasant, None));
}

#[test]
fn an_exit_is_blocked_only_by_a_blocker_the_player_knows() {
    let (mut w, me, them) = game();
    let home = square_of(&w, unit(&w, me, "townhall"));
    let exit = w
        .square(home)
        .exits
        .iter()
        .copied()
        .find(|&x| {
            w.entity(x)
                .unwrap()
                .exit()
                .as_ref()
                .unwrap()
                .other_side
                .is_some()
        })
        .unwrap();
    let other_side = w
        .entity(exit)
        .unwrap()
        .exit()
        .as_ref()
        .unwrap()
        .other_side
        .unwrap();
    let blocker = unit(&w, them, "peasant");
    assert!(!seat(&w, me).exit_is_blocked(exit, true));
    assert!(!seat(&w, me).exit_is_blocked(exit, false));

    // A blocker on the other side that the player has never seen: the
    // exit is open for the player, though it is closed.
    w.entity_mut(other_side)
        .unwrap()
        .exit_mut()
        .unwrap()
        .blockers
        .push(blocker);
    let v = seat(&w, me);
    assert!(v.known(blocker).is_none());
    assert!(!v.exit_is_blocked(exit, true));
    assert!(!v.exit_is_blocked(exit, false));

    // Once the player remembers the blocker and the other side with it,
    // the exit is closed.
    remember(&mut w, me, other_side);
    remember(&mut w, me, blocker);
    let v = seat(&w, me);
    assert!(v.exit_is_blocked(exit, true));
    assert!(v.exit_is_blocked(exit, false));
    // The exit's place on the other side is part of the map.
    assert_eq!(
        v.other_side_of_exit(exit),
        w.entity(other_side).unwrap().place
    );
    assert_eq!(v.other_side_of_exit(blocker), None);
}

#[test]
fn a_friendly_gate_lets_the_player_through() {
    let (mut w, me, _) = game();
    let home = square_of(&w, unit(&w, me, "townhall"));
    let exit = w.square(home).exits[0];
    let gate = unit(&w, me, "farm");
    w.entity_mut(gate).unwrap().stats.is_a_gate = 1;
    w.entity_mut(exit)
        .unwrap()
        .exit_mut()
        .unwrap()
        .blockers
        .push(gate);
    let v = seat(&w, me);
    assert!(v.perceived(exit).is_some());
    assert!(!v.exit_is_blocked(exit, true));
    assert!(v.exit_is_blocked(exit, false));
}

#[test]
fn the_comrades_of_a_unit_are_its_owners_units_the_player_sees_beside_it() {
    let (w, me, them) = game();
    let v = seat(&w, me);
    // The computer's peasant has a town hall and a farm beside it, unseen.
    let peasant = unit(&w, them, "peasant");
    let place = w.entity(peasant).unwrap().place;
    let beside = w
        .player(them)
        .units
        .iter()
        .filter_map(|&u| w.entity(u))
        .filter(|u| u.place == place && u.id != peasant)
        .count();
    assert_eq!(beside, 2);
    assert_eq!(
        v.comrades(them, place, peasant),
        Comrades {
            units: 0,
            soldiers: 0
        }
    );
    // The player's own peasant: its town hall and farm, seen.
    let mine = unit(&w, me, "peasant");
    let place = w.entity(mine).unwrap().place;
    assert_eq!(
        v.comrades(me, place, mine),
        Comrades {
            units: 2,
            soldiers: 0
        }
    );
}

#[test]
fn only_a_perceived_unit_has_upgrades() {
    let (mut w, me, them) = game();
    for p in [me, them] {
        let square = w
            .square(square_of(&w, unit(&w, p, "townhall")))
            .name
            .clone();
        command(&mut w, p, &[format!("cmd add_units {square} footman")]);
        let melee = w.rules.type_id("melee_weapon").unwrap();
        w.player_mut(p).upgrades.add(melee);
    }
    let theirs = unit(&w, them, "footman");
    let mine = unit(&w, me, "footman");
    remember(&mut w, me, theirs);
    let v = seat(&w, me);
    // The computer's footman, remembered: its owner's upgrades now are
    // not the player's to know (the interface keeps those it saw).
    assert!(v.perceived(theirs).is_none());
    assert_eq!(w.unit_upgrades(theirs), ["melee_weapon"]);
    assert_eq!(v.unit_upgrades(theirs), Vec::<String>::new());
    // The player's own footman.
    assert_eq!(v.unit_upgrades(mine), ["melee_weapon"]);
}

#[test]
fn a_remembered_unit_is_busy_with_what_it_did_when_last_seen() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    let farm = unit(&w, them, "farm");
    // Last seen idle; repairing now.
    remember(&mut w, me, peasant);
    w.entity_mut(farm).unwrap().creature_mut().unwrap().hp = 1000;
    command(
        &mut w,
        them,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 repair {}", farm.0),
        ],
    );
    for _ in 0..30 {
        w.update();
    }
    let v = seat(&w, me);
    assert!(v.perceived(peasant).is_none());
    assert_eq!(w.activity(peasant).as_deref(), Some("building"));
    assert_eq!(v.activity(peasant), None);
    assert_eq!(v.remembered_activity(peasant), None);

    // Last seen repairing.
    remember(&mut w, me, peasant);
    let v = seat(&w, me);
    assert_eq!(v.remembered_activity(peasant).as_deref(), Some("building"));
}

#[test]
fn a_remembered_unit_gathers_from_what_the_player_knows() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    // A gold mine the player knows nothing of. The computer is made to
    // see it, so that its order aims at the mine itself.
    let mine = *w
        .player(them)
        .memory
        .keys()
        .find(|&&o| &*w.entity(o).unwrap().type_name == "goldmine")
        .unwrap();
    assert!(seat(&w, me).known(mine).is_none());
    w.player_mut(them).perception.insert(mine);
    command(
        &mut w,
        them,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 gather {}", mine.0),
        ],
    );
    for _ in 0..1000 {
        if w.activity(peasant).is_some() {
            break;
        }
        w.update();
    }
    assert_eq!(w.activity(peasant).as_deref(), Some("exploiting_goldmine"));
    // Last seen gathering from a mine the player knows nothing of: no
    // activity; once the player remembers the mine, it is named.
    remember(&mut w, me, peasant);
    assert_eq!(seat(&w, me).remembered_activity(peasant), None);
    remember(&mut w, me, mine);
    assert_eq!(
        seat(&w, me).remembered_activity(peasant).as_deref(),
        Some("exploiting_goldmine")
    );
}

#[test]
fn a_recallable_group_holds_only_the_players_units() {
    let (mut w, me, them) = game();
    let theirs = unit(&w, them, "peasant");
    let mine = unit(&w, me, "peasant");
    for (p, u) in [(them, theirs), (me, mine)] {
        command(
            &mut w,
            p,
            &[format!("control {}", u.0), "order 0 0 join_group 1".into()],
        );
    }
    // The game's group 1 holds both peasants; the player recalls its own.
    assert_eq!(w.group_list("1").unwrap(), &[theirs, mine]);
    let v = seat(&w, me);
    assert_eq!(v.recallable_group("1"), [mine]);
    assert_eq!(v.recallable_group("2"), []);
    assert_eq!(seat(&w, them).recallable_group("1"), [theirs]);
}

#[test]
fn the_order_menus_are_the_worlds() {
    let (w, me, _) = game();
    let v = seat(&w, me);
    for u in w.player(me).units.clone() {
        for &k in Keyword::ALL {
            for strict in [true, false] {
                assert_eq!(v.order_menu(u, k, strict), w.order_menu(u, k, strict));
            }
        }
    }
    let peasant = unit(&w, me, "peasant");
    let farm = w.rules.type_id("farm");
    assert_eq!(
        v.order_menu(peasant, Keyword::Go, true),
        [MenuEntry::plain(Keyword::Go).unwrap()]
    );
    let build = v.order_menu(peasant, Keyword::Build, false);
    let build_farm = build.iter().find(|e| e.type_id == farm).unwrap();
    assert_eq!(build_farm.text(&w.rules), "build farm");
    // A farm requires a town hall.
    assert_eq!(build_farm.requirements, ["townhall"]);
    assert_eq!(v.order_menu(EntityId(99_999), Keyword::Go, true), []);
    let info = v.order_info(peasant, Keyword::Build, farm).unwrap();
    let old = w.order_info(peasant, Keyword::Build, farm).unwrap();
    assert_eq!(
        (info.cost, info.food_cost, info.nb_args),
        (old.cost, old.food_cost, old.nb_args)
    );
}

#[test]
fn the_default_order_is_the_worlds() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, me, "peasant");
    let farm = unit(&w, me, "farm");
    w.entity_mut(farm).unwrap().creature_mut().unwrap().hp = 1000;
    let townhall = unit(&w, them, "townhall");
    let b2 = w.square_by_name("b2").unwrap();
    let v = seat(&w, me);
    let (x, y) = v.square(b2).center();
    let cases = [
        (OrderTarget::Square(b2), Some(Keyword::Go)),
        (OrderTarget::Point { square: b2, x, y }, Some(Keyword::Go)),
        (OrderTarget::Entity(farm), Some(Keyword::Repair)),
        // Not perceived nor remembered: the target is unknown.
        (OrderTarget::Entity(townhall), None),
    ];
    for (target, expected) in cases {
        assert_eq!(v.default_order(peasant, target), expected);
    }
}

#[test]
fn a_requirement_is_known_only_for_a_controlled_owner() {
    let (w, me, them) = game();
    let mine = unit(&w, me, "peasant");
    let theirs = unit(&w, them, "peasant");
    let v = seat(&w, me);
    assert_eq!(v.owner_has(mine, "farm"), Some(true));
    assert_eq!(v.owner_has(mine, "barracks"), Some(false));
    // The other player has a farm: not the player's to know.
    assert!(w.has(them, "farm"));
    assert_eq!(v.owner_has(theirs, "farm"), None);
}

#[test]
fn the_squares_glimpsed_from_the_next_square_are_known_each_turn() {
    let (mut w, me, _) = game();
    let v = seat(&w, me);
    let pl = w.player(me);
    // Seen from the next square: not observed, but observed before.
    let partly = v.partly_observed_squares();
    assert!(!partly.is_empty());
    assert!(partly.is_disjoint(&pl.observed_squares));
    assert!(partly.is_subset(&pl.observed_before_squares));
    // In cheat mode every square is observed, none partly.
    w.player_mut(me).cheatmode = true;
    w.update();
    assert!(seat(&w, me).partly_observed_squares().is_empty());
}

#[test]
fn a_production_tells_its_time_left_to_the_player_who_controls_it() {
    let (mut w, me, them) = game();
    for p in [me, them] {
        let townhall = unit(&w, p, "townhall");
        command(
            &mut w,
            p,
            &[
                format!("control {}", townhall.0),
                "order 0 0 train peasant".into(),
            ],
        );
    }
    for _ in 0..10 {
        w.update();
    }
    let v = seat(&w, me);
    let first = |id| {
        let own = v.known(id).and_then(|e| e.own)?;
        own.orders.first().map(|o| o.time_left)
    };
    // A peasant takes 10 seconds: 11 turns of 300 milliseconds have gone.
    assert_eq!(first(unit(&w, me, "townhall")), Some(Some(6700)));
    assert_eq!(first(unit(&w, them, "townhall")), None);
}

#[test]
fn a_defeated_player_is_known_to_every_player() {
    let (mut w, me, them) = game();
    assert!(seat(&w, me).defeated().is_empty());
    w.defeat(them, true);
    let v = seat(&w, me);
    assert_eq!(v.defeated(), [them]);
    assert_eq!(v.players(), [me]);
}

#[test]
fn a_seat_takes_note_of_what_it_sees_only_when_told_to() {
    let (mut w, me, _) = game();
    let home = square_of(&w, unit(&w, me, "townhall"));
    let square = w.square(home).name.clone();
    command(&mut w, me, &[format!("cmd add_units {square} footman")]);
    let footman = unit(&w, me, "footman");
    let mut s = SeatView::new(Arc::new(GameStart::of(&w)), w.turn_message(me, Vec::new()));
    // A seat that has just joined remembers nothing.
    assert_eq!(s.terrain(home), "");
    s.remember();
    assert_eq!(s.terrain(home), "_meadows");

    // The square's terrain and the footman's upgrades change. A new
    // message tells them; what the seat remembers of them changes only
    // once it takes note.
    let melee = w.rules.type_id("melee_weapon").unwrap();
    w.player_mut(me).upgrades.add(melee);
    w.squares[home.0 as usize].type_name = "_forest".into();
    s.install(w.turn_message(me, Vec::new()));
    assert_eq!(s.terrain(home), "_meadows");
    assert_eq!(s.unit_upgrades(footman), ["melee_weapon"]);
    assert_eq!(s.remembered_upgrades(footman), Vec::<String>::new());
    s.remember();
    assert_eq!(s.terrain(home), "_forest");
    assert_eq!(s.remembered_upgrades(footman), ["melee_weapon"]);

    // Forgotten once the seat neither perceives nor remembers it.
    w.player_mut(me).perception.remove(&footman);
    s.install(w.turn_message(me, Vec::new()));
    assert_eq!(s.remembered_upgrades(footman), ["melee_weapon"]);
    s.remember();
    assert_eq!(s.remembered_upgrades(footman), Vec::<String>::new());
}
