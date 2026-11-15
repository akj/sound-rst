//! Games the server runs itself, through a real server on 127.0.0.1: each
//! human seat is played by a test client that rebuilds its player's view
//! from the stream alone, and what it rebuilt is checked against a host
//! that plays the same game locally.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use soundrts_protocol::game::{SeatFrame, decode_frame, encode_command};
use soundrts_protocol::*;
use soundrts_server::{ServerConfig, ServerHandle};
use soundrts_world::{
    Command, Controller, EntityEvent, EntityId, Event, GameEnd, GameSetup, GameStart, Host,
    KeywordOrder, Order, OrderTarget, PlayerId, Queue, Rules, SeatView, TurnMessage,
    orders::Keyword,
};

/// Every game here is played from this seed, so a local host can play it
/// too.
const SEED: i64 = 7;

/// A turn at the speed of the rooms here, [`Speed::Fast4`].
const TURN: Duration = Duration::from_millis(75);

/// The room's map: jl2's file.
fn jl2() -> (MapInfo, MapData) {
    let bytes = read("res/multi/jl2.txt").into_bytes();
    let file_name = MapFileName::new("jl2.txt").unwrap();
    let info = MapInfo {
        file_name: file_name.clone(),
        hash: MapHash::of(&bytes),
        title_sounds: vec![3002],
        nb_players_min: 2,
        nb_players_max: 4,
    };
    let data = MapData {
        file_name,
        bytes: MapBytes::new(bytes).unwrap(),
    };
    (info, data)
}

/// Who plays a slot, as the server makes it of the room: every human told
/// the events.
fn controller(slot: &Slot) -> Controller {
    let mut c = match &slot.player {
        SlotPlayer::Human(login) => Controller::human(login.as_str(), true),
        SlotPlayer::Computer(level) => Controller::computer(level.ai_type()),
    };
    c.faction = slot.faction.as_str().to_owned();
    c.alliance = Some(slot.alliance.to_string());
    c
}

/// The slots of a room here: the seats' humans in order, then easy
/// computers, as many as the game has players, each in an alliance of
/// its own, with random factions.
fn room_slots(seats: &[&Seat]) -> Vec<Slot> {
    let players = seats[0].view().game_start().player_names.len();
    (0..players)
        .map(|i| Slot {
            player: match seats.iter().find(|s| s.slot == i) {
                Some(s) => SlotPlayer::Human(s.c.login.clone()),
                None => SlotPlayer::Computer(AiLevel::Easy),
            },
            faction: Faction::random(),
            alliance: u8::try_from(i + 1).unwrap(),
            ready: true,
        })
        .collect()
}

fn setup(texts: &GameTexts, slots: &[Slot]) -> GameSetup {
    GameSetup::multiplayer(
        texts.rules.as_str().to_owned(),
        texts.ai.as_str().to_owned(),
        texts.map.as_str().to_owned(),
        SEED,
        slots.iter().map(controller).collect(),
    )
}

/// A server whose games are played from [`SEED`].
fn hosting_server(tweak: impl FnOnce(&mut ServerConfig)) -> ServerHandle {
    server(|c| {
        c.seed = Some(SEED);
        tweak(c);
    })
}

/// `clients[0]` hosts a room on jl2 that every other client joins, ready,
/// with `computers` easy computers after them.
async fn room(clients: &mut [Client], computers: usize) -> RoomId {
    let room = open_room(clients, 0, &jl2(), true).await;
    for i in 1..clients.len() {
        join_room(clients, i, room).await;
    }
    for _ in 0..computers {
        add_computer(clients, room).await;
    }
    let members: Vec<usize> = (0..clients.len()).collect();
    for i in 1..clients.len() {
        ready(clients, i, &members).await;
    }
    room
}

/// What a client is told: a JSON message, or a frame of a game
/// that is not of the stream itself.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum Heard {
    Msg(ServerMsg),
    Game(SeatFrame),
}

impl From<ServerMsg> for Heard {
    fn from(m: ServerMsg) -> Heard {
        Heard::Msg(m)
    }
}

impl From<SeatFrame> for Heard {
    fn from(f: SeatFrame) -> Heard {
        Heard::Game(f)
    }
}

/// What happened to a seat, as the others are told it.
fn news(slot: SlotIndex, change: SeatChange) -> SeatFrame {
    SeatFrame::Seat { slot, change }
}

/// The next message or frame a client is told.
async fn heard(c: &mut Client) -> Heard {
    match c.frame().await {
        Frame::Msg(m) => Heard::Msg(m),
        Frame::Binary(b) => Heard::Game(decode_frame(&b).unwrap()),
    }
}

/// Reads until the client is told what `pred` accepts, and returns it.
async fn heard_until(c: &mut Client, pred: impl Fn(&Heard) -> bool) -> Heard {
    loop {
        let h = heard(c).await;
        if pred(&h) {
            return h;
        }
    }
}

/// How a message reached a seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Came {
    Whole,
    Change,
}

