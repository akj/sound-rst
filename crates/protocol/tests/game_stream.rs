//! The proof that the game stream gives every seat exactly what the host
//! made: whole games are played, every seat is told the events of each
//! turn, and for every seat on every turn
//!
//! - the whole message is encoded and decoded, and must come back equal;
//! - the change from the seat's last message is encoded, decoded and
//!   applied, and must give back exactly the current message;
//! - so must the change for a seat 10 and 50 turns behind, with the events
//!   it missed that are not about an entity;
//! - a `SeatView` fed only what was decoded must answer as one fed the
//!   host's own messages.
//!
//! The start, with the rules the client loads itself, and the end are
//! checked the same way.

use std::collections::VecDeque;
use std::sync::Arc;

use soundrts_protocol::game::{
    apply_change, decode_end, decode_setup, decode_start, decode_turn, encode_change, encode_end,
    encode_setup, encode_start, encode_turn,
};
use soundrts_world::orders::Keyword;
use soundrts_world::test_games::{
    Case, Game, allies_sharing_control, cheat_mode_and_defeat, mods, sixteen,
};
use soundrts_world::{
    EntityId, Event, GameStart, Rules, SeatView, SquareId, TurnChange, TurnMessage,
};

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/game-data")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What was checked, to show the proof covered something.
#[derive(Default, Debug)]
struct Count {
    messages: u64,
    changes: u64,
    behind_10: u64,
    behind_50: u64,
    events: u64,
    events_not_about_an_entity_caught_up: u64,
    seen_again: u64,
    answers: u64,
}

/// Asks both views what the interface asks, beyond the message itself:
/// what they keep of earlier turns, and what they compute from the rules.
fn same_answers(count: &mut Count, a: &SeatView, b: &SeatView, at: &str) {
    assert_eq!(a.message(), b.message(), "{at}: the messages differ");
    for q in 0..a.square_count() as u32 {
        assert_eq!(
            a.terrain(SquareId(q)),
            b.terrain(SquareId(q)),
            "{at}: terrain {q}"
        );
    }
    let m = a.message();
    for e in m.perceived.iter().chain(&m.remembered) {
        assert_eq!(
            a.remembered_upgrades(e.id),
            b.remembered_upgrades(e.id),
            "{at}: upgrades of {}",
            e.id
        );
        count.answers += 1;
    }
    for u in a.controlled_units() {
        for &k in Keyword::ALL {
            assert_eq!(
                a.order_menu(u.id, k, false),
                b.order_menu(u.id, k, false),
                "{at}: menu of {}",
                u.id
            );
            count.answers += 1;
        }
        let target = soundrts_world::OrderTarget::Entity(EntityId(u.id.0.wrapping_add(1)));
        assert_eq!(a.default_order(u.id, target), b.default_order(u.id, target));
    }
    assert_eq!(a.stats(), b.stats(), "{at}: stats");
}

