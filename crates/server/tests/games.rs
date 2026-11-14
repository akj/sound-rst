//! The turn clock, orders, waiting for slow players, out-of-sync
//! detection, and leaving a game.

mod common;

use std::time::Duration;

use common::*;
use soundrts_protocol::*;

/// A lead no test reaches, for tests that are not about waiting: they
/// then never see `Waiting`, however slow the machine.
const NO_WAIT: u32 = 1_000_000;

/// Tests about waiting give the player who keeps up this many turns of
/// margin (750 ms at the fastest speed) before a slow test machine would
/// make them late too.
const LEAD: u32 = 10;

fn order(s: &str) -> Order {
    Order::new(s).unwrap()
}

fn orders(list: &[&str]) -> Orders {
    Orders::new(list.iter().map(|s| order(s)).collect()).unwrap()
}

fn slot_orders(slot: SlotIndex, list: &[&str]) -> SlotOrders {
    SlotOrders {
        slot,
        orders: orders(list),
    }
}

/// Andrew hosts and Bob joins a room on a 2 to 4 player map, and they
/// start a game and load; the `extra` players stay in the lobby.
async fn two_player_game(
    tweak: impl FnOnce(&mut soundrts_server::ServerConfig),
    extra: &[&str],
) -> (soundrts_server::ServerHandle, Vec<Client>, RoomId) {
    let (server, mut all, room) = two_player_game_unloaded(tweak, extra).await;
    for c in &mut all[..2] {
        c.send(ClientMsg::Loaded).await;
    }
    (server, all, room)
}

/// As [`two_player_game`], but nobody has loaded.
async fn two_player_game_unloaded(
    tweak: impl FnOnce(&mut soundrts_server::ServerConfig),
    extra: &[&str],
) -> (soundrts_server::ServerHandle, Vec<Client>, RoomId) {
    let server = server(tweak);
    let mut names = vec!["andrew", "bob"];
    names.extend_from_slice(extra);
    let mut all = clients(server.local_addr(), &names).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;
    ready(&mut all, 1, &[0, 1]).await;
    start_game_unloaded(&mut all, room, &[0, 1]).await;
    (server, all, room)
}

/// The first turn at or after index `from` of the client's recorded turns,
/// receiving more if needed.
async fn turn_at(c: &mut Client, from: usize) -> (Turn, Vec<SlotOrders>) {
    while c.turns.len() <= from {
        c.next_turn().await;
    }
    c.turns[from].clone()
}

#[tokio::test]
async fn turns_come_in_order_and_carry_orders_received_before_them() {
    let server = server(|c| c.max_lead = NO_WAIT);
    let mut all = clients(server.local_addr(), &["andrew"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    add_computer(&mut all, room).await;
    start_game(&mut all, room, &[0]).await;
    let a = &mut all[0];

    a.play_to(3, 0).await;
    let sent_after = a.turns.last().unwrap().0;
    a.send(ClientMsg::Orders {
        orders: orders(&["order 0 0 go 12", "order 0 0 stop"]),
    })
    .await;
    let carried = loop {
        let (turn, list) = a.next_turn().await;
        a.send(ClientMsg::TurnDone { turn, digest: 0 }).await;
        a.done = Some(turn);
        if !list.is_empty() {
            break turn;
        }
    };
    assert!(carried > sent_after);
    let numbers: Vec<Turn> = a.turns.iter().map(|(t, _)| *t).collect();
    assert_eq!(numbers, (0..=carried).collect::<Vec<_>>());
    for (turn, list) in &a.turns {
        if *turn == carried {
            assert_eq!(
                *list,
                vec![slot_orders(0, &["order 0 0 go 12", "order 0 0 stop"])]
            );
        } else {
            assert!(list.is_empty(), "turn {turn} has orders {list:?}");
        }
    }

    // A turn reported twice, or before it came, is refused.
    a.send(ClientMsg::TurnDone {
        turn: carried,
        digest: 0,
    })
    .await;
    a.expect_event(false, refused(Refusal::NotNow)).await;
    a.send(ClientMsg::TurnDone {
        turn: carried + 1000,
        digest: 0,
    })
    .await;
    a.expect_event(false, refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn orders_go_into_the_next_turn_by_slot_then_arrival() {
    // Nobody reports turns, so the clock stops after three; the orders
    // sent while it waits all go into the turn after the wait.
    let (_server, mut all, _) = two_player_game(|c| c.max_lead = 2, &[]).await;
    let both = vec![login("andrew"), login("bob")];
    for c in &mut all {
        c.expect_event(
            false,
            ServerMsg::Waiting {
                players: both.clone(),
            },
        )
        .await;
        let numbers: Vec<Turn> = c.turns.iter().map(|(t, _)| *t).collect();
        assert_eq!(numbers, [0, 1, 2]);
        assert!(c.turns.iter().all(|(_, o)| o.is_empty()));
    }

    all[1]
        .send(ClientMsg::Orders {
            orders: orders(&["b1"]),
        })
        .await;
    all[1]
        .send(ClientMsg::Orders {
            orders: orders(&["b2", "b3"]),
        })
        .await;
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&["a1"]),
        })
        .await;
    for c in &mut all {
        c.ack_turns(0).await;
    }
    for c in &mut all {
        c.expect(ServerMsg::Resumed).await;
        c.expect(ServerMsg::Turn {
            turn: 3,
            orders: vec![slot_orders(0, &["a1"]), slot_orders(1, &["b1", "b2", "b3"])],
        })
        .await;
    }
}

#[tokio::test]
async fn more_orders_than_fit_in_a_message_are_split_in_order() {
    let (_server, mut all, _) = two_player_game(|c| c.max_lead = 0, &[]).await;
    // max_lead 0: no turn comes until both report the last one.
    for c in &mut all {
        wait_for_turn(c, 0).await;
    }
    let first: Vec<String> = (0..MAX_ORDERS_PER_MESSAGE)
        .map(|i| format!("a{i}"))
        .collect();
    let refs: Vec<&str> = first.iter().map(String::as_str).collect();
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&refs),
        })
        .await;
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&["last"]),
        })
        .await;
    for c in &mut all {
        c.send(ClientMsg::TurnDone { turn: 0, digest: 0 }).await;
    }
    for c in &mut all {
        wait_for_turn(c, 1).await;
        assert_eq!(
            c.turns[1],
            (1, vec![slot_orders(0, &refs), slot_orders(0, &["last"])])
        );
    }
}

