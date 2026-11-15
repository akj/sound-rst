//! Watching a game on a real server, in-process and on 127.0.0.1: Andrew
//! hosts a game on jl2 with Bob and a computer, Carol watches it from
//! before the start, Dave joins it while it runs. Each person runs the
//! client's own loop on real time; what each hears is checked, and what
//! each knows at every turn is compared.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use soundrts::config::Config;
use soundrts::input::{Key, Mods};
use soundrts::net::conn::Liveness;
use soundrts::testkit::{Cable, Harness, Script, TestGame, WAIT_LIMIT, controller, test_server};
use soundrts_protocol::{AiLevel, ClientMsg, Speed};
use soundrts_server::ServerConfig;
use soundrts_world::{Command, GameSetup, Host, PlayerId, TurnMessage};

const SEED: i64 = 5;

const ANDREW: usize = 0;
const BOB: usize = 1;
const CAROL: usize = 2;
const DAVE: usize = 3;

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

/// What a person knew at a game time: the player whose side it was, and
/// the message without its events.
type Seen = (i64, PlayerId, TurnMessage);

/// Someone on the server, on real time, with what they knew at each game
/// time they took.
struct Person {
    h: Harness,
    g: Option<TestGame>,
    seen: Rc<RefCell<Vec<Seen>>>,
}

impl Person {
    fn new(login: &str) -> Person {
        let mut h = Harness::with_config("en", Script::new().keep_open(), config(login));
        h.clock.follow_real_time();
        h.client.net_config.liveness = Liveness {
            ping: Duration::from_millis(500),
            silence: Duration::from_millis(1500),
        };
        let seen: Rc<RefCell<Vec<Seen>>> = Rc::default();
        let record = seen.clone();
        h.client.game.hook = Some(Rc::new(move |g, _| {
            let m = g.view().message();
            let mut seen = record.borrow_mut();
            if seen
                .last()
                .is_none_or(|(t, p, _)| (*t, *p) != (m.time, m.seat))
            {
                let mut eventless = m.clone();
                eventless.events.clear();
                seen.push((m.time, m.seat, eventless));
            }
        }));
        Person { h, g: None, seen }
    }

    fn game(&self) -> &TestGame {
        self.g.as_ref().expect("in the game")
    }

    fn turn(&self) -> i64 {
        self.game().game.view().turn()
    }

    fn me(&self) -> PlayerId {
        self.game().game.me
    }

    /// How many times `line` was said since the `from`th utterance.
    fn times(&self, from: usize, line: &str) -> usize {
        self.h
            .said_since(from)
            .iter()
            .filter(|s| *s == line)
            .count()
    }

    /// What this person knew of `side` from game time `from`.
    fn of(&self, side: PlayerId, from: i64) -> Vec<Seen> {
        self.seen
            .borrow()
            .iter()
            .filter(|(t, p, _)| *p == side && *t >= from)
            .cloned()
            .collect()
    }
}

/// Plays every game of `people`, each pass of each loop in turn, until
/// `done` holds. A game that has ended is not played.
fn run_until(people: &mut [Person], what: &str, done: impl Fn(&[Person]) -> bool) {
    let start = Instant::now();
    while !done(people) {
        assert!(
            start.elapsed() < WAIT_LIMIT,
            "{what}: {:#?}",
            people.iter().map(|p| p.h.said_since(0)).collect::<Vec<_>>()
        );
        for p in people.iter_mut() {
            if let Some(g) = &mut p.g
                && !g.game.end_loop
            {
                p.h.step(g);
            }
        }
    }
}

/// Checks that at every game time both the watcher `w` and the player `p`
/// were seen to know, from `from`, `w` knew what `p` knew of `p`'s side.
/// How many times were compared. (A client that takes two turns in one
/// pass of its loop is seen to know only the second.)
fn as_the_player_knew(w: &Person, p: &Person, from: i64) -> usize {
    let theirs = p.of(p.me(), 0);
    let mut compared = 0;
    for (t, _, m) in w.of(p.me(), from) {
        let Some((_, _, pm)) = theirs.iter().find(|(pt, _, _)| *pt == t) else {
            continue;
        };
        assert!(pm == &m, "at {t} ms the watcher knew another game");
        compared += 1;
    }
    compared
}

/// Andrew's game played here, where nobody watches: his message at each
/// game time, without its events, up to `until`.
fn played_here(andrew: &mut Person, until: i64) -> Vec<(i64, TurnMessage)> {
    let map = andrew.h.multiplayer_map("jl2");
    let texts = soundrts::multiplayer::game_texts(&mut andrew.h.client, &map).unwrap();
    let net = andrew.h.client.net.as_ref().unwrap();
    let room = &net.room.as_ref().unwrap().state;
    let setup = GameSetup::multiplayer(
        texts.rules.into_string(),
        texts.ai.into_string(),
        texts.map.into_string(),
        SEED,
        room.slots.iter().map(controller).collect(),
    );
    let mut host = Host::new(&setup).unwrap();
    let humans: Vec<PlayerId> = host.seats()[..2].to_vec();
    let mut messages = vec![(0, host.message(humans[0]))];
    let mut time = 0;
    while time < until {
        let turn = host.step_seats(&humans).swap_remove(0);
        time = turn.message.time;
        let mut m = (*turn.message).clone();
        m.events.clear();
        messages.push((time, m));
    }
    messages
}

