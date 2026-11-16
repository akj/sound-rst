//! The multiplayer menus and games against a fake server: a script of
//! server messages and frames at set times on the test clock, the frames
//! made by a game run here as a server runs one. What the client says and
//! sends is checked. The games whose turns run play on a real server on
//! 127.0.0.1, as does `net_play.rs`.

use soundrts::clock::Clock;
use soundrts::config::Config;
use soundrts::input::{Key, Mods};
use soundrts::multiplayer::play_on;
use soundrts::testkit::{FakeServer, FakeStream, Harness, LiveKeys, Script, TestGame, snapshot};
use soundrts_protocol::game::SeatFrame;
use soundrts_protocol::*;
use soundrts_world::{Command, GameStart, Order, OrderTarget, PlayerId, Queue, Rules, SquareId};

fn login(s: &str) -> Login {
    Login::new(s).unwrap()
}

fn chat(from: &str, text: &str) -> ServerMsg {
    ServerMsg::Chat {
        line: ChatLine {
            channel: Channel::Lobby,
            from: login(from),
            watcher: false,
            text: ChatText::clean(text).unwrap(),
        },
    }
}

fn arrived(l: &str) -> ServerMsg {
    ServerMsg::Lobby {
        change: LobbyChange::PlayerArrived {
            player: LobbyPlayer {
                login: login(l),
                whereabouts: Whereabouts::Lobby,
            },
        },
    }
}

fn config(login: &str) -> Config {
    Config {
        login: soundrts::config::Login::parse(login).unwrap(),
        ..Config::default()
    }
}

/// A client that has just joined a fake server as "andrew", with "bob"
/// there, and the server; `schedule` sets what the server sends.
fn on_fake_server(
    script: Script,
    config: Config,
    schedule: impl FnOnce(&mut Harness, &FakeServer),
) -> (Harness, FakeServer) {
    let mut h = Harness::with_config("en", script, config);
    let (server, conn) = FakeServer::new(h.clock.clone());
    schedule(&mut h, &server);
    let content = h.client.res.content_hash();
    // The script ends by closing the window.
    let result = play_on(
        &mut h.client,
        conn,
        snapshot("andrew", &["bob"]),
        content,
        None,
        server.redial(),
    );
    assert!(result.is_err(), "the window closed");
    (h, server)
}

fn said(h: &Harness) -> Vec<String> {
    h.said_since(0)
}

/// The utterances that follow `first`, `first` included.
fn from<'a>(said: &'a [String], first: &str) -> &'a [String] {
    let i = said
        .iter()
        .position(|s| s == first)
        .unwrap_or_else(|| panic!("{first:?} not said in {said:#?}"));
    &said[i..]
}

#[test]
fn arrivals_and_departures_are_said_in_the_lobby() {
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(10.0, arrived("carol"));
        s.at(
            15.0,
            ServerMsg::Lobby {
                change: LobbyChange::PlayerLeft {
                    login: login("bob"),
                },
            },
        );
        s.at(
            20.0,
            ServerMsg::Lobby {
                change: LobbyChange::PlayerMoved {
                    player: LobbyPlayer {
                        login: login("carol"),
                        whereabouts: Whereabouts::InRoom(RoomId(1)),
                    },
                },
            },
        );
    });
    assert_eq!(
        said(&h),
        [
            "welcome andrew on the server of test",
            "please make a selection",
            "carol has just logged in",
            "bob has just logged out",
        ]
    );
}

#[test]
fn arrivals_and_departures_are_not_said_with_the_option_off() {
    let mut c = config("andrew");
    c.announce_arrivals = false;
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, c, |_, s| {
        s.at(10.0, arrived("carol"));
        s.at(
            15.0,
            ServerMsg::Lobby {
                change: LobbyChange::PlayerLeft {
                    login: login("bob"),
                },
            },
        );
        s.at(20.0, chat("carol", "still said"));
    });
    assert_eq!(
        said(&h),
        [
            "welcome andrew on the server of test",
            "please make a selection",
            "<6114> carol says still said",
        ]
    );
}

/// jl1 as a host describes it.
fn jl1(h: &mut Harness) -> MapInfo {
    h.client
        .res
        .multiplayer_maps()
        .into_iter()
        .find(|m| m.name == "jl1")
        .unwrap()
        .info()
        .unwrap()
}

fn slot(player: SlotPlayer, alliance: u8, ready: bool) -> Slot {
    Slot {
        player,
        faction: Faction::random(),
        alliance,
        ready,
        income: Income::FULL,
    }
}

fn human(l: &str) -> SlotPlayer {
    SlotPlayer::Human(login(l))
}

fn room_state(map: &MapInfo, slots: Vec<Slot>) -> RoomState {
    RoomState {
        id: RoomId(1),
        host: login("andrew"),
        map: map.clone(),
        speed: Speed::Normal,
        open: true,
        content: ContentHash::new("x").unwrap(),
        takes_watchers: true,
        watchers: Vec::new(),
        slots,
    }
}

fn room(change: RoomChange, state: &RoomState) -> ServerMsg {
    ServerMsg::Room {
        change,
        room: state.clone(),
    }
}

#[test]
fn what_changes_in_the_room_is_said() {
    let script = Script::new().at(100.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        let alone = room_state(&map, vec![slot(human("andrew"), 1, true)]);
        let mut two = alone.clone();
        two.slots.push(slot(human("bob"), 2, false));
        s.at(10.0, room(RoomChange::Entered, &alone));
        s.at(
            15.0,
            room(
                RoomChange::Joined {
                    player: human("bob"),
                },
                &two,
            ),
        );
        let mut ready = two.clone();
        ready.slots[1].ready = true;
        s.at(35.0, room(RoomChange::Ready { slot: 1 }, &ready));
        let mut allied = ready.clone();
        allied.slots[1].alliance = 1;
        s.at(45.0, room(RoomChange::Alliance { slot: 1 }, &allied));
        let mut faction = allied.clone();
        faction.slots[1].faction = Faction::new("human_faction").unwrap();
        s.at(55.0, room(RoomChange::Faction { slot: 1 }, &faction));
        s.at(
            65.0,
            room(
                RoomChange::Left {
                    player: human("bob"),
                },
                &alone,
            ),
        );
        // Someone joins only to watch, and the room's list names her as a
        // watcher, in no slot.
        let mut watched = alone.clone();
        watched.watchers.push(login("carol"));
        s.at(
            70.0,
            room(
                RoomChange::Watching {
                    login: login("carol"),
                },
                &watched,
            ),
        );
        let mut three = watched.clone();
        three.slots.push(slot(human("bob"), 2, false));
        s.at(
            75.0,
            room(
                RoomChange::Joined {
                    player: human("bob"),
                },
                &three,
            ),
        );
        s.at(
            80.0,
            room(
                RoomChange::StoppedWatching {
                    login: login("carol"),
                },
                &two,
            ),
        );
    });
    assert_eq!(
        from(&said(&h), "jl1 1"),
        [
            "jl1 1",
            "bob has just joined 2 players on 2 the game will start when the organizer is ready . andrew . bob",
            "bob is ready",
            "move bob to alliance 1",
            "bob human",
            "bob has just quit the game . 1 players on 2 Not enough players to start. The minimum is 2 . andrew",
            "carol is watching",
            "bob has just joined 2 players on 2 the game will start when the organizer is ready . andrew . bob . watching: carol",
            "carol stopped watching",
        ]
    );
}

#[test]
fn chat_is_said_while_a_submenu_is_open_before_the_queued_messages() {
    let script = Script::new()
        .at(8.0)
        .tap(Key::Down)
        .at(9.0)
        .tap(Key::Return)
        .at(40.0)
        .tap(Key::Escape);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(15.0, arrived("carol"));
        s.at(15.0, arrived("dave"));
        s.at(15.0, chat("bob", "hi"));
        s.at(15.0, chat("bob", "hi"));
    });
    assert_eq!(
        from(&said(&h), "carol has just logged in"),
        [
            "carol has just logged in",
            "<6114> bob says hi",
            "<6114> bob says hi",
            "dave has just logged in",
            "cancel",
        ]
    );
    assert!(
        h.audio.played().contains(&(0, "6114".to_owned())),
        "the chat sound plays on the voice channel"
    );
}

#[test]
fn chat_is_said_while_the_chat_prompt_is_open_and_the_line_typed_is_sent() {
    let script = Script::new()
        .at(8.0)
        .tap(Key::F7)
        .at(20.0)
        .type_text("y")
        .at(21.0)
        .type_text("é")
        .at(22.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(12.0, chat("bob", "are you there"));
    });
    assert_eq!(
        from(&said(&h), "enter your message"),
        [
            "enter your message",
            "<6114> bob says are you there",
            "y . y",
            "é . yé",
            "yé",
        ]
    );
    assert_eq!(
        server.sent(),
        [ClientMsg::Chat {
            channel: Channel::Lobby,
            text: ChatText::clean("yé").unwrap(),
        }]
    );
}

#[test]
fn the_chat_is_read_again_with_control_f5_and_f6() {
    let script = Script::new()
        .at(20.0)
        .tap_with(Key::F5, Mods::CTRL)
        .at(25.0)
        .tap_with(Key::F5, Mods::CTRL)
        .at(30.0)
        .tap_with(Key::F6, Mods::CTRL)
        .at(35.0)
        .tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(10.0, chat("bob", "one"));
        s.at(10.0, chat("bob", "two"));
    });
    assert_eq!(
        from(&said(&h), "<6114> bob says one"),
        [
            "<6114> bob says one",
            "<6114> bob says two",
            // Read again without the sound.
            "bob says two",
            "bob says one",
            "bob says two",
        ]
    );
}

#[test]
fn a_game_starting_during_the_room_chat_prompt_cancels_it_and_starts() {
    let script = Script::new()
        .at(14.0)
        .tap(Key::F7)
        .at(16.0)
        .type_text("x")
        .at(60.0)
        .tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        game_starting(h, s, 18.0, 0);
    });
    let said = said(&h);
    let typed = from(&said, "x . x");
    // The objective of jl1 is said once the game is built.
    assert!(
        typed.iter().any(|s| s.starts_with("objective")),
        "{said:#?}"
    );
    // The window closed during the game: the server is told.
    assert_eq!(server.sent(), [ClientMsg::LeaveGame]);
}