#[tokio::test]
async fn orders_beyond_the_limit_for_a_turn_are_refused() {
    let (_server, mut all, _) = two_player_game(|c| c.max_lead = 0, &[]).await;
    // max_lead 0: no turn comes until both report the last one, so the
    // refusal is the next message.
    for c in &mut all {
        wait_for_turn(c, 0).await;
    }
    let big = "x".repeat(MAX_ORDER_BYTES);
    let fill: Vec<&str> = vec![big.as_str(); MAX_TURN_ORDER_BYTES / MAX_ORDER_BYTES];
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&fill),
        })
        .await;
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&["one too many"]),
        })
        .await;
    all[0].expect(refused(Refusal::SlowDown)).await;
    for c in &mut all {
        c.send(ClientMsg::TurnDone { turn: 0, digest: 0 }).await;
    }
    for c in &mut all {
        wait_for_turn(c, 1).await;
        assert_eq!(c.turns[1], (1, vec![slot_orders(0, &fill)]));
    }
}

/// Reads `n` more turns and checks that `slot` has no orders in any.
async fn no_orders_from(c: &mut Client, slot: SlotIndex, n: usize) {
    let from = c.turns.len();
    for i in from..from + n {
        let (turn, orders) = turn_at(c, i).await;
        assert!(
            orders.iter().all(|o| o.slot != slot),
            "slot {slot} has orders in turn {turn}: {orders:?}"
        );
    }
}

#[tokio::test]
async fn a_slow_player_makes_the_game_wait_until_they_catch_up() {
    let (_server, mut all, _) = two_player_game(|c| c.max_lead = LEAD, &[]).await;
    let bob = vec![login("bob")];
    all[0]
        .expect_event(
            true,
            ServerMsg::Waiting {
                players: bob.clone(),
            },
        )
        .await;
    all[1]
        .expect_event(
            false,
            ServerMsg::Waiting {
                players: bob.clone(),
            },
        )
        .await;
    // Bob had every turn the clock sent: max_lead + 1 of them.
    let numbers: Vec<Turn> = all[1].turns.iter().map(|(t, _)| *t).collect();
    assert_eq!(numbers, (0..=LEAD).collect::<Vec<_>>());

    all[1].ack_turns(0).await;
    all[0].expect_event(true, ServerMsg::Resumed).await;
    all[1].expect(ServerMsg::Resumed).await;
    let (turn, _) = all[1].next_turn().await;
    assert_eq!(turn, LEAD + 1);
}

