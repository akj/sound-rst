//! Players on a real server, in-process: each client runs on its own
//! thread with its own harness, its keys scripted and waiting for what it
//! hears, since the server keeps real time. Nothing leaves 127.0.0.1.

use std::cell::RefCell;
use std::net::Ipv4Addr;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use soundrts::client::GameHook;
use soundrts::config::Config;
use soundrts::input::Key;
use soundrts::mainmenu::main_menu;
use soundrts::menu::py_repr;
use soundrts::testkit::{Harness, Script, WAIT_LIMIT};
use soundrts_protocol::{Digest, MapHash, Turn};

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
    let map_hash = MapHash::of(&private_map());

    let andrew_progress = progress.clone();
    let andrew = std::thread::spawn(move || {
        let mut s = keys(Script::new(), &[Key::Down, Key::Down, Key::Return]);
        s = s
            .at(1.5)
            .tap(Key::Down)
            .at(1.8)
            .tap(Key::Down)
            .at(2.1)
            .tap(Key::Down)
            .at(2.4)
            .tap(Key::Return);
        s = s.wait_for("welcome andrew on the server of andrew");
        // The welcome takes the key pressed while it is said.
        s = s.wait_for_exact("please make a selection");
        // start a game on > the private map > normal speed
        s = keys(s, &[Key::Down, Key::Return]);
        // the private map is remembered, so first
        s = s.at(1.2).tap(Key::Down).at(1.5).tap(Key::Return);
        // The map's sound plays before the speed menu.
        s = s.wait_for("start a game on <1097> zzmap 1");
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for_exact("zzmap 1");
        s = s.wait_for("bob has just logged in");
        // invite bob
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for("bob has just joined");
        s = s.at(0.3).tap(Key::F7);
        s = typed(s, 0.6, "héllo bob");
        s = s.wait_for("bob says salut à tous");
        s = s.wait_for("bob has just logged out");
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

    let clipboard = clip_rx.recv().unwrap();
    // Bob comes once Andrew is in his room: the fourth wait.
    let andrew = wait_for_progress(&progress, 4, "andrew", andrew);
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
        let mut s = keys(
            Script::new(),
            &[
                Key::Down,
                Key::Down,
                Key::Return,
                Key::Down,
                Key::Down,
                Key::Return,
            ],
        );
        s = typed(s, 2.2, &bob_address);
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
fn two_players_play_a_game_in_step_and_leave_it() {
    let server = soundrts_server::start(soundrts_server::ServerConfig {
        name: "test".into(),
        ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        ..soundrts_server::ServerConfig::default()
    })
    .expect("a server");
    let address = server.local_addr().to_string();
    let progress = Arc::new(AtomicUsize::new(0));

    /// What a player's game hook gathers: the digests sent, and whether
    /// the game is over for them.
    type Digests = Rc<RefCell<Vec<(Turn, Digest)>>>;

    /// Moves the player's peasants to the next square and back, every ten
    /// turns, and quits at turn `quit_at` if given.
    fn orders_hook(digests: Digests, quit_at: Option<i64>) -> GameHook {
        let done = std::cell::Cell::new(0);
        Rc::new(move |g, cx| {
            if let Some(net) = &g.net {
                *digests.borrow_mut() = net.digests.clone();
            }
            let turn = g.world.turn();
            if turn == 0 || turn % 10 != 0 || done.get() == turn {
                return;
            }
            done.set(turn);
            if quit_at.is_some_and(|q| turn >= q) {
                g.write_line(cx, "quit");
                return;
            }
            let me = g.me;
            let units: Vec<_> = g
                .world
                .player(me)
                .units
                .iter()
                .filter_map(|&u| g.world.entity(u))
                .filter(|e| &*e.type_name == "peasant")
                .map(|e| (e.id.0, e.square()))
                .collect();
            let Some((_, Some(home))) = units.first().copied() else {
                return;
            };
            let sq = g.world.square(home);
            let next = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .find_map(|(dc, dr)| g.world.square_at(sq.col + dc, sq.row + dr));
            let Some(next) = next else { return };
            let target = if (turn / 10) % 2 == 1 { next } else { home };
            let name = g.world.square(target).name.clone();
            let ids: Vec<String> = units.iter().map(|(id, _)| id.to_string()).collect();
            g.write_line(cx, &format!("control {}", ids.join(" ")));
            g.write_line(cx, &format!("order 0 0 go {name}"));
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
        s = s.wait_for("bob has just logged in");
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for("bob is ready");
        s = s.at(0.3).type_text("s").at(0.6).tap(Key::Return);
        s = s.wait_for("Menu please make a selection");
        let mut h = harness("andrew", s.progress(andrew_progress));
        let digests: Digests = Rc::default();
        h.client.game.hook = Some(orders_hook(digests.clone(), Some(120)));
        let maps = h.client.res.multiplayer_maps();
        let jl1 = maps.iter().find(|m| m.name == "jl1").unwrap();
        std::fs::write(
            h.client.paths.tmp().join("mapmenu.txt"),
            py_repr(&jl1.title),
        )
        .unwrap();
        let _ = soundrts::multiplayer::connect(&mut h.client, &andrew_address, None);
        let digests = digests.borrow().clone();
        (h.said_since(0), h.log_text(), digests)
    });

    // Bob comes once Andrew is in his room: the second wait.
    let andrew = wait_for_progress(&progress, 2, "andrew", andrew);
    let bob_address = address.clone();
    let bob = std::thread::spawn(move || {
        let mut s = Script::new().wait_for("andrew invites you");
        s = keys(s, &[Key::Down, Key::Return]);
        s = s.wait_for_exact("jl1 1");
        // "I am ready"
        s = s.at(0.3).type_text("i").at(0.6).tap(Key::Return);
        s = s.wait_for("Menu please make a selection");
        let mut h = harness("bob", s);
        let digests: Digests = Rc::default();
        h.client.game.hook = Some(orders_hook(digests.clone(), None));
        let _ = soundrts::multiplayer::connect(&mut h.client, &bob_address, None);
        let digests = digests.borrow().clone();
        (h.said_since(0), h.log_text(), digests)
    });

    let (bob_said, bob_log, bob_digests) = join("bob", bob);
    let (andrew_said, andrew_log, andrew_digests) = join("andrew", andrew);
    server.shutdown();

    assert!(
        andrew_digests.len() >= 100,
        "{} turns",
        andrew_digests.len()
    );
    let common = andrew_digests.len().min(bob_digests.len());
    assert!(common >= 100, "{common} turns in common");
    for i in 0..common {
        assert_eq!(andrew_digests[i].0, i as Turn);
        assert_eq!(bob_digests[i].0, i as Turn);
        assert_eq!(
            andrew_digests[i].1, bob_digests[i].1,
            "the digests differ after turn {i}"
        );
    }
    for (who, log, said) in [
        ("andrew", &andrew_log, &andrew_said),
        ("bob", &bob_log, &bob_said),
    ] {
        assert!(!log.contains("out of sync"), "{who}: {log}");
        assert!(!log.contains("connection closed"), "{who}: {log}");
        assert!(
            !said.iter().any(|s| s.contains("out of sync")),
            "{who}: {said:#?}"
        );
        assert!(
            said.iter().any(|s| s.starts_with("objective")),
            "{who}: {said:#?}"
        );
    }
    // Andrew quit; Bob saw it and won.
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
        name: "the test server".into(),
        ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        ..soundrts_server::ServerConfig::default()
    })
    .expect("a server");
    let addr = server.local_addr();
    let found = soundrts::net::discovery::discover(&[addr], Duration::from_millis(500));
    assert_eq!(
        found,
        [soundrts::net::discovery::FoundServer {
            addr,
            name: "the test server".into(),
            players: 0,
        }]
    );
}