/// A human seat, played by a client that knows only what it was sent.
struct Seat {
    c: Client,
    slot: SlotIndex,
    token: SeatToken,
    rules: String,
    start: Option<Arc<GameStart>>,
    view: Option<SeatView>,
    /// Every message the seat had, and how it came.
    history: Vec<(Came, TurnMessage)>,
    end: Option<GameEnd>,
    /// What the seat was told since the game started, beside the stream.
    told: Vec<Heard>,
    /// When each change came.
    arrivals: Vec<Instant>,
    /// The game is over for this seat: it is back in the lobby.
    left: bool,
}

impl Seat {
    /// Waits for the game to start for this client, and takes its start
    /// and whole message.
    async fn join(c: Client, rules: &str) -> Seat {
        let mut seat = Seat {
            c,
            slot: 0,
            token: SeatToken::from_bits(0),
            rules: rules.to_owned(),
            start: None,
            view: None,
            history: Vec::new(),
            end: None,
            told: Vec::new(),
            arrivals: Vec::new(),
            left: false,
        };
        seat.until_seated().await;
        seat
    }

    /// Reads until the server says the client has a seat, then its start
    /// and whole message.
    async fn until_seated(&mut self) {
        loop {
            match heard(&mut self.c).await {
                Heard::Game(SeatFrame::Seated { you, token, .. }) => {
                    self.slot = you;
                    self.token = token;
                    break;
                }
                Heard::Msg(_) => {}
                Heard::Game(f) => panic!("{}: {f:?} before the game", self.c.login),
            }
        }
        self.start = None;
        self.view = None;
        // The lobby's own news may come between.
        let mut frames = 0;
        while frames < 2 {
            match self.c.frame().await {
                Frame::Binary(b) => {
                    self.take(&b);
                    frames += 1;
                }
                Frame::Msg(m) => self.told.push(Heard::Msg(m)),
            }
        }
        assert!(self.view.is_some(), "the whole message came");
    }

    fn player(&self) -> PlayerId {
        self.view().player()
    }

    fn view(&self) -> &SeatView {
        self.view.as_ref().expect("a view")
    }

    fn time(&self) -> i64 {
        self.view().time()
    }

    /// Takes a binary frame, checking it as a client must.
    fn take(&mut self, bytes: &[u8]) {
        let who = self.c.login.clone();
        match decode_frame(bytes).unwrap_or_else(|e| panic!("{who}: {e}")) {
            SeatFrame::Start(s) => {
                s.check().unwrap();
                let rules = Rules::load(&[self.rules.as_str()]);
                self.start = Some(Arc::new(GameStart::from_message(s, rules)));
            }
            SeatFrame::Whole(m) => {
                let start = self.start.clone().expect("the start first");
                start.check(&m).unwrap();
                self.history.push((Came::Whole, m.clone()));
                self.view = Some(SeatView::new(start, m));
            }
            SeatFrame::Change(change) => {
                let start = self.start.clone().expect("the start first");
                let view = self.view.as_mut().expect("a whole message first");
                let m = change.apply(view.message()).unwrap();
                start.check(&m).unwrap();
                self.history.push((Came::Change, m.clone()));
                view.install(m);
                self.arrivals.push(Instant::now());
            }
            SeatFrame::End(end) => {
                assert!(self.end.is_none(), "{who}: two ends");
                self.end = Some(end.clone());
                self.view.as_mut().expect("a view").install_end(end);
            }
            news => self.told.push(Heard::Game(news)),
        }
    }

    /// Reads one frame or message, if one comes within `wait`.
    async fn step(&mut self, wait: Duration) -> bool {
        match self.c.frame_within(wait).await {
            Some(Frame::Binary(b)) => {
                self.take(&b);
                true
            }
            Some(Frame::Msg(m)) => {
                if m == ServerMsg::GameEnded {
                    self.left = true;
                }
                self.told.push(Heard::Msg(m));
                true
            }
            None => false,
        }
    }

    /// Reads until the seat's game time is at least `turn` turns.
    async fn play_to(&mut self, turn: i64) {
        while self.time() < turn * 300 && !self.left {
            assert!(self.step(TIMEOUT).await, "{}: nothing came", self.c.login);
        }
    }

    /// Reads until the seat is told `expected`, beside the stream.
    async fn hear(&mut self, expected: impl Into<Heard>) {
        let expected = expected.into();
        let seen = self.told.len();
        while !self.told[seen..].contains(&expected) {
            assert!(
                self.step(TIMEOUT).await,
                "{}: nothing came, waiting for {expected:?}",
                self.c.login
            );
        }
    }