fn two_slots() -> Vec<Slot> {
    vec![
        slot(human("andrew"), 1, true),
        slot(
            SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)),
            2,
            true,
        ),
    ]
}

/// A room with a computer on jl1, entered at 10 s, whose game the server
/// starts at `start` with `you` as the player's slot: the seat, the start
/// of the game and the seat's whole message. No turn comes.
fn game_starting(h: &mut Harness, s: &FakeServer, start: f64, you: SlotIndex) {
    let map = jl1(h);
    let state = room_state(&map, two_slots());
    s.at(10.0, room(RoomChange::Entered, &state));
    let stream = FakeStream::new(h, "jl1", &two_slots(), you, 1);
    for frame in stream.opening(map, Speed::Normal) {
        s.frame_at(start, frame);
    }
}

/// A game of Andrew against a computer on a fake server, joined, and its
/// stream.
fn fake_game(h: &mut Harness, slots: &[Slot]) -> (FakeServer, FakeStream, TestGame) {
    let server = h.join_fake_server("andrew", &["bob"]);
    let stream = FakeStream::new(h, "jl1", slots, 0, 1);
    let g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
    (server, stream, g)
}

/// Sends the frames of the stream's next turn at `time`.
fn turn_at(server: &FakeServer, stream: &mut FakeStream, time: f64) {
    for frame in stream.turn(Vec::new()) {
        server.frame_at(time, frame);
    }
}

#[test]
fn turns_keep_running_while_a_chat_line_is_typed_in_the_game() {
    let live = LiveKeys::default();
    let mut h = Harness::with_config(
        "en",
        Script::new().keep_open().with_live(live.clone()),
        config("andrew"),
    );
    let (server, mut g) =
        h.start_on_server("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Normal, 1);
    h.clock.follow_real_time();
    let t0 = h.clock.now();
    live.tap_at(t0 + 1.0, Key::F7, Mods::NONE);
    live.type_at(t0 + 2.0, 0.5, "gg wp");
    live.tap_at(t0 + 6.0, Key::Return, Mods::NONE);
    h.run_for(&mut g, 1.5);
    let typing_from = g.game.view().turn();
    h.run_for(&mut g, 4.4);
    let typing_to = g.game.view().turn();
    h.run_for(&mut g, 1.5);
    server.shutdown();
    // 14 turns at normal speed: a slow machine may take fewer.
    assert!(
        typing_to - typing_from >= 10,
        "turns {typing_from} to {typing_to} while typing"
    );
    // The letters went to the line, not to the game's keys, and the line
    // went to the game's players, Andrew among them.
    let said = h.said_since(0);
    assert!(said.contains(&"g . g".to_owned()), "{said:#?}");
    assert!(said.contains(&"gg wp".to_owned()), "{said:#?}");
    assert!(
        said.contains(&"<6114> andrew says gg wp".to_owned()),
        "{said:#?}"
    );
}

#[test]
fn turns_keep_running_while_the_game_menu_is_open() {
    let live = LiveKeys::default();
    let mut h = Harness::with_config(
        "en",
        Script::new().keep_open().with_live(live.clone()),
        config("andrew"),
    );
    let (server, mut g) =
        h.start_on_server("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Normal, 1);
    h.clock.follow_real_time();
    let t0 = h.clock.now();
    live.tap_at(t0 + 1.0, Key::F10, Mods::NONE);
    live.tap_at(t0 + 7.0, Key::Escape, Mods::NONE);
    live.tap_at(t0 + 9.0, Key::F10, Mods::NONE);
    h.run_for(&mut g, 1.5);
    let open_from = g.game.view().turn();
    let said_from = h.said_since(0).len();
    h.run_for(&mut g, 5.0);
    let open_to = g.game.view().turn();
    h.run_for(&mut g, 4.0);
    server.shutdown();
    // 16 turns at normal speed: a slow machine may take fewer.
    assert!(
        open_to - open_from >= 12,
        "turns {open_from} to {open_to} with the menu open"
    );
    assert!(live.is_empty(), "every key was read");
    // The news went on with the menu open; Escape chose to continue, and
    // F10 then went to the game, which opened the menu again. News cut off
    // by either is said again after it.
    let said = h.said_since(said_from);
    let (news, menu): (Vec<_>, Vec<_>) = said.iter().partition(|s| s.contains(" at "));
    assert!(!news.is_empty(), "{said:#?}");
    assert_eq!(menu, ["continue game", "Menu"], "{said:#?}");
}

#[test]
fn turns_that_come_together_are_taken_together() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    let t0 = h.clock.now() + 1.0;
    for _ in 0..3 {
        turn_at(&server, &mut stream, t0);
    }
    while h.clock.now() < t0 {
        h.step(&mut g);
        assert_eq!(g.game.view().turn(), 0);
    }
    h.step(&mut g);
    // The server's clock is the game's: each turn is current as it comes.
    assert_eq!(g.game.view().turn(), 3);
}

#[test]
fn commands_beyond_what_a_turn_takes_wait_for_the_next_turn() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    let command = Command::Order {
        order: Order::ResetGroup("x".repeat(996)),
        queue: Queue::Replace,
        imperative: false,
    };
    let size = game::encode_command(&command).unwrap().len();
    for _ in 0..40 {
        g.game.send_command(&h.client, command.clone());
    }
    let sent_bytes = |server: &FakeServer| -> usize { server.commands().iter().map(|c| c.1).sum() };
    h.run_for(&mut g, 0.5);
    let first = sent_bytes(&server);
    assert!(first > 0 && first <= MAX_TURN_ORDER_BYTES / 2, "{first}");
    assert!(!g.game.pending_commands().is_empty());
    turn_at(&server, &mut stream, h.clock.now());
    h.run_for(&mut g, 0.5);
    turn_at(&server, &mut stream, h.clock.now());
    h.run_for(&mut g, 0.5);
    assert_eq!(sent_bytes(&server), 40 * size);
    assert!(server.commands().iter().all(|c| c.0 == command));
    assert!(g.game.pending_commands().is_empty());
}

#[test]
fn a_game_starting_as_the_player_leaves_the_room_is_played() {
    // Escape and the start in the same pass of the menu.
    let script = Script::new()
        .at(20.0)
        .tap(Key::Escape)
        .at(60.0)
        .tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        game_starting(h, s, 20.0, 0)
    });
    assert_eq!(server.sent(), [ClientMsg::LeaveGame]);
    let said = said(&h);
    assert!(said.iter().any(|s| s.starts_with("objective")), "{said:#?}");
}

#[test]
fn a_game_started_after_the_player_asked_to_leave_is_played() {
    // The server started the game before it read the leave.
    let script = Script::new()
        .at(20.0)
        .tap(Key::Escape)
        .at(60.0)
        .tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        game_starting(h, s, 25.0, 0)
    });
    assert_eq!(server.sent(), [ClientMsg::LeaveRoom, ClientMsg::LeaveGame]);
    let said = said(&h);
    assert!(said.iter().any(|s| s.starts_with("objective")), "{said:#?}");
}

#[test]
fn a_game_on_a_map_the_player_does_not_have_is_left() {
    let script = Script::new().at(40.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        let mut map = jl1(h);
        map.hash = MapHash::of(b"a map nobody has");
        let stream = FakeStream::new(h, "jl1", &two_slots(), 0, 1);
        for frame in stream.opening(map, Speed::Normal) {
            s.frame_at(20.0, frame);
        }
    });
    assert_eq!(server.sent(), [ClientMsg::LeaveGame]);
    let said = said(&h);
    assert!(
        said.contains(&"<1029> this game cannot start here".to_owned()),
        "{said:#?}"
    );
}

/// `rules` with the peasant's definition moved after the footman's: the
/// same types, two of them in each other's place.
fn peasant_after_footman(rules: String) -> String {
    let block = |name: &str| {
        let start = rules.find(&format!("\ndef {name}\n")).unwrap() + 1;
        let end = rules[start..]
            .find("\ndef ")
            .map_or(rules.len(), |e| start + e + 1);
        start..end
    };
    let (peasant, footman) = (block("peasant"), block("footman"));
    assert_eq!(
        peasant.end, footman.start,
        "the footman follows the peasant"
    );
    [
        &rules[..peasant.start],
        &rules[footman.clone()],
        &rules[peasant],
        &rules[footman.end..],
    ]
    .concat()
}

#[test]
fn a_game_whose_rules_differ_from_the_players_is_not_played() {
    let script = Script::new().at(40.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        s.at(
            10.0,
            room(RoomChange::Entered, &room_state(&map, two_slots())),
        );
        let stream = FakeStream::with_rules(h, "jl1", &two_slots(), 0, 1, peasant_after_footman);
        // What the check prevents: read with the player's own rules, the
        // stream passes every other check, and names peasants footmen.
        let ours = Rules::load(&[h.client.res.rules_text()]);
        let game = &stream.host().game_start().rules;
        assert_eq!(ours.types().len(), game.types().len());
        let peasant = game.type_id("peasant").unwrap();
        assert_eq!(ours.ty(peasant).name, "footman");
        let opening = stream.opening(map, Speed::Normal);
        let (SeatFrame::Start { start, .. }, SeatFrame::Whole(whole)) = (&opening[1], &opening[2])
        else {
            panic!("the start, then the whole message");
        };
        let read_with_ours = GameStart::from_message(start.clone(), ours);
        read_with_ours.check(whole).unwrap();
        for frame in opening {
            s.frame_at(20.0, frame);
        }
    });
    assert_eq!(server.sent(), [ClientMsg::LeaveGame]);
    let said = said(&h);
    assert!(
        said.contains(&"cannot play: your game data differs from this game's".to_owned()),
        "{said:#?}"
    );
    assert!(
        !said.iter().any(|s| s.starts_with("objective")),
        "the game was played: {said:#?}"
    );
}

