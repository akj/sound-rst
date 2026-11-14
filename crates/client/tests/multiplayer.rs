//! The multiplayer menus and games against a fake server: a script of
//! server messages at set times on the test clock. What the client says
//! and sends is checked; `net_play.rs` plays against a real server.

use soundrts::clock::Clock;
use soundrts::config::Config;
use soundrts::input::{Key, Mods};
use soundrts::multiplayer::play_on;
use soundrts::testkit::{FakeServer, Harness, LiveKeys, Script, snapshot};
use soundrts_protocol::*;

fn login(s: &str) -> Login {
    Login::new(s).unwrap()
}

fn chat(from: &str, text: &str) -> ServerMsg {
    ServerMsg::Chat {
        line: ChatLine {
            channel: Channel::Lobby,
            from: login(from),
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
    c.announce_arrivals = 0;
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
        let map = jl1(h);
        let slots = vec![
            slot(human("andrew"), 1, true),
            slot(SlotPlayer::Computer(AiLevel::Easy), 2, true),
        ];
        let state = room_state(&map, slots.clone());
        s.at(10.0, room(RoomChange::Entered, &state));
        s.at(
            18.0,
            ServerMsg::GameStarted {
                seed: 1,
                slots,
                you: 0,
                map,
                speed: Speed::Normal,
            },
        );
    });
    let said = said(&h);
    let typed = from(&said, "x . x");
    // The objective of jl1 is said once the game is built.
    assert!(
        typed.iter().any(|s| s.starts_with("objective")),
        "{said:#?}"
    );
    let sent = server.sent();
    assert!(
        !sent.iter().any(|m| matches!(m, ClientMsg::Chat { .. })),
        "{sent:?}"
    );
    assert_eq!(sent.first(), Some(&ClientMsg::Loaded), "{sent:?}");
    // The window closed during the game: the server is told.
    assert_eq!(sent.last(), Some(&ClientMsg::LeaveGame), "{sent:?}");
}

fn two_slots() -> Vec<Slot> {
    vec![
        slot(human("andrew"), 1, true),
        slot(SlotPlayer::Computer(AiLevel::Easy), 2, true),
    ]
}