    /// Reads for `wait`, and gives the changes that came meanwhile.
    async fn read_for(&mut self, wait: Duration) -> usize {
        let before = self.arrivals.len();
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            self.step(left).await;
        }
        self.arrivals.len() - before
    }

    async fn command(&mut self, command: Command) {
        self.c.send_binary(&encode_command(&command).unwrap()).await;
    }

    /// The seat's own units of a type, from what it knows.
    fn own(&self, type_name: &str) -> Vec<EntityId> {
        self.view()
            .own_units()
            .filter(|u| &*u.type_name == type_name)
            .map(|u| u.id)
            .collect()
    }

    /// The units that answered an order with `ok`, in what the seat was
    /// told since message `from`.
    fn answered(&self, from: usize) -> Vec<EntityId> {
        self.history
            .iter()
            .skip(from)
            .flat_map(|(_, m)| &m.events)
            .filter_map(|e| match e {
                Event::Entity {
                    entity,
                    event: EntityEvent::OrderOk,
                } => Some(entity.id),
                _ => None,
            })
            .collect()
    }
}

/// Sends `units` to `square`.
fn go(units: Vec<EntityId>, square: soundrts_world::SquareId) -> [Command; 2] {
    [
        Command::Control(units),
        Command::Order {
            order: Order::Keyword(
                KeywordOrder::new(Keyword::Go)
                    .unwrap()
                    .at(OrderTarget::Square(square)),
            ),
            queue: Queue::Replace,
            imperative: false,
        },
    ]
}

/// Starts a game of the room: `clients[0]` hosts, `players` are the
/// humans in slot order.
async fn start(clients: Vec<Client>, texts: &GameTexts) -> Vec<Seat> {
    let mut clients = clients;
    clients[0]
        .send(ClientMsg::Start {
            texts: texts.clone(),
        })
        .await;
    let mut seats = Vec::new();
    for c in clients {
        seats.push(Seat::join(c, texts.rules.as_str()).await);
    }
    for (i, s) in seats.iter().enumerate() {
        assert_eq!(s.slot, i, "humans come first in the room");
    }
    seats
}

/// Plays the game locally from the same set-up, with the commands `given`
/// to players (by the turn they ran in), and checks every message each
/// seat rebuilt, and its end, against the host's own.
///
/// Each turn a seat is told the end of its turn and, when something came
/// after it, what it knows after the step. A seat that was not sent some
/// turns was told, in its next change, the events of those turns that are
/// not about an entity; a seat sent a whole message was told no event with
/// it.
fn check_against_host(texts: &GameTexts, seats: &[&Seat], given: &[(PlayerId, i64, Command)]) {
    let mut host = Host::new(&setup(texts, &room_slots(seats))).unwrap();
    let players: Vec<PlayerId> = host.seats().to_vec();
    let humans: Vec<PlayerId> = seats.iter().map(|s| players[s.slot]).collect();
    for s in seats {
        assert_eq!(s.player(), players[s.slot], "the seat's player");
    }
    let last = seats
        .iter()
        .flat_map(|s| s.history.last())
        .map(|(_, m)| m.time / 300)
        .max()
        .unwrap();
    // What each seat is told, by turn, from the host.
    let mut by_turn: Vec<Vec<Vec<TurnMessage>>> =
        vec![humans.iter().map(|&p| vec![host.message(p)]).collect()];
    let mut ends: Vec<Option<GameEnd>> = vec![None; seats.len()];
    for turn in 1..=last {
        for (p, at, c) in given {
            if *at == turn {
                let _ = host.queue(*p, c.clone());
            }
        }
        let turns = host.step_seats(&humans);
        by_turn.push(
            turns
                .iter()
                .map(|t| {
                    let after = TurnMessage {
                        events: t.then.clone(),
                        ..(*t.message).clone()
                    };
                    match &t.end_of_turn {
                        Some(end) if t.then.is_empty() && Arc::ptr_eq(end, &t.message) => {
                            vec![(**end).clone()]
                        }
                        Some(end) => vec![(**end).clone(), after],
                        None => vec![after],
                    }
                })
                .collect(),
        );
        for (i, t) in turns.iter().enumerate() {
            if ends[i].is_none() && !t.message.state.players.contains(&humans[i]) {
                ends[i] = Some(host.game_end(humans[i]));
            }
        }
    }
    let mut checked = 0;
    for (i, s) in seats.iter().enumerate() {
        // The turn, and which of its messages, the seat last had.
        let mut previous: Option<(usize, usize)> = None;
        for (came, m) in &s.history {
            let turn = (m.time / 300) as usize;
            let told = &by_turn[turn][i];
            let (expected, at) = match (came, previous) {
                (Came::Whole, _) => {
                    let mut whole = told.last().unwrap().clone();
                    whole.events.clear();
                    (whole, told.len() - 1)
                }
                (Came::Change, Some((t, k))) if t == turn && k + 1 < told.len() => {
                    (told[k + 1].clone(), k + 1)
                }
                (Came::Change, Some((t, _))) => {
                    let mut first = told[0].clone();
                    first.events = ((t + 1)..turn)
                        .flat_map(|t| by_turn[t][i].iter().flat_map(|m| m.events.clone()))
                        .filter(|e| !matches!(e, Event::Entity { .. }))
                        .chain(first.events)
                        .collect();
                    (first, 0)
                }
                (Came::Change, None) => panic!("a change before a whole message"),
            };
            assert_eq!(m, &expected, "{} at turn {turn}", s.c.login);
            previous = Some((turn, at));
            checked += 1;
        }
        if let Some(end) = &s.end {
            assert_eq!(Some(end), ends[i].as_ref(), "{}'s end", s.c.login);
        }
    }
    assert!(checked > 0);
}

