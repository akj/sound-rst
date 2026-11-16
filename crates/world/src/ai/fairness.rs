//! Where what a computer knows and the truth differ, the computer acts on
//! what it knows: each test sets the two at odds.

use std::sync::Arc;

use super::tests::world;
use super::*;
use crate::entity::{MemoryRef, PlayerId, Target};
use crate::msg::{Msg, MsgPart, msg};
use crate::phrases::Phrase;
use crate::player::{Controller, Remembrance};
use crate::world::World;

/// A computer alone on a map of three squares in a row, a1 to c1, with a
/// meadow on each.
fn three_squares() -> (World, PlayerId) {
    let mut w = world(
        "
square_width 12
nb_columns 3
nb_lines 1
west_east_paths a1 b1
nb_meadows_by_square 1
nb_players_min 1
nb_players_max 1
starting_squares a1
",
    );
    let p = w
        .populate_map(&[Controller::computer("easy")], false, false)
        .unwrap()[0];
    (w, p)
}

fn meadow_on(w: &World, s: SquareId) -> EntityId {
    w.square(s)
        .objects
        .iter()
        .copied()
        .find(|&o| w.e(o).is_meadow())
        .unwrap()
}

#[test]
fn a_computer_looks_for_a_meadow_where_it_remembers_it() {
    let (mut w, p) = three_squares();
    let [a1, b1, c1] = ["a1", "b1", "c1"].map(|n| w.square_by_name(n).unwrap());
    // It perceives the meadow of b1.
    let near = meadow_on(&w, b1);
    w.plm(p).perception.insert(near);
    // It remembers the meadow of c1 on a1. A meadow does not move, so the
    // copy and the truth disagree only in this test: the point is which of
    // the two the walk reads.
    let far = meadow_on(&w, c1);
    let mut copy = w.e(far).clone();
    copy.place = Some(Place::Square(a1));
    let snapshot = Arc::new(copy);
    let time_stamp = w.time;
    w.plm(p).memory.insert(
        far,
        Remembrance {
            time_stamp,
            snapshot: Arc::clone(&snapshot),
        },
    );
    let remembered = Mark::of(Target::Memory(MemoryRef { owner: p, snapshot }));
    let candidates = [Mark::of(Target::Entity(near)), remembered];
    let first = w.door(p).remove_far_candidates(&candidates, a1, 1);
    let ids: Vec<EntityId> = first.iter().filter_map(Mark::entity_id).collect();
    assert_eq!(ids, [far]);
}

/// The orders of a unit as the words they store.
fn orders(w: &World, u: EntityId) -> Vec<Vec<String>> {
    w.e(u).orders().iter().map(|o| o.words(w)).collect()
}

#[test]
fn a_computer_judges_whether_an_exit_is_closed_by_what_it_has_seen() {
    // Two wide squares. From the high ground of a1, the computer's town
    // hall and peasant see b1 only near the exit; on a1 they would bump
    // into the blockers of the exit when looking, and see them.
    let mut w = world(
        "
square_width 40
nb_columns 2
nb_lines 1
west_east_paths a1
high_grounds a1
nb_meadows_by_square 0
nb_players_min 2
nb_players_max 2
starting_squares a1 b1
starting_units townhall peasant
",
    );
    let ps = w
        .populate_map(
            &[
                Controller::computer("easy"),
                Controller::human("you", false),
            ],
            false,
            false,
        )
        .unwrap();
    let (c, you) = (ps[0], ps[1]);
    w.plm(c).cheat_has_all = true;
    w.plm(c).resources = vec![1_000_000; w.pl(c).resources.len()];
    let [a1, b1] = ["a1", "b1"].map(|n| w.square_by_name(n).unwrap());
    let townhall = w.pl(c).units[0];
    assert_eq!(&*w.e(townhall).type_name, "townhall");
    w.update_effect_users_and_workers(c);
    w.computer_state(c).sensible_building = Some(townhall);
    let exit = w.square(a1).exits[0];
    // A footman of the other player blocks the other side of the exit from
    // the far corner of b1, where the computer does not see.
    let other = w.exit_other_side(exit).unwrap();
    let (x, y, d) = (w.square(b1).x, w.square(b1).y, w.square_width() * 2 / 5);
    let footman = w.rules.type_id("footman").unwrap();
    let wall = w
        .create_creature(footman, Some(you), Place::Square(b1), x + d, y + d)
        .unwrap();
    w.block(wall, other);
    w.update_perception_and_memory(c).unwrap();
    assert!(!w.pl(c).perception.contains(&wall));
    assert!(!w.pl(c).memory.contains_key(&wall));
    assert!(w.exit_is_blocked(exit, crate::pathfinding::Viewer::Nobody, false, false));
    // As far as the computer knows, the exit is open: it builds a gate.
    w.door(c).defensive_routine().unwrap();
    let gate = vec!["build".to_owned(), "gate".to_owned(), exit.to_string()];
    let workers = w.computer_state(c).workers.clone();
    assert_eq!(workers.len(), 1);
    assert_eq!(orders(&w, workers[0]), [gate]);
    // Once it knows of the footman, the exit is closed: no gate.
    w.cancel_all_orders(workers[0], true);
    let time_stamp = w.time;
    let snapshot = Arc::new(w.e(wall).clone());
    w.plm(c).memory.insert(
        wall,
        Remembrance {
            time_stamp,
            snapshot,
        },
    );
    w.door(c).defensive_routine().unwrap();
    assert_eq!(orders(&w, workers[0]), Vec::<Vec<String>>::new());
}

