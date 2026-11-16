//! Returning to a game on a real server after the connection is lost,
//! in-process and on 127.0.0.1. Andrew reaches the server through a
//! [`Cable`] the test pulls out and plugs back in; Bob, when he plays,
//! reaches it directly. The server's heartbeat and the client's check
//! that the server still answers run at a tenth of their usual times, so
//! that a pulled cable is noticed in seconds; the seat is held as long as
//! a server usually holds it, unless a test says otherwise.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use soundrts::config::Config;
use soundrts::net::conn::Liveness;
use soundrts::testkit::{Cable, Harness, Script, TestGame, WAIT_LIMIT, test_server};
use soundrts_protocol::{Computer, Login, Skill, Speed};
use soundrts_server::{ServerConfig, ServerHandle};
use soundrts_world::{GameSetup, Host, TurnMessage};

const SEED: i64 = 5;

const LOST: &str = "connection lost, trying to return";
const BACK: &str = "back in the game";

/// Speech lasts a tenth of its usual time, as the clock is real.
fn config(login: &str) -> Config {
    Config {
        login: soundrts::config::Login::parse(login).unwrap(),
        wait_delay_per_character: 0.01,
        ..Config::default()
    }
}

/// The server pings every half second, and gives a connection up after 2
/// seconds of silence: a tenth of its usual times.
fn quick_heartbeat(c: &mut ServerConfig) {
    c.heartbeat_interval = Duration::from_millis(500);
    c.heartbeat_timeout = Duration::from_secs(2);
}

/// A player on real time whose client pings a silent server after half a
/// second, and gives the connection up after a second and a half: a tenth
/// of the usual times.
fn player(login: &str) -> Harness {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config(login));
    h.clock.follow_real_time();
    h.client.net_config.liveness = Liveness {
        ping: Duration::from_millis(500),
        silence: Duration::from_millis(1500),
    };
    h
}

/// How many times `line` was said by `h` since its `from`th utterance.
fn times(h: &Harness, from: usize, line: &str) -> usize {
    h.said_since(from).iter().filter(|s| *s == line).count()
}

/// The seat's message without its events, which a seat is told once.
fn eventless(m: &TurnMessage) -> TurnMessage {
    let mut m = m.clone();
    m.events.clear();
    m
}

/// Notes Andrew's message at each game time he takes, from now on.
fn record_messages(h: &mut Harness) -> Rc<RefCell<Vec<TurnMessage>>> {
    let seen: Rc<RefCell<Vec<TurnMessage>>> = Rc::default();
    let record = seen.clone();
    h.client.game.hook = Some(Rc::new(move |g, _| {
        let m = g.view().message();
        let mut seen = record.borrow_mut();
        if seen.last().is_none_or(|last| last.time != m.time) {
            seen.push(eventless(m));
        }
    }));
    seen
}

/// Andrew, through a cable, and Bob, directly, playing on jl2 with an
/// aggressive computer, at the fastest speed. Andrew hosts.
struct Table {
    andrew: Harness,
    a: TestGame,
    bob: Harness,
    b: TestGame,
    cable: Cable,
    _server: ServerHandle,
}

impl Table {
    fn new(andrew: Harness, adjust: impl FnOnce(&mut ServerConfig)) -> Table {
        let server = test_server(SEED, adjust);
        let cable = Cable::new(server.local_addr());
        let mut andrew = andrew;
        andrew.connect_to(&cable.addr().to_string(), |c| c);
        let mut bob = player("bob");
        bob.connect_to(&server.local_addr().to_string(), |c| c);
        let room = andrew.open_room("jl2", &[Computer::Aggressive(Skill::NORMAL)], Speed::Fast4);
        andrew.invite("bob");
        bob.join_room(room);
        andrew.wait_for_ready(3);
        let a = andrew.start_room_game("jl2");
        let b = bob.join_room_game("jl2");
        Table {
            andrew,
            a,
            bob,
            b,
            cable,
            _server: server,
        }
    }