/// The turn a seat's player left the game, as its stream tells it.
fn left_at(seat: &Seat) -> i64 {
    seat.history
        .iter()
        .find(|(_, m)| !m.state.players.contains(&seat.player()))
        .map(|(_, m)| m.time / 300)
        .expect("the player left")
}

#[tokio::test]
async fn a_game_runs_to_its_end_and_each_seat_sees_what_the_host_says() {
    let server = hosting_server(|_| {});
    let mut clients = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = room(&mut clients, 2).await;
    let texts = texts();
    let mut seats = start(clients, &texts).await;
    let (a, b) = seats.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);
    assert_ne!(a.token, b.token);
    tokio::join!(a.play_to(40), b.play_to(40));
    a.command(Command::Quit).await;
    tokio::join!(a.play_to(60), b.play_to(60));
    b.command(Command::Quit).await;
    for s in [&mut *a, &mut *b] {
        while !s.left {
            assert!(s.step(TIMEOUT).await);
        }
        assert!(
            s.end.is_some(),
            "{} got its end before the lobby",
            s.c.login
        );
        let last = s.history.last().unwrap();
        assert!(!last.1.state.players.contains(&s.player()));
    }
    // b was told a left at 40, as a player leaving.
    let gone = b
        .history
        .iter()
        .find(|(_, m)| !m.state.players.contains(&a.player()));
    assert!(gone.is_some(), "b saw a quit");
    // Every seat is gone, so the room closes.
    a.c.skip_to(&lobby(LobbyChange::RoomClosed { id: room }))
        .await;
    let quits = [
        (a.player(), left_at(a), Command::Quit),
        (b.player(), left_at(b), Command::Quit),
    ];
    check_against_host(&texts, &[&*a, &*b], &quits);
    let turns: usize = [&*a, &*b].iter().map(|s| s.history.len()).sum();
    assert!(turns > 100, "{turns} messages");
}

#[tokio::test]
async fn a_seat_hears_how_the_game_ended_before_its_end() {
    let server = hosting_server(|_| {});
    let mut clients = clients(server.local_addr(), &["andrew"]).await;
    room(&mut clients, 1).await;
    let texts = texts();
    let mut seats = start(clients, &texts).await;
    let a = &mut seats[0];
    a.play_to(3).await;
    // The host removes the only other player: Andrew has won, and the game
    // ends at the end of a turn.
    a.c.send(ClientMsg::RemovePlayer { slot: 1 }).await;
    while !a.left {
        assert!(a.step(TIMEOUT).await);
    }
    // The last turn came as two changes of the same time: its end, won,
    // with Andrew still playing, then everyone leaving.
    let n = a.history.len();
    let (end, after) = (&a.history[n - 2].1, &a.history[n - 1].1);
    assert_eq!(end.time, after.time);
    assert!(end.state.has_victory);
    assert!(end.state.players.contains(&a.player()));
    assert!(
        matches!(after.events[..], [Event::Quit]),
        "{:?}",
        after.events
    );
    assert!(after.state.players.is_empty());
    // Then the end, then the lobby.
    assert!(a.end.is_some(), "the end came");
    assert_eq!(a.told.last(), Some(&Heard::Msg(ServerMsg::GameEnded)));
    let computer = PlayerId(1);
    let removed = a
        .history
        .iter()
        .find(|(_, m)| !m.state.players.contains(&computer))
        .map(|(_, m)| m.time / 300)
        .unwrap();
    check_against_host(&texts, &[&*a], &[(computer, removed, Command::Quit)]);
}

#[tokio::test]
async fn a_command_takes_effect_and_one_from_outside_the_game_is_refused() {
    let server = hosting_server(|_| {});
    let mut clients = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let mut carol = clients.pop().unwrap();
    room(&mut clients, 1).await;
    let mut seats = start(clients, &texts()).await;
    let (a, b) = seats.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);
    tokio::join!(a.play_to(3), b.play_to(3));

    let b2 = a.view().square_at(1, 1).unwrap();
    let go = |units| go(units, b2);
    // Andrew orders Bob's peasants, then his own: only his own obey, and
    // Bob hears nothing of either.
    let mine = a.own("peasant");
    let bobs = b.own("peasant");
    assert!(!mine.is_empty() && !bobs.is_empty());
    let (from_a, from_b) = (a.history.len(), b.history.len());
    for c in go(bobs.clone()) {
        a.command(c).await;
    }
    for c in go(mine.clone()) {
        a.command(c).await;
    }
    tokio::join!(a.play_to(12), b.play_to(12));
    let mut obeyed = a.answered(from_a);
    obeyed.sort();
    obeyed.dedup();
    let mut expected = mine.clone();
    expected.sort();
    assert_eq!(obeyed, expected);
    assert_eq!(b.answered(from_b), [], "Bob's peasants were not ordered");

    // No cheat and no speed from a seat.
    for c in [
        Command::Cheat(soundrts_world::Cheat::Resources),
        Command::Speed(2.0),
    ] {
        a.command(c).await;
        a.hear(refused(Refusal::NotNow)).await;
    }
    // A player outside the game has no seat to command.
    carol
        .send_binary(&encode_command(&Command::Quit).unwrap())
        .await;
    carol.skip_to(&refused(Refusal::NotNow)).await;
    // Bob is still playing.
    tokio::join!(a.play_to(15), b.play_to(15));
    assert!(b.view().players().contains(&b.player()));
}