fn turn(n: Turn) -> ServerMsg {
    ServerMsg::Turn {
        turn: n,
        orders: Vec::new(),
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
    let server = h.join_fake_server("andrew", &[]);
    let mut g = h.start_multiplayer("jl1", &two_slots(), 0, 1, Speed::Normal);
    let t0 = h.clock.now();
    for n in 0..40 {
        server.at(t0 + 0.3 * f64::from(n), turn(n));
    }
    live.tap_at(t0 + 1.0, Key::F7, Mods::NONE);
    live.type_at(t0 + 2.0, 0.5, "gg wp");
    live.tap_at(t0 + 6.0, Key::Return, Mods::NONE);
    h.run_for(&mut g, 1.5);
    let typing_from = g.game.world.turn();
    h.run_for(&mut g, 4.4);
    let typing_to = g.game.world.turn();
    h.run_for(&mut g, 1.0);
    assert!(
        typing_to - typing_from >= 12,
        "turns {typing_from} to {typing_to} while typing"
    );
    let sent = server.sent();
    assert!(
        sent.contains(&ClientMsg::Chat {
            channel: Channel::Game,
            text: ChatText::clean("gg wp").unwrap(),
        }),
        "{sent:?}"
    );
    let done = sent
        .iter()
        .filter(|m| matches!(m, ClientMsg::TurnDone { .. }))
        .count();
    assert_eq!(done as i64, g.game.world.turn());
    // The letters went to the line, not to the game's keys.
    let said = h.said_since(0);
    assert!(said.contains(&"g . g".to_owned()), "{said:#?}");
    assert!(said.contains(&"gg wp".to_owned()), "{said:#?}");
}

#[test]
fn turns_keep_running_while_the_game_menu_is_open() {
    let live = LiveKeys::default();
    let mut h = Harness::with_config(
        "en",
        Script::new().keep_open().with_live(live.clone()),
        config("andrew"),
    );
    let server = h.join_fake_server("andrew", &[]);
    let mut g = h.start_multiplayer("jl1", &two_slots(), 0, 1, Speed::Normal);
    let t0 = h.clock.now();
    for n in 0..60 {
        server.at(t0 + 0.3 * f64::from(n), turn(n));
    }
    live.tap_at(t0 + 1.0, Key::F10, Mods::NONE);
    live.tap_at(t0 + 7.0, Key::Escape, Mods::NONE);
    live.tap_at(t0 + 9.0, Key::F10, Mods::NONE);
    h.run_for(&mut g, 1.5);
    let open_from = g.game.world.turn();
    let said_from = h.said_since(0).len();
    h.run_for(&mut g, 5.0);
    let open_to = g.game.world.turn();
    assert!(
        open_to - open_from >= 15,
        "turns {open_from} to {open_to} with the menu open"
    );
    h.run_for(&mut g, 4.0);
    assert!(live.is_empty(), "every key was read");
    let done = server
        .sent()
        .iter()
        .filter(|m| matches!(m, ClientMsg::TurnDone { .. }))
        .count();
    assert_eq!(done as i64, g.game.world.turn());
    // The news went on with the menu open; Escape chose to continue, and
    // F10 then went to the game, which opened the menu again.
    let said = h.said_since(said_from);
    let (news, last) = said.split_at(said.len().saturating_sub(2));
    assert!(!news.is_empty(), "{said:#?}");
    assert_eq!(last, ["continue game", "Menu"], "{said:#?}");
}

#[test]
fn out_of_sync_says_whether_the_player_leaves_the_game() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let server = h.join_fake_server("andrew", &["bob"]);
    let slots = vec![slot(human("andrew"), 1, true), slot(human("bob"), 2, true)];
    let mut g = h.start_multiplayer("jl1", &slots, 0, 1, Speed::Normal);
    let t0 = h.clock.now();
    server.at(
        t0 + 1.0,
        ServerMsg::OutOfSync {
            turn: 0,
            slots: vec![1],
        },
    );
    server.at(
        t0 + 10.0,
        ServerMsg::OutOfSync {
            turn: 0,
            slots: vec![0, 1],
        },
    );
    let from = h.said_since(0).len();
    h.run_for(&mut g, 5.0);
    let others = h.said_since(from);
    assert!(
        others
            .iter()
            .any(|s| s.ends_with("other players are out of sync: they leave the game")),
        "{others:#?}"
    );
    let from = h.said_since(0).len();
    h.run_for(&mut g, 10.0);
    let you = h.said_since(from);
    assert!(
        you.iter().any(|s| s
            .ends_with("the game is out of sync: you no longer play the same game as the others")),
        "{you:#?}"
    );
}

/// The orders that send each slot's peasants to a square: the next square
/// east or west of their own with `away`, else their own.
fn peasant_orders(g: &soundrts::testkit::TestGame, away: bool) -> Vec<SlotOrders> {
    let world = &g.game.world;
    let net = g.game.net.as_ref().unwrap();
    net.slots
        .iter()
        .enumerate()
        .map(|(slot, &p)| {
            let peasants: Vec<_> = world
                .player(p)
                .units
                .iter()
                .filter_map(|&u| world.entity(u))
                .filter(|e| &*e.type_name == "peasant")
                .collect();
            let home = world.square(peasants[0].square().unwrap());
            let target = if away {
                let next = world
                    .square_at(home.col + 1, home.row)
                    .or_else(|| world.square_at(home.col - 1, home.row))
                    .unwrap();
                world.square(next).name.clone()
            } else {
                home.name.clone()
            };
            let ids: Vec<String> = peasants.iter().map(|e| e.id.0.to_string()).collect();
            let orders = [
                format!("control {}", ids.join(" ")),
                format!("order 0 0 go {target}"),
            ]
            .map(|o| Order::new(o).unwrap());
            SlotOrders {
                slot,
                orders: Orders::new(orders.to_vec()).unwrap(),
            }
        })
        .collect()
}

