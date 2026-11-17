//! Hostile streams of frames, as bytes a server sends a seat: the cases of
//! the client's own tests (`crates/client/tests/hostile.rs`), read the way
//! the client reads them, through [`JoinStream`], [`SeatStream`] and
//! [`ReturnStream`]. A stream out of order, or with a frame that cannot be
//! right for the game, is refused at that frame, with the reason. The real
//! streams play to their end.
//!
//! Some of the client's cases are the interface's to refuse, not the
//! stream's: news of a slot the room does not have, and more events than
//! the interface can tell. The stream takes them, and the interface checks
//! them against what it keeps.

use soundrts_protocol::game::{
    Checked, JoinStream, Joining, ReturnStream, Returning, Role, RulesHash, SeatFrame, SeatStream,
    decode_frame, encode_frame,
};
use soundrts_protocol::{
    ClockChange, Computer, Faction, Income, Login, MapFileName, MapHash, MapInfo, SeatChange,
    SeatToken, Skill, Slot, SlotPlayer, Speed, StartRefusal,
};
use soundrts_world::player::{Objective, StatEvent, StatTarget};
use soundrts_world::{Event, GameSetup, Host, Place, PlayerId, TurnMessage};

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/game-data")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The turns of a stream, as in the client's tests.
const STREAM_TURNS: usize = 30;

/// The rules the game is built from, and the seat reads it with.
fn rules() -> String {
    read("res/rules.txt")
}

/// Andrew, alone against a computer.
fn slots() -> [Slot; 2] {
    let slot = |player, alliance| Slot {
        player,
        faction: Faction::random(),
        alliance,
        ready: true,
        income: Income::FULL,
    };
    [
        slot(SlotPlayer::Human(Login::new("andrew").unwrap()), 1),
        slot(SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL)), 2),
    ]
}

/// A game run as a server runs one, and the frames a server sends the seat
/// of slot `you`, or a watcher following that slot's player.
struct Stream {
    host: Host,
    map: MapInfo,
    rules: RulesHash,
    you: Role,
    seat: PlayerId,
    last: TurnMessage,
}

impl Stream {
    /// The game on the multiplayer map `map`, for the seat of slot `you`.
    fn new(map: &str, you: usize) -> Stream {
        let text = read(&format!("res/multi/{map}.txt"));
        let rules = rules();
        let setup = GameSetup::multiplayer(
            rules.clone(),
            read("res/ai.txt"),
            text.clone(),
            1,
            slots().iter().map(Slot::controller).collect(),
        );
        let host = Host::new(&setup).unwrap();
        let seat = host.seats()[you];
        let last = host.message(seat);
        Stream {
            host,
            map: MapInfo {
                file_name: MapFileName::new(format!("{map}.txt")).unwrap(),
                hash: MapHash::of(text.as_bytes()),
                title: Vec::new(),
                nb_players_min: 2,
                nb_players_max: 2,
            },
            rules: RulesHash::of(&rules),
            you: Role::Player(you),
            seat,
            last,
        }
    }

    fn watching(mut self) -> Stream {
        self.you = Role::Watcher;
        self
    }

    /// What a server sends when the seat joins or returns: its seat, the
    /// start and its whole message.
    fn opening(&self) -> Vec<SeatFrame> {
        vec![
            SeatFrame::Seated {
                you: self.you,
                map: self.map.clone(),
                speed: Speed::Normal,
                paused: false,
                hold: std::time::Duration::from_secs(30),
                token: SeatToken::from_bits(1),
            },
            SeatFrame::Start {
                start: self.host.game_start().message(),
                rules: self.rules,
            },
            SeatFrame::Whole(self.last.clone()),
        ]
    }

    /// Plays a turn, and gives the frames a server sends the seat for it.
    fn turn(&mut self) -> Vec<SeatFrame> {
        let turn = self.host.step_seats(&[self.seat]).pop().unwrap();
        let frames = turn
            .changes(&self.last, [])
            .into_iter()
            .map(SeatFrame::Change)
            .collect();
        self.last = (*turn.message).clone();
        frames
    }