#[test]
fn what_happens_to_the_players_and_the_clock_is_said() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let slots = vec![slot(human("andrew"), 1, true), slot(human("bob"), 2, true)];
    let (server, _, mut g) = fake_game(&mut h, &slots);
    let map = jl1(&mut h);
    h.client.net.as_mut().unwrap().room = Some(soundrts::net::lobby::RoomModel {
        state: room_state(&map, slots),
        map: soundrts::net::lobby::MapState::Downloading,
    });
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    let t0 = h.clock.now();
    let news = [
        SeatFrame::Seat {
            slot: 1,
            change: SeatChange::Dropped,
        },
        SeatFrame::Seat {
            slot: 1,
            change: SeatChange::Returned,
        },
        SeatFrame::Clock {
            by: login("andrew"),
            change: ClockChange::Paused,
        },
        SeatFrame::Clock {
            by: login("andrew"),
            change: ClockChange::Speed {
                speed: Speed::Fast2,
            },
        },
        SeatFrame::Seat {
            slot: 1,
            change: SeatChange::Removed {
                by: login("andrew"),
            },
        },
        SeatFrame::Seat {
            slot: 1,
            change: SeatChange::GivenUp,
        },
        SeatFrame::Seat {
            slot: 1,
            change: SeatChange::Left,
        },
        SeatFrame::Stopped(StopReason::TooSlow),
    ];
    for (i, frame) in news.into_iter().enumerate() {
        server.frame_at(t0 + 10.0 * i as f64, frame);
    }
    h.run_for(&mut g, 100.0);
    assert!(g.game.end_loop, "the server stopped the game");
    assert_eq!(
        h.said_since(from),
        [
            "bob lost the connection",
            "bob is back",
            "paused by andrew",
            "speed fast 2 set by andrew",
            "bob removed from the game by andrew",
            "bob did not come back, and is out of the game",
            "bob left the game",
            "game stopped by the server: a turn took too long",
        ]
    );
}

/// Andrew chooses the second speed of the game menu in a game on a fake
/// server whose room `host` hosts: what was said from the menu on, and
/// what his client sent.
fn speed_from_the_game_menu(host: &str) -> (Vec<String>, Vec<ClientMsg>) {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let slots = vec![slot(human("andrew"), 1, true), slot(human("bob"), 2, true)];
    let (server, _, mut g) = fake_game(&mut h, &slots);
    let map = jl1(&mut h);
    let mut state = room_state(&map, slots);
    state.host = login(host);
    h.client.net.as_mut().unwrap().room = Some(soundrts::net::lobby::RoomModel {
        state,
        map: soundrts::net::lobby::MapState::Downloading,
    });
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    for key in [Key::F10, Key::Down, Key::Down, Key::Return] {
        h.key(&mut g, key, Mods::NONE);
    }
    h.run_for(&mut g, 1.0);
    (h.said_since(from), server.sent())
}

#[test]
fn the_rooms_host_sets_the_speed_of_a_game_on_a_server() {
    let (said, sent) = speed_from_the_game_menu("andrew");
    assert_eq!(
        sent,
        [ClientMsg::SetGameSpeed { speed: Speed::Slow }],
        "{said:#?}"
    );
}

#[test]
fn only_the_rooms_host_sets_the_speed_of_a_game_on_a_server() {
    let (said, sent) = speed_from_the_game_menu("bob");
    assert_eq!(sent, [], "{said:#?}");
    assert_eq!(
        said.last().map(String::as_str),
        Some("only the organizer can change the speed"),
        "{said:#?}"
    );
}

/// The frames of a turn whose events end with the player's game over,
/// as the world tells it.
fn turn_told_over(stream: &mut FakeStream) -> Vec<SeatFrame> {
    let mut frames = stream.turn(Vec::new());
    if let Some(SeatFrame::Change(c)) = frames.first_mut() {
        c.events.push(soundrts_world::Event::Quit);
    }
    frames
}

#[test]
fn a_game_told_over_waits_for_its_end_and_says_the_score() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    let t0 = h.clock.now();
    for frame in turn_told_over(&mut stream) {
        server.frame_at(t0, frame);
    }
    server.frame_at(t0 + 2.0, stream.end());
    h.run_for(&mut g, 1.0);
    assert!(!g.game.end_loop, "the game ended before its end came");
    h.run_for(&mut g, 2.0);
    assert!(g.game.end_loop, "the end came, and the game went on");
    h.finish(&mut g);
    assert!(
        g.game.view().stats().is_some(),
        "the score has its statistics"
    );
}

#[test]
fn what_came_before_a_lost_connection_is_taken() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    let t0 = h.clock.now();
    for frame in turn_told_over(&mut stream) {
        server.frame_at(t0, frame);
    }
    server.frame_at(t0, stream.end());
    server.close_at(t0);
    h.run_for(&mut g, 1.0);
    assert!(g.game.end_loop);
    assert_eq!(g.game.view().turn(), 1);
    h.finish(&mut g);
    assert!(
        g.game.view().stats().is_some(),
        "the score has its statistics"
    );
}

#[test]
fn a_game_on_a_server_that_gives_ever_more_objectives_is_ended() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, stream, mut g) = fake_game(&mut h, &two_slots());
    let map = jl1(&mut h);
    let SeatFrame::Whole(mut last) = stream.opening(map, Speed::Normal).remove(2) else {
        panic!("a whole message");
    };
    // Five turns that come together, each with a full set of new
    // objectives.
    let t0 = h.clock.now();
    for turn in 1..=5 {
        let mut next = last.clone();
        next.time = turn * 300;
        next.events.clear();
        next.state.objectives = (0..soundrts_world::MAX_OBJECTIVES)
            .map(|i| soundrts_world::player::Objective {
                number: format!("{turn}.{i}"),
                description: vec![1],
            })
            .collect();
        let change = soundrts_world::TurnChange::between(&last, &next);
        server.frame_at(t0, SeatFrame::Change(change));
        last = next;
    }
    h.run_for(&mut g, 1.0);
    assert!(g.game.end_loop, "the game went on");
    h.finish(&mut g);
    let said = h.said_since(0);
    assert_eq!(
        said.last().map(String::as_str),
        Some("game ended: the server sent what cannot be right"),
        "{said:#?}"
    );
}

#[test]
fn a_frame_that_cannot_be_right_ends_the_game_and_leaves_it() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    let t0 = h.clock.now();
    turn_at(&server, &mut stream, t0);
    // The start comes once, before the game.
    server.frame_at(t0 + 1.0, stream.start());
    h.run_for(&mut g, 10.0);
    assert!(g.game.end_loop, "the game went on");
    assert_eq!(g.game.view().turn(), 1);
    h.finish(&mut g);
    let said = h.said_since(from);
    assert_eq!(
        said.last().map(String::as_str),
        Some("game ended: the server sent what cannot be right"),
        "{said:#?}"
    );
    soundrts::session::leave_server_game(&mut h.client);
    assert_eq!(server.sent(), [ClientMsg::LeaveGame]);
}

#[test]
fn a_game_that_did_not_start_says_why() {
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        s.at(
            10.0,
            room(RoomChange::Entered, &room_state(&map, two_slots())),
        );
        s.frame_at(
            15.0,
            SeatFrame::NotStarted(StartRefusal::CannotBuild {
                error: ChatText::clean("no starting place").unwrap(),
            }),
        );
        s.frame_at(20.0, SeatFrame::NotStarted(StartRefusal::Busy));
    });
    let said = said(&h);
    assert!(
        said.contains(
            &"game not started: the server cannot build it: no starting place".to_owned()
        ),
        "{said:#?}"
    );
    assert!(
        said.contains(&"game not started: the server is busy, try again later".to_owned()),
        "{said:#?}"
    );
}

#[test]
fn a_map_package_that_unpacks_too_large_is_refused() {
    use std::io::Write;
    // 65 MiB of zeros: a few tens of KiB once compressed.
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("map.txt", options).unwrap();
    let zeros = vec![0u8; 1024 * 1024];
    for _ in 0..65 {
        w.write_all(&zeros).unwrap();
    }
    let bomb = w.finish().unwrap().into_inner();
    let hash = MapHash::of(&bomb);
    let file_name = MapFileName::new("bomb.zip").unwrap();
    let map = MapInfo {
        file_name: file_name.clone(),
        hash: hash.clone(),
        title_sounds: Vec::new(),
        nb_players_min: 2,
        nb_players_max: 2,
    };
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |_, s| {
        let mut state = room_state(&map, vec![slot(human("bob"), 1, true)]);
        state.host = login("bob");
        state.slots.push(slot(human("andrew"), 2, false));
        s.at(10.0, room(RoomChange::Entered, &state));
        s.at(
            15.0,
            ServerMsg::Map {
                map: MapData {
                    file_name,
                    bytes: MapBytes::new(bomb).unwrap(),
                },
            },
        );
    });
    assert!(server.sent().contains(&ClientMsg::GetMap { hash }));
    let said = said(&h);
    assert!(
        said.contains(&"<1029> the map of this game is too large once unpacked".to_owned()),
        "{said:#?}"
    );
    let saved = std::fs::read_dir(h.client.paths.downloaded().join("multi"))
        .unwrap()
        .count();
    assert_eq!(saved, 0, "the map is not kept");
    assert!(h.log_text().contains("refused the map received"));
}

/// Andrew hosts Bob and Carol; Bob leaves just after Andrew confirms the
/// item chosen with `letters`. What Andrew's client sends.
fn bob_leaves_as_andrew_confirms(letters: &[&str]) -> Vec<ClientMsg> {
    let mut script = Script::new();
    let mut t = 20.0;
    for l in letters {
        script = script.at(t).type_text(l);
        t += 1.0;
    }
    let confirm_at = t;
    script = script
        .at(confirm_at)
        .tap(Key::Return)
        .at(40.0)
        .tap(Key::LShift);
    let (_, server) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        let three = room_state(
            &map,
            vec![
                slot(human("andrew"), 1, true),
                slot(human("bob"), 2, false),
                slot(human("carol"), 3, false),
            ],
        );
        let mut two = three.clone();
        two.slots.remove(1);
        s.at(10.0, room(RoomChange::Entered, &three));
        // While the confirmation is said, before the item runs.
        s.at(
            confirm_at + 0.05,
            room(
                RoomChange::Left {
                    player: human("bob"),
                },
                &two,
            ),
        );
    });
    server.sent()
}