/// Andrew hosts an open room on jl2 with Bob and an easy computer, at the
/// fastest speed; Carol watches it from before the start. Nobody gives an
/// order. Carol reaches the server at `carol_server`, the others at
/// `server`.
fn party(server: &str, carol_server: &str) -> Vec<Person> {
    let mut andrew = Person::new("andrew");
    let mut bob = Person::new("bob");
    let mut carol = Person::new("carol");
    andrew.h.connect_to(server, |c| c);
    bob.h.connect_to(server, |c| c);
    carol.h.connect_to(carol_server, |c| c);
    let room = andrew
        .h
        .create_room("jl2", &[AiLevel::Easy], Speed::Fast4, true, true);
    andrew.h.invite("bob");
    bob.h.join_room(room);
    carol.h.watch_room(room);
    andrew.h.wait_for_ready(3);
    // The players hear of the watcher in the room, before the game.
    for who in [&mut andrew, &mut bob] {
        let start = Instant::now();
        while who.times(0, "carol is watching") == 0 {
            assert!(start.elapsed() < WAIT_LIMIT, "{:#?}", who.h.said_since(0));
            soundrts::net::pump(&mut who.h.client);
            who.h.run_voice_for(0.05);
        }
    }
    andrew.g = Some(andrew.h.start_room_game("jl2"));
    bob.g = Some(bob.h.join_room_game("jl2"));
    carol.g = Some(carol.h.join_room_game("jl2"));
    vec![andrew, bob, carol]
}

#[test]
fn watchers_hear_the_side_of_the_player_they_follow_and_change_nothing() {
    let server = test_server(SEED, |_| {});
    let addr = server.local_addr().to_string();
    let mut p = party(&addr, &addr);
    let andrew = p[ANDREW].me();
    let bob = p[BOB].me();
    assert_eq!(p[CAROL].me(), andrew, "Carol follows the first player");
    run_until(&mut p, "10 turns", |p| p[ANDREW].turn() >= 10);

    // Dave joins the game that runs: he follows Andrew from now on, and
    // hears nothing of what happened before.
    let mut dave = Person::new("dave");
    dave.h.connect_to(&addr, |c| c);
    let room = p[ANDREW].h.room_id();
    dave.h.watch_room(room);
    dave.g = Some(dave.h.join_room_game("jl2"));
    assert_eq!(dave.me(), andrew, "Dave follows the first player");
    let joined = dave.game().game.view().message().time;
    assert!(joined >= 10 * 300, "Dave starts from the game as it is");
    p.push(dave);
    run_until(&mut p, "10 turns more", |p| p[DAVE].turn() >= 20);

    // Carol's orders are not taken: she hears so once.
    let from_c = p[CAROL].h.utterances().len();
    for _ in 0..2 {
        let c = &mut p[CAROL];
        let g = c.g.as_mut().unwrap();
        g.game
            .send_command(&c.h.client, Command::Control(Vec::new()));
    }
    assert!(p[CAROL].game().game.pending_commands().is_empty());

    // Carol follows the next player, Bob.
    p[CAROL].h.press(Key::F12, Mods::CTRL);
    run_until(&mut p, "Carol follows Bob", |p| p[CAROL].me() == bob);
    let on_bob = p[CAROL].game().game.view().message().time;
    let at = p[ANDREW].turn() + 10;
    run_until(&mut p, "10 turns on Bob's side", |p| p[CAROL].turn() >= at);
    run_until(&mut p, "the voices idle", |p| {
        p.iter().all(|p| p.h.client.voice.is_idle())
    });

    assert!(p.iter().all(|p| !p.game().game.end_loop), "a game ended");
    // Each watcher knew, at each turn, what the player it followed knew.
    assert!(as_the_player_knew(&p[CAROL], &p[ANDREW], 0) >= 15);
    assert!(as_the_player_knew(&p[DAVE], &p[ANDREW], joined) >= 8);
    assert!(as_the_player_knew(&p[CAROL], &p[BOB], on_bob) >= 8);
    // What each heard of the watching, once.
    let heard = |who: usize, line: &str| p[who].times(0, line);
    assert_eq!(heard(CAROL, "watching andrew"), 1);
    assert_eq!(heard(CAROL, "watching bob"), 1);
    assert_eq!(heard(DAVE, "watching andrew"), 1);
    assert_eq!(p[CAROL].times(from_c, "watching: you give no orders"), 1);
    for who in [ANDREW, BOB] {
        assert_eq!(heard(who, "carol is watching"), 1, "{who}");
        assert_eq!(heard(who, "dave is watching"), 1, "{who}");
    }
    // Dave heard nothing of the game before he came.
    let dave_said = p[DAVE].h.said_since(0);
    assert!(
        !dave_said.iter().any(|s| s.contains("carol is watching")),
        "{dave_said:#?}"
    );
    // F11 names the players, then who watches.
    let from_a = p[ANDREW].h.utterances().len();
    p[ANDREW].h.press(Key::F11, Mods::NONE);
    run_until(&mut p, "F11", |p| p[ANDREW].h.utterances().len() > from_a);
    let names = p[ANDREW].h.said_since(from_a).join(" ");
    assert!(names.contains("watching: carol . dave"), "{names}");

    // The game is the one played here with nobody watching, turn for turn.
    let seen = p[ANDREW].of(andrew, 0);
    let last = seen.last().unwrap().0;
    let here = played_here(&mut p[ANDREW], last);
    for (t, _, m) in &seen {
        assert!(
            here.iter().any(|(ht, hm)| ht == t && hm == m),
            "andrew's view at {t} ms differs from the game played here"
        );
    }
    server.shutdown();
}