#[test]
fn a_game_out_of_sync_saves_a_record_that_replays_the_same_game() {
    use soundrts::record::{Check, Difference, GameRecord, RecordedPlayer, Who};

    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let server = h.join_fake_server("andrew", &["bob"]);
    let slots = vec![slot(human("andrew"), 1, true), slot(human("bob"), 2, true)];
    let mut g = h.start_multiplayer("jl1", &slots, 0, 1, Speed::Normal);
    let (away, back) = (peasant_orders(&g, true), peasant_orders(&g, false));
    let t0 = h.clock.now();
    for n in 0..40 {
        let orders = match n {
            5 => away.clone(),
            20 => back.clone(),
            _ => Vec::new(),
        };
        server.at(t0 + 0.3 * f64::from(n), ServerMsg::Turn { turn: n, orders });
    }
    server.at(
        t0 + 13.0,
        ServerMsg::OutOfSync {
            turn: 39,
            slots: vec![1],
        },
    );
    h.run_for(&mut g, 14.0);
    assert_eq!(g.game.world.turn(), 40);

    let path = h
        .client
        .paths
        .replays()
        .join("out-of-sync-andrew-seed1-turn39.jsonl");
    let record = GameRecord::read(&path).unwrap();
    assert_eq!(&record, &g.game.net.as_ref().unwrap().record);
    assert!(
        h.log_text()
            .contains(&format!("game record written to {}", path.display())),
        "{}",
        h.log_text()
    );
    assert_eq!(record.setup.you, 0);
    assert_eq!(record.setup.seed, 1);
    assert_eq!(
        record.setup.players,
        [("andrew", "1"), ("bob", "2")].map(|(login, alliance)| RecordedPlayer {
            who: Who::Human(login.into()),
            faction: "random_faction".into(),
            alliance: Some(alliance.into()),
        })
    );
    let turns: Vec<Turn> = record.turns.iter().map(|t| t.turn).collect();
    assert_eq!(turns, (0..40).collect::<Vec<Turn>>());
    assert_eq!(record.turns[5].orders.len(), 4);
    assert_eq!(record.turns[5].orders[3].slot, 1);
    assert!(record.turns[5].orders[3].order.starts_with("order 0 0 go "));
    assert_eq!(
        record.check(),
        Check::Replayed {
            turns: 40,
            first_difference: None
        }
    );

    // A digest changed: the check names that turn.
    let mut changed = record.clone();
    let digest = record.turns[12].digest;
    changed.turns[12].digest ^= 1;
    assert_eq!(
        changed.check(),
        Check::Replayed {
            turns: 13,
            first_difference: Some(Difference {
                turn: 12,
                recorded: digest ^ 1,
                replayed: digest,
            })
        }
    );

    // Bob's order to go away at turn 5 removed: that turn differs at once.
    let mut missing = record.clone();
    missing.turns[5].orders.remove(3);
    let Check::Replayed {
        turns: 6,
        first_difference: Some(d),
    } = missing.check()
    else {
        panic!("{:?}", missing.check());
    };
    assert_eq!(d.turn, 5);
    assert_eq!(d.recorded, record.turns[5].digest);
}

#[test]
fn turns_that_come_together_are_spread_out() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let server = h.join_fake_server("andrew", &[]);
    let mut g = h.start_multiplayer("jl1", &two_slots(), 0, 1, Speed::Normal);
    let t0 = h.clock.now() + 1.0;
    for n in 0..3 {
        server.at(t0, turn(n));
    }
    let mut times = Vec::new();
    let mut last = g.game.world.turn();
    while times.len() < 3 {
        h.step(&mut g);
        let now = g.game.world.turn();
        if now != last {
            times.push(h.clock.now());
            last = now;
        }
        assert!(h.clock.now() < t0 + 5.0, "the turns did not all run");
    }
    // 300 ms between turns at normal speed, divided by one plus the turns
    // still waiting.
    let gaps = [times[1] - times[0], times[2] - times[1]];
    assert!((gaps[0] - 0.1).abs() < 0.003, "{gaps:?}");
    assert!((gaps[1] - 0.15).abs() < 0.003, "{gaps:?}");
    assert!(times[0] - t0 < 0.003, "{times:?}");
}

