//! Encrypted connections (`wss://`), to a real server on 127.0.0.1 behind
//! a TLS proxy written here, as a server on the Internet runs behind Caddy
//! or nginx. Each test makes its certificates in memory, and the client
//! trusts only the test's own root, through the same code that trusts the
//! built-in roots. Nothing is written to disk or to a system store.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use soundrts::config::Config;
use soundrts::net::conn::{
    self, BadCertificate, ConnectError, Liveness, ServerAddress, Transport, Trust,
};
use soundrts::net::servers;
use soundrts::testkit::{Cable, Harness, Script, TestGame, WAIT_LIMIT, test_server};
use soundrts_protocol::{ClientMsg, Computer, ContentHash, Login, PROTOCOL_VERSION, Skill, Speed};

/// A root of trust made for one test, and what it signs.
struct Pki {
    issuer: Issuer<'static, KeyPair>,
    roots: rustls::RootCertStore,
}

impl Pki {
    fn new() -> Pki {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "sound-rst test root");
        let key = KeyPair::generate().unwrap();
        let root = params.self_signed(&key).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(root.der().clone()).unwrap();
        Pki {
            issuer: Issuer::new(params, key),
            roots,
        }
    }

    /// What the client trusts: this root only.
    fn trust(&self) -> Trust {
        Trust::only(self.roots.clone())
    }

    /// A server's certificate for `name`, signed by the root.
    fn certificate(&self, name: &str) -> Certified {
        let (params, key) = leaf(name);
        let cert = params.signed_by(&key, &self.issuer).unwrap();
        Certified::new(cert.der().clone(), &key)
    }

    /// As [`Pki::certificate`], valid only in 2000.
    fn expired(&self, name: &str) -> Certified {
        let (mut params, key) = leaf(name);
        params.not_before = date_time_ymd(2000, 1, 1);
        params.not_after = date_time_ymd(2001, 1, 1);
        let cert = params.signed_by(&key, &self.issuer).unwrap();
        Certified::new(cert.der().clone(), &key)
    }
}

/// A certificate signed by its own key, which no root vouches for.
fn self_signed(name: &str) -> Certified {
    let (params, key) = leaf(name);
    let cert = params.self_signed(&key).unwrap();
    Certified::new(cert.der().clone(), &key)
}

fn leaf(name: &str) -> (CertificateParams, KeyPair) {
    let mut params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    (params, KeyPair::generate().unwrap())
}

