//! Players on a real server, in-process: each client runs on its own
//! thread with its own harness, its keys scripted and waiting for what it
//! hears, since the server keeps real time. Nothing leaves 127.0.0.1. The
//! last test plays one game here and on a server, and hears the same.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use std::collections::VecDeque;

use soundrts::client::GameHook;
use soundrts::clock::{Clock, ManualClock};
use soundrts::config::Config;
use soundrts::input::{Key, Mods};
use soundrts::mainmenu::main_menu;
use soundrts::menu::py_repr;
use soundrts::message::{Category, Policy};
use soundrts::net::conn::{Connection, Link, NetEvent, Outgoing};
use soundrts::testkit::{Harness, LiveKeys, Script, WAIT_LIMIT, controller, slot};
use soundrts_protocol::game::SeatFrame;
use soundrts_protocol::{AiLevel, Login, MapHash, SlotPlayer, Speed};
use soundrts_world::orders::Keyword;
use soundrts_world::{
    Command, EntityId, GameSetup, KeywordOrder, Order, OrderTarget, Place, Queue, SquareId,
};

/// Speech lasts a tenth of its usual time, as the clock is real.
fn config(login: &str) -> Config {
    Config {
        login: soundrts::config::Login::parse(login).unwrap(),
        wait_delay_per_character: 0.01,
        ..Config::default()
    }
}

/// A harness on real time, as the server is.
fn harness(login: &str, script: Script) -> Harness {
    let h = Harness::with_config("en", script, config(login));
    h.clock.follow_real_time();
    h
}

/// Keys 0.3 s apart (a menu drops the keys that pile up).
fn keys(mut s: Script, list: &[Key]) -> Script {
    for (i, k) in list.iter().enumerate() {
        s = s.at(0.3 + 0.3 * i as f64).tap(*k);
    }
    s
}

/// From the main menu to the multiplayer menu.
fn main_menu_to_multiplayer() -> Script {
    let s = Script::new().wait_for("please make a selection");
    keys(s, &[Key::Down, Key::Down, Key::Return]).wait_for_exact("please make a selection")
}

/// Typed text from `start` on, then Enter: a prompt reads one key at a
/// time.
fn typed(mut s: Script, start: f64, text: &str) -> Script {
    let mut t = start;
    for c in text.chars() {
        s = s.at(t).type_text(&c.to_string());
        t += 0.1;
    }
    s.at(t + 0.2).tap(Key::Return)
}