fn play(case: &Case, count: &mut Count) {
    let setup = case.setup(read);
    assert_eq!(decode_setup(&encode_setup(&setup).unwrap()).unwrap(), setup);

    let mut game = Game::new(case, read, true);
    let host_start = Arc::new(game.start());
    let sent = decode_start(&encode_start(&host_start.message()).unwrap()).unwrap();
    assert_eq!(sent, host_start.message());
    // The client supplies the rules, from the same texts.
    let rules = Rules::load(&[setup.rules.as_str()]);
    assert_eq!(rules.types(), host_start.rules.types(), "{}", case.name);
    let start = Arc::new(GameStart::from_message(sent, rules));

    let first = game.messages();
    let mut host_views: Vec<SeatView> = first
        .iter()
        .map(|m| {
            let mut v = SeatView::new(Arc::clone(&host_start), m.clone());
            v.remember();
            v
        })
        .collect();
    let mut seat_views: Vec<SeatView> = first
        .iter()
        .map(|m| {
            let m = decode_turn(&encode_turn(m).unwrap(), &start).unwrap();
            let mut v = SeatView::new(Arc::clone(&start), m);
            v.remember();
            v
        })
        .collect();
    // Each seat's last 50 messages as sent whole, with their events.
    let mut history: Vec<VecDeque<(Vec<u8>, Vec<Event>)>> =
        first.iter().map(|_| VecDeque::new()).collect();
    let mut turn = 0;
    while let Some(now) = game.step() {
        turn += 1;
        for (s, m) in now.iter().enumerate() {
            let at = format!("{} turn {turn} seat {s}", case.name);
            let whole = encode_turn(m).unwrap();
            assert_eq!(&decode_turn(&whole, &start).unwrap(), m, "{at}: whole");
            count.messages += 1;
            count.events += m.events.len() as u64;

            let last = seat_views[s].message().clone();
            let change = TurnChange::between(&last, m);
            count.seen_again += change
                .seen_again
                .iter()
                .map(|g| g.ids.as_slice().len() as u64)
                .sum::<u64>();
            let bytes = encode_change(&change).unwrap();
            let applied = apply_change(&bytes, &last, &start).unwrap();
            assert_eq!(&applied, m, "{at}: change");
            count.changes += 1;
            seat_views[s].install(applied);
            seat_views[s].remember();
            host_views[s].install(m.clone());
            host_views[s].remember();
            same_answers(count, &host_views[s], &seat_views[s], &at);

            let h = &history[s];
            for back in [10, 50] {
                if h.len() < back {
                    continue;
                }
                let old = decode_turn(&h[h.len() - back].0, &start).unwrap();
                let missed: Vec<&Event> = h
                    .iter()
                    .skip(h.len() - back + 1)
                    .flat_map(|(_, e)| e)
                    .collect();
                let told: Vec<Event> = missed
                    .iter()
                    .filter(|e| !matches!(e, Event::Entity { .. }))
                    .map(|&e| e.clone())
                    .chain(m.events.iter().cloned())
                    .collect();
                count.events_not_about_an_entity_caught_up += (told.len() - m.events.len()) as u64;
                let change = TurnChange::catching_up(&old, missed, m);
                let applied = apply_change(&encode_change(&change).unwrap(), &old, &start).unwrap();
                let expected = TurnMessage {
                    events: told,
                    ..m.clone()
                };
                assert_eq!(applied, expected, "{at}: {back} turns behind");
                if back == 10 {
                    count.behind_10 += 1;
                } else {
                    count.behind_50 += 1;
                }
            }
            let h = &mut history[s];
            h.push_back((whole, m.events.clone()));
            if h.len() > 50 {
                h.pop_front();
            }
        }
    }
    for (s, &p) in game.players().iter().enumerate() {
        let end = game.game_end(p);
        assert_eq!(decode_end(&encode_end(&end).unwrap()).unwrap(), end);
        host_views[s].install_end(end.clone());
        seat_views[s].install_end(decode_end(&encode_end(&end).unwrap()).unwrap());
        same_answers(count, &host_views[s], &seat_views[s], case.name);
    }
}

#[test]
fn every_seat_gets_back_exactly_what_the_host_made() {
    let mut count = Count::default();
    for case in [
        allies_sharing_control(600),
        cheat_mode_and_defeat(1000),
        mods(400),
        sixteen(60),
    ] {
        play(&case, &mut count);
    }
    eprintln!("{count:#?}");
    assert!(count.seen_again > 0, "no remembered entity was seen again");
    assert!(
        count.events_not_about_an_entity_caught_up > 0,
        "no event was caught up"
    );
    assert!(count.behind_50 > 0);
}

/// A unit replaced by an upgrade is told as an event about no entity:
/// whole, in a change, and to a seat that fell behind.
#[test]
fn a_unit_replaced_is_told_whole_in_a_change_and_to_a_seat_behind() {
    let mut game = Game::new(&cheat_mode_and_defeat(100), read, false);
    let start = game.start();
    let last = game.messages().remove(0);
    let next = game.step().unwrap().remove(0);
    let replaced = Event::Replaced {
        old: EntityId(41),
        new: EntityId(57),
    };
    let now = TurnMessage {
        events: vec![replaced.clone()],
        ..next.clone()
    };
    assert_eq!(
        decode_turn(&encode_turn(&now).unwrap(), &start).unwrap(),
        now
    );
    let change = TurnChange::between(&last, &now);
    let bytes = encode_change(&change).unwrap();
    assert_eq!(apply_change(&bytes, &last, &start).unwrap(), now);
    let change = TurnChange::catching_up(&last, [&replaced], &next);
    let bytes = encode_change(&change).unwrap();
    let caught_up = apply_change(&bytes, &last, &start).unwrap();
    assert_eq!(caught_up.events[..1], [replaced]);
}