#[tokio::test]
async fn a_player_who_keeps_the_game_waiting_too_long_is_taken_out() {
    let (_server, mut all, _) = two_player_game(
        |c| {
            c.max_lead = LEAD;
            c.lag_timeout = Duration::from_secs(1);
        },
        &[],
    )
    .await;
    let bob = login("bob");
    all[0]
        .expect_event(
            true,
            ServerMsg::Waiting {
                players: vec![bob.clone()],
            },
        )
        .await;
    all[0]
        .expect_event(true, ServerMsg::PlayerLeftGame { slot: 1 })
        .await;
    let left_at = all[0].turns.len();
    all[0].expect_event(true, ServerMsg::Resumed).await;
    all[0]
        .expect_event(true, moved(&bob, Whereabouts::Lobby))
        .await;
    assert_eq!(
        turn_at(&mut all[0], left_at).await.1,
        vec![slot_orders(1, &["quit"])]
    );
    // The quit comes once.
    no_orders_from(&mut all[0], 1, 3).await;

    all[1]
        .expect_event(
            false,
            ServerMsg::Waiting {
                players: vec![bob.clone()],
            },
        )
        .await;
    all[1].expect_event(false, ServerMsg::GameEnded).await;
    all[1].expect(moved(&bob, Whereabouts::Lobby)).await;
    // Back in the lobby, for good.
    all[1]
        .send(ClientMsg::TurnDone { turn: 0, digest: 0 })
        .await;
    all[1].expect(refused(Refusal::NotNow)).await;
}

/// Andrew, Bob and Carol in a game where the clock waits for everyone
/// (max_lead 0), each having reported turn 0 with their digest.
async fn three_player_game_after_turn_0(
    digests: [Digest; 3],
) -> (soundrts_server::ServerHandle, Vec<Client>) {
    let server = server(|c| c.max_lead = 0);
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let room = open_room(&mut all, 0, &map(3, 4), true).await;
    join_room(&mut all, 1, room).await;
    join_room(&mut all, 2, room).await;
    ready(&mut all, 1, &[0, 1, 2]).await;
    ready(&mut all, 2, &[0, 1, 2]).await;
    start_game(&mut all, room, &[0, 1, 2]).await;
    // With max_lead 0, turn t + 1 comes only after everyone reported t, so
    // a desync of turn t is always told before turn t + 1.
    for (c, digest) in all.iter_mut().zip(digests) {
        wait_for_turn(c, 0).await;
        c.send(ClientMsg::TurnDone { turn: 0, digest }).await;
    }
    (server, all)
}

/// The next turn, skipping the messages that come and go with the speed of
/// the test and the lobby's news of players moving.
async fn next_turn_ignoring_news(c: &mut Client) -> Turn {
    loop {
        match next_ignoring_waits(c).await {
            ServerMsg::Turn { turn, .. } => return turn,
            ServerMsg::Lobby {
                change: LobbyChange::PlayerMoved { .. },
            } => {}
            other => panic!("{}: expected a turn, got {other:?}", c.login),
        }
    }
}

#[tokio::test]
async fn out_of_sync_takes_the_odd_player_out() {
    let (_server, mut all) = three_player_game_after_turn_0([7, 7, 8]).await;
    let out = ServerMsg::OutOfSync {
        turn: 0,
        slots: vec![2],
    };
    assert_eq!(next_ignoring_waits(&mut all[2]).await, out);
    assert_eq!(next_ignoring_waits(&mut all[2]).await, ServerMsg::GameEnded);
    all[2]
        .expect(moved(&login("carol"), Whereabouts::Lobby))
        .await;
    for c in &mut all[..2] {
        assert_eq!(next_ignoring_waits(c).await, out);
        assert_eq!(
            next_ignoring_waits(c).await,
            ServerMsg::PlayerLeftGame { slot: 2 }
        );
    }
    // The others play on, and their digests are still compared.
    for turn in 1..4 {
        for c in &mut all[..2] {
            assert_eq!(next_turn_ignoring_news(c).await, turn);
            c.send(ClientMsg::TurnDone { turn, digest: 1 }).await;
        }
    }
    for (c, digest) in all[..2].iter_mut().zip([1, 2]) {
        assert_eq!(next_turn_ignoring_news(c).await, 4);
        c.send(ClientMsg::TurnDone { turn: 4, digest }).await;
    }
    for c in &mut all[..2] {
        assert_eq!(
            next_ignoring_waits(c).await,
            ServerMsg::OutOfSync {
                turn: 4,
                slots: vec![0, 1],
            }
        );
    }
    all[0].expect(ServerMsg::GameEnded).await;
}