/// A certificate and its private key, as a TLS server gives them.
struct Certified {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl Certified {
    fn new(cert: CertificateDer<'static>, key: &KeyPair) -> Certified {
        Certified {
            cert,
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        }
    }
}

/// A TLS proxy on 127.0.0.1: it speaks TLS with `certified`, and passes
/// what comes through to `upstream` as it is. It counts the handshakes
/// that succeed. Dropping it stops it.
struct Proxy {
    addr: SocketAddr,
    handshakes: Arc<AtomicUsize>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Proxy {
    fn new(certified: Certified, upstream: SocketAddr) -> Proxy {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert], certified.key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let handshakes = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&handshakes);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let serve = async {
                    loop {
                        let Ok((tcp, _)) = listener.accept().await else {
                            continue;
                        };
                        let acceptor = acceptor.clone();
                        let counted = Arc::clone(&counted);
                        tokio::spawn(async move {
                            let Ok(mut tls) = acceptor.accept(tcp).await else {
                                return;
                            };
                            counted.fetch_add(1, Ordering::SeqCst);
                            let Ok(mut up) = tokio::net::TcpStream::connect(upstream).await else {
                                return;
                            };
                            let _ = tokio::io::copy_bidirectional(&mut tls, &mut up).await;
                        });
                    }
                };
                tokio::select! {
                    _ = stopped => {}
                    _ = serve => {}
                }
            });
        });
        Proxy {
            addr,
            handshakes,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    fn handshakes(&self) -> usize {
        self.handshakes.load(Ordering::SeqCst)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `wss://localhost` at the port of `addr`.
fn wss(addr: SocketAddr) -> ServerAddress {
    ServerAddress::parse(&format!("wss://localhost:{}", addr.port())).unwrap()
}

fn config(login: &str) -> Config {
    Config {
        login: soundrts::config::Login::parse(login).unwrap(),
        wait_delay_per_character: 0.01,
        ..Config::default()
    }
}

/// A player who trusts `trust`, connects to `address` from the multiplayer
/// menu, and closes the window once on the server: what was said.
fn connect_from_the_menu(trust: Trust, address: &ServerAddress) -> (Harness, Vec<String>) {
    let mut h = Harness::with_config("en", Script::new(), config("andrew"));
    h.client.net_config.trust = trust;
    let _ = soundrts::multiplayer::connect(&mut h.client, address, None);
    let said = h.said_since(0);
    (h, said)
}

fn dialer(address: ServerAddress, trust: Trust) -> conn::Dialer {
    conn::Dialer {
        address,
        trust,
        hello: ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            login: Login::new("andrew").unwrap(),
            content: ContentHash::new("x").unwrap(),
        },
        timeout: Duration::from_secs(10),
        liveness: Liveness::default(),
    }
}

#[test]
fn an_encrypted_address_connects_with_tls_and_says_so() {
    let server = test_server(1, |_| {});
    let pki = Pki::new();
    let proxy = Proxy::new(pki.certificate("localhost"), server.local_addr());
    let address = wss(proxy.addr);
    let (h, said) = connect_from_the_menu(pki.trust(), &address);
    assert!(
        said.contains(&"welcome andrew on the server of test , encrypted".to_owned()),
        "{said:#?}"
    );
    assert_eq!(proxy.handshakes(), 1);
    // The recent servers keep it encrypted.
    let recent = servers::recent(&h.client.paths);
    assert_eq!(recent[0].address, address);
    assert_eq!(recent[0].address.transport(), Transport::Encrypted);
}

#[test]
fn a_plain_address_connects_plainly_and_says_nothing_of_encryption() {
    let server = test_server(1, |_| {});
    let address = ServerAddress::from(server.local_addr());
    let (_, said) = connect_from_the_menu(Trust::built_in(), &address);
    assert!(
        said.contains(&"welcome andrew on the server of test".to_owned()),
        "{said:#?}"
    );
    assert!(!said.iter().any(|s| s.contains("encrypted")), "{said:#?}");
}

/// What a player hears who connects to a proxy with `certified`, trusting
/// only the test's root; and that nothing got through.
fn refused(pki: &Pki, certified: Certified) -> Vec<String> {
    let server = test_server(1, |_| {});
    let proxy = Proxy::new(certified, server.local_addr());
    let (h, said) = connect_from_the_menu(pki.trust(), &wss(proxy.addr));
    assert_eq!(proxy.handshakes(), 0, "{said:#?}");
    assert!(!said.iter().any(|s| s.starts_with("welcome")), "{said:#?}");
    assert!(servers::recent(&h.client.paths).is_empty());
    said
}

#[test]
fn a_certificate_for_another_name_is_refused_with_the_reason() {
    let pki = Pki::new();
    let said = refused(&pki, pki.certificate("other.example"));
    assert_eq!(
        said.last().map(String::as_str),
        Some("not connected: the server's certificate is for another name"),
        "{said:#?}"
    );
}

#[test]
fn an_expired_certificate_is_refused_with_the_reason() {
    let pki = Pki::new();
    let said = refused(&pki, pki.expired("localhost"));
    assert_eq!(
        said.last().map(String::as_str),
        Some("not connected: the server's certificate is out of date"),
        "{said:#?}"
    );
}

#[test]
fn a_self_signed_certificate_is_refused_with_the_reason() {
    let pki = Pki::new();
    let said = refused(&pki, self_signed("localhost"));
    assert_eq!(
        said.last().map(String::as_str),
        Some("not connected: the server's certificate is not trusted"),
        "{said:#?}"
    );
}

#[test]
fn a_certificate_the_built_in_roots_do_not_vouch_for_is_refused() {
    // The test's root is not among the game's own.
    let pki = Pki::new();
    let server = test_server(1, |_| {});
    let proxy = Proxy::new(pki.certificate("localhost"), server.local_addr());
    let opened = dialer(wss(proxy.addr), Trust::built_in()).open();
    assert_eq!(
        opened.err(),
        Some(ConnectError::Certificate(BadCertificate::NotTrusted))
    );
    assert_eq!(proxy.handshakes(), 0);
}

#[test]
fn an_encrypted_address_of_a_plain_server_is_not_reached_plainly() {
    let server = test_server(1, |_| {});
    let pki = Pki::new();
    let opened = dialer(wss(server.local_addr()), pki.trust()).open();
    // The server hangs up on what it cannot read as a WebSocket handshake.
    assert!(
        matches!(
            opened,
            Err(ConnectError::Unreachable(_) | ConnectError::NotAServer(_))
        ),
        "{:?}",
        opened.err()
    );
}

#[test]
fn a_lost_encrypted_connection_returns_encrypted() {
    let server = test_server(5, |c| {
        c.heartbeat_interval = Duration::from_millis(500);
        c.heartbeat_timeout = Duration::from_secs(2);
    });
    let pki = Pki::new();
    let proxy = Proxy::new(pki.certificate("localhost"), server.local_addr());
    // The cable is between the player and the proxy: what crosses it is
    // encrypted.
    let cable = Cable::new(proxy.addr);
    let mut h = Harness::with_config("en", Script::new().keep_open(), config("andrew"));
    h.clock.follow_real_time();
    h.client.net_config.trust = pki.trust();
    h.client.net_config.liveness = Liveness {
        ping: Duration::from_millis(500),
        silence: Duration::from_millis(1500),
    };
    h.connect_to(&wss(cable.addr()).to_string(), |c| c);
    h.open_room("jl1", &[Computer::Quiet(Skill::NORMAL)], Speed::Fast4);
    let mut g = h.start_room_game("jl1");
    let from = h.utterances().len();
    let heard = |h: &Harness, line: &str| h.said_since(from).iter().any(|s| s == line);
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
    run_until(&mut h, &mut g, "10 turns", &|_, g| {
        g.game.view().turn() >= 10
    });
    assert_eq!(proxy.handshakes(), 1);
    cable.pull();
    run_until(&mut h, &mut g, "the loss noticed", &|h, _| {
        heard(h, "connection lost, trying to return")
    });
    cable.plug_in();
    run_until(&mut h, &mut g, "back", &|h, _| heard(h, "back in the game"));
    assert_eq!(proxy.handshakes(), 2, "the return was not encrypted");
    let turn = g.game.view().turn();
    run_until(&mut h, &mut g, "10 turns more", &|_, g| {
        g.game.view().turn() >= turn + 10
    });
    drop(server);
}