#[tokio::test]
async fn a_seat_that_stops_reading_is_caught_up_and_holds_up_nobody() {
    let server = hosting_server(|c| c.send_buffer = Some(2048));
    let addr = server.local_addr();
    let (andrew, _) = Client::hello(addr, "andrew").await;
    let (bob, _) = Client::hello_with_small_buffer(addr, "bob", 1024).await;
    let mut clients = vec![andrew, bob];
    clients[0]
        .skip_until(|m| {
            matches!(
                m,
                ServerMsg::Lobby {
                    change: LobbyChange::PlayerArrived { .. }
                }
            )
        })
        .await;
    room(&mut clients, 2).await;
    let mut seats = start(clients, &texts()).await;
    let (a, b) = seats.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);
    tokio::join!(a.play_to(2), b.play_to(2));

    // Bob stops reading, but keeps sending his units one way then the
    // other, so that each turn he is told their answers and moves, and his
    // connection soon falls behind. Andrew's turns keep coming at the
    // clock's pace.
    let units: Vec<EntityId> = b.view().own_units().map(|u| u.id).collect();
    let (cols, rows) = b.view().grid_max();
    let corners: Vec<_> = [(0, 0), (cols, rows)]
        .into_iter()
        .map(|(c, r)| b.view().square_at(c, r).unwrap())
        .collect();
    let stalled = Duration::from_secs(4);
    let before = a.arrivals.len();
    let orders = async {
        let until = Instant::now() + stalled;
        let mut n = 0;
        while Instant::now() < until {
            for c in go(units.clone(), corners[n % 2]) {
                b.command(c).await;
            }
            n += 1;
            tokio::time::sleep(TURN).await;
        }
    };
    tokio::join!(a.read_for(stalled), orders);
    let gaps: Vec<Duration> = a.arrivals[before..]
        .windows(2)
        .map(|w| w[1] - w[0])
        .collect();
    let longest = gaps.iter().max().copied().unwrap();
    let mean = gaps.iter().sum::<Duration>() / gaps.len() as u32;
    eprintln!(
        "while Bob stalled: {} turns to Andrew, mean {mean:?}, longest {longest:?}",
        gaps.len() + 1
    );
    assert!(gaps.len() as u32 >= stalled.as_millis() as u32 / 75 / 2);
    assert!(mean < TURN * 2, "mean {mean:?}");
    assert!(longest < TURN * 6, "longest {longest:?}");

    // Bob reads again: the turns he was not sent come as one change.
    let at = a.time() / 300;
    b.play_to(at + 5).await;
    a.play_to(at + 5).await;
    let skipped = b
        .history
        .windows(2)
        .map(|w| (w[1].1.time - w[0].1.time) / 300)
        .max()
        .unwrap();
    eprintln!("Bob's largest catch-up: {skipped} turns");
    assert!(skipped > 1, "Bob was never behind");
    // Every change applied to what Bob had and was checked against the
    // game as it came; he is now as far on as Andrew.
    assert_eq!(b.time(), a.time());
}

#[tokio::test]
async fn a_dropped_player_returns_with_their_token() {
    let server = hosting_server(|_| {});
    let addr = server.local_addr();
    let mut clients = clients(addr, &["andrew", "bob"]).await;
    let room = room(&mut clients, 1).await;
    let texts = texts();
    let mut seats = start(clients, &texts).await;
    let mut b = seats.pop().unwrap();
    let mut a = seats.pop().unwrap();
    tokio::join!(a.play_to(5), b.play_to(5));

    let token = b.token.clone();
    let old = std::mem::replace(&mut b.c, Client::connect(addr).await);
    old.close().await;
    a.hear(news(1, SeatChange::Dropped)).await;
    let (mut c, _) = Client::hello(addr, "bob").await;
    // Another seat's token, and a token for no seat, are refused.
    for wrong in [a.token.clone(), SeatToken::from_bits(1)] {
        c.send(ClientMsg::ReturnToGame { room, token: wrong }).await;
        c.skip_to(&refused(Refusal::NotNow)).await;
    }
    a.play_to(a.time() / 300 + 10).await;
    c.send(ClientMsg::ReturnToGame {
        room,
        token: token.clone(),
    })
    .await;
    b.c = c;
    b.until_seated().await;
    assert_eq!((b.slot, &b.token), (1, &token));
    a.hear(news(1, SeatChange::Returned)).await;
    // Bob plays on, and a second return to the seat he holds is refused.
    let (mut d, _) = Client::hello(addr, "dave").await;
    d.send(ClientMsg::ReturnToGame { room, token }).await;
    d.skip_to(&refused(Refusal::NotNow)).await;
    let at = a.time() / 300;
    tokio::join!(a.play_to(at + 5), b.play_to(at + 5));
    check_against_host(&texts, &[&a, &b], &[]);
}

