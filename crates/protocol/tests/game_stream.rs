//! The proof that the game stream gives every seat exactly what the host
//! made: whole games are played, every seat is told the events of each
//! turn, and for every seat on every turn
//!
//! - the whole message is encoded and decoded, and must come back equal
//!   from the seat's stream, as to a seat that returns;
//! - the change from the seat's last message is encoded, decoded, and
//!   taken by the seat's stream, and must give back exactly the current
//!   message;
//! - so must the change for a seat 10 and 50 turns behind, with the events
//!   it missed that are not about an entity;
//! - a `SeatView` fed only what was decoded must answer as one fed the
//!   host's own messages.
//!
//! Each seat reads the stream as its client does, through
//! [`JoinStream`] and [`SeatStream`]: the start, with the rules the client
//! loads itself, and the end are checked the same way.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use soundrts_protocol::game::{
    Back, Checked, JoinStream, Joined, Joining, Returning, Role, RulesHash, SeatFrame, SeatStream,
    decode_frame, decode_setup, encode_frame, encode_setup,
};
use soundrts_protocol::{MapFileName, MapHash, MapInfo, SeatToken, Speed};
use soundrts_world::orders::Keyword;
use soundrts_world::test_games::{
    Case, Game, allies_sharing_control, cheat_mode_and_defeat, mods, sixteen,
};
use soundrts_world::{EntityId, Event, SeatView, SquareId, TurnChange, TurnMessage};

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

/// A frame as a seat receives it: encoded, then decoded.
fn sent(frame: SeatFrame) -> SeatFrame {
    decode_frame(&encode_frame(&frame).unwrap()).unwrap()
}

/// The stream of the player of slot `you`, joined as its client joins a
/// game on the map of hash `map`: the start, read with the rules of the
/// text `rules`, then `first`, its whole message.
fn join(rules: &str, map: &MapHash, start: &SeatFrame, you: usize, first: &TurnMessage) -> Joined {
    let join = JoinStream::new(Role::Player(you), map.clone(), rules);
    let Joining::Started(join) = join.take(sent(start.clone())).unwrap() else {
        panic!("the start did not ask for the whole message");
    };
    let Joining::Joined(joined) = join.take(sent(SeatFrame::Whole(first.clone()))).unwrap() else {
        panic!("the whole message did not join the game");
    };
    joined
}

/// The message `stream` takes from a change.
fn applied(stream: &mut SeatStream, change: TurnChange) -> Arc<TurnMessage> {
    match stream.take(sent(SeatFrame::Change(change))).unwrap() {
        Checked::Turn(m) => m,
        other => panic!("a change gave {other:?}"),
    }
}

/// What a seat that returns is sent: its seat, the start and `whole`, its
/// whole message as it is now. What its stream takes of it.
fn returned(stream: &SeatStream, opening: &[SeatFrame; 2], whole: &TurnMessage) -> Back {
    let mut back = stream.returning().unwrap();
    for frame in opening {
        back = match back.take(sent(frame.clone())).unwrap() {
            Returning::More(more) => more,
            other => panic!("the opening of a return gave {other:?}"),
        };
    }
    match back.take(sent(SeatFrame::Whole(whole.clone()))).unwrap() {
        Returning::Back(back) => back,
        other => panic!("the whole message of a return gave {other:?}"),
    }
}