#[test]
fn removing_a_player_who_has_just_left_does_nothing() {
    let sent = bob_leaves_as_andrew_confirms(&["r"]);
    assert!(
        !sent
            .iter()
            .any(|m| matches!(m, ClientMsg::RemoveSlot { .. })),
        "{sent:?}"
    );
}

#[test]
fn removing_a_player_whose_slot_moved_removes_that_player() {
    // Carol was in slot 2; with Bob gone she is in slot 1.
    let sent = bob_leaves_as_andrew_confirms(&["r", "r"]);
    assert_eq!(sent, [ClientMsg::RemoveSlot { slot: 1 }]);
}

#[test]
fn moving_a_player_whose_slot_moved_moves_that_player() {
    // andrew to 2 and 3, bob to 1 and 3, then "move carol to alliance 1".
    let sent = bob_leaves_as_andrew_confirms(&["m"; 5]);
    assert_eq!(
        sent,
        [ClientMsg::SetAlliance {
            slot: 1,
            alliance: 1
        }]
    );
}

#[test]
fn a_line_being_typed_is_said_as_written() {
    let script = Script::new()
        .at(8.0)
        .tap(Key::F7)
        .at(20.0)
        .type_text("6")
        .at(21.0)
        .type_text("1")
        .at(22.0)
        .type_text("1")
        .at(23.0)
        .type_text("4")
        .at(24.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, _| {});
    assert_eq!(
        from(&said(&h), "enter your message"),
        [
            "enter your message",
            "6 . 6",
            "1 . 61",
            "1 . 611",
            "4 . 6114",
            "6114",
        ]
    );
}

/// Andrew opens the game menu, presses F7 with `mods`, and writes "hi".
/// What his client sends.
fn chat_from_the_game_menu(mods: Mods) -> Vec<ClientMsg> {
    let live = LiveKeys::default();
    let mut h = Harness::with_config(
        "en",
        Script::new().keep_open().with_live(live.clone()),
        config("andrew"),
    );
    let server = h.join_fake_server("andrew", &[]);
    // The room of the game is still known during the game.
    let map = jl1(&mut h);
    h.client.net.as_mut().unwrap().room = Some(soundrts::net::lobby::RoomModel {
        state: room_state(&map, two_slots()),
        map: soundrts::net::lobby::MapState::Downloading,
    });
    let stream = FakeStream::new(&mut h, "jl1", &two_slots(), 0, 1);
    let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
    let t0 = h.clock.now();
    live.tap_at(t0 + 1.0, Key::F10, Mods::NONE);
    live.tap_at(t0 + 5.0, Key::F7, mods);
    live.type_at(t0 + 8.0, 0.5, "hi");
    live.tap_at(t0 + 10.0, Key::Return, Mods::NONE);
    live.tap_at(t0 + 15.0, Key::Escape, Mods::NONE);
    h.run_for(&mut g, 20.0);
    assert!(live.is_empty(), "every key was read");
    server.sent()
}

#[test]
fn chat_from_the_game_menu_goes_to_the_game() {
    assert!(
        chat_from_the_game_menu(Mods::NONE).contains(&ClientMsg::Chat {
            channel: Channel::Game,
            text: ChatText::clean("hi").unwrap(),
        })
    );
}

#[test]
fn shift_f7_from_the_game_menu_writes_to_the_allies() {
    let sent = chat_from_the_game_menu(Mods::SHIFT);
    let chats: Vec<&ClientMsg> = sent
        .iter()
        .filter(|m| matches!(m, ClientMsg::Chat { .. }))
        .collect();
    assert_eq!(
        chats,
        [&ClientMsg::Chat {
            channel: Channel::Allies,
            text: ChatText::clean("hi").unwrap(),
        }]
    );
}

#[test]
fn what_players_wrote_and_their_logins_are_said_as_written() {
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(10.0, arrived("4000"));
        // A sound number, a count as the world encodes it, a phrase number.
        s.at(15.0, chat("4000", "6114"));
        s.at(15.0, chat("4000", "1000005"));
        s.at(15.0, chat("bob", "4056"));
    });
    assert_eq!(
        from(&said(&h), "4000 has just logged in"),
        [
            "4000 has just logged in",
            "<6114> 4000 says 6114",
            "<6114> 4000 says 1000005",
            "<6114> bob says 4056",
        ]
    );
}

#[test]
fn a_players_login_is_said_as_written_in_the_game() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let slots = vec![slot(human("andrew"), 1, true), slot(human("4000"), 2, true)];
    let (_, _, g) = fake_game(&mut h, &slots);
    let other = g.game.view().players()[1];
    let name = g.game.view().player_name(other);
    assert_eq!(
        h.client.sounds.translate(&name[0]),
        soundrts::sound_cache::Spoken::Text("4000".into())
    );
}

#[test]
fn a_removal_that_crosses_the_players_leave_does_not_close_the_lobby_prompts() {
    // Andrew leaves Bob's room at 20 s; Bob's removal of Andrew reaches
    // him in the lobby at 27 s, then he writes a chat line there.
    let script = Script::new()
        .at(20.0)
        .tap(Key::Escape)
        .at(30.0)
        .tap(Key::F7)
        .at(32.0)
        .type_text("y")
        .at(34.0)
        .tap(Key::Return)
        .at(50.0)
        .tap(Key::LShift);
    let (_, server) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        let mut state = room_state(
            &map,
            vec![slot(human("bob"), 1, true), slot(human("andrew"), 2, false)],
        );
        state.host = login("bob");
        s.at(10.0, room(RoomChange::Entered, &state));
        s.at(
            27.0,
            ServerMsg::RoomExited {
                reason: RoomExit::Removed,
            },
        );
    });
    assert_eq!(
        server.sent(),
        [
            ClientMsg::LeaveRoom,
            ClientMsg::Chat {
                channel: Channel::Lobby,
                text: ChatText::clean("y").unwrap(),
            },
        ]
    );
}

#[test]
fn a_map_that_cannot_be_read_is_said_and_the_room_does_not_wait_for_it() {
    use std::io::Write;
    // A package without a map in it.
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file("readme.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(b"no map here").unwrap();
    let package = w.finish().unwrap().into_inner();
    let file_name = MapFileName::new("broken.zip").unwrap();
    let map = MapInfo {
        file_name: file_name.clone(),
        hash: MapHash::of(&package),
        title_sounds: Vec::new(),
        nb_players_min: 2,
        nb_players_max: 2,
    };
    // Andrew goes to the item before "leave this game".
    let script = Script::new()
        .at(20.0)
        .tap(Key::Up)
        .at(22.0)
        .tap(Key::Up)
        .at(30.0)
        .tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        let mut state = room_state(&map, vec![slot(human("bob"), 1, true)]);
        state.host = login("bob");
        state.slots.push(slot(human("andrew"), 2, false));
        s.at(10.0, room(RoomChange::Entered, &state));
        s.at(
            15.0,
            ServerMsg::Map {
                map: MapData {
                    file_name,
                    bytes: MapBytes::new(package).unwrap(),
                },
            },
        );
    });
    assert_eq!(
        from(&said(&h), "<1029> this game cannot start here"),
        [
            "<1029> this game cannot start here",
            "quit, leave this game",
            "this game cannot start here",
        ]
    );
}

#[test]
fn keys_send_the_commands_the_interface_always_gave() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g) = fake_game(&mut h, &two_slots());
    turn_at(&server, &mut stream, h.clock.now());
    h.run_for(&mut g, 0.5);
    // A peasant, the square to its east, the default order there; the
    // north subsquare of that square, the default order there in the
    // imperative; recallable group 6; quit from the game menu.
    h.key(&mut g, Key::E, Mods::NONE);
    h.key(&mut g, Key::Right, Mods::NONE);
    h.key(&mut g, Key::Backspace, Mods::NONE);
    h.key(&mut g, Key::F8, Mods::NONE);
    h.key(&mut g, Key::Up, Mods::NONE);
    h.key(&mut g, Key::Backspace, Mods::CTRL);
    h.key(&mut g, Key::Num6, Mods::CTRL);
    h.key(&mut g, Key::F10, Mods::NONE);
    h.key(&mut g, Key::Down, Mods::NONE);
    h.key(&mut g, Key::Return, Mods::NONE);
    h.run_for(&mut g, 0.5);
    let peasant = g.game.group[0];
    assert_eq!(&*g.game.view().known(peasant).unwrap().type_name, "peasant");
    let sq = g.game.place.unwrap();
    let (x, y) = g.game.view().square(sq).center();
    let order = |order, imperative| Command::Order {
        order,
        queue: Queue::Replace,
        imperative,
    };
    let commands: Vec<Command> = server.commands().into_iter().map(|c| c.0).collect();
    // A square is 12 m wide: the north subsquare's center is 4 m north.
    assert_eq!(
        commands,
        [
            Command::Control(vec![peasant]),
            order(Order::Default(OrderTarget::Square(sq)), false),
            order(
                Order::Default(OrderTarget::Point {
                    square: sq,
                    x,
                    y: y + 4000
                }),
                true
            ),
            // `reset_group` is in no menu: the units it goes to are none.
            Command::Control(Vec::new()),
            order(Order::ResetGroup("6".into()), false),
            Command::Control(vec![peasant]),
            order(Order::JoinGroup("6".into()), false),
            Command::Quit,
        ]
    );
}

// -- A server that sends more than a client keeps -------------------------

/// What the client says when it leaves a server that sent too much.
const UNEXPECTED: &str =
    "error: unexpected data was received from the server. This is not a SoundRTS server";

/// A client on a fake server, as with [`on_fake_server`], that leaves the
/// server on its own before the window closes at 60 s.
fn left_fake_server(
    config: Config,
    schedule: impl FnOnce(&mut Harness, &FakeServer),
) -> (Harness, FakeServer) {
    let mut h = Harness::with_config("en", Script::new().at(60.0).tap(Key::LShift), config);
    let (server, conn) = FakeServer::new(h.clock.clone());
    schedule(&mut h, &server);
    let content = h.client.res.content_hash();
    let result = play_on(
        &mut h.client,
        conn,
        snapshot("andrew", &["bob"]),
        content,
        None,
        server.redial(),
    );
    assert!(result.is_ok(), "the client stayed on the server");
    (h, server)
}