/// Waits until `progress` reaches `n`; if it does not in time, fails the
/// test with what the thread that should have reached it says.
fn wait_for_progress<T>(
    progress: &AtomicUsize,
    n: usize,
    name: &str,
    t: std::thread::JoinHandle<T>,
) -> std::thread::JoinHandle<T> {
    let start = Instant::now();
    while progress.load(Ordering::SeqCst) < n {
        if start.elapsed() > WAIT_LIMIT * 2 || t.is_finished() {
            join(name, t);
            panic!("{name} stopped before step {n}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    t
}

/// Runs a thread, and fails the test with its panic if it fails.
fn join<T>(name: &str, t: std::thread::JoinHandle<T>) -> T {
    match t.join() {
        Ok(v) => v,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            panic!("{name} failed: {msg}");
        }
    }
}

/// jl1 with a comment added: a map only Andrew has.
fn private_map() -> Vec<u8> {
    let root = soundrts::testkit::upstream_root();
    let mut bytes = std::fs::read(root.join("res/multi/jl1.txt")).unwrap();
    bytes.extend_from_slice(b"\n; a map of Andrew's own\n");
    bytes
}

#[test]
fn a_player_hosts_another_joins_by_address_and_they_chat() {
    let progress = Arc::new(AtomicUsize::new(0));
    let (clip_tx, clip_rx) = std::sync::mpsc::channel();
    let (room_tx, room_rx) = std::sync::mpsc::channel();
    let map_hash = MapHash::of(&private_map());

    let andrew_progress = progress.clone();
    let andrew = std::thread::spawn(move || {
        // multiplayer > server
        let mut s = main_menu_to_multiplayer();
        s = keys(s, &[Key::Down, Key::Down, Key::Down, Key::Return]);
        s = s.wait_for("welcome andrew on the server of andrew");
        // The welcome takes the key pressed while it is said.
        s = s.wait_for_exact("please make a selection");
        // start a game on > the private map > normal speed
        s = keys(s, &[Key::Down, Key::Return]);
        // the private map is remembered: the menu opens on it
        s = s.at(1.2).tap(Key::Down).at(1.5).tap(Key::Return);
        // The map's sound plays before the speed menu.
        s = s.wait_for("start a game on <1097> zzmap 1");
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for_exact("zzmap 1");
        let in_room = s.wait_count();
        s = s.wait_for("bob has just logged in");
        // invite bob
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for("bob has just joined");
        s = s.at(0.3).tap(Key::F7);
        s = typed(s, 0.6, "héllo bob");
        s = s.wait_for("bob says salut à tous");
        s = s.wait_for("bob has just logged out");
        room_tx.send(in_room).unwrap();
        let mut h = harness("andrew", s.progress(andrew_progress));
        let multi = h.client.paths.downloaded().join("multi");
        std::fs::write(multi.join("zzmap.txt"), private_map()).unwrap();
        h.client.res.forget_multiplayer_maps();
        let maps = h.client.res.multiplayer_maps();
        let zz = maps.iter().find(|m| m.name == "zzmap").unwrap();
        std::fs::write(h.client.paths.tmp().join("mapmenu.txt"), py_repr(&zz.title)).unwrap();
        clip_tx.send(h.desktop.clipboard.clone()).unwrap();
        let _ = main_menu(&mut h.client);
        (h.said_since(0), h.log_text())
    });

    let in_room = room_rx.recv().unwrap();
    let clipboard = clip_rx.recv().unwrap();
    // Bob comes once Andrew is in his room.
    let andrew = wait_for_progress(&progress, in_room, "andrew", andrew);
    let address = clipboard
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("an address copied");
    assert!(address.starts_with("127.0.0.1:"), "{address}");

    let bob_address = address.clone();
    let bob_hash = map_hash.clone();
    let bob = std::thread::spawn(move || {
        // multiplayer > enter the address
        let mut s = main_menu_to_multiplayer();
        s = keys(s, &[Key::Down, Key::Down, Key::Return]);
        s = s.wait_for("enter the I P address of the server");
        s = typed(s, 0.3, &bob_address);
        s = s.wait_for("andrew invites you");
        // accept the invitation
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for("andrew says héllo bob");
        s = s.at(0.3).tap(Key::F7);
        s = typed(s, 0.6, "salut à tous");
        s = s.wait_for("bob says salut à tous");
        let mut h = harness("bob", s);
        let _ = main_menu(&mut h.client);
        let found = h.client.res.find_multiplayer_map(&bob_hash).is_some();
        let saved: Vec<String> = std::fs::read_dir(h.client.paths.downloaded().join("multi"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        (h.said_since(0), found, saved)
    });

    let (bob_said, found, saved) = join("bob", bob);
    let (andrew_said, andrew_log) = join("andrew", andrew);

    let has = |said: &[String], text: &str| said.iter().any(|s| s == text);
    let said_like = |said: &[String], text: &str| said.iter().any(|s| s.contains(text));
    // Andrew heard the address to give, Bob arrive and join, and Bob's
    // line with its sound.
    assert!(
        said_like(&andrew_said, "the address to give to the other players is"),
        "{andrew_said:#?}"
    );
    assert!(
        said_like(&andrew_said, "bob has just joined 2 players on 2"),
        "{andrew_said:#?}"
    );
    assert!(
        has(&andrew_said, "<6114> bob says salut à tous"),
        "{andrew_said:#?}"
    );
    // Bob downloaded the map, kept it, and heard Andrew's line.
    assert!(has(&bob_said, "Downloading..."), "{bob_said:#?}");
    assert!(
        has(&bob_said, "<6114> andrew says héllo bob"),
        "{bob_said:#?}"
    );
    let expected = format!("zzmap_{}.txt", &map_hash.as_str()[..8]);
    assert_eq!(saved, [expected]);
    assert!(found, "the map is among Bob's maps for the next time");
    assert!(!andrew_log.contains("connection closed"), "{andrew_log}");
}

#[test]
fn two_players_play_a_game_on_a_server_and_leave_it() {
    let server = soundrts_server::start(soundrts_server::ServerConfig {
        name: soundrts_protocol::ServerName::new("test").unwrap(),
        ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        ..soundrts_server::ServerConfig::default()
    })
    .expect("a server");
    let address = server.local_addr().to_string();
    let progress = Arc::new(AtomicUsize::new(0));
    let (room_tx, room_rx) = std::sync::mpsc::channel();

    /// What a player's game hook gathers from what the player knows: the
    /// last turn taken, the square its peasants were sent to, and every
    /// square where it saw its own peasants.
    #[derive(Default)]
    struct Seen {
        turn: i64,
        target: Option<String>,
        squares: BTreeSet<String>,
    }
    type Record = Rc<RefCell<Seen>>;

    /// Sends the player's peasants to the next square at turn 10 and back
    /// at turn 70, and quits at turn `quit_at` if given.
    fn orders_hook(record: Record, quit_at: Option<i64>) -> GameHook {
        let done = std::cell::Cell::new(0);
        Rc::new(move |g, cx| {
            let view = g.view();
            let turn = view.turn();
            let peasants: Vec<(EntityId, Option<SquareId>)> = view
                .own_units()
                .filter(|e| &*e.type_name == "peasant")
                .map(|e| match e.place {
                    Some(Place::Square(sq)) => (e.id, Some(sq)),
                    _ => (e.id, None),
                })
                .collect();
            {
                let mut r = record.borrow_mut();
                r.turn = turn;
                for &(_, sq) in &peasants {
                    if let Some(sq) = sq {
                        r.squares.insert(view.square(sq).name().to_owned());
                    }
                }
            }
            if done.get() == turn || !matches!(turn, 10 | 70) && quit_at.is_none_or(|q| turn != q) {
                return;
            }
            done.set(turn);
            if quit_at == Some(turn) {
                g.send_command(cx, Command::Quit);
                return;
            }
            let Some((_, Some(home))) = peasants.first().copied() else {
                return;
            };
            let target = if turn == 10 {
                let sq = view.square(home);
                let Some(next) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .find_map(|(dc, dr)| view.square_at(sq.col() + dc, sq.row() + dr))
                else {
                    return;
                };
                record.borrow_mut().target = Some(view.square(next).name().to_owned());
                next
            } else {
                home
            };
            let go = KeywordOrder::new(Keyword::Go).expect("go takes no type");
            g.send_command(
                cx,
                Command::Control(peasants.iter().map(|&(id, _)| id).collect()),
            );
            g.send_command(
                cx,
                Command::Order {
                    order: Order::Keyword(go.at(OrderTarget::Square(target))),
                    queue: Queue::Replace,
                    imperative: false,
                },
            );
        })
    }

    let andrew_address = address.clone();
    let andrew_progress = progress.clone();
    let andrew = std::thread::spawn(move || {
        let mut s = Script::new().wait_for("please make a selection");
        // start a public game on > jl1 (remembered) > fast speed (x4)
        s = keys(
            s,
            &[
                Key::Down,
                Key::Down,
                Key::Return,
                Key::Down,
                Key::Return,
                Key::Down,
                Key::Down,
                Key::Down,
                Key::Return,
            ],
        );
        s = s.wait_for_exact("jl1 1");
        room_tx.send(s.wait_count()).unwrap();
        s = s.wait_for("bob has just logged in");
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for("bob is ready");
        s = s.at(0.3).type_text("s").at(0.6).tap(Key::Return);
        s = s.wait_for("Menu please make a selection");
        let mut h = harness("andrew", s.progress(andrew_progress));
        let record: Record = Rc::default();
        h.client.game.hook = Some(orders_hook(record.clone(), Some(120)));
        let maps = h.client.res.multiplayer_maps();
        let jl1 = maps.iter().find(|m| m.name == "jl1").unwrap();
        std::fs::write(
            h.client.paths.tmp().join("mapmenu.txt"),
            py_repr(&jl1.title),
        )
        .unwrap();
        let _ = soundrts::multiplayer::connect(&mut h.client, &andrew_address, None);
        let r = record.borrow();
        (
            h.said_since(0),
            h.log_text(),
            r.turn,
            r.target.clone(),
            r.squares.clone(),
        )
    });

    // Bob comes once Andrew is in his room.
    let in_room = room_rx.recv().unwrap();
    let andrew = wait_for_progress(&progress, in_room, "andrew", andrew);
    let bob_address = address.clone();
    let bob = std::thread::spawn(move || {
        let mut s = Script::new().wait_for("andrew invites you");
        s = keys(s, &[Key::Down, Key::Return]);
        // Who joins hears whether the room takes watchers.
        s = s.wait_for_exact("jl1 1 , watchers welcome");
        // "I am ready"
        s = s.at(0.3).type_text("i").at(0.6).tap(Key::Return);
        s = s.wait_for("Menu please make a selection");
        let mut h = harness("bob", s);
        let record: Record = Rc::default();
        h.client.game.hook = Some(orders_hook(record.clone(), None));
        let _ = soundrts::multiplayer::connect(&mut h.client, &bob_address, None);
        let r = record.borrow();
        (
            h.said_since(0),
            h.log_text(),
            r.turn,
            r.target.clone(),
            r.squares.clone(),
        )
    });

    let (bob_said, bob_log, bob_turn, bob_target, bob_seen) = join("bob", bob);
    let (andrew_said, andrew_log, andrew_turn, andrew_target, andrew_seen) = join("andrew", andrew);
    server.shutdown();

    // Andrew quit at turn 120; Bob's game ended then, with his victory.
    assert!(andrew_turn >= 120, "andrew: {andrew_turn} turns");
    assert!(bob_turn >= 120, "bob: {bob_turn} turns");
    for (who, log, said) in [
        ("andrew", &andrew_log, &andrew_said),
        ("bob", &bob_log, &bob_said),
    ] {
        assert!(!log.contains("connection closed"), "{who}: {log}");
        assert!(!log.contains("cannot be right"), "{who}: {log}");
        assert!(
            said.iter().any(|s| s.starts_with("objective")),
            "{who}: {said:#?}"
        );
    }
    // The commands had effect: each player saw their own peasants reach
    // the square they were sent to.
    for (who, target, seen) in [
        ("andrew", andrew_target, &andrew_seen),
        ("bob", bob_target, &bob_seen),
    ] {
        let target = target.unwrap_or_else(|| panic!("{who} gave no order"));
        assert!(seen.contains(&target), "{who} never saw {target}: {seen:?}");
    }
    assert!(
        bob_said
            .iter()
            .any(|s| s.contains("andrew has just quit the game")),
        "{bob_said:#?}"
    );
}

#[test]
fn a_server_on_the_local_network_answers_the_discovery() {
    let server = soundrts_server::start(soundrts_server::ServerConfig {
        name: soundrts_protocol::ServerName::new("the test server").unwrap(),
        ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        ..soundrts_server::ServerConfig::default()
    })
    .expect("a server");
    let addr = server.local_addr();
    // UDP may lose a request or be slow on a busy machine: ask again, as a
    // player would.
    let mut found = Vec::new();
    for _ in 0..10 {
        found = soundrts::net::discovery::discover(&[addr], Duration::from_millis(500));
        if !found.is_empty() {
            break;
        }
    }
    assert_eq!(
        found,
        [soundrts::net::discovery::FoundServer {
            addr,
            name: soundrts_protocol::ServerName::new("the test server").unwrap(),
            players: 0,
        }]
    );
}

// -- The same game, here and on a server ----------------------------------

/// A connection that gives the client each turn of the server's stream at
/// a set time on the test clock, the time a game run here took the same
/// turn, and holds the turns after the last. Everything else passes as it
/// comes. When a turn is due in the game joined and has not come yet, it
/// waits for it in real time.
struct PacedLink {
    inner: Connection,
    clock: Rc<ManualClock>,
    /// When each turn not yet given is due, in order.
    due: VecDeque<f64>,
    buffer: VecDeque<NetEvent>,
    /// The game time of the last turn given.
    time: i64,
    /// The seat's whole message has been given: the game is joined.
    joined: bool,
}

impl PacedLink {
    fn turn_due(&self) -> bool {
        self.joined && self.due.front().is_some_and(|&at| self.clock.now() >= at)
    }

    /// The game time of the change at the front, if a change is there.
    fn front_change(&self) -> Option<i64> {
        match self.buffer.front() {
            Some(NetEvent::Frame(Ok(f))) => match &**f {
                SeatFrame::Change(c) => Some(c.time),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Link for PacedLink {
    fn send(&mut self, out: Outgoing) {
        match out {
            Outgoing::Msg(m) => self.inner.send(m),
            Outgoing::Command(c) => self.inner.send_command(c),
        }
    }

    fn poll(&mut self) -> Option<NetEvent> {
        let start = Instant::now();
        loop {
            while let Some(e) = self.inner.poll() {
                self.buffer.push_back(e);
            }
            match self.front_change() {
                // What came with the turn just given.
                Some(t) if t == self.time => return self.buffer.pop_front(),
                Some(t) if self.turn_due() => {
                    self.due.pop_front();
                    self.time = t;
                    return self.buffer.pop_front();
                }
                Some(_) => return None,
                None if !self.buffer.is_empty() => {
                    let e = self.buffer.pop_front();
                    if let Some(NetEvent::Frame(Ok(f))) = &e
                        && let SeatFrame::Whole(_) = &**f
                    {
                        self.joined = true;
                    }
                    return e;
                }
                None if !self.turn_due() => return None,
                None => {
                    assert!(start.elapsed() < WAIT_LIMIT, "the server sent no turn");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

/// Keys that ask questions and move the cursor, and change nothing in the
/// game: the same in both games, at the same times. Not the keys of what
/// was missed: the game on a server has the room's news unheard before it.
fn questions(live: &LiveKeys) {
    let keys = [
        (Key::B, Mods::NONE),
        (Key::N, Mods::NONE),
        (Key::X, Mods::SHIFT),
        (Key::F12, Mods::NONE),
        (Key::Right, Mods::NONE),
        (Key::Down, Mods::NONE),
        (Key::Tab, Mods::NONE),
        (Key::V, Mods::NONE),
        (Key::M, Mods::NONE),
        (Key::Left, Mods::NONE),
        (Key::F11, Mods::NONE),
        (Key::Up, Mods::NONE),
    ];
    for (i, &(key, mods)) in keys.iter().enumerate() {
        live.tap_at(12.0 + 1.5 * i as f64, key, mods);
    }
}

/// The settings of [`config`] with the story kept silently in the log: a
/// game run here waits, on purpose, while its story is told, and a game on
/// a server does not.
fn untold(login: &str) -> Config {
    let mut c = config(login);
    c.news.set(Category::Story, Policy::Log);
    c
}

/// Notes the time each turn is taken, from now on.
fn record_turns(h: &mut Harness) -> Rc<RefCell<Vec<f64>>> {
    let turns: Rc<RefCell<Vec<f64>>> = Rc::default();
    let seen = turns.clone();
    h.client.game.hook = Some(Rc::new(move |g, cx| {
        let mut t = seen.borrow_mut();
        if g.view().turn() > t.len() as i64 {
            t.push(cx.clock.now());
        }
    }));
    turns
}

/// What was said since the `from`th utterance, and every sound event.
fn heard(h: &Harness, from: usize) -> (Vec<String>, Vec<String>) {
    let played = h.audio.events().iter().map(|e| format!("{e:?}")).collect();
    (h.said_since(from), played)
}

#[test]
fn a_game_on_a_server_is_heard_as_the_same_game_played_here() {
    const SEED: i64 = 3;
    const TURNS: usize = 400;
    let slots = [
        slot(SlotPlayer::Human(Login::new("andrew").unwrap()), 1),
        slot(SlotPlayer::Computer(AiLevel::Aggressive), 2),
    ];

    // Here: a host in the client, built as the server builds the game.
    let live = LiveKeys::default();
    questions(&live);
    let script = Script::new().keep_open().with_live(live.clone());
    let mut h = Harness::with_config("en", script, untold("andrew"));
    let map = h.multiplayer_map("jl1");
    let texts = soundrts::multiplayer::game_texts(&mut h.client, &map).unwrap();
    let setup = GameSetup::multiplayer(
        texts.rules.into_string(),
        texts.ai.into_string(),
        texts.map.into_string(),
        SEED,
        slots.iter().map(controller).collect(),
    );
    let mut g = h.start_local("jl1", &setup);
    g.game.speed = Speed::Fast4.factor();
    let from = h.utterances().len();
    h.audio.clear();
    let turns = record_turns(&mut h);
    while g.game.view().turn() < TURNS as i64 {
        h.step(&mut g);
    }
    // The server's turns after the last are held: none is taken here
    // either.
    g.game.set_turn_due(f64::INFINITY);
    let here_turns = turns.take();
    let end = here_turns[TURNS - 1] + 5.0;
    while h.clock.now() < end {
        h.step(&mut g);
    }
    let (here_said, here_played) = heard(&h, from);
    assert!(live.is_empty(), "every key was read here");

    // On a server: the same game, each turn taken when the game here took
    // it.
    let live = LiveKeys::default();
    questions(&live);
    let script = Script::new().keep_open().with_live(live.clone());
    let mut h = Harness::with_config("en", script, untold("andrew"));
    let clock = h.clock.clone();
    let due: VecDeque<f64> = here_turns.iter().copied().collect();
    let (server, mut g) = h.start_on_server_with(
        "jl1",
        &[AiLevel::Aggressive],
        Speed::Fast4,
        SEED,
        move |inner| {
            Connection::new(Box::new(PacedLink {
                inner,
                clock,
                due,
                buffer: VecDeque::new(),
                time: 0,
                joined: false,
            }))
        },
    );
    let room = h.client.net.as_ref().unwrap().room.as_ref().unwrap();
    assert_eq!(
        room.state.slots, slots,
        "the server's room is the game here"
    );
    let from = h.utterances().len();
    h.audio.clear();
    let turns = record_turns(&mut h);
    while h.clock.now() < end {
        assert!(!g.game.end_loop, "the game ended");
        h.step(&mut g);
    }
    let (there_said, there_played) = heard(&h, from);
    server.shutdown();
    assert!(live.is_empty(), "every key was read on the server");

    assert_eq!(*turns.borrow(), here_turns, "the turns were taken alike");
    assert!(
        here_said.len() > 10,
        "too little heard to compare: {here_said:#?}"
    );
    assert_eq!(there_said, here_said, "what was said differs");
    assert_eq!(there_played, here_played, "what was played differs");
}