    /// Plays both games, each pass of each loop in turn, until `done`
    /// holds. A game that has ended is not played.
    fn run_until(&mut self, what: &str, done: impl Fn(&Table) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(
                start.elapsed() < WAIT_LIMIT,
                "{what}: andrew heard {:#?}\nbob heard {:#?}",
                self.andrew.said_since(0),
                self.bob.said_since(0)
            );
            if !self.a.game.end_loop {
                self.andrew.step(&mut self.a);
            }
            if !self.b.game.end_loop {
                self.bob.step(&mut self.b);
            }
        }
    }
}

/// The game of the table played here, as the server built it: Andrew's
/// message at each game time, up to `until`.
fn played_here(t: &mut Table, until: i64) -> HashMap<i64, TurnMessage> {
    let map = t.andrew.multiplayer_map("jl2");
    let texts = soundrts::multiplayer::game_texts(&mut t.andrew.client, &map).unwrap();
    let net = t.andrew.client.net.as_ref().unwrap();
    let room = &net.room.as_ref().unwrap().state;
    let you = room
        .slots
        .iter()
        .position(|s| {
            s.player == soundrts_protocol::SlotPlayer::Human(Login::new("andrew").unwrap())
        })
        .unwrap();
    let setup = GameSetup::multiplayer(
        texts.rules.into_string(),
        texts.ai.into_string(),
        texts.map.into_string(),
        SEED,
        room.slots
            .iter()
            .map(soundrts_protocol::Slot::controller)
            .collect(),
    );
    let mut host = Host::new(&setup).unwrap();
    let me = host.seats()[you];
    let first = host.message(me);
    let mut messages = HashMap::from([(first.time, eventless(&first))]);
    let mut time = first.time;
    while time < until {
        let turn = host.step_seats(&[me]).pop().unwrap();
        time = turn.message.time;
        messages.insert(time, eventless(&turn.message));
    }
    messages
}

#[test]
fn a_game_on_a_server_survives_a_pulled_cable() {
    let mut t = Table::new(player("andrew"), quick_heartbeat);
    let seen = record_messages(&mut t.andrew);
    t.run_until("20 turns", |t| t.a.game.view().turn() >= 20);
    let (andrew_from, bob_from) = (t.andrew.utterances().len(), t.bob.utterances().len());

    // The cable is out for 3 seconds, a tenth of 30, and at least until
    // both ends have noticed.
    t.cable.pull();
    let pulled = Instant::now();
    t.run_until("the loss noticed", |t| {
        times(&t.andrew, andrew_from, LOST) == 1
            && times(&t.bob, bob_from, "andrew lost the connection") == 1
            && pulled.elapsed() >= Duration::from_secs(3)
    });
    let away = t.a.game.view().turn();
    assert!(t.a.game.returning());
    t.cable.plug_in();
    t.run_until("andrew back", |t| times(&t.andrew, andrew_from, BACK) == 1);
    let back = t.a.game.view().turn();
    let back_time = t.a.game.view().message().time;
    assert!(back > away, "turns {away} then {back}");
    t.run_until("30 turns more", |t| t.a.game.view().turn() >= back + 30);
    t.run_until("the voices idle", |t| {
        t.andrew.client.voice.is_idle() && t.bob.client.voice.is_idle()
    });

    assert!(!t.a.game.end_loop && !t.b.game.end_loop, "a game ended");
    for (who, h, from, lines) in [
        ("andrew", &t.andrew, andrew_from, [LOST, BACK]),
        (
            "bob",
            &t.bob,
            bob_from,
            ["andrew lost the connection", "andrew is back"],
        ),
    ] {
        for line in lines {
            assert_eq!(
                times(h, from, line),
                1,
                "{who}: {line}: {:#?}",
                h.said_since(from)
            );
        }
        let log = h.log_text();
        assert!(!log.contains("cannot be right"), "{who}: {log}");
    }

    // What Andrew knew at each turn he took, before the cable was pulled
    // and after he came back, is what the seat knows in the same game
    // played here, where it never left.
    let seen = seen.take();
    let last = seen.last().unwrap().time;
    let here = played_here(&mut t, last);
    let after = seen.iter().filter(|m| m.time >= back_time).count();
    assert!(after > 30, "{after} turns seen after the return");
    for m in &seen {
        assert!(
            here.get(&m.time) == Some(m),
            "andrew's view at {} ms differs from the seat's here",
            m.time
        );
    }
}