#[test]
fn a_server_that_lists_more_players_than_a_client_keeps_is_left() {
    let mut c = config("andrew");
    c.announce_arrivals = false;
    let (h, _) = left_fake_server(c, |_, s| {
        // With Andrew and Bob, one more than the client keeps.
        for i in 0..soundrts::net::lobby::MAX_LOBBY_PLAYERS - 1 {
            s.at(10.0, arrived(&format!("p{i}")));
        }
    });
    assert_eq!(
        said(&h)[2..],
        [UNEXPECTED, "the connection was interrupted"],
        "{:#?}",
        said(&h)
    );
}

#[test]
fn a_room_with_more_players_than_a_map_can_have_is_left() {
    let (h, _) = left_fake_server(config("andrew"), |h, s| {
        let map = jl1(h);
        let mut slots = vec![slot(human("andrew"), 1, true)];
        slots.extend((0..soundrts::net::lobby::MAX_ROOM_SLOTS).map(|_| {
            slot(
                SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)),
                2,
                true,
            )
        }));
        s.at(10.0, room(RoomChange::Entered, &room_state(&map, slots)));
    });
    assert_eq!(
        said(&h)[2..],
        [UNEXPECTED, "the connection was interrupted"],
        "{:#?}",
        said(&h)
    );
}

#[test]
fn an_invitation_to_a_room_the_server_never_told_of_is_ignored() {
    let script = Script::new().at(30.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |_, s| {
        s.at(
            10.0,
            ServerMsg::Invited {
                room: RoomId(9),
                by: login("bob"),
            },
        );
    });
    assert_eq!(
        said(&h),
        [
            "welcome andrew on the server of test",
            "please make a selection",
        ]
    );
}

#[test]
fn a_server_that_sends_frames_far_ahead_of_the_game_is_left() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let server = h.join_fake_server("andrew", &[]);
    let map = jl1(&mut h);
    let stream = FakeStream::new(&mut h, "jl1", &two_slots(), 0, 1);
    for frame in stream.opening(map, Speed::Normal) {
        server.frame_at(0.0, frame);
    }
    // No game takes them.
    for _ in 0..soundrts::net::MAX_QUEUED_FRAMES {
        server.frame_at(
            0.0,
            SeatFrame::Seat {
                slot: 0,
                change: SeatChange::Returned,
            },
        );
    }
    for _ in 0..soundrts::net::MAX_QUEUED_FRAMES / soundrts::net::PUMP_LIMIT + 2 {
        soundrts::net::pump(&mut h.client);
    }
    let net = h.client.net.as_ref().unwrap();
    assert_eq!(
        net.closed.as_deref(),
        Some("too many frames ahead of the game")
    );
}

// Returning to a game after the connection is lost.

/// A game of Andrew against a computer on a fake server, in its room,
/// joined and told; the room.
fn game_in_room(h: &mut Harness) -> (FakeServer, FakeStream, TestGame, RoomState) {
    let (server, stream, mut g) = fake_game(h, &two_slots());
    let room = soundrts::testkit::fake_room(jl1(h), two_slots());
    h.enter_fake_room(&room);
    h.run_until_voice_idle(&mut g);
    (server, stream, g, room)
}

/// How many times `line` was said in `said`.
fn times(said: &[String], line: &str) -> usize {
    said.iter().filter(|s| *s == line).count()
}

/// The seat's message without its events, which a seat is told once.
fn eventless(m: &soundrts_world::TurnMessage) -> soundrts_world::TurnMessage {
    let mut m = m.clone();
    m.events.clear();
    m
}

#[test]
fn a_lost_connection_returns_to_the_seat_and_the_game_plays_on() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g, room) = game_in_room(&mut h);
    let from = h.utterances().len();
    let t0 = h.clock.now();
    turn_at(&server, &mut stream, t0);
    server.close_at(t0 + 1.0);
    // The turns the seat misses, then the seat given back.
    for _ in 0..4 {
        stream.turn(Vec::new());
    }
    let back = server.dial_to(soundrts::testkit::welcome_back("andrew", &room, &[]));
    for frame in stream.opening(jl1(&mut h), Speed::Normal) {
        back.frame_at(t0 + 1.5, frame);
    }
    turn_at(&back, &mut stream, t0 + 2.0);
    h.run_for(&mut g, 3.0);
    h.run_until_voice_idle(&mut g);
    assert!(!g.game.end_loop, "the game ended");
    assert_eq!(g.game.view().turn(), 6);
    assert_eq!(
        eventless(g.game.view().message()),
        eventless(stream.message()),
        "the view is the seat's"
    );
    let said = h.said_since(from);
    assert_eq!(
        times(&said, "connection lost, trying to return"),
        1,
        "{said:#?}"
    );
    assert_eq!(times(&said, "back in the game"), 1, "{said:#?}");
    assert_eq!(
        back.sent(),
        [ClientMsg::ReturnToGame {
            room: room.id,
            token: SeatToken::from_bits(1),
        }]
    );
    // The commands go on the new connection.
    g.game.send_command(&h.client, Command::Control(Vec::new()));
    h.run_for(&mut g, 0.5);
    assert_eq!(back.commands().len(), 1);
    assert!(server.commands().is_empty());
}

#[test]
fn a_return_waits_until_the_server_has_noticed_the_lost_connection() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, stream, mut g, room) = game_in_room(&mut h);
    server.close_at(h.clock.now());
    // The server still has Andrew playing on the old connection: the new
    // one is "andrew2".
    let early = server.dial_to(soundrts::testkit::welcome_back(
        "andrew2",
        &room,
        &["andrew"],
    ));
    let back = server.dial_to(soundrts::testkit::welcome_back("andrew", &room, &[]));
    for frame in stream.opening(jl1(&mut h), Speed::Normal) {
        back.frame_at(0.0, frame);
    }
    h.run_for(&mut g, 3.0);
    assert!(early.sent().is_empty(), "{:?}", early.sent());
    assert_eq!(back.sent().len(), 1);
    assert!(!g.game.returning(), "the seat is back");
}

#[test]
fn orders_given_while_away_are_not_taken_and_the_player_hears_it_once() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, _, mut g, _) = game_in_room(&mut h);
    let from = h.utterances().len();
    server.close_at(h.clock.now());
    h.run_for(&mut g, 1.0);
    h.run_until_voice_idle(&mut g);
    assert!(g.game.returning());
    g.game.send_command(&h.client, Command::Control(Vec::new()));
    g.game.send_command(&h.client, Command::Control(Vec::new()));
    h.run_for(&mut g, 1.0);
    assert!(g.game.pending_commands().is_empty());
    // The server is never reached again: the client gives up once the
    // server no longer holds the seat.
    let hold = soundrts::testkit::FAKE_HOLD.as_secs_f64();
    h.run_for(&mut g, hold);
    assert!(g.game.end_loop, "the game went on");
    h.run_voice_for(10.0);
    let said = h.said_since(from);
    assert_eq!(
        said,
        [
            "connection lost, trying to return",
            "order not taken: not connected",
            "could not return: the server could not be reached in time",
        ]
    );
    assert!(server.commands().is_empty());
}

#[test]
fn chat_lines_and_flares_given_while_away_are_not_sent_and_the_player_hears_it_once() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, _, mut g, _) = game_in_room(&mut h);
    server.close_at(h.clock.now());
    h.run_for(&mut g, 1.0);
    h.run_until_voice_idle(&mut g);
    assert!(g.game.returning());
    let from = h.utterances().len();
    for mods in [Mods::NONE, Mods::SHIFT] {
        h.key(&mut g, Key::F7, mods);
        h.key(&mut g, Key::H, Mods::NONE);
        h.key(&mut g, Key::I, Mods::NONE);
        h.key(&mut g, Key::Return, Mods::NONE);
    }
    h.key(&mut g, Key::F7, Mods::CTRL);
    h.run_for(&mut g, 1.0);
    h.run_until_voice_idle(&mut g);
    let said = h.said_since(from);
    assert_eq!(
        said,
        [
            "enter your message",
            "h . h",
            "i . hi",
            "hi",
            "not sent: not connected",
            "enter your message to your allies",
            "h . h",
            "i . hi",
            "hi",
        ]
    );
    assert!(
        !server
            .sent()
            .iter()
            .any(|m| matches!(m, ClientMsg::Chat { .. } | ClientMsg::Flare { .. })),
        "{:?}",
        server.sent()
    );
}

#[test]
fn a_host_back_under_a_numbered_login_hosts_and_has_the_login_again() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, stream, mut g, room) = game_in_room(&mut h);
    let has_pause = |h: &mut Harness, g: &mut TestGame| {
        let from = h.utterances().len();
        h.key(g, Key::F10, Mods::NONE);
        for _ in 0..6 {
            h.key(g, Key::Down, Mods::NONE);
        }
        let said = h.said_since(from);
        h.key(g, Key::Escape, Mods::NONE);
        said.contains(&"pause ,".to_owned())
    };
    assert!(has_pause(&mut h, &mut g));
    server.close_at(h.clock.now());
    // Another connection held "andrew" when this one said hello.
    let back = server.dial_to(soundrts::testkit::welcome_back("andrew2", &room, &[]));
    for frame in stream.opening(jl1(&mut h), Speed::Normal) {
        back.frame_at(0.0, frame);
    }
    h.run_for(&mut g, 3.0);
    assert!(!g.game.returning(), "the seat is back");
    assert!(has_pause(&mut h, &mut g), "the host's item is there");
    let renamed = ServerMsg::Lobby {
        change: LobbyChange::PlayerRenamed {
            from: login("andrew2"),
            to: login("andrew"),
        },
    };
    back.at(h.clock.now(), renamed);
    h.run_for(&mut g, 1.0);
    assert_eq!(h.client.net.as_ref().unwrap().lobby.you, login("andrew"));
}

