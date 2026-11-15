//! Where what a computer knows and the truth differ, the computer acts on
//! what it knows: each test sets the two at odds.

use std::sync::Arc;

use super::tests::world;
use super::*;
use crate::entity::{MemoryRef, PlayerId};
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
    let remembered = Target::Memory(MemoryRef { owner: p, snapshot });
    let candidates = [Target::Entity(near), remembered];
    let first = w
        .door(p)
        .remove_far_candidates(&candidates, Some(Place::Square(a1)), 1)
        .unwrap();
    let ids: Vec<EntityId> = first.iter().filter_map(Target::entity_id).collect();
    assert_eq!(ids, [far]);
}