#[test]
fn a_client_whose_connection_is_cut_twice_returns_twice() {
    let server = test_server(SEED, quick_heartbeat);
    let cable = Cable::new(server.local_addr());
    let mut h = player("andrew");
    h.connect_to(&cable.addr().to_string(), |c| c);
    h.open_room("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4);
    let mut g = h.start_room_game("jl1");
    let from = h.utterances().len();
    let run_until = |h: &mut Harness,
                     g: &mut TestGame,
                     what: &str,
                     done: &dyn Fn(&Harness, &TestGame) -> bool| {
        let start = Instant::now();
        while !done(h, g) {
            assert!(
                !g.game.end_loop,
                "{what}: the game ended: {:#?}",
                h.said_since(0)
            );
            assert!(
                start.elapsed() < WAIT_LIMIT,
                "{what}: {:#?}",
                h.said_since(0)
            );
            h.step(g);
        }
    };
    for cut in 1..=2 {
        let turn = g.game.view().turn();
        run_until(&mut h, &mut g, "10 turns", &|_, g| {
            g.game.view().turn() >= turn + 10
        });
        cable.pull();
        run_until(&mut h, &mut g, "the loss noticed", &|h, _| {
            times(h, from, LOST) == cut
        });
        cable.plug_in();
        run_until(&mut h, &mut g, "back", &|h, _| times(h, from, BACK) == cut);
    }
    let turn = g.game.view().turn();
    run_until(&mut h, &mut g, "10 turns more", &|_, g| {
        g.game.view().turn() >= turn + 10
    });
    h.run_until_voice_idle(&mut g);
    assert_eq!(times(&h, from, LOST), 2, "{:#?}", h.said_since(from));
    assert_eq!(times(&h, from, BACK), 2, "{:#?}", h.said_since(from));
    drop(server);
}

#[test]
fn a_client_that_returns_after_its_seat_was_given_up_hears_why() {
    // Andrew's client notices only when the cable is plugged back in,
    // which cuts the old connection; the server holds the seat a second.
    let mut andrew = player("andrew");
    andrew.client.net_config.liveness = Liveness::default();
    let mut t = Table::new(andrew, |c| {
        quick_heartbeat(c);
        c.grace_period = Duration::from_secs(1);
    });
    t.run_until("10 turns", |t| t.a.game.view().turn() >= 10);
    let (andrew_from, bob_from) = (t.andrew.utterances().len(), t.bob.utterances().len());
    t.cable.pull();
    t.run_until("the seat given up", |t| {
        times(
            &t.bob,
            bob_from,
            "andrew did not come back, and is out of the game",
        ) == 1
    });
    t.cable.plug_in();
    t.run_until("andrew's game over", |t| t.a.game.end_loop);
    t.andrew.run_voice_for(10.0);
    let said = t.andrew.said_since(andrew_from);
    assert!(
        said.ends_with(&[
            LOST.into(),
            "could not return: your seat was given up".into()
        ]),
        "{said:#?}"
    );
    assert_eq!(times(&t.andrew, andrew_from, LOST), 1, "{said:#?}");
    assert_eq!(
        times(&t.bob, bob_from, "andrew lost the connection"),
        1,
        "{:#?}",
        t.bob.said_since(bob_from)
    );
    // Bob plays on.
    let turn = t.b.game.view().turn();
    t.run_until("bob plays on", |t| t.b.game.view().turn() >= turn + 10);
}