#[tokio::test]
async fn out_of_sync_on_a_tie_ends_the_game_for_everyone() {
    let server = server(|c| c.max_lead = 0);
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;
    ready(&mut all, 1, &[0, 1]).await;
    start_game(&mut all, room, &[0, 1]).await;
    for (c, digest) in all.iter_mut().zip([7, 8]) {
        wait_for_turn(c, 0).await;
        c.send(ClientMsg::TurnDone { turn: 0, digest }).await;
    }
    let out = ServerMsg::OutOfSync {
        turn: 0,
        slots: vec![0, 1],
    };
    assert_eq!(next_ignoring_waits(&mut all[0]).await, out);
    all[0].expect(ServerMsg::GameEnded).await;
    assert_eq!(next_ignoring_waits(&mut all[1]).await, out);
    all[1].skip_to(&ServerMsg::GameEnded).await;
}

#[tokio::test]
async fn out_of_sync_players_all_leave_before_the_next_turn_is_compared() {
    let server = server(|c| c.max_lead = NO_WAIT);
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol", "dave"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    for who in 1..4 {
        join_room(&mut all, who, room).await;
    }
    for who in 1..4 {
        ready(&mut all, who, &[0, 1, 2, 3]).await;
    }
    start_game(&mut all, room, &[0, 1, 2, 3]).await;
    // Bob, Carol and Dave report turns 0 and 1; Bob differs in both.
    for (c, digests) in all[1..].iter_mut().zip([[8, 9], [7, 7], [7, 7]]) {
        wait_for_turn(c, 1).await;
        for (turn, digest) in (0..).zip(digests) {
            c.send(ClientMsg::TurnDone { turn, digest }).await;
        }
    }
    // Andrew reports turn 0 last, and differs too. Once he is out, turn 1
    // has every digest it waits for, Bob's included.
    wait_for_turn(&mut all[0], 0).await;
    all[0]
        .send(ClientMsg::TurnDone { turn: 0, digest: 6 })
        .await;
    let mut told = Vec::new();
    while told.last() != Some(&ServerMsg::PlayerLeftGame { slot: 1 }) {
        let msg = all[2].recv().await;
        if matches!(
            msg,
            ServerMsg::OutOfSync { .. } | ServerMsg::PlayerLeftGame { .. }
        ) {
            told.push(msg);
        }
    }
    assert_eq!(
        told,
        [
            ServerMsg::OutOfSync {
                turn: 0,
                slots: vec![0, 1],
            },
            ServerMsg::PlayerLeftGame { slot: 0 },
            ServerMsg::PlayerLeftGame { slot: 1 },
        ]
    );
}

/// Waits for turn `n` if it has not come yet.
async fn wait_for_turn(c: &mut Client, n: Turn) {
    while !c.turns.iter().any(|(t, _)| *t == n) {
        match next_ignoring_waits(c).await {
            ServerMsg::Turn { turn, orders } => c.turns.push((turn, orders)),
            other => panic!("expected a turn, got {other:?}"),
        }
    }
}

/// The next message other than `Waiting` and `Resumed`, which come and go
/// with the speed of the test.
async fn next_ignoring_waits(c: &mut Client) -> ServerMsg {
    loop {
        match c.recv().await {
            ServerMsg::Waiting { .. } | ServerMsg::Resumed => {}
            other => return other,
        }
    }
}

#[tokio::test]
async fn leaving_a_game_sends_quit_and_the_last_one_out_closes_the_room() {
    let (_server, mut all, room) = two_player_game(|c| c.max_lead = NO_WAIT, &["carol"]).await;
    let (andrew, bob) = (login("andrew"), login("bob"));

    all[1].send(ClientMsg::LeaveGame).await;
    all[0]
        .expect_event(false, ServerMsg::PlayerLeftGame { slot: 1 })
        .await;
    let left_at = all[0].turns.len();
    all[0]
        .expect_event(false, moved(&bob, Whereabouts::Lobby))
        .await;
    assert_eq!(
        turn_at(&mut all[0], left_at).await.1,
        vec![slot_orders(1, &["quit"])]
    );
    // The quit comes once.
    no_orders_from(&mut all[0], 1, 3).await;
    all[1].expect_event(false, ServerMsg::GameEnded).await;
    all[1].expect(moved(&bob, Whereabouts::Lobby)).await;
    all[2].expect(moved(&bob, Whereabouts::Lobby)).await;

    // Out of the game, game messages are refused.
    for msg in [
        ClientMsg::Orders {
            orders: orders(&["x"]),
        },
        ClientMsg::TurnDone { turn: 0, digest: 0 },
        ClientMsg::LeaveGame,
    ] {
        all[1].send(msg).await;
        all[1].expect(refused(Refusal::NotNow)).await;
    }

    all[0].send(ClientMsg::LeaveGame).await;
    all[0].expect_event(false, ServerMsg::GameEnded).await;
    for c in &mut all {
        c.expect_event(false, moved(&andrew, Whereabouts::Lobby))
            .await;
        c.expect(lobby(LobbyChange::RoomClosed { id: room })).await;
    }
}