    /// The watcher follows the player of `slot` from now on.
    fn follow(&mut self, slot: usize) -> SeatFrame {
        self.seat = self.host.seats()[slot];
        self.last = self.host.message(self.seat);
        SeatFrame::Whole(self.last.clone())
    }

    fn end(&self) -> SeatFrame {
        SeatFrame::End(self.host.game_end(self.seat))
    }
}

fn encoded(frames: Vec<SeatFrame>) -> Vec<Vec<u8>> {
    frames.iter().map(|f| encode_frame(f).unwrap()).collect()
}

fn frame(frames: &[Vec<u8>], i: usize) -> SeatFrame {
    decode_frame(&frames[i]).unwrap()
}

/// Where a stream was refused: the frame's place, and why.
type Refused = (usize, String);

/// The seat's stream once it has joined the game, as the client reads the
/// frames: the seat, taken by the lobby's side of the client, then the
/// frames of a seat that joins.
fn joined(rules: &str, frames: &[Vec<u8>]) -> Result<(SeatStream, usize), Refused> {
    let read = |i: usize| decode_frame(&frames[i]).map_err(|e| (i, e.0));
    let SeatFrame::Seated { you, map, .. } = read(0)? else {
        return Err((0, "no seat".into()));
    };
    let mut join = JoinStream::new(you, map.hash, rules);
    for i in 1..frames.len() {
        match join.take(read(i)?).map_err(|e| (i, e.to_string()))? {
            Joining::Started(next) => join = next,
            Joining::Joined(joined) => return Ok((joined.stream, i + 1)),
        }
    }
    Err((frames.len(), "the game did not start".into()))
}

/// Takes `frames` from `from` on with `stream`.
fn play(stream: &mut SeatStream, frames: &[Vec<u8>], from: usize) -> Result<(), Refused> {
    for (i, f) in frames.iter().enumerate().skip(from) {
        let f = decode_frame(f).map_err(|e| (i, e.0))?;
        stream.take(f).map_err(|e| (i, e.0))?;
    }
    Ok(())
}

/// Reads the stream of a seat that joins the game, to its end.
fn read_stream(frames: &[Vec<u8>]) -> Result<(), Refused> {
    let (mut stream, from) = joined(&rules(), frames)?;
    play(&mut stream, frames, from)
}

/// The frames a server sends Andrew: the seat, the start, the whole
/// message, a change a turn, then the end.
fn real_stream() -> Vec<Vec<u8>> {
    let mut stream = Stream::new("jl1", 0);
    let mut frames = stream.opening();
    for _ in 0..STREAM_TURNS {
        frames.extend(stream.turn());
    }
    frames.push(stream.end());
    encoded(frames)
}

