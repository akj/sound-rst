//! The digest that players of a game on a server compare after each turn.

mod common;

use soundrts_world::{Controller, PlayerId, World, multiplayer_default_triggers};

/// A game on jl2 between a human (`local` or not) and two computers.
fn game(local: bool) -> (World, PlayerId) {
    let mut w = World::new(
        common::rules(),
        common::ai(),
        multiplayer_default_triggers(),
        5,
    );
    w.load_and_build_map(&common::read("res/multi/jl2.txt"))
        .unwrap();
    let players = w
        .populate_map(
            &[
                Controller::human("me", local),
                common::computer("aggressive", "1"),
                common::computer("easy", "2"),
            ],
            true,
            true,
        )
        .unwrap();
    (w, players[0])
}

fn square_id(w: &World, name: &str) -> u32 {
    w.square(w.square_by_name(name).unwrap()).id
}

#[test]
fn only_the_local_human_differs_and_the_digests_agree() {
    let (mut a, me) = game(true);
    let (mut b, _) = game(false);
    let peasant = common::find_player_unit(&a, me, "peasant", 0).unwrap();
    let townhall = common::find_player_unit(&a, me, "townhall", 0).unwrap();
    let b2 = square_id(&a, "b2");
    for turn in 0..400 {
        let commands = match turn {
            0 => vec![
                format!("control {townhall}"),
                "order 0 0 train peasant".into(),
            ],
            10 => vec![format!("control {peasant}"), format!("order 0 0 go {b2}")],
            200 => vec![
                format!("control {townhall}"),
                "order 0 0 train footman".into(),
            ],
            _ => Vec::new(),
        };
        for c in &commands {
            a.queue_command(me, c);
            b.queue_command(me, c);
        }
        a.update();
        b.update();
        assert_eq!(a.digest_u64(), b.digest_u64(), "turn {turn}");
        a.drain_events(me);
    }
    assert!(b.drain_events(me).is_empty(), "the human is remote in b");
}

/// The peasant goes to the next square, then is told where to go after:
/// the worlds differ only in the target of that second order, which
/// changes nothing else during the turn it is taken.
#[test]
fn the_target_of_a_queued_order_changes_the_digest() {
    let (mut a, me) = game(true);
    let (mut b, _) = game(true);
    let peasant = common::find_player_unit(&a, me, "peasant", 0).unwrap();
    let next = square_id(&a, "c1");
    for w in [&mut a, &mut b] {
        w.queue_command(me, &format!("control {peasant}"));
        w.queue_command(me, &format!("order 0 0 go {next}"));
        w.update();
    }
    assert_eq!(a.digest_u64(), b.digest_u64());
    let (a1, c3) = (square_id(&a, "a1"), square_id(&a, "c3"));
    a.queue_command(me, &format!("order 1 0 go {a1}"));
    b.queue_command(me, &format!("order 1 0 go {c3}"));
    a.update();
    b.update();
    assert_eq!(a.entity(peasant).unwrap().orders.len(), 2);
    assert_eq!(b.entity(peasant).unwrap().orders.len(), 2);
    assert_ne!(a.digest_u64(), b.digest_u64());
}