#[test]
fn a_computer_stops_a_unit_with_the_order_a_human_gives() {
    let (mut w, p) = three_squares();
    let [a1, b1, c1] = ["a1", "b1", "c1"].map(|n| w.square_by_name(n).unwrap());
    let (x, y) = (w.square(a1).x, w.square(a1).y);
    let footman = w.rules.type_id("footman").unwrap();
    let f = w
        .create_creature(footman, Some(p), Place::Square(a1), x, y)
        .unwrap();
    // The footman walks to b1, and is to wait for the right time to go to
    // c1, where the computer knows of a menace far beyond it.
    w.cm(f).unwrap().action = Some(crate::entity::Action {
        kind: crate::entity::ActionKind::Plain,
        target: Some(Target::Square(b1)),
    });
    let wait = crate::command::KeywordOrder::new(Keyword::Wait)
        .unwrap()
        .at(crate::command::OrderTarget::Square(c1));
    w.take_order(f, Request::from(&wait), false, false, None)
        .unwrap();
    assert_eq!(orders(&w, f).len(), 1);
    w.plm(p).enemy_menace.insert(c1, i64::MAX / 1000);
    // Too weak, it is stopped: its orders are gone, and it stands still.
    w.door(p).update_time_has_come().unwrap();
    assert!(orders(&w, f).is_empty());
    assert!(w.e(f).is_idle());
}

#[test]
fn a_computer_is_named_for_its_handicap() {
    let words = |m: Msg| m.iter().map(MsgPart::to_string).collect::<Vec<_>>();
    assert_eq!(
        words(computer_name("ai2")),
        [
            "random computer",
            "with unlimited resources and no requirements"
        ]
    );
    assert_eq!(computer_name("aggressive"), msg(Phrase::AggressiveComputer));
    assert_eq!(computer_name("easy"), msg(Phrase::QuietComputer));
}

#[test]
fn a_handicap_is_granted_as_its_settings_say() {
    let (mut w, p) = three_squares();
    let fair = w.pl(p).resources.clone();
    w.door(p).grant_handicap();
    assert_eq!(w.pl(p).resources, fair);
    assert!(!w.pl(p).cheat_has_all);
    // A quiet computer with unlimited resources, and the requirements.
    w.plm(p).handicap = Handicap {
        resources: Some(5),
        ignores_requirements: false,
    };
    w.door(p).grant_handicap();
    assert_eq!(w.pl(p).resources, vec![5000; fair.len()]);
    assert!(!w.pl(p).cheat_has_all);
    let name: Vec<String> = w.pl(p).name().iter().map(MsgPart::to_string).collect();
    assert_eq!(
        name.last().map(String::as_str),
        Some("with unlimited resources")
    );
}

/// The computer of [`three_squares`], without units, and dense forests that
/// close the path between b1 and c1 on both sides. With `seen`, it
/// remembers the exit of b1 toward c1 as it is now.
fn forests_between_b1_and_c1(seen: bool) -> (World, PlayerId) {
    let (mut w, p) = three_squares();
    let [b1, c1] = ["b1", "c1"].map(|n| w.square_by_name(n).unwrap());
    for (here, there) in [(b1, c1), (c1, b1)] {
        for e in w.square(here).exits.clone() {
            if w.exit_other_side(e).and_then(|o| w.e(o).square()) == Some(there) {
                w.em(e).exit_mut().unwrap().is_blocked_by_forests = true;
                if seen && here == b1 {
                    let time_stamp = w.time;
                    let snapshot = Arc::new(w.e(e).clone());
                    w.plm(p).memory.insert(
                        e,
                        Remembrance {
                            time_stamp,
                            snapshot,
                        },
                    );
                }
            }
        }
    }
    (w, p)
}

#[test]
fn a_computer_plans_its_paths_through_the_forests_it_has_seen() {
    let (mut w, p) = forests_between_b1_and_c1(false);
    let [a1, c1] = ["a1", "c1"].map(|n| w.square_by_name(n).unwrap());
    // Units walk the true paths: there is none.
    let truth = w.shortest_path_distance_to(a1, c1, Some(p), AirGround::Ground, false);
    assert_eq!(truth.unwrap(), crate::INF);
    // The computer has not seen the forests: as far as it knows, the way
    // is open.
    let planned = w.door(p).distance(a1, c1, false).unwrap();
    assert!(planned < crate::INF, "{planned}");
    // Once it has seen them, it is closed.
    let (mut w, p) = forests_between_b1_and_c1(true);
    assert_eq!(w.door(p).distance(a1, c1, false).unwrap(), crate::INF);
}