#[test]
fn a_watcher_returns_after_its_connection_is_cut_and_the_players_hear_nothing() {
    let server = test_server(SEED, quick_heartbeat);
    let addr = server.local_addr().to_string();
    let cable = Cable::new(server.local_addr());
    let mut p = party(&addr, &cable.addr().to_string());
    run_until(&mut p, "10 turns", |p| p[CAROL].turn() >= 10);
    let from: Vec<usize> = p.iter().map(|p| p.h.utterances().len()).collect();

    cable.pull();
    let pulled = Instant::now();
    run_until(&mut p, "the loss noticed", |p| {
        p[CAROL].times(from[CAROL], "connection lost, trying to return") == 1
            && pulled.elapsed() >= Duration::from_secs(3)
    });
    cable.plug_in();
    run_until(&mut p, "Carol back", |p| {
        p[CAROL].times(from[CAROL], "back in the game") == 1
    });
    let back = p[CAROL].game().game.view().message().time;
    let at = p[ANDREW].turn() + 10;
    run_until(&mut p, "10 turns more", |p| p[CAROL].turn() >= at);
    run_until(&mut p, "the voices idle", |p| {
        p.iter().all(|p| p.h.client.voice.is_idle())
    });

    assert!(as_the_player_knew(&p[CAROL], &p[ANDREW], back) >= 8);
    // The players heard nothing of a watcher who came back in time.
    for who in [ANDREW, BOB] {
        let said = p[who].h.said_since(from[who]);
        assert!(
            !said.iter().any(|s| s.contains("carol")),
            "{who}: {said:#?}"
        );
    }
    server.shutdown();
}

/// What `who` hears when they ask to watch `room` on the server at `addr`.
fn asks_to_watch(who: &str, addr: &str, room: soundrts_protocol::RoomId) -> Vec<String> {
    let mut h = Person::new(who).h;
    h.connect_to(addr, |c| c);
    let from = h.utterances().len();
    soundrts::net::send(&mut h.client, ClientMsg::Watch { room });
    let start = Instant::now();
    while h.said_since(from).is_empty() {
        assert!(start.elapsed() < WAIT_LIMIT, "nothing said");
        soundrts::net::pump(&mut h.client);
        h.run_voice_for(0.05);
    }
    h.run_voice_for(5.0);
    h.said_since(from)
}

#[test]
fn a_watcher_too_many_or_unwelcome_hears_why() {
    let server = test_server(SEED, |c| c.max_watchers = 1);
    let addr = server.local_addr().to_string();
    let mut andrew = Person::new("andrew").h;
    andrew.connect_to(&addr, |c| c);
    let room = andrew.create_room("jl1", &[AiLevel::Easy], Speed::Fast4, true, true);
    let mut carol = Person::new("carol").h;
    carol.connect_to(&addr, |c| c);
    carol.watch_room(room);
    let said = asks_to_watch("dave", &addr, room);
    assert!(
        said.iter()
            .any(|s| s.ends_with("no room for another watcher")),
        "{said:#?}"
    );

    let mut zoe = Person::new("zoe").h;
    zoe.connect_to(&addr, |c| c);
    let unwatched = zoe.create_room("jl1", &[AiLevel::Easy], Speed::Fast4, true, false);
    let said = asks_to_watch("eve", &addr, unwatched);
    assert!(
        said.iter()
            .any(|s| s.ends_with("this game takes no watchers")),
        "{said:#?}"
    );
    server.shutdown();
}
