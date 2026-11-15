//! Saying hello, being turned away, and the lobby's players.

mod common;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};

use common::*;
use soundrts_protocol::*;

#[tokio::test]
async fn a_good_hello_gets_the_lobby() {
    let server = server(|c| c.name = server_name("Andrew's server"));
    let (_c, lobby) = Client::hello(server.local_addr(), "andrew").await;
    assert_eq!(
        lobby,
        LobbySnapshot {
            server_name: server_name("Andrew's server"),
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
async fn connections_that_have_not_said_hello_are_limited() {
    let server = server(|_| {});
    let addr = server.local_addr();
    // Connections that send nothing, accepted before the next one.
    let mut silent = Vec::new();
    for _ in 0..soundrts_server::MAX_PENDING_HELLOS {
        silent.push(tokio::net::TcpStream::connect(addr).await.unwrap());
    }
    let one_more = tokio::time::timeout(
        TIMEOUT,
        tokio_tungstenite::connect_async(format!("ws://{addr}")),
    )
    .await
    .expect("an answer in time");
    assert!(
        one_more.is_err(),
        "a connection beyond the limit was served"
    );
    // Once they are gone, players get in again.
    drop(silent);
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Ok((mut ws, _)) = tokio_tungstenite::connect_async(format!("ws://{addr}")).await {
            let hello = ClientMsg::Hello {
                protocol: PROTOCOL_VERSION,
                login: login("andrew"),
                content: content(),
            };
            ws.send(hello.to_json().into()).await.unwrap();
            let welcome = ws.next().await.unwrap().unwrap();
            assert!(welcome.to_text().unwrap().contains("welcome"), "{welcome}");
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "never served again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_connection_cannot_send_much_before_its_hello() {
    let server = server(|_| {});
    let mut c = Client::connect(server.local_addr()).await;
    // Far more than a hello: the server refuses it and hangs up, where
    // it would otherwise read it all and answer that it is no hello.
    c.send_text_lossy(&"x".repeat(1024 * 1024)).await;
    c.expect_closed().await;
}

#[tokio::test]
async fn a_first_frame_larger_than_a_hello_is_refused_before_it_comes() {
    use tokio::io::AsyncWriteExt;
    // Long enough that only refusing the frame closes the connection in
    // time.
    let server = server(|c| c.hello_timeout = Duration::from_secs(60));
    let mut c = Client::connect(server.local_addr()).await;
    // The header of a masked text frame of 20 MB, then nothing.
    let mut header = vec![0x81, 0xFF];
    header.extend_from_slice(&20_000_000u64.to_be_bytes());
    header.extend_from_slice(&[1, 2, 3, 4]);
    c.tcp().write_all(&header).await.unwrap();
    c.expect_closed().await;
}

#[tokio::test]
async fn a_message_larger_than_the_server_holds_closes_the_connection() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    // About 1.5 MB, where the server holds 1 MB of messages other than
    // maps for a connection.
    let (mut info, _) = map(2, 4);
    info.title_sounds = vec![1000; 300_000];
    c.send(ClientMsg::CreateRoom {
        map: info,
        speed: Speed::Normal,
        open: true,
        watchers: true,
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
    let server = server(|c| c.name = server_name("lan party"));
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
            name: server_name("lan party"),
            port: addr.port(),
            players: 1,
        }
    );
}

#[tokio::test]
async fn discovery_answers_are_rate_limited() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for _ in 0..100 {
        socket.send_to(DISCOVERY_REQUEST, addr).await.unwrap();
    }
    // The first answer may be slow on a busy machine; the others follow at
    // once, and a quiet second means no more are coming.
    let mut buf = [0u8; 1024];
    tokio::time::timeout(TIMEOUT, socket.recv_from(&mut buf))
        .await
        .expect("an answer in time")
        .unwrap();
    let mut answers = 1;
    while let Ok(Ok(_)) =
        tokio::time::timeout(Duration::from_secs(1), socket.recv_from(&mut buf)).await
    {
        answers += 1;
    }
    // A burst of 20, then 5 a second: 50 would take 6 seconds to answer.
    assert!(answers < 50, "{answers} answers to 100 requests");
}

#[tokio::test]
async fn a_server_on_any_port_answers_discovery_when_the_next_ports_are_taken_for_udp() {
    // Systems hand out ports in sequence: on Windows a block of them can be
    // reserved for UDP. Holding the next ports on UDP stands for that.
    let next = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _held: Vec<std::net::UdpSocket> = (1..=40u16)
        .filter_map(|i| std::net::UdpSocket::bind(("127.0.0.1", next.checked_add(i)?)).ok())
        .collect();
    let server = server(|_| {});
    let addr = server.local_addr();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(DISCOVERY_REQUEST, addr).await.unwrap();
    let mut buf = [0u8; 1024];
    let (n, _) = tokio::time::timeout(TIMEOUT, socket.recv_from(&mut buf))
        .await
        .expect("an answer in time")
        .expect("an answer, not a request that reached no one");
    let answer = ServerAnnouncement::from_json(std::str::from_utf8(&buf[..n]).unwrap()).unwrap();
    assert_eq!(answer.port, addr.port());
}

#[tokio::test]
async fn shutting_down_closes_connections() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    server.shutdown();
    c.expect_closed().await;
}
