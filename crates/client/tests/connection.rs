//! The connection to a server against servers that misbehave: one that
//! floods the client, one that stops reading. Each is a bare WebSocket
//! server on 127.0.0.1 written here.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use soundrts::net::conn::{self, EVENT_QUEUE, NetEvent};
use soundrts::testkit::{Harness, Script};
use soundrts_protocol::*;
use tokio_tungstenite::tungstenite::Message;

fn hello() -> ClientMsg {
    ClientMsg::Hello {
        protocol: PROTOCOL_VERSION,
        login: Login::new("andrew").unwrap(),
        content: ContentHash::new("x").unwrap(),
    }
}

fn welcome() -> ServerMsg {
    ServerMsg::Welcome {
        lobby: LobbySnapshot {
            server_name: "bad".into(),
            you: Login::new("andrew").unwrap(),
            players: Vec::new(),
            rooms: Vec::new(),
            chat: Vec::new(),
        },
    }
}

fn line(i: usize) -> ServerMsg {
    ServerMsg::Chat {
        line: ChatLine {
            channel: Channel::Lobby,
            from: Login::new("flood").unwrap(),
            text: ChatText::clean(&format!("{i} {}", "x".repeat(390))).unwrap(),
        },
    }
}

/// A server for one client on its own thread: it welcomes the client,
/// then runs `then` on the socket.
fn bad_server<F, Fut>(then: F) -> SocketAddr
where
    F: FnOnce(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.next().await;
            ws.send(Message::Text(welcome().to_json().into()))
                .await
                .unwrap();
            then(ws).await;
        });
    });
    addr
}

#[test]
fn a_server_that_floods_the_client_waits_for_it() {
    const TOTAL: usize = 100_000;
    let sent = Arc::new(AtomicUsize::new(0));
    let counter = sent.clone();
    let addr = bad_server(move |mut ws| async move {
        for i in 0..TOTAL {
            if ws
                .send(Message::Text(line(i).to_json().into()))
                .await
                .is_err()
            {
                return;
            }
            counter.store(i + 1, Ordering::SeqCst);
        }
    });
    let (mut conn, _) = conn::open(&addr.to_string(), hello(), Duration::from_secs(5)).unwrap();
    // The client reads nothing for a while: the server cannot send it all
    // (about 40 MB), the socket's buffers and the queue being full.
    std::thread::sleep(Duration::from_secs(2));
    let stalled = sent.load(Ordering::SeqCst);
    assert!(
        stalled < TOTAL / 2,
        "{stalled} messages sent to a client that reads none"
    );
    // Then everything comes, in order, as the client reads.
    let mut got = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while got < TOTAL {
        match conn.poll() {
            Some(NetEvent::Msg(m)) => {
                assert_eq!(m, line(got));
                got += 1;
            }
            Some(NetEvent::Closed(r)) => panic!("closed after {got}: {r}"),
            None => {
                assert!(Instant::now() < deadline, "{got} messages read");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    assert!(EVENT_QUEUE < TOTAL);
}

#[test]
fn a_connection_to_a_server_that_stops_reading_closes_in_time() {
    let addr = bad_server(|ws| async move {
        // Never reads again, and keeps the socket open.
        tokio::time::sleep(Duration::from_secs(60)).await;
        drop(ws);
    });
    let (mut conn, _) = conn::open(&addr.to_string(), hello(), Duration::from_secs(5)).unwrap();
    // Far more than the socket's buffers hold.
    let text = ChatText::clean(&"y".repeat(400)).unwrap();
    for _ in 0..200_000 {
        conn.send(ClientMsg::Chat {
            channel: Channel::Lobby,
            text: text.clone(),
        });
    }
    std::thread::sleep(Duration::from_millis(500));
    let start = Instant::now();
    drop(conn);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "closing took {:?}",
        start.elapsed()
    );
}

#[test]
fn one_pass_of_the_menus_reads_a_bounded_number_of_messages() {
    let mut h = Harness::new("en", Script::new());
    let server = h.join_fake_server("andrew", &[]);
    for i in 0..200 {
        server.at(0.0, line(i));
    }
    soundrts::net::pump(&mut h.client);
    let read = h.client.net.as_ref().unwrap().chat.len();
    assert_eq!(read, soundrts::net::PUMP_LIMIT);
    for _ in 0..10 {
        soundrts::net::pump(&mut h.client);
    }
    assert_eq!(
        h.client.net.as_ref().unwrap().chat.len(),
        100,
        "the log keeps 100"
    );
}
