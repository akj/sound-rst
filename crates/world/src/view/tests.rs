use std::sync::Arc;

use super::*;
use crate::player::Controller;
use crate::setup::GameSetup;
use crate::world::multiplayer_default_triggers;

fn read(relative: &str) -> String {
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
fn game() -> (World, PlayerId, PlayerId) {
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
fn unit(w: &World, p: PlayerId, type_name: &str) -> EntityId {
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
fn remember(w: &mut World, p: PlayerId, id: EntityId) {
    let r = Remembrance {
        time_stamp: w.time,
        snapshot: Arc::new(w.entity(id).unwrap().clone()),
    };
    w.player_mut(p).memory.insert(id, r);
}

/// Runs the commands of `p` in the next turn.
fn command(w: &mut World, p: PlayerId, lines: &[String]) {
    for l in lines {
        w.queue_command(p, l);
    }
    w.update();
}

#[test]
fn the_view_returns_no_entity_the_player_neither_perceives_nor_remembers() {
    let (mut w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    let peasant = unit(&w, me, "peasant");
    assert!(!w.player(me).perception.contains(&townhall));
    let v = View::new(&w, me);
    assert!(v.perceived(townhall).is_none());
    assert!(v.remembered(townhall).is_none());
    assert_eq!(v.perceived(peasant).map(|e| e.id), Some(peasant));
    assert!(v.remembered(peasant).is_none());

    remember(&mut w, me, townhall);
    let v = View::new(&w, me);
    assert!(v.perceived(townhall).is_none());
    assert_eq!(
        v.remembered(townhall)
            .map(|r| (r.snapshot.id, r.time_stamp)),
        Some((townhall, 1500))
    );
}

#[test]
fn an_entity_is_known_as_perceived_else_as_remembered() {
    let (mut w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    let peasant = unit(&w, me, "peasant");
    // Neither perceived nor remembered: unknown, though it is there.
    assert!(w.entity(townhall).is_some());
    assert!(View::new(&w, me).known(townhall).is_none());

    // Remembered: the copy as last seen, not the town hall now.
    remember(&mut w, me, townhall);
    let seen_hp = w.entity(townhall).unwrap().hp;
    w.entity_mut(townhall).unwrap().hp = 1000;
    assert_ne!(seen_hp, 1000);
    let v = View::new(&w, me);
    let (e, ts) = v.known(townhall).unwrap();
    assert_eq!((e.id, e.hp, ts), (townhall, seen_hp, Some(1500)));

    // Perceived: as it is now.
    let (e, ts) = v.known(peasant).unwrap();
    assert_eq!((e.id, ts), (peasant, None));
}

#[test]
fn an_allys_memory_is_read_from_the_ally() {
    let (mut w, me, them) = game();
    let farm = unit(&w, me, "farm");
    remember(&mut w, them, farm);
    let v = View::new(&w, me);
    assert!(v.remembered(farm).is_none());
    let old = w.player(them).memory.get(&farm).map(|r| r.time_stamp);
    assert_eq!(old, Some(1500));
    assert_eq!(v.leaks().memory_of(them, farm).map(|r| r.time_stamp), old);
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
                .exit
                .as_ref()
                .unwrap()
                .other_side
                .is_some()
        })
        .unwrap();
    let other_side = w
        .entity(exit)
        .unwrap()
        .exit
        .as_ref()
        .unwrap()
        .other_side
        .unwrap();
    let blocker = unit(&w, them, "peasant");
    assert!(!View::new(&w, me).exit_is_blocked(exit, true));
    assert!(!View::new(&w, me).exit_is_blocked(exit, false));

    // A blocker on the other side that the player has never seen: the
    // exit is open for the player, though it is closed.
    w.entity_mut(other_side)
        .unwrap()
        .exit
        .as_mut()
        .unwrap()
        .blockers
        .push(blocker);
    let v = View::new(&w, me);
    assert!(v.known(blocker).is_none());
    assert!(!v.exit_is_blocked(exit, true));
    assert!(!v.exit_is_blocked(exit, false));

    // Once the player remembers the blocker and the other side with it,
    // the exit is closed.
    remember(&mut w, me, other_side);
    remember(&mut w, me, blocker);
    let v = View::new(&w, me);
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
        .exit
        .as_mut()
        .unwrap()
        .blockers
        .push(gate);
    let v = View::new(&w, me);
    assert!(v.perceived(exit).is_some());
    assert!(!v.exit_is_blocked(exit, true));
    assert!(v.exit_is_blocked(exit, false));
}

#[test]
fn the_comrades_of_a_unit_are_its_owners_units_the_player_sees_beside_it() {
    let (w, me, them) = game();
    let v = View::new(&w, me);
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
fn a_remembered_units_upgrades_are_its_owners_now() {
    let (mut w, me, them) = game();
    let square = w
        .square(square_of(&w, unit(&w, them, "townhall")))
        .name
        .clone();
    command(&mut w, them, &[format!("cmd add_units {square} footman")]);
    let footman = unit(&w, them, "footman");
    remember(&mut w, me, footman);
    w.player_mut(them).upgrades.push("melee_weapon".into());
    let v = View::new(&w, me);
    assert!(v.perceived(footman).is_none());
    assert_eq!(w.unit_upgrades(footman), ["melee_weapon"]);
    assert_eq!(
        v.leaks().remembered_unit_upgrades(footman),
        ["melee_weapon"]
    );
}

#[test]
fn a_remembered_units_activity_is_the_live_ones() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    let farm = unit(&w, them, "farm");
    remember(&mut w, me, peasant);
    w.entity_mut(farm).unwrap().hp = 1000;
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
    let v = View::new(&w, me);
    assert!(v.perceived(peasant).is_none());
    assert!(v.remembered(peasant).unwrap().snapshot.orders.is_empty());
    assert_eq!(w.activity(peasant).as_deref(), Some("building"));
    assert_eq!(
        v.leaks().live_activity(peasant).as_deref(),
        Some("building")
    );
}

#[test]
fn the_objects_on_a_square_are_all_there_seen_or_not() {
    let (w, me, them) = game();
    let there = square_of(&w, unit(&w, them, "townhall"));
    let v = View::new(&w, me);
    assert!(!w.player(me).observed_squares.contains(&there));
    let old: Vec<&str> = w
        .square(there)
        .objects
        .iter()
        .map(|&o| &*w.entity(o).unwrap().type_name)
        .collect();
    let leaked: Vec<&str> = v
        .leaks()
        .objects_on(there)
        .iter()
        .map(|e| &*e.type_name)
        .collect();
    assert_eq!(leaked, old);
    for t in ["townhall", "farm", "peasant", "meadow"] {
        assert!(leaked.contains(&t), "{t} in {leaked:?}");
    }
    assert!(
        v.leaks()
            .objects_on(there)
            .iter()
            .all(|e| v.perceived(e.id).is_none())
    );
}

#[test]
fn a_container_is_read_live() {
    let (w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    let v = View::new(&w, me);
    assert!(v.perceived(townhall).is_none());
    assert_eq!(
        v.leaks().container_place(townhall),
        w.entity(townhall).unwrap().place
    );
    assert!(v.leaks().container_place(townhall).is_some());
    assert_eq!(v.leaks().container_type(townhall), Some("townhall"));
}

#[test]
fn the_terrain_in_the_fog_is_the_terrain_now() {
    let (w, me, _) = game();
    let v = View::new(&w, me);
    let fog: Vec<SquareId> = (0..v.square_count() as u32)
        .map(SquareId)
        .filter(|s| !w.player(me).observed_squares.contains(s))
        .filter(|&s| w.square(s).type_name == "_forest")
        .collect();
    assert!(!fog.is_empty());
    for s in fog {
        assert_eq!(v.leaks().unobserved_terrain(s), "_forest");
        assert_eq!(v.terrain(s), "_forest");
    }
}

#[test]
fn a_recallable_group_holds_every_players_units() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    command(
        &mut w,
        them,
        &[
            format!("control {}", peasant.0),
            "order 0 0 join_group 1".into(),
        ],
    );
    let v = View::new(&w, me);
    assert!(v.perceived(peasant).is_none());
    assert_eq!(v.leaks().recallable_group("1"), [peasant]);
    assert_eq!(v.leaks().recallable_group("2"), []);
}

#[test]
fn the_order_menus_are_the_worlds() {
    let (w, me, _) = game();
    let v = View::new(&w, me);
    for u in w.player(me).units.clone() {
        for &k in Keyword::ALL {
            for strict in [true, false] {
                assert_eq!(v.order_menu(u, k, strict), w.order_menu(u, k, strict));
            }
        }
    }
    let peasant = unit(&w, me, "peasant");
    assert_eq!(v.order_menu(peasant, Keyword::Go, true), ["go"]);
    assert!(
        v.order_menu(peasant, Keyword::Build, false)
            .contains(&"build farm".to_owned())
    );
    assert_eq!(
        v.order_menu(EntityId(99_999), Keyword::Go, true),
        Vec::<String>::new()
    );
    let info = v.order_info(peasant, Keyword::Build, Some("farm")).unwrap();
    let old = w.order_info(peasant, Keyword::Build, Some("farm")).unwrap();
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
    w.entity_mut(farm).unwrap().hp = 1000;
    let townhall = unit(&w, them, "townhall");
    let b2 = w.square_by_name("b2").unwrap();
    let v = View::new(&w, me);
    let old = |target: &str| {
        w.get_default_order(peasant, target)
            .map(|k| k.and_then(Keyword::parse))
    };
    let cases = [
        (OrderTarget::Square(b2), w.square(b2).id, Some(Keyword::Go)),
        (OrderTarget::Entity(farm), farm.0, Some(Keyword::Repair)),
        // Not perceived nor remembered: the target is unknown.
        (OrderTarget::Entity(townhall), townhall.0, None),
    ];
    for (target, id, expected) in cases {
        assert_eq!(v.default_order(peasant, target), Ok(expected));
        assert_eq!(v.default_order(peasant, target), old(&id.to_string()));
    }
}

#[test]
fn a_requirement_of_another_players_unit_is_a_leak() {
    let (w, me, them) = game();
    let mine = unit(&w, me, "peasant");
    let theirs = unit(&w, them, "peasant");
    let v = View::new(&w, me);
    assert_eq!(v.owner_has(mine, "farm"), Some(true));
    assert_eq!(v.owner_has(mine, "barracks"), Some(false));
    assert_eq!(v.owner_has(theirs, "farm"), None);
    assert_eq!(
        v.leaks().unit_owner_has(theirs, "farm"),
        w.has(them, "farm")
    );
    assert!(v.leaks().unit_owner_has(theirs, "farm"));
    assert!(!v.leaks().unit_owner_has(theirs, "barracks"));
}
