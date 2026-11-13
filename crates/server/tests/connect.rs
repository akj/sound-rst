//! Saying hello, being turned away, and the lobby's players.

mod common;

use std::time::Duration;

use common::*;
use soundrts_protocol::*;

#[tokio::test]
async fn a_good_hello_gets_the_lobby() {
    let server = server(|c| c.name = "Andrew's server".into());
    let (_c, lobby) = Client::hello(server.local_addr(), "andrew").await;
    assert_eq!(
        lobby,
        LobbySnapshot {
            server_name: "Andrew's server".into(),
            you: login("andrew"),
            players: vec![LobbyPlayer {
                login: login("andrew"),
                whereabouts: Whereabouts::Lobby,
            }],
            rooms: vec![],
            chat: vec![],
        }
    );
}

#[tokio::test]
async fn a_login_already_taken_gets_a_number() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let (_a, _) = Client::hello(addr, "andrew").await;
    let (_b, lobby) = Client::hello(addr, "andrew").await;
    assert_eq!(lobby.you, login("andrew2"));
    // Logins that differ only in case sound the same.
    let (_c, lobby) = Client::hello(addr, "Andrew").await;
    assert_eq!(lobby.you, login("Andrew3"));
    // The number replaces the end of a login that would be too long.
    let long = "a".repeat(MAX_LOGIN_CHARS);
    let (_d, _) = Client::hello(addr, &long).await;
    let (_e, lobby) = Client::hello(addr, &long).await;
    assert_eq!(
        lobby.you,
        login(&format!("{}2", "a".repeat(MAX_LOGIN_CHARS - 1)))
    );
}

#[tokio::test]
async fn a_login_is_free_again_once_its_player_leaves() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let mut all = clients(addr, &["andrew", "bob"]).await;
    let andrew = all.remove(0);
    andrew.close().await;
    all[0]
        .expect(lobby(LobbyChange::PlayerLeft {
            login: login("andrew"),
        }))
        .await;
    let (_c, lobby) = Client::hello(addr, "andrew").await;
    assert_eq!(lobby.you, login("andrew"));
}

#[tokio::test]
async fn the_lobby_snapshot_then_its_changes() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let (mut a, _) = Client::hello(addr, "andrew").await;
    let (b, lobby) = Client::hello(addr, "bob").await;
    let bob = LobbyPlayer {
        login: login("bob"),
        whereabouts: Whereabouts::Lobby,
    };
    assert_eq!(
        lobby.players,
        vec![
            LobbyPlayer {
                login: login("andrew"),
                whereabouts: Whereabouts::Lobby,
            },
            bob.clone(),
        ]
    );
    a.expect(lobby_change_arrived(bob)).await;
    b.close().await;
    a.expect(common::lobby(LobbyChange::PlayerLeft {
        login: login("bob"),
    }))
    .await;
}

fn lobby_change_arrived(player: LobbyPlayer) -> ServerMsg {
    common::lobby(LobbyChange::PlayerArrived { player })
}

#[tokio::test]
async fn a_hello_of_another_protocol_is_rejected() {
    let server = server(|_| {});
    let mut c = Client::connect(server.local_addr()).await;
    c.send(ClientMsg::Hello {
        protocol: PROTOCOL_VERSION + 1,
        login: login("andrew"),
        content: content(),
    })
    .await;
    c.expect(ServerMsg::Rejected {
        reason: Rejection::ProtocolMismatch {
            server: PROTOCOL_VERSION,
        },
    })
    .await;
    c.expect_closed().await;
}