#[test]
fn orders_beyond_what_a_turn_takes_wait_for_the_next_turn() {
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    let server = h.join_fake_server("andrew", &[]);
    let mut g = h.start_multiplayer("jl1", &two_slots(), 0, 1, Speed::Normal);
    let command = format!("cmd {}", "x".repeat(996));
    for _ in 0..40 {
        g.game.write_line(&h.client, &command);
    }
    let order_bytes = |server: &FakeServer| -> usize {
        server
            .sent()
            .iter()
            .filter_map(|m| match m {
                ClientMsg::Orders { orders } => Some(
                    orders
                        .as_slice()
                        .iter()
                        .map(|o| o.as_str().len())
                        .sum::<usize>(),
                ),
                _ => None,
            })
            .sum()
    };
    h.run_for(&mut g, 0.5);
    let first = order_bytes(&server);
    assert!(first > 0 && first <= MAX_TURN_ORDER_BYTES / 2, "{first}");
    assert!(!g.game.pending_commands().is_empty());
    server.at(h.clock.now(), turn(0));
    h.run_for(&mut g, 0.5);
    server.at(h.clock.now(), turn(1));
    h.run_for(&mut g, 0.5);
    assert_eq!(order_bytes(&server), 40 * command.len());
    assert!(g.game.pending_commands().is_empty());
}

/// A room with a computer on jl1, entered at 10 s, whose game the server
/// starts at `start` with `you` as the player's slot.
fn game_starting(h: &mut Harness, s: &FakeServer, start: f64, you: SlotIndex) {
    let map = jl1(h);
    let slots = two_slots();
    let state = room_state(&map, slots.clone());
    s.at(10.0, room(RoomChange::Entered, &state));
    s.at(
        start,
        ServerMsg::GameStarted {
            seed: 1,
            slots,
            you,
            map,
            speed: Speed::Normal,
        },
    );
}

#[test]
fn a_game_starting_as_the_player_leaves_the_room_is_played() {
    // Escape and the start in the same pass of the menu.
    let script = Script::new()
        .at(20.0)
        .tap(Key::Escape)
        .at(60.0)
        .tap(Key::LShift);
    let (_, server) = on_fake_server(script, config("andrew"), |h, s| {
        game_starting(h, s, 20.0, 0)
    });
    let sent = server.sent();
    assert!(!sent.contains(&ClientMsg::LeaveRoom), "{sent:?}");
    assert_eq!(sent.first(), Some(&ClientMsg::Loaded), "{sent:?}");
}

#[test]
fn a_game_started_after_the_player_asked_to_leave_is_played() {
    // The server started the game before it read the leave.
    let script = Script::new()
        .at(20.0)
        .tap(Key::Escape)
        .at(60.0)
        .tap(Key::LShift);
    let (_, server) = on_fake_server(script, config("andrew"), |h, s| {
        game_starting(h, s, 25.0, 0)
    });
    let sent = server.sent();
    assert_eq!(
        sent[..2],
        [ClientMsg::LeaveRoom, ClientMsg::Loaded],
        "{sent:?}"
    );
}

#[test]
fn a_game_that_does_not_give_the_player_their_slot_is_left() {
    for you in [1, 7] {
        let script = Script::new().at(40.0).tap(Key::LShift);
        let (h, server) = on_fake_server(script, config("andrew"), |h, s| {
            game_starting(h, s, 20.0, you)
        });
        assert_eq!(server.sent(), [ClientMsg::LeaveGame], "slot {you}");
        let said = said(&h);
        assert!(
            said.contains(&"<1029> this game cannot start here".to_owned()),
            "{said:#?}"
        );
    }
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
    let mut g = h.start_multiplayer("jl1", &two_slots(), 0, 1, Speed::Normal);
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
    h.join_fake_server("andrew", &[]);
    let slots = vec![slot(human("andrew"), 1, true), slot(human("4000"), 2, true)];
    let g = h.start_multiplayer("jl1", &slots, 0, 1, Speed::Normal);
    let other = g.game.world.players[1];
    let name = g.game.world.player(other).name();
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
