//! What the game interface relies on: commands in, events out, state read
//! through the public API.

mod common;

use soundrts_world::events::{EntityEvent, Event};
use soundrts_world::orders::Keyword;
use soundrts_world::{Controller, PlayerId, World, multiplayer_default_triggers};

fn game() -> (World, PlayerId, PlayerId) {
    let mut w = World::new(
        common::rules(),
        common::ai(),
        multiplayer_default_triggers(),
        3,
    );
    w.load_and_build_map(&common::read("res/multi/jl1.txt"))
        .unwrap();
    let p = w
        .populate_map(
            &[
                Controller::human("me", true),
                Controller::human("you", false),
            ],
            false,
            true,
        )
        .unwrap();
    (w, p[0], p[1])
}

fn entity_events(events: &[Event]) -> Vec<EntityEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Entity { event, .. } => Some(event.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_order_gives_order_ok_to_the_local_human_only() {
    let (mut w, me, you) = game();
    let peasant = common::find_player_unit(&w, me, "peasant", 0).unwrap();
    let square = w.square(w.square_by_name("b2").unwrap()).id;
    w.queue_command(me, &format!("control {peasant}"));
    w.queue_command(me, &format!("order 0 0 go {square}"));
    w.update();
    let events = w.drain_events(me);
    assert!(entity_events(&events).contains(&EntityEvent::OrderOk));
    assert!(w.drain_events(me).is_empty(), "draining empties the queue");
    let other = common::find_player_unit(&w, you, "peasant", 0).unwrap();
    w.queue_command(you, &format!("control {other}"));
    w.queue_command(you, &format!("order 0 0 go {square}"));
    w.update();
    assert!(
        w.drain_events(you).is_empty(),
        "a remote human has no interface"
    );
    let p = w.entity(peasant).unwrap();
    assert_eq!(p.orders[0].keyword, Keyword::Go);
    assert_eq!(p.orders[0].args, [square.to_string()]);
}

#[test]
fn the_menu_lists_what_the_townhall_can_do() {
    let (w, me, _) = game();
    let th = common::find_player_unit(&w, me, "townhall", 0).unwrap();
    let menu = w.full_order_menu(th, false);
    assert!(menu.contains(&"train peasant".to_owned()), "{menu:?}");
    assert!(menu.contains(&"upgrade_to keep".to_owned()), "{menu:?}");
    let info = w.order_info(th, Keyword::Train, Some("peasant")).unwrap();
    assert_eq!(info.cost, [4000, 0]);
    assert_eq!(info.food_cost, 1);
}

#[test]
fn add_units_adds_at_most_as_many_units_as_the_starts_of_a_map() {
    let (mut w, me, _) = game();
    w.allow_cheatmode = true;
    w.queue_command(me, "cmd add_units a1 999999999999 footman");
    w.update();
    let added = w.entities().filter(|e| &*e.type_name == "footman").count();
    let refused = w
        .drain_warnings()
        .iter()
        .filter(|m| *m == "not enough space")
        .count();
    assert!(added > 0 && refused > 0, "{added} added, {refused} refused");
    assert_eq!(added + refused, 10_000);
}

#[test]
fn cheats_are_ignored_unless_allowed() {
    let (mut w, _, you) = game();
    w.queue_command(you, "toggle_cheatmode");
    w.queue_command(you, "cmd victory");
    w.update();
    assert!(!w.player(you).cheatmode);
    assert!(!w.player(you).has_victory);
    assert_eq!(
        w.drain_warnings(),
        [
            "cheat mode is not allowed: toggle_cheatmode",
            "cheat mode is not allowed: cmd victory",
        ]
    );
    w.allow_cheatmode = true;
    w.queue_command(you, "toggle_cheatmode");
    w.queue_command(you, "cmd victory");
    w.update();
    assert!(w.player(you).cheatmode);
    assert!(w.player(you).has_victory);
}

#[test]
fn quitting_defeats_and_removes_the_player() {
    let (mut w, me, you) = game();
    w.queue_command(you, "quit");
    w.update();
    assert!(!w.players.contains(&you));
    assert!(w.ex_players.contains(&you));
    assert!(w.player(you).has_been_defeated);
    assert!(w.player(you).units.is_empty());
    // The default triggers then give the victory to the last player.
    for _ in 0..5 {
        w.update();
    }
    assert!(w.player(me).has_victory);
}

#[test]
fn an_unknown_command_is_a_warning() {
    let (mut w, me, _) = game();
    w.queue_command(me, "dance");
    w.update();
    let warnings = w.drain_warnings();
    assert!(
        warnings.iter().any(|m| m.contains("cmd_dance")),
        "{warnings:?}"
    );
}

/// The client may run the world on another thread.
#[test]
fn the_world_can_move_to_another_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<World>();
}