#[test]
fn escape_leaves_the_game_at_once_while_returning() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, _, mut g, _) = game_in_room(&mut h);
    server.close_at(h.clock.now());
    h.run_for(&mut g, 1.0);
    assert!(g.game.returning());
    h.press(Key::Escape, Mods::NONE);
    h.step(&mut g);
    assert!(g.game.end_loop, "the game went on");
}

/// What Andrew hears when the server the connection is opened again to
/// welcomes it with `welcome` and then sends `answer`.
fn return_refused(
    welcome: impl FnOnce(&RoomState) -> LobbySnapshot,
    answer: Option<ServerMsg>,
) -> Vec<String> {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, _, mut g, room) = game_in_room(&mut h);
    let from = h.utterances().len();
    server.close_at(h.clock.now());
    let back = server.dial_to(welcome(&room));
    if let Some(answer) = answer {
        back.at(0.0, answer);
    }
    h.run_for(&mut g, 3.0);
    assert!(g.game.end_loop, "the game went on");
    h.run_voice_for(10.0);
    h.said_since(from)
}

#[test]
fn a_seat_the_server_no_longer_holds_ends_the_game_with_the_reason() {
    let refused = |reason| {
        return_refused(
            |room| soundrts::testkit::welcome_back("andrew", room, &[]),
            Some(ServerMsg::Refused { reason }),
        )
    };
    assert_eq!(
        refused(Refusal::NotNow),
        [
            "connection lost, trying to return",
            "could not return: your seat was given up"
        ]
    );
    assert_eq!(
        refused(Refusal::NoSuchRoom),
        [
            "connection lost, trying to return",
            "could not return: the game is over"
        ]
    );
    // The room is no longer in the lobby: the game is over.
    assert_eq!(
        return_refused(|_| snapshot("andrew", &[]), None),
        [
            "connection lost, trying to return",
            "could not return: the game is over"
        ]
    );
}

// Pausing a game on a server.

/// Andrew opens the game menu of a game on a fake server whose room `host`
/// hosts, and goes down its items: what he heard.
fn game_menu_items(host: &str) -> Vec<String> {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let slots = vec![slot(human("andrew"), 1, true), slot(human("bob"), 2, true)];
    let (_server, _, mut g) = fake_game(&mut h, &slots);
    let mut room = soundrts::testkit::fake_room(jl1(&mut h), slots);
    room.host = login(host);
    h.enter_fake_room(&room);
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    h.key(&mut g, Key::F10, Mods::NONE);
    for _ in 0..7 {
        h.key(&mut g, Key::Down, Mods::NONE);
    }
    let said = h.said_since(from);
    h.key(&mut g, Key::Escape, Mods::NONE);
    said
}

#[test]
fn only_the_rooms_host_is_offered_to_pause_a_game_on_a_server() {
    let speeds = [
        "set the game speed to slow ,",
        "set the game speed to normal ,",
        "set the game speed to fast ,",
        "set the game speed to fast 4 ,",
    ];
    let host = [
        &["Menu", "cancel game ,"][..],
        &speeds,
        &["pause ,", "continue game ,"],
    ]
    .concat();
    assert_eq!(game_menu_items("andrew"), host);
    let others = [
        &["Menu", "cancel game ,"][..],
        &speeds,
        &["continue game ,", "cancel game ,"],
    ]
    .concat();
    assert_eq!(game_menu_items("bob"), others);
}

#[test]
fn the_rooms_host_pauses_and_resumes_a_game_on_a_server() {
    let live = LiveKeys::default();
    let mut h = Harness::with_config(
        "en",
        Script::new().keep_open().with_live(live.clone()),
        config("andrew"),
    );
    let (server, mut g) =
        h.start_on_server("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4, 1);
    h.clock.follow_real_time();
    h.run_for(&mut g, 1.0);
    // F10, then the item after the four speeds.
    let choose = |live: &LiveKeys, at: f64| {
        live.tap_at(at, Key::F10, Mods::NONE);
        for i in 0..6 {
            live.tap_at(at + 0.3 * (i + 1) as f64, Key::Down, Mods::NONE);
        }
        live.tap_at(at + 2.4, Key::Return, Mods::NONE);
    };
    let from = h.utterances().len();
    choose(&live, h.clock.now());
    let start = std::time::Instant::now();
    while !h.said_since(from).contains(&"paused by andrew".to_owned()) {
        assert!(
            start.elapsed() < soundrts::testkit::WAIT_LIMIT,
            "{:#?}",
            h.said_since(from)
        );
        h.step(&mut g);
    }
    let paused_at = g.game.view().turn();
    h.run_for(&mut g, 1.0);
    assert_eq!(g.game.view().turn(), paused_at, "turns came while paused");
    choose(&live, h.clock.now());
    let start = std::time::Instant::now();
    while g.game.view().turn() < paused_at + 10
        || !h.said_since(from).contains(&"resumed by andrew".to_owned())
    {
        assert!(
            start.elapsed() < soundrts::testkit::WAIT_LIMIT,
            "{:#?}",
            h.said_since(from)
        );
        h.step(&mut g);
    }
    server.shutdown();
    let said = h.said_since(from);
    assert!(said.contains(&"pause ,".to_owned()), "{said:#?}");
    assert!(said.contains(&"resume ,".to_owned()), "{said:#?}");
}

/// A player on a real server, on real time, whose speech lasts a tenth of
/// its usual time.
fn real_player(login: &str) -> Harness {
    let h = Harness::with_config(
        "en",
        Script::new().keep_open(),
        Config {
            wait_delay_per_character: 0.01,
            ..config(login)
        },
    );
    h.clock.follow_real_time();
    h
}

/// Opens the game menu and goes down its items: what was heard.
fn menu_items(h: &mut Harness, g: &mut TestGame) -> Vec<String> {
    let from = h.utterances().len();
    h.key(g, Key::F10, Mods::NONE);
    for _ in 0..7 {
        h.key(g, Key::Down, Mods::NONE);
    }
    let said = h.said_since(from);
    h.key(g, Key::Escape, Mods::NONE);
    said
}

#[test]
fn when_the_host_leaves_a_game_on_a_server_the_next_player_hosts_it() {
    let server = soundrts::testkit::test_server(1, |_| {});
    let addr = server.local_addr().to_string();
    let (mut andrew, mut bob) = (real_player("andrew"), real_player("bob"));
    andrew.connect_to(&addr, |c| c);
    bob.connect_to(&addr, |c| c);
    let room = andrew.open_room("jl2", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4);
    andrew.invite("bob");
    bob.join_room(room);
    andrew.wait_for_ready(3);
    let mut a = andrew.start_room_game("jl2");
    let mut b = bob.join_room_game("jl2");
    bob.run_until_turn(&mut b, 3);
    assert!(!menu_items(&mut bob, &mut b).contains(&"pause ,".to_owned()));
    // Andrew pauses, then chooses "cancel game" once.
    soundrts::net::send(&mut andrew.client, ClientMsg::PauseGame);
    let (a_from, from) = (andrew.utterances().len(), bob.utterances().len());
    let start = std::time::Instant::now();
    let paused = "paused by andrew".to_owned();
    while !(andrew.said_since(a_from).contains(&paused) && bob.said_since(from).contains(&paused)) {
        assert!(start.elapsed() < soundrts::testkit::WAIT_LIMIT);
        andrew.step(&mut a);
        bob.step(&mut b);
    }
    andrew.key(&mut a, Key::F10, Mods::NONE);
    andrew.key(&mut a, Key::Down, Mods::NONE);
    andrew.key(&mut a, Key::Return, Mods::NONE);
    assert!(a.game.end_loop, "Andrew left the paused game");
    soundrts::session::leave_server_game(&mut andrew.client);
    let start = std::time::Instant::now();
    while !bob
        .said_since(from)
        .contains(&"bob is now the organizer".to_owned())
    {
        assert!(
            start.elapsed() < soundrts::testkit::WAIT_LIMIT,
            "{:#?}",
            bob.said_since(from)
        );
        bob.step(&mut b);
    }
    bob.run_until_voice_idle(&mut b);
    let said = bob.said_since(from);
    assert_eq!(
        said,
        [
            "paused by andrew",
            "andrew left the game",
            "bob is now the organizer"
        ],
        "{said:#?}"
    );
    // The game waits for Bob, who can resume it.
    assert!(menu_items(&mut bob, &mut b).contains(&"resume ,".to_owned()));
    server.shutdown();
}

// A flare that points the allies at a square.

/// Andrew, with Bob in `bob_alliance`, on a fake server: what Andrew
/// hears of the frames `frames(bob, andrew)` gives, one by one.
fn flares_heard(
    bob_alliance: u8,
    frames: impl FnOnce(PlayerId, PlayerId) -> Vec<SeatFrame>,
) -> (Harness, TestGame, Vec<String>) {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let slots = vec![
        slot(human("andrew"), 1, true),
        slot(human("bob"), bob_alliance, true),
    ];
    let (server, _, mut g) = fake_game(&mut h, &slots);
    h.run_until_voice_idle(&mut g);
    let andrew = g.game.view().player();
    let bob = g.game.view().players()[1];
    let from = h.utterances().len();
    let t0 = h.clock.now();
    let frames = frames(bob, andrew);
    let n = frames.len();
    for (i, frame) in frames.into_iter().enumerate() {
        server.frame_at(t0 + 5.0 * i as f64, frame);
    }
    h.run_for(&mut g, 5.0 * n as f64);
    h.run_until_voice_idle(&mut g);
    let said = h.said_since(from);
    (h, g, said)
}

#[test]
fn an_allys_flare_is_an_alert_at_its_square_and_ones_own_is_confirmed() {
    let square = SquareId(3);
    let (mut h, mut g, said) = flares_heard(1, |bob, andrew| {
        vec![
            SeatFrame::Flare { from: bob, square },
            SeatFrame::Flare {
                from: andrew,
                square,
            },
            SeatFrame::Flare {
                from: bob,
                square: SquareId(u32::MAX),
            },
        ]
    });
    assert_eq!(said, ["bob : flare at a 4", "flare sent"]);
    assert!(
        h.audio.played().iter().any(|(_, s)| s.contains("1339")),
        "{:?}",
        h.audio.played()
    );
    assert!(
        h.log_text()
            .contains(&format!("a flare at square {}, not on the map", u32::MAX)),
        "{}",
        h.log_text()
    );
    g.game.place = None;
    h.key(&mut g, Key::Delete, Mods::NONE);
    assert_eq!(g.game.place, Some(square));
}

#[test]
fn a_flare_from_an_enemy_is_not_said() {
    let (h, _, said) = flares_heard(2, |bob, _| {
        vec![SeatFrame::Flare {
            from: bob,
            square: SquareId(3),
        }]
    });
    assert_eq!(said, Vec::<String>::new());
    let log = h.log_text();
    assert!(
        log.contains("a flare from player 1, no ally in the game"),
        "{log}"
    );
}

#[test]
fn a_flare_needs_a_server_and_an_ally() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let mut g = h.start_training("jl1", &["easy"], 1);
    h.run_until_turn(&mut g, 2);
    assert_eq!(h.key(&mut g, Key::F7, Mods::CTRL)[0], "nobody to tell");
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut g) =
        h.start_on_server("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Normal, 1);
    h.run_until_turn(&mut g, 2);
    assert_eq!(h.key(&mut g, Key::F7, Mods::CTRL)[0], "no ally in the game");
    server.shutdown();
}