#[tokio::test]
async fn a_hello_of_another_shape_is_still_a_protocol_mismatch() {
    let server = server(|_| {});
    let mut c = Client::connect(server.local_addr()).await;
    c.send_text(r#"{"type":"hello","protocol":99,"name":"andrew"}"#)
        .await;
    c.expect(ServerMsg::Rejected {
        reason: Rejection::ProtocolMismatch {
            server: PROTOCOL_VERSION,
        },
    })
    .await;
    c.expect_closed().await;
}

#[tokio::test]
async fn a_full_server_rejects() {
    let server = server(|c| c.max_clients = 1);
    let addr = server.local_addr();
    let (_a, _) = Client::hello(addr, "andrew").await;
    let mut b = Client::connect(addr).await;
    b.send(ClientMsg::Hello {
        protocol: PROTOCOL_VERSION,
        login: login("bob"),
        content: content(),
    })
    .await;
    b.expect(ServerMsg::Rejected {
        reason: Rejection::ServerFull,
    })
    .await;
    b.expect_closed().await;
}

#[tokio::test]
async fn a_first_message_that_is_not_a_hello_is_rejected() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let no_hello = ServerMsg::Rejected {
        reason: Rejection::NoHello,
    };

    let mut c = Client::connect(addr).await;
    c.send(ClientMsg::LeaveRoom).await;
    c.expect(no_hello.clone()).await;
    c.expect_closed().await;

    let mut c = Client::connect(addr).await;
    c.send_text("not json").await;
    c.expect(no_hello.clone()).await;
    c.expect_closed().await;

    let mut c = Client::connect(addr).await;
    c.send_binary(b"\x00\x01").await;
    c.expect(no_hello).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn no_hello_in_time_is_rejected() {
    let server = server(|c| c.hello_timeout = Duration::from_millis(300));
    let mut c = Client::connect(server.local_addr()).await;
    c.expect(ServerMsg::Rejected {
        reason: Rejection::NoHello,
    })
    .await;
    c.expect_closed().await;
}

#[tokio::test]
async fn a_second_hello_is_refused() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    c.send(ClientMsg::Hello {
        protocol: PROTOCOL_VERSION,
        login: login("bob"),
        content: content(),
    })
    .await;
    c.expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn unreadable_messages_are_refused_then_the_server_hangs_up() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    // A good message in between resets the count.
    for _ in 0..4 {
        c.send_text("{}").await;
        c.expect(refused(Refusal::Invalid)).await;
    }
    c.send(ClientMsg::LeaveRoom).await;
    c.expect(refused(Refusal::NotNow)).await;
    for _ in 0..4 {
        c.send_text(r#"{"type":"chat","channel":"lobby","text":" padded "}"#)
            .await;
        c.expect(refused(Refusal::Invalid)).await;
    }
    c.send_binary(b"bytes").await;
    c.expect(refused(Refusal::Invalid)).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn a_flood_of_messages_is_slowed_down() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    for _ in 0..300 {
        c.send(ClientMsg::LeaveRoom).await;
    }
    let mut slow_down = 0;
    for _ in 0..300 {
        match c.recv().await {
            ServerMsg::Refused {
                reason: Refusal::NotNow,
            } => {}
            ServerMsg::Refused {
                reason: Refusal::SlowDown,
            } => slow_down += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(slow_down > 0, "the flood was never slowed down");
}

#[tokio::test]
async fn the_server_answers_discovery_on_its_port() {
    let server = server(|c| c.name = "lan party".into());
    let addr = server.local_addr();
    let (_a, _) = Client::hello(addr, "andrew").await;
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(DISCOVERY_REQUEST, addr).await.unwrap();
    let mut buf = [0u8; 1024];
    let (n, _) = tokio::time::timeout(TIMEOUT, socket.recv_from(&mut buf))
        .await
        .expect("an answer in time")
        .unwrap();
    let answer = ServerAnnouncement::from_json(std::str::from_utf8(&buf[..n]).unwrap()).unwrap();
    assert_eq!(
        answer,
        ServerAnnouncement {
            protocol: PROTOCOL_VERSION,
            name: "lan party".into(),
            port: addr.port(),
            players: 1,
        }
    );
}

#[tokio::test]
async fn shutting_down_closes_connections() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    server.shutdown();
    c.expect_closed().await;
}