#[test]
fn a_hostile_stream_of_a_game_is_refused_at_the_frame_that_cannot_be_right() {
    let frames = real_stream();
    assert_eq!(read_stream(&frames), Ok(()), "the real stream");
    let n = frames.len();
    let with = |at: usize, f: Vec<u8>| {
        let mut s = frames.to_vec();
        s.insert(at, f);
        s
    };
    let changed = |at: usize, f: SeatFrame| {
        let mut s = frames.to_vec();
        s[at] = encode_frame(&f).unwrap();
        s
    };
    let whole = || {
        let SeatFrame::Whole(m) = frame(&frames, 2) else {
            panic!("a whole message");
        };
        m
    };
    let inside_itself = {
        let mut m = whole();
        let e = &mut m.perceived[0];
        e.place = Some(Place::Inside(e.id));
        let id = e.id;
        (changed(2, SeatFrame::Whole(m)), id)
    };
    let refused: Vec<(&str, Vec<Vec<u8>>, Refused)> = vec![
        (
            "a change before the start",
            {
                let mut s = frames.to_vec();
                s.swap(1, 3);
                s
            },
            (1, "before the game: a change".into()),
        ),
        (
            "the start twice",
            with(2, frames[1].clone()),
            (2, "before the game: a start".into()),
        ),
        (
            "the whole message twice",
            with(3, frames[2].clone()),
            (3, "a whole message during the game".into()),
        ),
        (
            "a whole message during the game",
            with(10, frames[2].clone()),
            (10, "a whole message during the game".into()),
        ),
        (
            "a turn played again",
            with(10, frames[5].clone()),
            (10, "a turn back in time".into()),
        ),
        (
            "the end twice",
            with(n - 1, frames[n - 1].clone()),
            (n, "an end after the end".into()),
        ),
        (
            "a turn after the end",
            with(n, frames[n - 2].clone()),
            (n, "a change after the end".into()),
        ),
        (
            "a seat during the game",
            with(8, frames[0].clone()),
            (8, "a seat during the game".into()),
        ),
        (
            "a refused start during the game",
            with(
                8,
                encode_frame(&SeatFrame::NotStarted(StartRefusal::Busy)).unwrap(),
            ),
            (8, "a refused start during the game".into()),
        ),
        (
            "food out of range",
            {
                let mut m = whole();
                m.state.available_food = i64::MAX;
                m.state.used_food = -1;
                changed(2, SeatFrame::Whole(m))
            },
            (2, "the first message: an amount out of range".into()),
        ),
        (
            "an entity inside itself",
            inside_itself.0,
            (
                2,
                format!(
                    "the first message: entity {} is inside itself",
                    inside_itself.1
                ),
            ),
        ),
        (
            "too many objectives",
            {
                let mut m = whole();
                m.state.objectives = (0..=soundrts_world::MAX_OBJECTIVES)
                    .map(|i| Objective {
                        number: i.to_string(),
                        description: soundrts_world::msg::nb2msg(1),
                    })
                    .collect();
                changed(2, SeatFrame::Whole(m))
            },
            (
                2,
                "the first message: too many objectives, or too long".into(),
            ),
        ),
        (
            "statistics out of range",
            {
                let SeatFrame::End(mut end) = frame(&frames, n - 1) else {
                    panic!("an end");
                };
                end.stats
                    .add(StatEvent::Gathered, StatTarget::Resource(9), i64::MAX);
                changed(n - 1, SeatFrame::End(end))
            },
            (n - 1, "the end: a statistic out of range".into()),
        ),
    ];
    let mut failures = Vec::new();
    for (what, stream, expected) in refused {
        let got = read_stream(&stream);
        if got != Err(expected.clone()) {
            failures.push(format!("{what}: {got:?}, not {expected:?}"));
        }
    }
    let no_frame = read_stream(&with(8, vec![0xff; 40]));
    if !matches!(&no_frame, Err((8, why)) if why.starts_with("cannot decode")) {
        failures.push(format!("bytes that are no frame: {no_frame:?}"));
    }
    // The interface's to check against the room and against what it can
    // tell; the clock's news names its player by login, who need not be in
    // the room.
    let taken = [
        (
            "news of a slot that is not in the room",
            with(
                8,
                encode_frame(&SeatFrame::Seat {
                    slot: usize::MAX,
                    change: SeatChange::Returned,
                })
                .unwrap(),
            ),
        ),
        ("more events than a game can tell", {
            let SeatFrame::Change(mut c) = frame(&frames, 3) else {
                panic!("a change");
            };
            c.events
                .extend(std::iter::repeat_n(Event::Speed(1.0), 70_000));
            changed(3, SeatFrame::Change(c))
        }),
        (
            "a clock change",
            with(
                8,
                encode_frame(&SeatFrame::Clock {
                    by: Login::new("nobody").unwrap(),
                    change: ClockChange::Paused,
                })
                .unwrap(),
            ),
        ),
    ];
    for (what, stream) in taken {
        let got = read_stream(&stream);
        if got.is_err() {
            failures.push(format!("{what}: {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Andrew's stream before his connection is lost (the seat, the start, the
/// whole message, 5 turns); after 3 turns he missed, what is sent when he
/// returns (the seat, the start, the whole message); and 3 turns more,
/// then the end.
fn return_stream() -> [Vec<Vec<u8>>; 3] {
    let mut stream = Stream::new("jl1", 0);
    let mut before = stream.opening();
    for _ in 0..5 {
        before.extend(stream.turn());
    }
    for _ in 0..3 {
        stream.turn();
    }
    let back = stream.opening();
    let mut later = Vec::new();
    for _ in 0..3 {
        later.extend(stream.turn());
    }
    later.push(stream.end());
    [encoded(before), encoded(back), encoded(later)]
}

/// Where a return was refused: in the frames of the return, or after.
#[derive(Debug, PartialEq)]
enum ReturnRefused {
    Back(Refused),
    Later(Refused),
}

/// Reads `before`, then, on a new connection, `back` as a seat that returns
/// does, then `later` from the seat given back.
fn read_return(
    before: &[Vec<u8>],
    back: &[Vec<u8>],
    later: &[Vec<u8>],
) -> Result<(), ReturnRefused> {
    let (mut stream, from) = joined(&rules(), before).unwrap();
    play(&mut stream, before, from).unwrap();
    let mut returning: ReturnStream = stream.returning();
    for (i, f) in back.iter().enumerate() {
        let f = decode_frame(f).map_err(|e| ReturnRefused::Back((i, e.0)))?;
        match returning
            .take(f)
            .map_err(|e| ReturnRefused::Back((i, e.0)))?
        {
            Returning::More(next) => returning = next,
            Returning::Back(mut b) => {
                return play(&mut b.stream, later, 0).map_err(ReturnRefused::Later);
            }
            Returning::Stopped(r) => return Err(ReturnRefused::Back((i, format!("{r:?}")))),
        }
    }
    let what = "the seat did not come back".to_string();
    Err(ReturnRefused::Back((back.len(), what)))
}

#[test]
fn a_hostile_return_to_a_game_is_refused_at_the_frame_that_cannot_be_right() {
    let [before, back, later] = return_stream();
    assert_eq!(
        read_return(&before, &back, &later),
        Ok(()),
        "the real return"
    );
    let with = |i: usize, f: Vec<u8>| {
        let mut s = back.clone();
        s[i] = f;
        s
    };
    let other = |map: &str, you: usize| {
        let mut stream = Stream::new(map, you);
        for _ in 0..8 {
            stream.turn();
        }
        encoded(stream.opening())
    };
    let another_game = other("jl6", 0);
    let another_player = other("jl1", 1);
    let SeatFrame::Seated {
        map,
        speed,
        paused,
        hold,
        token,
        ..
    } = frame(&back, 0)
    else {
        panic!("a seat");
    };
    let SeatFrame::Start { start, .. } = frame(&back, 1) else {
        panic!("a start");
    };
    let SeatFrame::Whole(theirs) = frame(&another_player, 2) else {
        panic!("a whole message");
    };
    let refused = |i: usize, why: &str| Err(ReturnRefused::Back((i, why.to_string())));
    let cases = [
        (
            "the seat of another slot",
            with(
                0,
                encode_frame(&SeatFrame::Seated {
                    you: Role::Player(1),
                    map,
                    speed,
                    paused,
                    hold,
                    token,
                })
                .unwrap(),
            ),
            refused(0, "the seat Player(1)"),
        ),
        (
            "the start of another game",
            with(1, another_game[1].clone()),
            refused(1, "the start of another game"),
        ),
        (
            "the start of the game with other rules",
            with(
                1,
                encode_frame(&SeatFrame::Start {
                    start,
                    rules: RulesHash::of("other rules"),
                })
                .unwrap(),
            ),
            refused(1, "the start of another game"),
        ),
        (
            "the message of another player",
            with(2, another_player[2].clone()),
            refused(2, &format!("the message of player {}", theirs.seat.0)),
        ),
        (
            "a message back in time",
            with(2, before[2].clone()),
            refused(2, "a message back in time"),
        ),
        (
            "a change before the whole message",
            with(2, later[0].clone()),
            refused(2, "a change before the seat was back"),
        ),
        (
            "the whole message before the start",
            {
                let mut s = back.clone();
                s.swap(1, 2);
                s
            },
            refused(1, "a whole message before the seat was back"),
        ),
        (
            "the start before the seat",
            {
                let mut s = back.clone();
                s.swap(0, 1);
                s
            },
            refused(0, "a start before the seat was back"),
        ),
    ];
    let mut failures = Vec::new();
    for (what, back, expected) in cases {
        let got = read_return(&before, &back, &later);
        if got != expected {
            failures.push(format!("{what}: {got:?}, not {expected:?}"));
        }
    }
    let no_frame = read_return(&before, &with(1, vec![0xff; 40]), &later);
    if !matches!(&no_frame, Err(ReturnRefused::Back((1, why))) if why.starts_with("cannot decode"))
    {
        failures.push(format!("bytes that are no frame: {no_frame:?}"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The frames a server sends Carol, who watches Andrew's game: the seat,
/// the start, Andrew's whole message, then a change a turn, and Andrew's
/// whole message again every other turn, as when she asks to follow the
/// next player and he is the only one; then the end.
fn watcher_stream() -> Vec<Vec<u8>> {
    let mut stream = Stream::new("jl1", 0).watching();
    let mut frames = stream.opening();
    for turn in 0..STREAM_TURNS {
        frames.extend(stream.turn());
        if turn % 2 == 1 {
            frames.push(stream.follow(0));
        }
    }
    frames.push(stream.end());
    encoded(frames)
}

#[test]
fn a_hostile_stream_of_a_watched_game_is_refused_at_the_frame_that_cannot_be_right() {
    let frames = watcher_stream();
    assert_eq!(read_stream(&frames), Ok(()), "the real stream");
    let later = frames
        .iter()
        .rposition(|f| matches!(decode_frame(f), Ok(SeatFrame::Whole(_))))
        .unwrap();
    let SeatFrame::Whole(whole) = frame(&frames, later) else {
        panic!("a whole message");
    };
    let with = |f: SeatFrame| {
        let mut s = frames.clone();
        s.insert(later + 1, encode_frame(&f).unwrap());
        s
    };
    let at = later + 1;
    let gone = {
        let mut m = whole.clone();
        m.state.players.retain(|&p| p != m.seat);
        m
    };
    let none = TurnMessage {
        seat: PlayerId(200),
        ..whole.clone()
    };
    let cases = [
        (
            "a whole message back in time",
            with(frame(&frames, 2)),
            (at, "a whole message back in time".to_string()),
        ),
        (
            "a whole message of a player no longer in the game",
            with(SeatFrame::Whole(gone)),
            (
                at,
                format!("to follow player {}, not in the game", whole.seat.0),
            ),
        ),
        (
            "a whole message of a player that is none",
            with(SeatFrame::Whole(none)),
            (at, "to follow player 200, not in the game".to_string()),
        ),
    ];
    let mut failures = Vec::new();
    for (what, stream, expected) in cases {
        let got = read_stream(&stream);
        if got != Err(expected.clone()) {
            failures.push(format!("{what}: {got:?}, not {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A whole message during the game is a watcher's only, and the stream
/// says whom the watcher follows from then on.
#[test]
fn a_watcher_is_told_whom_it_follows() {
    let frames = watcher_stream();
    let (mut stream, from) = joined(&rules(), &frames).unwrap();
    let mut followed = Vec::new();
    for f in frames.iter().skip(from) {
        if let Checked::Follow(m) = stream.take(decode_frame(f).unwrap()).unwrap() {
            followed.push(m.seat);
        }
    }
    assert_eq!(followed, vec![PlayerId(0); STREAM_TURNS / 2]);
}