#[test]
fn a_flare_on_a_server_takes_an_ally_to_its_square() {
    let server = soundrts::testkit::test_server(1, |_| {});
    let addr = server.local_addr().to_string();
    let (mut andrew, mut bob) = (real_player("andrew"), real_player("bob"));
    andrew.connect_to(&addr, |c| c);
    bob.connect_to(&addr, |c| c);
    let room = andrew.open_room("jl2", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4);
    andrew.invite("bob");
    bob.join_room(room);
    andrew.wait_for_ready(3);
    soundrts::net::send(
        &mut andrew.client,
        // Bob is in slot 2, after the computer.
        ClientMsg::SetAlliance {
            slot: 2,
            alliance: 1,
        },
    );
    let mut a = andrew.start_room_game("jl2");
    let mut b = bob.join_room_game("jl2");
    andrew.run_until_turn(&mut a, 3);
    bob.run_until_turn(&mut b, 3);
    let square = a.game.place.unwrap();
    assert_ne!(b.game.place, Some(square));
    // Four flares at once: three go, and the fourth, within ten seconds of
    // the first, does not.
    let (a_from, b_from) = (andrew.utterances().len(), bob.utterances().len());
    for _ in 0..4 {
        andrew.press(Key::F7, Mods::CTRL);
        andrew.step(&mut a);
    }
    let flared = |h: &Harness| {
        h.said_since(b_from)
            .iter()
            .filter(|s| s.starts_with("andrew : flare at "))
            .count()
    };
    let too_fast = |h: &Harness| {
        h.said_since(a_from)
            .iter()
            .filter(|s| s.ends_with("too fast: wait a moment"))
            .count()
    };
    let start = std::time::Instant::now();
    while !(times(&andrew.said_since(a_from), "flare sent") == 3
        && too_fast(&andrew) == 1
        && flared(&bob) == 3)
    {
        assert!(
            start.elapsed() < soundrts::testkit::WAIT_LIMIT,
            "{:#?} {:#?}",
            andrew.said_since(a_from),
            bob.said_since(b_from)
        );
        andrew.step(&mut a);
        bob.step(&mut b);
    }
    andrew.run_until_voice_idle(&mut a);
    bob.run_until_voice_idle(&mut b);
    // Andrew hears that each went, and nothing of them else.
    let said = andrew.said_since(a_from);
    assert_eq!(times(&said, "flare sent"), 3, "{said:#?}");
    assert_eq!(too_fast(&andrew), 1, "{said:#?}");
    assert!(!said.iter().any(|s| s.contains("flare at")), "{said:#?}");
    assert_eq!(flared(&bob), 3, "{:#?}", bob.said_since(b_from));
    bob.key(&mut b, Key::Delete, Mods::NONE);
    assert_eq!(b.game.place, Some(square));
    server.shutdown();
}

#[test]
fn the_hold_starts_again_while_the_server_has_not_noticed_the_loss() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, stream, mut g, room) = game_in_room(&mut h);
    server.close_at(h.clock.now());
    // For longer than the hold, the server still has Andrew playing on the
    // old connection; then it gives the seat back.
    let hold = soundrts::testkit::FAKE_HOLD.as_secs_f64();
    for _ in 0..(hold as usize + 10) {
        server.dial_to(soundrts::testkit::welcome_back(
            "andrew2",
            &room,
            &["andrew"],
        ));
    }
    let back = server.dial_to(soundrts::testkit::welcome_back("andrew", &room, &[]));
    for frame in stream.opening(jl1(&mut h), Speed::Normal) {
        back.frame_at(0.0, frame);
    }
    h.run_for(&mut g, hold + 20.0);
    assert!(!g.game.end_loop, "the client gave up");
    assert!(!g.game.returning(), "the seat is not back");
}

// Watching.

#[test]
fn the_lobby_offers_to_watch_an_open_game_that_takes_watchers() {
    let mut script = Script::new();
    for i in 0..4 {
        script = script.at(10.0 + f64::from(i)).tap(Key::Down);
    }
    for i in 0..3 {
        script = script.at(15.0 + f64::from(i)).tap(Key::Up);
    }
    let script = script.at(20.0).tap(Key::Return).at(30.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        let map = jl1(h);
        let content = h.client.res.content_hash();
        let summary = |id, takes_watchers, open| RoomSummary {
            id: RoomId(id),
            host: login("bob"),
            map: map.clone(),
            speed: Speed::Normal,
            open,
            content: content.clone(),
            players: vec![
                human("bob"),
                SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)),
            ],
            started: true,
            takes_watchers,
        };
        // A game that takes watchers; one that takes none; one closed.
        for room in [
            summary(1, true, true),
            summary(2, false, true),
            summary(3, true, false),
        ] {
            s.at(
                5.0,
                ServerMsg::Lobby {
                    change: LobbyChange::RoomOpened { room },
                },
            );
        }
    });
    let said = said(&h);
    let items: Vec<&String> = said
        .iter()
        .filter(|s| s.starts_with("watch") || s.contains("game on") || s.contains("quit"))
        .collect();
    assert!(items[0].starts_with("watch bob jl1 1"), "{said:#?}");
    assert!(items[1].starts_with("start a game on"), "{said:#?}");
    // The one game to watch, then back to it, and chosen; the others are
    // not offered.
    assert_eq!(
        said.iter().filter(|s| s.starts_with("watch bob")).count(),
        3,
        "{said:#?}"
    );
    assert_eq!(server.sent(), [ClientMsg::Watch { room: RoomId(1) }]);
}

#[test]
fn the_host_says_whether_a_room_takes_watchers_when_creating_it() {
    let mut script = Script::new();
    let mut at = 10.0;
    let mut key = |s: Script, k: Key| {
        at += 1.0;
        s.at(at).tap(k)
    };
    // start a game on > the first map > the watchers setting, changed >
    // normal speed.
    for k in [Key::Down, Key::Return, Key::Down, Key::Return] {
        script = key(script, k);
    }
    for k in [Key::Down; 4] {
        script = key(script, k);
    }
    script = key(script, Key::Return);
    for k in [Key::Up; 3] {
        script = key(script, k);
    }
    script = key(script, Key::Return);
    let script = script.at(at + 10.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |_, _| {});
    let said = said(&h);
    assert!(said.contains(&"watchers , welcome".to_owned()), "{said:#?}");
    assert!(said.contains(&"not welcome".to_owned()), "{said:#?}");
    let created: Vec<bool> = server
        .sent()
        .iter()
        .filter_map(|m| match m {
            ClientMsg::CreateRoom {
                speed: Speed::Normal,
                watchers,
                ..
            } => Some(*watchers),
            _ => None,
        })
        .collect();
    assert_eq!(created, [false], "{:?}", server.sent());
}

/// Carol watches Andrew's game against a computer on jl1, on a fake
/// server, in its room: nothing of it is handled yet.
fn watching_fake_game(h: &mut Harness) -> (FakeServer, FakeStream, TestGame) {
    let server = h.join_fake_server("carol", &["andrew"]);
    let stream = FakeStream::new(h, "jl1", &two_slots(), 0, 1).watching();
    let mut room = soundrts::testkit::fake_room(jl1(h), two_slots());
    room.watchers.push(login("carol"));
    h.enter_fake_room(&room);
    let g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
    (server, stream, g)
}

#[test]
fn a_watcher_starts_on_the_first_unit_of_the_player_it_follows_and_leaves_at_once() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("carol"));
    let (server, _stream, mut g) = watching_fake_game(&mut h);
    // The game may be paused: what the first message shows is known at
    // once, with no turn played.
    h.step(&mut g);
    h.step(&mut g);
    assert!(!g.game.dobjets.is_empty(), "the units are known");
    assert!(g.game.place.is_some(), "the cursor is on a square");
    h.run_until_voice_idle(&mut g);
    assert!(
        said(&h).contains(&"watching andrew".to_owned()),
        "{:#?}",
        said(&h)
    );
    // Cancel game, once, leaves.
    h.key(&mut g, Key::F10, Mods::NONE);
    h.key(&mut g, Key::Down, Mods::NONE);
    h.press(Key::Return, Mods::NONE);
    h.step(&mut g);
    assert!(g.game.end_loop, "still watching: {:#?}", said(&h));
    assert!(server.commands().is_empty());
}

#[test]
fn a_returning_player_learns_who_came_to_watch_meanwhile() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let (server, mut stream, mut g, room) = game_in_room(&mut h);
    let t0 = h.clock.now();
    server.close_at(t0 + 0.5);
    let back = server.dial_to(soundrts::testkit::welcome_back("andrew", &room, &[]));
    let mut watched = room.clone();
    watched.watchers.push(login("dave"));
    back.at(t0 + 1.0, self::room(RoomChange::Entered, &watched));
    stream.turn(Vec::new());
    for frame in stream.opening(jl1(&mut h), Speed::Normal) {
        back.frame_at(t0 + 1.5, frame);
    }
    h.run_for(&mut g, 4.0);
    assert!(!g.game.returning(), "back");
    let net = h.client.net.as_ref().unwrap();
    assert_eq!(net.room.as_ref().unwrap().state.watchers, [login("dave")]);
}