#[tokio::test]
async fn disconnecting_from_a_game_sends_quit_and_the_last_one_out_closes_the_room() {
    let (_server, mut all, room) = two_player_game(|c| c.max_lead = NO_WAIT, &["carol"]).await;
    let bob = all.remove(1);
    bob.close().await;
    all[0]
        .expect_event(
            false,
            lobby(LobbyChange::PlayerLeft {
                login: login("bob"),
            }),
        )
        .await;
    all[0]
        .expect_event(false, ServerMsg::PlayerLeftGame { slot: 1 })
        .await;
    let left_at = all[0].turns.len();
    assert_eq!(
        turn_at(&mut all[0], left_at).await.1,
        vec![slot_orders(1, &["quit"])]
    );
    // The quit comes once.
    no_orders_from(&mut all[0], 1, 3).await;

    let andrew = all.remove(0);
    andrew.close().await;
    let carol = &mut all[0];
    carol
        .expect(lobby(LobbyChange::PlayerLeft {
            login: login("bob"),
        }))
        .await;
    carol
        .expect(lobby(LobbyChange::PlayerLeft {
            login: login("andrew"),
        }))
        .await;
    carol
        .expect(lobby(LobbyChange::RoomClosed { id: room }))
        .await;
}

#[tokio::test]
async fn no_turn_comes_before_everyone_has_loaded() {
    let (_server, mut all, _) = two_player_game_unloaded(|c| c.max_lead = NO_WAIT, &[]).await;
    all[0].send(ClientMsg::Loaded).await;
    // Several turn intervals pass: the clock must not start without Bob.
    tokio::time::sleep(Duration::from_millis(400)).await;
    all[0].send(ClientMsg::Loaded).await;
    all[0].expect(refused(Refusal::NotNow)).await;
    all[1]
        .send(ClientMsg::TurnDone { turn: 0, digest: 0 })
        .await;
    all[1].expect(refused(Refusal::NotNow)).await;

    all[1].send(ClientMsg::Loaded).await;
    for c in &mut all {
        c.expect(ServerMsg::Turn {
            turn: 0,
            orders: vec![],
        })
        .await;
    }
    // Loading is over.
    all[1].send(ClientMsg::Loaded).await;
    all[1].expect_event(false, refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_player_who_never_loads_is_taken_out_and_the_game_starts_without_them() {
    let (_server, mut all, _) = two_player_game_unloaded(
        |c| {
            c.max_lead = NO_WAIT;
            c.lag_timeout = Duration::from_millis(500);
        },
        &[],
    )
    .await;
    let bob = login("bob");
    all[0].send(ClientMsg::Loaded).await;
    all[0].expect(ServerMsg::PlayerLeftGame { slot: 1 }).await;
    all[0]
        .expect_event(false, moved(&bob, Whereabouts::Lobby))
        .await;
    assert_eq!(
        turn_at(&mut all[0], 0).await,
        (0, vec![slot_orders(1, &["quit"])])
    );

    // Bob gets no turn, and no word of waiting.
    all[1].expect(ServerMsg::GameEnded).await;
    all[1].expect(moved(&bob, Whereabouts::Lobby)).await;
}

#[tokio::test]
async fn orders_sent_while_loading_go_into_turn_0() {
    let (_server, mut all, _) = two_player_game_unloaded(|c| c.max_lead = NO_WAIT, &[]).await;
    all[1]
        .send(ClientMsg::Orders {
            orders: orders(&["b1"]),
        })
        .await;
    all[0]
        .send(ClientMsg::Orders {
            orders: orders(&["a1"]),
        })
        .await;
    for c in &mut all {
        c.send(ClientMsg::Loaded).await;
    }
    for c in &mut all {
        c.expect(ServerMsg::Turn {
            turn: 0,
            orders: vec![slot_orders(0, &["a1"]), slot_orders(1, &["b1"])],
        })
        .await;
    }
}