#[tokio::test]
async fn a_player_not_back_in_time_has_quit() {
    let server = hosting_server(|c| c.grace_period = Duration::from_millis(500));
    let addr = server.local_addr();
    let mut clients = clients(addr, &["andrew", "bob"]).await;
    let room = room(&mut clients, 1).await;
    let mut seats = start(clients, &texts()).await;
    let b = seats.pop().unwrap();
    let mut a = seats.pop().unwrap();
    let token = b.token.clone();
    let bob = b.player();
    b.c.close().await;
    let dropped = Instant::now();
    a.hear(news(1, SeatChange::Dropped)).await;
    a.hear(news(1, SeatChange::GivenUp)).await;
    let waited = dropped.elapsed();
    eprintln!("given up after {waited:?}");
    assert!(waited >= Duration::from_millis(500));
    // Bob's player quits in a coming turn.
    let at = a.time() / 300;
    a.play_to(at + 3).await;
    assert!(!a.view().players().contains(&bob));
    let (mut c, _) = Client::hello(addr, "bob").await;
    c.send(ClientMsg::ReturnToGame { room, token }).await;
    c.skip_to(&refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_dead_connection_is_closed_by_the_heartbeat() {
    let server = hosting_server(|c| {
        c.heartbeat_interval = Duration::from_millis(100);
        c.heartbeat_timeout = Duration::from_millis(600);
    });
    let mut clients = clients(server.local_addr(), &["andrew", "bob"]).await;
    room(&mut clients, 0).await;
    let mut seats = start(clients, &texts()).await;
    let mut b = seats.pop().unwrap();
    let mut a = seats.pop().unwrap();
    // Bob goes silent: he reads nothing, so answers no ping.
    let silent = Instant::now();
    a.hear(news(1, SeatChange::Dropped)).await;
    let waited = silent.elapsed();
    eprintln!("dropped after {waited:?} of silence");
    assert!(waited >= Duration::from_millis(500), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");
    // Andrew, who reads, is still connected; Bob's connection is closed.
    a.read_for(Duration::from_millis(800)).await;
    assert!(!a.left);
    while b.c.frame_within(TIMEOUT).await.is_some() {}
}

#[tokio::test]
async fn the_host_pauses_resumes_sets_the_speed_and_removes_a_player() {
    let server = hosting_server(|_| {});
    let mut clients = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    room(&mut clients, 1).await;
    let mut seats = start(clients, &texts()).await;
    let mut c = seats.pop().unwrap();
    let mut b = seats.pop().unwrap();
    let mut a = seats.pop().unwrap();
    let andrew = a.c.login.clone();
    let clock = |change| SeatFrame::Clock {
        by: andrew.clone(),
        change,
    };

    a.c.send(ClientMsg::PauseGame).await;
    for s in [&mut a, &mut b, &mut c] {
        s.hear(clock(ClockChange::Paused)).await;
    }
    assert_eq!(a.read_for(Duration::from_millis(500)).await, 0, "paused");
    // Only the host may.
    b.c.send(ClientMsg::ResumeGame).await;
    b.hear(refused(Refusal::NotNow)).await;
    a.c.send(ClientMsg::PauseGame).await;
    a.hear(refused(Refusal::NotNow)).await;
    a.c.send(ClientMsg::ResumeGame).await;
    for s in [&mut a, &mut b, &mut c] {
        s.hear(clock(ClockChange::Resumed)).await;
    }
    assert!(a.read_for(Duration::from_millis(500)).await >= 3, "running");

    a.c.send(ClientMsg::SetGameSpeed { speed: Speed::Slow })
        .await;
    let slow = clock(ClockChange::Speed { speed: Speed::Slow });
    for s in [&mut a, &mut b, &mut c] {
        s.hear(slow.clone()).await;
    }
    let came = a.read_for(Duration::from_millis(1500)).await;
    eprintln!("{came} turns in 1.5 s at the slow speed");
    assert!((1..=4).contains(&came), "{came} turns at 600 ms");

    // The host removes Carol, then the computer, and not himself.
    a.c.send(ClientMsg::RemovePlayer { slot: 2 }).await;
    let removed = news(2, SeatChange::Removed { by: andrew.clone() });
    for s in [&mut a, &mut b, &mut c] {
        s.hear(removed.clone()).await;
    }
    while !c.left {
        assert!(c.step(TIMEOUT).await);
    }
    assert!(c.end.is_some(), "Carol got her end");
    a.c.send(ClientMsg::RemovePlayer { slot: 3 }).await;
    let computer = news(3, SeatChange::Removed { by: andrew.clone() });
    a.hear(computer.clone()).await;
    b.hear(computer).await;
    for slot in [0, 2, 9] {
        a.c.send(ClientMsg::RemovePlayer { slot }).await;
        a.hear(refused(Refusal::NoSuchPlayer)).await;
    }
    let at = a.time() / 300;
    tokio::join!(a.play_to(at + 3), b.play_to(at + 3));
    let computer_player = PlayerId(3);
    assert!(!a.view().players().contains(&computer_player));
    assert!(!a.view().players().contains(&c.player()));
    assert!(a.view().players().contains(&b.player()));
}

#[tokio::test]
async fn a_game_that_cannot_be_built_is_refused_and_the_room_goes_on() {
    let server = hosting_server(|_| {});
    let mut clients = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = room(&mut clients, 0).await;
    let mut bad = texts();
    bad.map = text("nb_columns x\n");
    clients[0].send(ClientMsg::Start { texts: bad }).await;
    let refusal = heard_until(&mut clients[0], |h| {
        matches!(
            h,
            Heard::Game(SeatFrame::NotStarted(_) | SeatFrame::Seated { .. })
        )
    })
    .await;
    let Heard::Game(SeatFrame::NotStarted(StartRefusal::CannotBuild { error })) = refusal else {
        panic!("{refusal:?}");
    };
    eprintln!("refused: {}", error.as_str());
    // Everyone is back in the room, which starts with a good map.
    for c in clients.iter_mut() {
        c.skip_to(&moved(&login("bob"), Whereabouts::InRoom(room)))
            .await;
    }
    let seats = start(clients, &texts()).await;
    assert_eq!(seats.len(), 2);
}

#[tokio::test]
async fn a_game_slower_to_build_or_play_than_its_budget_is_stopped() {
    use soundrts_server::fault;
    let slow = |line: &str| {
        let mut t = texts();
        t.rules = text(&format!("{line}{}", t.rules.as_str()));
        t
    };
    let budget = fault::SLOW / 5;
    let server = hosting_server(|c| c.load_budget = budget);
    let mut first = clients(server.local_addr(), &["andrew"]).await;
    room(&mut first, 1).await;
    // Each build too slow is refused but runs on; while two do, the server
    // starts no game, and once they end it does again.
    let started = Instant::now();
    for expected in [
        StartRefusal::TooSlow,
        StartRefusal::TooSlow,
        StartRefusal::Busy,
    ] {
        first[0]
            .send(ClientMsg::Start {
                texts: slow(fault::SLOW_TO_BUILD),
            })
            .await;
        let answer = heard_until(&mut first[0], |h| {
            matches!(
                h,
                Heard::Game(SeatFrame::NotStarted(_) | SeatFrame::Seated { .. })
            )
        })
        .await;
        assert_eq!(answer, Heard::Game(SeatFrame::NotStarted(expected)));
    }
    assert!(started.elapsed() < fault::SLOW, "the refusals came in time");
    tokio::time::sleep(fault::SLOW * 2).await;
    start(first, &texts()).await;

    let server = hosting_server(|c| c.turn_budget = budget);
    let mut clients = clients(server.local_addr(), &["andrew"]).await;
    room(&mut clients, 1).await;
    let mut seats = start(clients, &slow(fault::SLOW_TURN)).await;
    let a = &mut seats[0];
    a.hear(SeatFrame::Stopped(StopReason::TooSlow)).await;
    a.hear(ServerMsg::GameEnded).await;
}

#[tokio::test]
async fn a_panic_in_one_game_leaves_another_running() {
    use soundrts_server::fault;
    let server = hosting_server(|_| {});
    let addr = server.local_addr();
    let mut steady = clients(addr, &["andrew"]).await;
    room(&mut steady, 1).await;
    let mut steady = start(steady, &texts()).await;
    for fault in [fault::PANIC_IN_TURN, fault::PANIC_IN_TASK] {
        let mut doomed = clients(addr, &["bob"]).await;
        room(&mut doomed, 1).await;
        let mut texts = texts();
        texts.rules = text(&format!("{fault}{}", texts.rules.as_str()));
        let mut doomed = start(doomed, &texts).await;
        let d = &mut doomed[0];
        d.hear(SeatFrame::Stopped(StopReason::Crashed)).await;
        d.hear(ServerMsg::GameEnded).await;
        // The other game plays on.
        let a = &mut steady[0];
        let at = a.time() / 300;
        a.play_to(at + 5).await;
        d.c.send(ClientMsg::LeaveRoom).await;
        d.c.skip_to(&refused(Refusal::NotNow)).await;
    }
}

#[tokio::test]
async fn hostile_start_texts_and_commands_are_refused_without_a_panic() {
    let server = hosting_server(|_| {});
    let addr = server.local_addr();
    let mut clients = clients(addr, &["andrew"]).await;
    room(&mut clients, 1).await;

    // A text over the loaders' limit cannot be read at all.
    let too_long = "x".repeat(soundrts_world::defs::MAX_TEXT_BYTES + 1);
    let json = format!(r#"{{"type":"start","texts":{{"rules":"{too_long}","ai":"","map":""}}}}"#);
    clients[0].send_text(&json).await;
    clients[0].skip_to(&refused(Refusal::Invalid)).await;

    // Texts cut and scrambled are refused or built, never a crash.
    let good = texts();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n.max(1) as u64) as usize
    };
    let mut built = 0;
    for i in 0..12 {
        let mut map = good.map.as_str().as_bytes().to_vec();
        for _ in 0..1 + next(8) {
            let at = next(map.len());
            match next(3) {
                0 => map[at] = b"0123456789 -\nxa"[next(15)],
                1 => map.truncate(at),
                _ => map.insert(at, b"\n;9 "[next(4)]),
            }
        }
        let scrambled = GameTexts {
            map: text(&String::from_utf8_lossy(&map)),
            ..good.clone()
        };
        clients[0].send(ClientMsg::Start { texts: scrambled }).await;
        match heard_until(&mut clients[0], |h| {
            matches!(
                h,
                Heard::Game(SeatFrame::NotStarted(_) | SeatFrame::Seated { .. })
            )
        })
        .await
        {
            Heard::Game(SeatFrame::Seated { .. }) => {
                built += 1;
                clients[0].send(ClientMsg::LeaveGame).await;
                clients[0].skip_to(&ServerMsg::GameEnded).await;
                let c = clients.pop().unwrap();
                c.close().await;
                clients = vec![Client::hello(addr, &format!("andrew{i}")).await.0];
                room(&mut clients, 1).await;
            }
            Heard::Game(SeatFrame::NotStarted(_)) => {
                clients[0]
                    .skip_until(|m| {
                        matches!(m, ServerMsg::Lobby { change: LobbyChange::RoomChanged { room } } if !room.started)
                    })
                    .await;
            }
            _ => unreachable!(),
        }
    }
    eprintln!("{built} of 12 scrambled maps built");

    // Commands that decode, but name what is not there, given to units of
    // the seat's own: a square off the map is refused at the door, the
    // rest is the world's to refuse.
    let mut seats = start(clients, &good).await;
    let a = &mut seats[0];
    a.play_to(2).await;
    let own: Vec<EntityId> = a.view().own_units().map(|u| u.id).collect();
    let here = a.view().square_at(0, 0).unwrap();
    let off_map = soundrts_world::SquareId(u32::MAX);
    let order = |target, imperative| Command::Order {
        order: Order::Keyword(KeywordOrder::new(Keyword::Go).unwrap().at(target)),
        queue: Queue::Replace,
        imperative,
    };
    let point = |square, x, y| OrderTarget::Point { square, x, y };
    a.command(Command::Control(own.clone())).await;
    for c in [
        order(OrderTarget::Square(off_map), true),
        order(point(off_map, 0, 0), true),
        order(point(off_map, 0, 0), false),
        Command::Order {
            order: Order::Default(OrderTarget::Square(off_map)),
            queue: Queue::Append,
            imperative: true,
        },
    ] {
        a.command(c).await;
        a.hear(refused(Refusal::Invalid)).await;
    }
    for c in [
        Command::Control(vec![EntityId(u32::MAX), EntityId(0)]),
        Command::Control(own),
        order(point(here, i64::MAX, i64::MIN), true),
        order(point(here, i64::MIN, i64::MAX), false),
        order(OrderTarget::Entity(EntityId(u32::MAX)), true),
        Command::Order {
            order: Order::Default(OrderTarget::Entity(EntityId(u32::MAX))),
            queue: Queue::Append,
            imperative: true,
        },
        Command::Order {
            order: Order::JoinGroup("é".repeat(1000)),
            queue: Queue::Replace,
            imperative: false,
        },
    ] {
        a.command(c).await;
    }
    let at = a.time() / 300;
    a.play_to(at + 3).await;
    // Bytes that are no command, and a command too long.
    for bytes in [vec![], vec![0xff; 3], vec![1; MAX_COMMAND_BYTES + 1]] {
        a.c.send_binary(&bytes).await;
        a.hear(refused(Refusal::Invalid)).await;
    }
    // A flood of commands for one turn is slowed down.
    let control = Command::Control((0..1000).map(EntityId).collect());
    for _ in 0..20 {
        a.command(control.clone()).await;
    }
    a.hear(refused(Refusal::SlowDown)).await;
    let at = a.time() / 300;
    a.play_to(at + 5).await;
}