#[test]
fn a_watcher_hears_the_flare_of_the_player_it_follows_and_sends_none() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("carol"));
    let (server, _stream, mut g) = watching_fake_game(&mut h);
    h.run_until_voice_idle(&mut g);
    let andrew = g.game.view().player();
    let square = SquareId(3);
    let from = h.utterances().len();
    server.frame_at(
        h.clock.now(),
        SeatFrame::Flare {
            from: andrew,
            square,
        },
    );
    h.run_for(&mut g, 1.0);
    h.run_until_voice_idle(&mut g);
    assert_eq!(h.said_since(from), ["andrew : flare at a 4"]);
    g.game.place = None;
    h.key(&mut g, Key::Delete, Mods::NONE);
    assert_eq!(g.game.place, Some(square));
    let from = h.utterances().len();
    h.key(&mut g, Key::F7, Mods::CTRL);
    h.run_for(&mut g, 0.5);
    h.run_until_voice_idle(&mut g);
    assert_eq!(h.said_since(from), ["watching: you give no orders"]);
    assert!(
        !server
            .sent()
            .iter()
            .any(|m| matches!(m, ClientMsg::Flare { .. })),
        "{:?}",
        server.sent()
    );
}

fn map_info(h: &mut Harness, name: &str) -> MapInfo {
    h.client
        .res
        .multiplayer_maps()
        .into_iter()
        .find(|m| m.name == name)
        .unwrap()
        .info()
        .unwrap()
}

/// Andrew organizes a room on jl2 with a quiet computer and the random
/// computer, and presses Down `downs` times in the room's menu, then
/// Return if `choose`: what he heard of the items, and what his client
/// sent.
fn organizer_menu(downs: usize, choose: bool) -> (Vec<String>, Vec<ClientMsg>) {
    let mut script = Script::new();
    let mut t = 20.0;
    for _ in 0..downs {
        script = script.at(t).tap(Key::Down);
        t += 8.0;
    }
    if choose {
        script = script.at(t).tap(Key::Return);
        t += 8.0;
    }
    script = script.at(t + 5.0).tap(Key::LShift);
    let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
        let map = map_info(h, "jl2");
        let state = room_state(
            &map,
            vec![
                slot(human("andrew"), 1, true),
                slot(
                    SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)),
                    2,
                    true,
                ),
                slot(SlotPlayer::Computer(Computer::Random), 3, true),
            ],
        );
        s.at(10.0, room(RoomChange::Entered, &state));
    });
    let said = said(&h);
    let items = from(&said, "jl2 2")[1..].to_vec();
    (items, server.sent())
}

#[test]
fn the_organizer_sets_a_computers_level_and_any_seats_income_from_the_room_menu() {
    let (items, _) = organizer_menu(40, false);
    let expected = [
        "invite bob",
        "invite quiet computer , level 3",
        "invite aggressive computer , level 3",
        "invite random computer with unlimited resources and no requirements",
        "start",
        "move andrew to alliance 2",
        "move andrew to alliance 3",
        "set andrew to 50 percent income",
        "set andrew to 75 percent income",
        "set andrew to 150 percent income",
        "set andrew to 200 percent income",
        "move quiet computer , level 3 , to alliance 1",
        "move quiet computer , level 3 , to alliance 3",
        "set quiet computer , level 3 , to level 1",
        "set quiet computer , level 3 , to level 2",
        "set quiet computer , level 3 , to level 4",
        "set quiet computer , level 3 , to level 5",
        "set quiet computer , level 3 , to 50 percent income",
        "set quiet computer , level 3 , to 75 percent income",
        "set quiet computer , level 3 , to 150 percent income",
        "set quiet computer , level 3 , to 200 percent income",
        "move random computer with unlimited resources and no requirements to alliance 1",
        "move random computer with unlimited resources and no requirements to alliance 2",
        "remove quiet computer , level 3",
        "remove random computer with unlimited resources and no requirements",
        "quit, leave this game",
    ];
    assert_eq!(items[..expected.len()], expected);
    // The menu goes round: nothing else is in it.
    assert_eq!(items[expected.len()], expected[0]);

    // Choosing an item asks the server; the room says what it did.
    let at = |item: &str| expected.iter().position(|e| *e == item).unwrap() + 1;
    let (_, sent) = organizer_menu(at("set quiet computer , level 3 , to level 4"), true);
    assert_eq!(
        sent,
        [ClientMsg::SetSkill {
            slot: 1,
            skill: Skill::Level4
        }]
    );
    let (_, sent) = organizer_menu(at("set andrew to 150 percent income"), true);
    assert_eq!(
        sent,
        [ClientMsg::SetIncome {
            slot: 0,
            income: Income::new(150).unwrap()
        }]
    );
}

#[test]
fn a_member_hears_each_level_and_income_the_organizer_sets() {
    let script = Script::new().at(100.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |h, s| {
        let map = map_info(h, "jl2");
        let mut state = RoomState {
            host: login("bob"),
            ..room_state(
                &map,
                vec![
                    slot(human("bob"), 1, true),
                    slot(human("andrew"), 2, false),
                    slot(
                        SlotPlayer::Computer(Computer::Aggressive(Skill::NORMAL)),
                        3,
                        true,
                    ),
                ],
            )
        };
        s.at(10.0, room(RoomChange::Entered, &state));
        state.slots[2].player = SlotPlayer::Computer(Computer::Aggressive(Skill::Level1));
        s.at(20.0, room(RoomChange::Skill { slot: 2 }, &state));
        state.slots[2].income = Income::new(150).unwrap();
        s.at(30.0, room(RoomChange::Income { slot: 2 }, &state));
        state.slots[1].income = Income::new(50).unwrap();
        s.at(40.0, room(RoomChange::Income { slot: 1 }, &state));
        state.slots[1].income = Income::FULL;
        s.at(50.0, room(RoomChange::Income { slot: 1 }, &state));
        state.slots[0].income = Income::new(200).unwrap();
        s.at(60.0, room(RoomChange::Income { slot: 0 }, &state));
        state.slots[2].alliance = 1;
        s.at(70.0, room(RoomChange::Alliance { slot: 2 }, &state));
        state.slots.push(slot(human("carol"), 3, false));
        s.at(
            80.0,
            room(
                RoomChange::Joined {
                    player: human("carol"),
                },
                &state,
            ),
        );
    });
    let said = said(&h);
    let news = from(&said, "aggressive computer , level 1");
    assert_eq!(
        news[..7],
        [
            "aggressive computer , level 1",
            "aggressive computer , level 1 , 150 percent income",
            "andrew , 50 percent income",
            "andrew , 100 percent income",
            "bob , 200 percent income",
            "move aggressive computer , level 1 , 150 percent income , to alliance 1",
            "carol has just joined 4 players on 4 the game will start when the organizer is ready . bob , 200 percent income . andrew . aggressive computer , level 1 , 150 percent income . carol",
        ]
    );
}

#[test]
fn a_member_who_enters_a_room_hears_the_incomes_set_before() {
    let script = Script::new().at(100.0).tap(Key::LShift);
    let (h, _) = on_fake_server(script, config("andrew"), |h, s| {
        let map = map_info(h, "jl2");
        let mut state = RoomState {
            host: login("bob"),
            ..room_state(
                &map,
                vec![slot(human("bob"), 1, true), slot(human("andrew"), 2, false)],
            )
        };
        state.slots[0].income = Income::new(200).unwrap();
        s.at(10.0, room(RoomChange::Entered, &state));
    });
    let said = said(&h);
    assert!(
        said.iter()
            .any(|l| l.ends_with(". bob , 200 percent income")),
        "{said:?}"
    );
}

#[test]
fn the_level_and_income_set_in_a_room_reach_the_game_the_server_builds() {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().to_path_buf();
    let server = soundrts::testkit::test_server(1, |c| c.records = Some(records.clone()));
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    h.connect_to(&server.local_addr().to_string(), |c| c);
    h.open_room("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4);
    let quiet2 = SlotPlayer::Computer(Computer::Quiet(Skill::Level2));
    h.change_room(
        ClientMsg::SetSkill {
            slot: 1,
            skill: Skill::Level2,
        },
        |r| r.slots[1].player == quiet2,
    );
    let income = Income::new(150).unwrap();
    h.change_room(ClientMsg::SetIncome { slot: 0, income }, |r| {
        r.slots[0].income == income
    });
    let mut g = h.start_room_game("jl1");
    h.clock.follow_real_time();
    h.run_until_turn(&mut g, 3);
    // The game's start silenced the room's news; its incomes are said again.
    assert!(
        said(&h).contains(&"andrew , 150 percent income".to_owned()),
        "{:?}",
        said(&h)
    );
    // Everyone in the game hears the computer named at its level.
    assert_eq!(
        g.game.view().game_start().player_names[1],
        soundrts_world::computer_name("easy", Skill::Level2)
    );
    // The server keeps the record of the game when its last player leaves.
    soundrts::net::send(&mut h.client, ClientMsg::LeaveGame);
    let start = std::time::Instant::now();
    let file = loop {
        let found = std::fs::read_dir(&records).ok().and_then(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "jsonl"))
        });
        if let Some(f) = found {
            break f;
        }
        assert!(start.elapsed() < soundrts::testkit::WAIT_LIMIT, "no record");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    server.shutdown();
    let (record, _) = soundrts_world::Record::read(&std::fs::read(&file).unwrap()).unwrap();
    let seats: Vec<_> = record
        .setup()
        .controllers
        .iter()
        .map(|c| (c.kind.clone(), c.income.percent()))
        .collect();
    assert_eq!(
        seats,
        [
            (
                soundrts_world::ControllerKind::Human {
                    login: "andrew".into(),
                    local: true
                },
                150
            ),
            (
                soundrts_world::ControllerKind::Computer {
                    ai_type: "easy".into(),
                    skill: Skill::Level2
                },
                100
            ),
        ]
    );
}