fn play(case: &Case, count: &mut Count) {
    let setup = case.setup(read);
    assert_eq!(decode_setup(&encode_setup(&setup).unwrap()).unwrap(), setup);

    let mut game = Game::new(case, read, true);
    let host_start = Arc::new(game.start());
    let start = SeatFrame::Start {
        start: host_start.message(),
        rules: RulesHash::of(&setup.rules),
    };
    let SeatFrame::Start { start: s, .. } = sent(start.clone()) else {
        panic!("a start");
    };
    assert_eq!(s, host_start.message());
    let map = MapHash::of(setup.map.as_bytes());

    let first = game.messages();
    let mut host_views: Vec<SeatView> = first
        .iter()
        .map(|m| {
            let mut v = SeatView::new(Arc::clone(&host_start), m.clone());
            v.remember();
            v
        })
        .collect();
    // Each seat joins: the client supplies the rules, from the same texts.
    let mut streams = Vec::new();
    let mut seat_views = Vec::new();
    for (s, m) in first.iter().enumerate() {
        let Joined { stream, message } = join(&setup.rules, &map, &start, s, m);
        assert_eq!(&*message, m, "{}: seat {s} joins", case.name);
        let rules = &stream.start().rules;
        assert_eq!(rules.types(), host_start.rules.types(), "{}", case.name);
        let mut v = SeatView::new(Arc::clone(stream.start()), message);
        v.remember();
        streams.push(stream);
        seat_views.push(v);
    }
    let opening = |s: usize| {
        [
            SeatFrame::Seated {
                you: Role::Player(s),
                map: MapInfo {
                    file_name: MapFileName::new("map.txt").unwrap(),
                    hash: map.clone(),
                    title: Vec::new(),
                    nb_players_min: 1,
                    nb_players_max: 16,
                },
                speed: Speed::Normal,
                paused: false,
                hold: Duration::from_secs(30),
                token: SeatToken::from_bits(7),
            },
            start.clone(),
        ]
    };
    // Each seat's stream and message as of each of its last 50 turns.
    let mut history: Vec<VecDeque<(SeatStream, Arc<TurnMessage>)>> =
        first.iter().map(|_| VecDeque::new()).collect();
    let mut turn = 0;
    while let Some(now) = game.step() {
        turn += 1;
        for (s, m) in now.iter().enumerate() {
            let at = format!("{} turn {turn} seat {s}", case.name);
            // Whole, as a seat that returns is sent it.
            let back = returned(&streams[s], &opening(s), m);
            assert_eq!(&*back.message, m, "{at}: whole");
            count.messages += 1;
            count.events += m.events.len() as u64;

            let last = seat_views[s].message().clone();
            let change = TurnChange::between(&last, m);
            count.seen_again += change
                .seen_again
                .iter()
                .map(|g| g.ids.as_slice().len() as u64)
                .sum::<u64>();
            let applied_now = applied(&mut streams[s], change);
            assert_eq!(&*applied_now, m, "{at}: change");
            count.changes += 1;
            seat_views[s].install(Arc::clone(&applied_now));
            seat_views[s].remember();
            host_views[s].install(m.clone());
            host_views[s].remember();
            same_answers(count, &host_views[s], &seat_views[s], &at);

            let h = &history[s];
            for back in [10, 50] {
                if h.len() < back {
                    continue;
                }
                let (mut old, old_message) = h[h.len() - back].clone();
                let missed: Vec<&Event> = h
                    .iter()
                    .skip(h.len() - back + 1)
                    .flat_map(|(_, m)| m.events.iter())
                    .collect();
                let told: Vec<Event> = missed
                    .iter()
                    .filter(|e| !matches!(e, Event::Entity { .. }))
                    .map(|&e| e.clone())
                    .chain(m.events.iter().cloned())
                    .collect();
                count.events_not_about_an_entity_caught_up += (told.len() - m.events.len()) as u64;
                let change = TurnChange::catching_up(&old_message, missed, m);
                let caught_up = applied(&mut old, change);
                let expected = TurnMessage {
                    events: told,
                    ..m.clone()
                };
                assert_eq!(*caught_up, expected, "{at}: {back} turns behind");
                if back == 10 {
                    count.behind_10 += 1;
                } else {
                    count.behind_50 += 1;
                }
            }
            let h = &mut history[s];
            h.push_back((streams[s].clone(), applied_now));
            if h.len() > 50 {
                h.pop_front();
            }
        }
    }
    for (s, &p) in game.players().iter().enumerate() {
        let end = game.game_end(p);
        let Checked::End(sent_end) = streams[s].take(sent(SeatFrame::End(end.clone()))).unwrap()
        else {
            panic!("an end");
        };
        assert_eq!(sent_end, end);
        host_views[s].install_end(end);
        seat_views[s].install_end(sent_end);
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
    let case = cheat_mode_and_defeat(100);
    let rules = case.setup(read).rules;
    let mut game = Game::new(&case, read, false);
    let start = SeatFrame::Start {
        start: game.start().message(),
        rules: RulesHash::of(&rules),
    };
    let map = MapHash::of(b"");
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
    assert_eq!(*join(&rules, &map, &start, 0, &now).message, now);
    let mut stream = join(&rules, &map, &start, 0, &last).stream;
    let mut behind = stream.clone();
    assert_eq!(*applied(&mut stream, TurnChange::between(&last, &now)), now);
    let change = TurnChange::catching_up(&last, [&replaced], &next);
    let caught_up = applied(&mut behind, change);
    assert_eq!(caught_up.events[..1], [replaced]);
}