#[test]
fn a_control_of_too_many_ids_is_ignored_and_the_game_goes_on() {
    let (mut w, me, _) = game();
    let peasant = common::find_player_unit(&w, me, "peasant", 0).unwrap();
    let square = w.square(w.square_by_name("b2").unwrap()).id;
    w.queue_command(me, &format!("control {peasant}"));
    // From another player: it would expand into more ids than memory holds.
    w.queue_command(me, "control 1-9223372036854775807");
    w.queue_command(me, &format!("order 0 0 go {square}"));
    w.update();
    let warnings = w.drain_warnings();
    assert!(
        warnings.iter().any(|m| m.contains("too many ids")),
        "{warnings:?}"
    );
    // The bad control leaves no unit controlled: the order goes nowhere.
    let events = w.drain_events(me);
    assert!(!entity_events(&events).contains(&EntityEvent::OrderOk));
    w.update();
}

#[test]
fn cancelling_a_training_that_was_never_paid_for_refunds_nothing() {
    let (mut w, me, _) = game();
    let th = common::find_player_unit(&w, me, "townhall", 0).unwrap();
    assert_eq!(w.player(me).resources, [10000, 10000]);
    // A peasant costs 4000 gold: two are paid for, 2000 is left.
    for _ in 0..2 {
        w.queue_command(me, &format!("control {th}"));
        w.queue_command(me, "order 0 0 train peasant");
        w.update();
    }
    assert_eq!(w.player(me).resources, [2000, 10000]);
    for _ in 0..3 {
        w.queue_command(me, &format!("control {th}"));
        w.queue_command(me, "order 0 0 train peasant");
        w.queue_command(me, "order 0 0 cancel_training");
        w.update();
    }
    assert_eq!(w.player(me).resources, [2000, 10000]);
    assert_eq!(w.entity(th).unwrap().orders.len(), 2);
}

#[test]
fn cancelling_a_training_that_has_finished_refunds_nothing() {
    let (mut w, me, _) = game();
    let th = common::find_player_unit(&w, me, "townhall", 0).unwrap();
    let peasants = |w: &World| {
        w.player(me)
            .units
            .iter()
            .filter(|&&u| &*w.entity(u).unwrap().type_name == "peasant")
            .count()
    };
    let before = peasants(&w);
    w.queue_command(me, &format!("control {th}"));
    w.queue_command(me, "order 0 0 train peasant");
    w.update();
    while peasants(&w) == before {
        w.update();
    }
    // The finished order waits in the queue until the townhall's next
    // update, which comes after the commands of the next turn.
    assert!(w.entity(th).unwrap().orders[0].is_complete);
    let used_food = w.player(me).used_food;
    w.queue_command(me, &format!("control {th}"));
    w.queue_command(me, "order 0 0 cancel_training");
    w.update();
    assert_eq!(w.player(me).resources, [6000, 10000]);
    assert_eq!(w.player(me).used_food, used_food);
    assert_eq!(peasants(&w), before + 1);
}
