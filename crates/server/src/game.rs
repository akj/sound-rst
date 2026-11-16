//! One task per game: the server runs the game itself and sends
//! each seat only what its player knows.
//!
//! The task builds the game from the set-up the lobby made, owns the
//! [`Host`], steps it on a clock that waits for nobody, and sends each
//! seat its stream: the start, with the hash of the rules text it was
//! built from, and the whole message when the seat joins or returns,
//! then the change since the last message it was sent, each turn. The world's work (building, and each turn with every seat's
//! message) runs on a blocking thread with a time budget; a panic there
//! ends this game and nothing else.
//!
//! A seat is a player's or a watcher's ([`SeatId`]). A watcher has no
//! player of its own: it is sent the side of a human player it follows,
//! the very stream that player's seat is sent, and the world never hears
//! of it. [`Link`], [`Pace`] and [`Game::deliver`] serve both alike. A
//! watcher joins a game that runs as a player returns to it: it is sent
//! the start and the whole message of the player it follows.
//!
//! The room's host's powers (the clock and removals) are the game's to
//! check: they stay with one player still in the game, and pass on when
//! that player's seat is gone ([`Game::hand_on_host`]).
//!
//! The host records every command it runs. When the game ends, or is
//! stopped, the server writes its record to the folder its settings name,
//! if they name one ([`keep_record`]). A turn that panics is recorded too,
//! so that the record replays to the panic; a turn that runs past its
//! budget runs on with the host, and its game is not recorded.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use soundrts_protocol::game::{Role, RulesHash, SeatFrame, encode_frame};
use soundrts_protocol::{
    ChatText, ClockChange, FLARE_WINDOW, Login, MAX_FLARES, MAX_TURN_ORDER_BYTES, MapInfo, Refusal,
    RoomId, SeatChange, SeatToken, ServerMsg, Slot, SlotIndex, Speed, StartRefusal, Step,
    StopReason,
};
use soundrts_world::{
    Command, Event, GameSetup, Host, PlayerId, Record, SeatTurn, SquareId, TurnMessage,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Bytes;
use tracing::{debug, info, warn};

use crate::connection::{ConnId, Outbox, Queued, Window};
use crate::lobby::{LobbyInput, LobbyTx};

/// A seat whose connection has more messages than this waiting to be
/// written is not keeping up: it is sent nothing more until it has fewer,
/// then one change that catches it up. A seat in step has none waiting
/// when a turn is played: a turn is one message, written in microseconds.
const SEAT_BACKLOG: usize = 4;

/// The most events not about an entity kept for a seat that is behind,
/// oldest dropped first. A seat that stops reading stops answering the
/// heartbeat too, so it is dropped long before this many pile up.
const MAX_MISSED_EVENTS: usize = 1000;

/// How a game runs: from the server's settings and the room.
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub speed: Speed,
    /// How long a dropped seat is held for its player.
    pub grace: Duration,
    /// The longest the game may take to build.
    pub load_budget: Duration,
    /// The longest one turn may take: the world's step and every seat's
    /// message.
    pub turn_budget: Duration,
    /// The slot of the room's host when the game starts.
    pub host: SlotIndex,
    /// The server's count of work past its budget.
    pub overruns: Overruns,
    /// Where the game's record goes; `None` keeps none.
    pub records: Option<Arc<Path>>,
}

/// The most records of games the server keeps in its records folder: the
/// oldest are removed.
pub const RECORDS_KEPT: usize = 100;

/// The extension of a record's file.
pub const RECORD_EXTENSION: &str = "jsonl";

/// While this many builds or turns of games run on past their
/// budget, the server starts no game.
pub(crate) const MAX_OVERRUNNING: usize = 2;

/// The world's work, of any game on the server, that ran past its
/// budget and still runs. Nothing can stop it, so it is counted until it
/// ends, and while there is too much of it no game starts: a
/// player who makes games slow on purpose cannot pile it up.
#[derive(Clone, Debug, Default)]
pub(crate) struct Overruns(Arc<AtomicUsize>);

impl Overruns {
    pub(crate) fn too_many(&self) -> bool {
        self.0.load(Ordering::Relaxed) >= MAX_OVERRUNNING
    }
}

/// Why the world's work gave nothing back.
enum Late {
    /// It panicked.
    Failed(String),
    /// It ran past its budget, and runs on.
    Overran,
}

/// Where a piece of the world's work is: running, past its budget and
/// counted, or done.
const RUNNING: u8 = 0;
const COUNTED: u8 = 1;
const DONE: u8 = 2;

/// Held by the work: when it ends, it is no longer counted.
struct Watch {
    state: Arc<AtomicU8>,
    overruns: Overruns,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.state.swap(DONE, Ordering::AcqRel) == COUNTED {
            self.overruns.0.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Runs the world's `work` on a blocking thread, within `budget`.
async fn within<T: Send + 'static>(
    budget: Duration,
    overruns: &Overruns,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Late> {
    let state = Arc::new(AtomicU8::new(RUNNING));
    let watch = Watch {
        state: Arc::clone(&state),
        overruns: overruns.clone(),
    };
    let job = tokio::task::spawn_blocking(move || {
        let _watch = watch;
        work()
    });
    match tokio::time::timeout(budget, job).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Late::Failed(e.to_string())),
        Err(_) => {
            // Counted first, so that the work ending meanwhile never
            // takes away more than was counted.
            overruns.0.fetch_add(1, Ordering::Relaxed);
            if state
                .compare_exchange(RUNNING, COUNTED, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                overruns.0.fetch_sub(1, Ordering::Relaxed);
            }
            Err(Late::Overran)
        }
    }
}

/// Who has a seat in a game: the player of a slot of the room, or a
/// watcher. A watcher's seat is sent the side of a player it follows, and
/// what it may send the game is only [`GameInput::Follow`] and
/// [`GameInput::Leave`]: the inputs that change the game take a
/// [`SlotIndex`], which a watcher does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SeatId {
    Player(SlotIndex),
    Watcher(WatcherId),
}

/// A watcher of a room and its game, for as long as it watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct WatcherId(pub u32);

/// A seat as the lobby hands it over when the game starts.
pub(crate) struct Joining {
    pub seat: SeatId,
    pub conn: ConnId,
    pub login: Login,
    pub outbox: Outbox,
}

/// What the lobby passes to a game. The lobby has checked whose seat
/// sends it: a player's for a command, the clock and removals, a watcher's
/// to follow another player. The game checks that the clock and removals
/// come from the player who holds the host's powers.
pub(crate) enum GameInput {
    Command {
        slot: SlotIndex,
        command: Command,
        bytes: usize,
    },
    /// The seat's player or watcher asked to leave.
    Leave {
        seat: SeatId,
    },
    /// The seat's connection is gone.
    Dropped {
        seat: SeatId,
    },
    /// A player in the lobby asks for a dropped seat back.
    Return {
        conn: ConnId,
        token: SeatToken,
        outbox: Outbox,
    },
    /// Someone joins the game that runs, to watch.
    Watch(Joining),
    /// A watcher follows the next player, or the previous one.
    Follow {
        watcher: WatcherId,
        step: Step,
    },
    Pause {
        by: SlotIndex,
    },
    Resume {
        by: SlotIndex,
    },
    Speed {
        by: SlotIndex,
        speed: Speed,
    },
    Remove {
        by: SlotIndex,
        slot: SlotIndex,
    },
    /// A player points their allies at a square.
    Flare {
        slot: SlotIndex,
        square: SquareId,
    },
}

/// Where the lobby passes inputs to a game. An input from a client
/// comes with its bytes, counted against the client's allowance until the
/// game has handled it.
pub(crate) type GameTx = mpsc::UnboundedSender<(GameInput, Option<Queued>)>;

/// How a game's task ended, for the lobby.
#[derive(Debug)]
pub(crate) enum Ending {
    /// The game could not be built: the room goes back to gathering.
    NotBuilt(StartRefusal),
    /// No seat is left: each was told its end and reported to the lobby.
    Over,
    /// The server stopped the game for everyone in it.
    Stopped(StopReason),
}

/// Starts a game's task. The game reports to the lobby each seat
/// whose game is over ([`LobbyInput::LeftGame`], or
/// [`LobbyInput::WatcherLeft`]), returns accepted or refused, and how it
/// ended ([`LobbyInput::GameEnded`]), even if it panics.
pub(crate) fn spawn(
    room: RoomId,
    setup: GameSetup,
    slots: Vec<Slot>,
    map: MapInfo,
    seats: Vec<Joining>,
    settings: Settings,
    lobby: LobbyTx,
) -> GameTx {
    let (tx, rx) = mpsc::unbounded_channel();
    let reporter = lobby.clone();
    let task = tokio::spawn(run(room, setup, slots, map, seats, settings, lobby, rx));
    tokio::spawn(async move {
        let ending = task.await.unwrap_or_else(|e| {
            warn!(room = room.0, error = %e, "game failed");
            Ending::Stopped(StopReason::Crashed)
        });
        info!(room = room.0, ?ending, "game ended");
        let _ = reporter.send(LobbyInput::GameEnded { room, ending });
    });
    tx
}

/// A seat still in the game: a player's, or a watcher's.
struct Seat {
    login: Login,
    token: SeatToken,
    link: Link,
    /// The player whose side of the game the seat is sent: its own
    /// player's, or the one a watcher follows.
    side: PlayerId,
}

/// A seat's connection. A seat that is gone is no longer in the game's
/// seats.
enum Link {
    Connected {
        conn: ConnId,
        outbox: Outbox,
        /// The last message the seat was sent: the next change is from it.
        last: Arc<TurnMessage>,
        pace: Pace,
    },
    /// The connection is lost; the seat is held for its player until
    /// `deadline`.
    Dropped { deadline: Instant },
}

enum Pace {
    /// The seat was sent the last turn played.
    InStep,
    /// The seat was not sent the turns since its last message: these are
    /// their events not about an entity, oldest first.
    Behind { missed: VecDeque<Event> },
}

enum Clock {
    Running(Interval),
    Paused,
}

impl Clock {
    fn running(speed: Speed) -> Clock {
        let period = speed.turn_interval();
        let mut clock = tokio::time::interval_at(after(period), period);
        clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Clock::Running(clock)
    }

    async fn tick(&mut self) {
        match self {
            Clock::Running(clock) => {
                clock.tick().await;
            }
            Clock::Paused => std::future::pending().await,
        }
    }
}

/// The most players a watcher's steps along the players count: more go
/// round again.
const MAX_STEPS: i64 = 256;

struct Game {
    room: RoomId,
    settings: Settings,
    lobby: LobbyTx,
    slots: Vec<Slot>,
    map: MapInfo,
    /// The start, encoded once for every seat.
    start: Bytes,
    /// The player of each slot.
    players: Vec<PlayerId>,
    seats: BTreeMap<SeatId, Seat>,
    /// How far along the players each watcher asked to move, not yet done:
    /// at the next turn, or after [`Game::switch_at`] while paused.
    steps: BTreeMap<WatcherId, i64>,
    /// While the game is paused, when the watchers' steps are done: a turn
    /// after the first was asked for, so that no watcher has the game
    /// build messages faster than a running game would.
    switch_at: Option<Instant>,
    /// The commands for the next turn, in the order they came.
    commands: Vec<(PlayerId, Command)>,
    /// The bytes of commands each player has given for the next turn.
    command_bytes: BTreeMap<SlotIndex, usize>,
    clock: Clock,
    turn: u64,
    /// The players still in the game, as of the last turn a seat was sent.
    in_game: Vec<PlayerId>,
    /// Each player's allies, the player among them, by player id, as of the
    /// last turn a seat was sent.
    allies: Vec<Vec<PlayerId>>,
    /// The flares each player sent lately.
    flares: BTreeMap<SlotIndex, Window>,
    /// The slot of the player who holds the room's host's powers: a player
    /// with a seat in the game, connected or held. `None` once no player
    /// is left to hold them.
    host: Option<SlotIndex>,
}

/// What a connected seat is sent of the coming turn.
enum Sending {
    /// The change from its last message, with the events it missed first.
    Change {
        last: Arc<TurnMessage>,
        missed: VecDeque<Event>,
    },
    /// The whole message of its side: a watcher that follows another
    /// player.
    Whole,
}

/// A connected seat's share of a turn: whose side it is sent, and what,
/// if it is sent the turn.
struct Plan {
    seat: SeatId,
    side: PlayerId,
    send: Option<Sending>,
}

/// What a turn gave a connected seat's side, and the frames that send it,
/// if it is sent the turn.
struct Delivery {
    seat: SeatId,
    side: PlayerId,
    turn: Arc<SeatTurn>,
    frames: Option<Sent>,
}

/// The frames a seat is sent of a turn.
struct Sent {
    frames: Vec<Bytes>,
    /// They are a whole message: the seat follows its side from now on.
    whole: bool,
}

#[allow(clippy::too_many_arguments)]
async fn run(
    room: RoomId,
    setup: GameSetup,
    slots: Vec<Slot>,
    map: MapInfo,
    joining: Vec<Joining>,
    settings: Settings,
    lobby: LobbyTx,
    mut inputs: mpsc::UnboundedReceiver<(GameInput, Option<Queued>)>,
) -> Ending {
    info!(room = room.0, "game building");
    let rules = RulesHash::of(&setup.rules);
    #[cfg(feature = "fault-injection")]
    let setup_rules: String = setup.rules.chars().take(64).collect();
    let mut host = match build(setup, settings.load_budget, &settings.overruns).await {
        Ok(host) => host,
        Err(refusal) => return Ending::NotBuilt(refusal),
    };
    for w in host.drain_warnings() {
        debug!(room = room.0, warning = %w, "while building");
    }
    #[cfg(feature = "fault-injection")]
    let fault = fault::of(&setup_rules);
    let start = SeatFrame::Start {
        start: host.game_start().message(),
        rules,
    };
    let start = match encode_frame(&start) {
        Ok(bytes) => Bytes::from(bytes),
        Err(e) => {
            warn!(room = room.0, error = %e, "cannot encode the start");
            return Ending::NotBuilt(StartRefusal::Crashed);
        }
    };
    let players = host.seats().to_vec();
    let mut game = Game {
        room,
        host: Some(settings.host),
        clock: Clock::running(settings.speed),
        settings,
        lobby,
        slots,
        map,
        start,
        seats: BTreeMap::new(),
        steps: BTreeMap::new(),
        switch_at: None,
        players,
        commands: Vec::new(),
        command_bytes: BTreeMap::new(),
        turn: 0,
        in_game: host.seats().to_vec(),
        allies: host
            .seats()
            .first()
            .map(|&p| host.message(p).state.allies)
            .unwrap_or_default(),
        flares: BTreeMap::new(),
    };
    for j in joining {
        game.seat(&host, j);
    }
    info!(room = room.0, seats = game.seats.len(), "game started");

    while !game.seats.is_empty() {
        let deadline = game.next_deadline();
        tokio::select! {
            _ = game.clock.tick() => {
                let budget = game.settings.turn_budget;
                let commands = std::mem::take(&mut game.commands);
                game.command_bytes.clear();
                let plans = game.plans();
                #[cfg(feature = "fault-injection")]
                fault::before_turn(fault);
                // A panic in the turn keeps the host, for its record.
                let work = move || {
                    let played = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        #[cfg(feature = "fault-injection")]
                        fault::in_turn(fault);
                        play_turn(&mut host, commands, plans)
                    }));
                    (host, played)
                };
                match within(budget, &game.settings.overruns, work).await {
                    Ok((h, Err(_panic))) => {
                        warn!(room = room.0, turn = game.turn, "the game failed in a turn");
                        keep_record(room, game.settings.records.clone(), &h).await;
                        return Ending::Stopped(StopReason::Crashed);
                    }
                    Ok((h, Ok(deliveries))) => {
                        host = h;
                        game.turn = game.turn.saturating_add(1);
                        for w in host.drain_warnings() {
                            debug!(room = room.0, warning = %w, "in a turn");
                        }
                        game.deliver(&host, deliveries);
                    }
                    Err(Late::Failed(e)) => {
                        warn!(room = room.0, turn = game.turn, error = %e, "the game failed in a turn");
                        return Ending::Stopped(StopReason::Crashed);
                    }
                    Err(Late::Overran) => {
                        warn!(room = room.0, turn = game.turn, ?budget, "a turn took too long");
                        return Ending::Stopped(StopReason::TooSlow);
                    }
                }
            }
            _ = sleep_until(deadline) => {
                let now = Instant::now();
                game.give_up_expired(now);
                game.switch_if_due(&host, now);
            }
            input = inputs.recv() => match input {
                Some((input, _queued)) => game.on_input(&host, input),
                None => break,
            },
        }
        game.hand_on_host();
    }
    info!(room = room.0, turns = game.turn, "game over");
    keep_record(room, game.settings.records.clone(), &host).await;
    Ending::Over
}

/// Writes the record of the game of `host` to `records`, on a blocking
/// thread, and removes the oldest beyond [`RECORDS_KEPT`]. What goes wrong
/// is logged.
async fn keep_record(room: RoomId, records: Option<Arc<Path>>, host: &Host) {
    let Some(dir) = records else {
        return;
    };
    let record = host.record().clone();
    let written = tokio::task::spawn_blocking(move || write_record(&dir, room, &record)).await;
    match written {
        Ok(Ok(path)) => info!(room = room.0, path = %path.display(), "game recorded"),
        Ok(Err(e)) => warn!(room = room.0, error = %e, "cannot write the game's record"),
        Err(e) => warn!(room = room.0, error = %e, "cannot write the game's record"),
    }
}

/// Writes `record` to a new file of `dir`, named after the time and the
/// room: written whole beside it first, then renamed. Then removes the
/// oldest records beyond [`RECORDS_KEPT`].
fn write_record(dir: &Path, room: RoomId, record: &Record) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let base = format!("game-{secs:012}-room{}", room.0);
    let mut path = dir.join(format!("{base}.{RECORD_EXTENSION}"));
    let mut n: u32 = 2;
    while path.exists() && n < 1000 {
        path = dir.join(format!("{base}-{n}.{RECORD_EXTENSION}"));
        n = n.saturating_add(1);
    }
    let beside = path.with_extension(format!("{RECORD_EXTENSION}.tmp"));
    let result = (|| {
        let mut file = std::io::BufWriter::new(std::fs::File::create(&beside)?);
        record.write(&mut file)?;
        file.flush()?;
        let file = file.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&beside, &path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&beside);
        return Err(e);
    }
    remove_oldest_records(dir)?;
    Ok(path)
}

/// Removes the oldest records of `dir` beyond [`RECORDS_KEPT`]: their
/// names start with their time.
fn remove_oldest_records(dir: &Path) -> std::io::Result<()> {
    let suffix = format!(".{RECORD_EXTENSION}");
    let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .filter(|n| {
            let n = n.to_string_lossy();
            n.starts_with("game-") && n.ends_with(&suffix)
        })
        .collect();
    names.sort();
    let extra = names.len().saturating_sub(RECORDS_KEPT);
    for name in names.iter().take(extra) {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

/// A frame encoded, or `None`, said in the log, if it cannot be.
fn encoded(frame: &SeatFrame) -> Option<Bytes> {
    match encode_frame(frame) {
        Ok(bytes) => Some(Bytes::from(bytes)),
        Err(e) => {
            warn!(error = %e, "cannot encode a frame");
            None
        }
    }
}

/// Sends one seat a frame.
pub(crate) fn tell(outbox: &Outbox, frame: &SeatFrame) {
    if let Some(bytes) = encoded(frame) {
        outbox.send_binary(bytes);
    }
}

/// The instant `d` from now, or a year from now if that is beyond what an
/// instant holds.
pub(crate) fn after(d: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(d)
        .or_else(|| now.checked_add(Duration::from_secs(365 * 24 * 3600)))
        .unwrap_or(now)
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

/// Builds the game on a blocking thread, within `budget`.
async fn build(
    setup: GameSetup,
    budget: Duration,
    overruns: &Overruns,
) -> Result<Host, StartRefusal> {
    let work = move || {
        #[cfg(feature = "fault-injection")]
        fault::while_building(fault::of(&setup.rules));
        Host::new(&setup)
    };
    match within(budget, overruns, work).await {
        Ok(Ok(host)) => Ok(host),
        Ok(Err(error)) => match ChatText::clean(&error).or_else(|| ChatText::clean("?")) {
            Some(error) => Err(StartRefusal::CannotBuild { error }),
            None => Err(StartRefusal::Crashed),
        },
        Err(Late::Failed(e)) => {
            warn!(error = %e, "building a game failed");
            Err(StartRefusal::Crashed)
        }
        Err(Late::Overran) => Err(StartRefusal::TooSlow),
    }
}

/// The world's part of a turn, on a blocking thread: the commands, the
/// step, and the frames of each connected seat that is sent the turn.
///
/// The world builds each side's message once, however many seats are
/// sent it, and seats sent the same thing share its encoding: a watcher
/// in step with the player it follows is sent the very bytes that player
/// is.
fn play_turn(
    host: &mut Host,
    commands: Vec<(PlayerId, Command)>,
    plans: Vec<Plan>,
) -> Vec<Delivery> {
    for (player, command) in commands {
        // A game with others refuses the cheats that act at once, so
        // nothing is told now.
        let _ = host.queue(player, command);
    }
    let mut sides: Vec<PlayerId> = Vec::new();
    for plan in &plans {
        if !sides.contains(&plan.side) {
            sides.push(plan.side);
        }
    }
    let turns: Vec<(PlayerId, Arc<SeatTurn>)> = sides
        .iter()
        .copied()
        .zip(host.step_seats(&sides).into_iter().map(Arc::new))
        .collect();
    let mut shared = Shared::default();
    plans
        .into_iter()
        .filter_map(|plan| {
            let (_, turn) = turns.iter().find(|(side, _)| *side == plan.side)?;
            let frames = plan
                .send
                .and_then(|send| shared.frames(plan.seat, plan.side, turn, send));
            Some(Delivery {
                seat: plan.seat,
                side: plan.side,
                turn: Arc::clone(turn),
                frames,
            })
        })
        .collect()
}

/// The frames of a turn encoded so far, for seats sent the same thing.
#[derive(Default)]
struct Shared {
    /// The changes of a side from a last message, for seats that missed
    /// nothing.
    changes: Vec<(PlayerId, Arc<TurnMessage>, Vec<Bytes>)>,
    /// The whole message of a side.
    wholes: Vec<(PlayerId, Bytes)>,
}

impl Shared {
    /// The frames that send `send` of `turn`, the turn of `side`, to
    /// `seat`; `None` if one cannot be encoded.
    fn frames(
        &mut self,
        seat: SeatId,
        side: PlayerId,
        turn: &SeatTurn,
        send: Sending,
    ) -> Option<Sent> {
        match send {
            Sending::Change { last, missed } if missed.is_empty() => {
                let found = self
                    .changes
                    .iter()
                    .find(|(s, l, _)| *s == side && Arc::ptr_eq(l, &last));
                let frames = match found {
                    Some((_, _, frames)) => frames.clone(),
                    None => {
                        let frames = frames(seat, &last, &missed, turn)?;
                        self.changes.push((side, last, frames.clone()));
                        frames
                    }
                };
                Some(Sent {
                    frames,
                    whole: false,
                })
            }
            Sending::Change { last, missed } => Some(Sent {
                frames: frames(seat, &last, &missed, turn)?,
                whole: false,
            }),
            Sending::Whole => {
                let bytes = match self.wholes.iter().find(|(s, _)| *s == side) {
                    Some((_, bytes)) => bytes.clone(),
                    None => {
                        let whole = TurnMessage {
                            events: Vec::new(),
                            ..(*turn.message).clone()
                        };
                        let bytes = encoded(&SeatFrame::Whole(whole))?;
                        self.wholes.push((side, bytes.clone()));
                        bytes
                    }
                };
                Some(Sent {
                    frames: vec![bytes],
                    whole: true,
                })
            }
        }
    }
}

/// The changes of [`SeatTurn::changes`], encoded; `None` if one cannot
/// be.
fn frames(
    seat: SeatId,
    last: &TurnMessage,
    missed: &VecDeque<Event>,
    turn: &SeatTurn,
) -> Option<Vec<Bytes>> {
    turn.changes(last, missed)
        .into_iter()
        .map(|c| match encode_frame(&SeatFrame::Change(c)) {
            Ok(bytes) => Some(Bytes::from(bytes)),
            Err(e) => {
                warn!(?seat, error = %e, "cannot encode a change");
                None
            }
        })
        .collect()
}

/// The events of a turn that a seat not sent it is told when it catches
/// up: those not about an entity.
fn told_later(turn: &SeatTurn) -> impl Iterator<Item = &Event> {
    turn.end_of_turn
        .iter()
        .flat_map(|end| &end.events)
        .chain(&turn.then)
        .filter(|e| !matches!(e, Event::Entity { .. }))
}

impl Game {
    // -- Seats ---------------------------------------------------------------

    /// Seats someone who joins the game: a player when it starts, or a
    /// watcher, then or later, who follows the first player still in it.
    fn seat(&mut self, host: &Host, j: Joining) {
        // A watcher who comes when no player is left to follow is told the
        // end of the first player's game.
        let side = match j.seat {
            SeatId::Player(slot) => self.players.get(slot).copied(),
            SeatId::Watcher(_) => self
                .along(None, 1)
                .or_else(|| self.players.first().copied()),
        };
        let Some(side) = side else {
            return;
        };
        let seat = Seat {
            login: j.login,
            token: SeatToken::from_bits(rand::random()),
            link: Link::Dropped {
                deadline: Instant::now(),
            },
            side,
        };
        info!(room = self.room.0, seat = ?j.seat, login = %seat.login, "seated");
        self.seats.insert(j.seat, seat);
        self.connect(host, j.seat, j.conn, j.outbox);
    }

    /// Connects a seat that is in `seats` to `conn`, and sends it the game
    /// and its whole message as it is now.
    fn connect(&mut self, host: &Host, id: SeatId, conn: ConnId, outbox: Outbox) {
        let paused = matches!(self.clock, Clock::Paused);
        let Some(seat) = self.seats.get_mut(&id) else {
            return;
        };
        let message = Arc::new(host.message(seat.side));
        let you = match id {
            SeatId::Player(slot) => Role::Player(slot),
            SeatId::Watcher(_) => Role::Watcher,
        };
        tell(
            &outbox,
            &SeatFrame::Seated {
                you,
                map: self.map.clone(),
                speed: self.settings.speed,
                paused,
                hold: self.settings.grace,
                token: seat.token.clone(),
            },
        );
        outbox.send_binary(self.start.clone());
        match encode_frame(&SeatFrame::Whole((*message).clone())) {
            Ok(bytes) => outbox.send_binary(Bytes::from(bytes)),
            Err(e) => {
                warn!(room = self.room.0, seat = ?id, error = %e, "cannot encode a whole message")
            }
        }
        let side_over = !message.state.players.contains(&seat.side);
        seat.link = Link::Connected {
            conn,
            outbox,
            last: message,
            pace: Pace::InStep,
        };
        let over = match id {
            SeatId::Player(_) => side_over,
            // A watcher's side is a player still in the game, if one is.
            SeatId::Watcher(_) => side_over && self.followable().is_empty(),
        };
        if over {
            self.end_seat(host, id);
        }
    }

    /// Ends the game for a seat: if connected, it is sent its end, the end
    /// of its side's player. The seat is gone, and the lobby hears of it: a
    /// player goes back to the lobby, and a watcher stops watching.
    fn end_seat(&mut self, host: &Host, id: SeatId) {
        let Some(seat) = self.seats.remove(&id) else {
            return;
        };
        let conn = match seat.link {
            Link::Connected { conn, outbox, .. } => {
                match encode_frame(&SeatFrame::End(host.game_end(seat.side))) {
                    Ok(bytes) => outbox.send_binary(Bytes::from(bytes)),
                    Err(e) => {
                        warn!(room = self.room.0, seat = ?id, error = %e, "cannot encode an end")
                    }
                }
                Some(conn)
            }
            Link::Dropped { .. } => None,
        };
        match (id, conn) {
            (SeatId::Player(_), Some(conn)) => {
                let _ = self.lobby.send(LobbyInput::LeftGame {
                    room: self.room,
                    conn,
                });
            }
            (SeatId::Player(_), None) => {}
            (SeatId::Watcher(watcher), _) => {
                self.steps.remove(&watcher);
                let _ = self.lobby.send(LobbyInput::WatcherLeft {
                    room: self.room,
                    watcher,
                });
            }
        }
        info!(room = self.room.0, seat = ?id, login = %seat.login, "seat left the game");
    }

    /// Takes a seat's player out of the game, as a player who leaves.
    fn quit(&mut self, player: PlayerId) {
        self.commands.push((player, Command::Quit));
    }

    /// The players a watcher may follow, in the room's order: humans
    /// still in the game. A computer's side is not told events, so it has
    /// no stream to follow.
    fn followable(&self) -> Vec<PlayerId> {
        self.slots
            .iter()
            .zip(&self.players)
            .filter(|(s, p)| {
                matches!(s.player, soundrts_protocol::SlotPlayer::Human(_))
                    && self.in_game.contains(p)
            })
            .map(|(_, &p)| p)
            .collect()
    }

    /// The player `steps` along the players a watcher may follow from
    /// `from` (from before the first if `None`): forward if positive, back
    /// if negative, and round again past either end. From a player who
    /// is no longer among them, one step forward is the next of them in
    /// the room's order. `None` if there is none to follow.
    fn along(&self, from: Option<PlayerId>, steps: i64) -> Option<PlayerId> {
        let order = self.followable();
        let len = i64::try_from(order.len()).ok().filter(|&n| n > 0)?;
        let slot = |p: PlayerId| self.players.iter().position(|&q| q == p);
        let from_slot = from.and_then(slot);
        // The index that counts as 0 steps, and the steps from it.
        let (base, steps) = match from.and_then(|f| order.iter().position(|&p| p == f)) {
            Some(i) => (i64::try_from(i).ok()?, steps),
            None => {
                // Between two of them: a step forward reaches the next.
                let next = order.iter().position(|&p| slot(p) > from_slot).unwrap_or(0);
                let next = i64::try_from(next).ok()?;
                if steps > 0 {
                    (next, steps.checked_sub(1)?)
                } else {
                    (next, steps)
                }
            }
        };
        let i = base.checked_add(steps)?.checked_rem_euclid(len)?;
        order.get(usize::try_from(i).ok()?).copied()
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.seats
            .values()
            .filter_map(|s| match s.link {
                Link::Dropped { deadline } => Some(deadline),
                Link::Connected { .. } => None,
            })
            .chain(self.switch_at)
            .min()
    }

    fn give_up_expired(&mut self, now: Instant) {
        let expired: Vec<SeatId> = self
            .seats
            .iter()
            .filter(|(_, s)| matches!(s.link, Link::Dropped { deadline } if deadline <= now))
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            let Some(seat) = self.seats.remove(&id) else {
                continue;
            };
            info!(room = self.room.0, seat = ?id, login = %seat.login, "seat given up");
            match id {
                SeatId::Player(slot) => {
                    self.quit(seat.side);
                    self.tell_all(&SeatFrame::Seat {
                        slot,
                        change: SeatChange::GivenUp,
                    });
                }
                SeatId::Watcher(watcher) => {
                    self.steps.remove(&watcher);
                    let _ = self.lobby.send(LobbyInput::WatcherLeft {
                        room: self.room,
                        watcher,
                    });
                }
            }
        }
    }

    // -- Following -----------------------------------------------------------

    /// A watcher asks to follow the next player, or the previous one: done
    /// at the next turn, or a turn from now while the game is paused.
    fn follow(&mut self, watcher: WatcherId, step: Step) {
        let connected = self
            .seats
            .get(&SeatId::Watcher(watcher))
            .is_some_and(|s| matches!(s.link, Link::Connected { .. }));
        if !connected {
            return;
        }
        let steps = self.steps.entry(watcher).or_default();
        let one = match step {
            Step::Next => 1,
            Step::Previous => -1,
        };
        *steps = steps.saturating_add(one).clamp(-MAX_STEPS, MAX_STEPS);
        if matches!(self.clock, Clock::Paused) && self.switch_at.is_none() {
            self.switch_at = Some(after(self.settings.speed.turn_interval()));
        }
    }

    /// The side a watcher follows from the coming turn: `steps` along from
    /// its own.
    fn stepped(&self, side: PlayerId, steps: i64) -> PlayerId {
        self.along(Some(side), steps).unwrap_or(side)
    }

    /// While the game is paused, does the steps asked for once their time
    /// has come: each watcher is sent its new side's whole message now.
    fn switch_if_due(&mut self, host: &Host, now: Instant) {
        if self.switch_at.is_none_or(|at| at > now) {
            return;
        }
        self.switch_at = None;
        for (watcher, steps) in std::mem::take(&mut self.steps) {
            let id = SeatId::Watcher(watcher);
            let Some(side) = self.seats.get(&id).map(|s| s.side) else {
                continue;
            };
            let side = self.stepped(side, steps);
            self.refollow(host, id, side);
        }
    }

    /// A watcher follows `side` from now on: if connected, it is sent
    /// that player's whole message as it is now.
    fn refollow(&mut self, host: &Host, id: SeatId, side: PlayerId) {
        let Some(seat) = self.seats.get_mut(&id) else {
            return;
        };
        seat.side = side;
        let Link::Connected {
            outbox, last, pace, ..
        } = &mut seat.link
        else {
            return;
        };
        let message = Arc::new(host.message(side));
        match encode_frame(&SeatFrame::Whole((*message).clone())) {
            Ok(bytes) => outbox.send_binary(Bytes::from(bytes)),
            Err(e) => {
                warn!(room = self.room.0, seat = ?id, error = %e, "cannot encode a whole message");
                return;
            }
        }
        *last = message;
        *pace = Pace::InStep;
    }

    // -- Turns ---------------------------------------------------------------

    /// What each connected seat is sent of the coming turn: nothing if its
    /// connection is not keeping up. A watcher that asked to follow
    /// another player and keeps up is sent that player's whole message.
    fn plans(&mut self) -> Vec<Plan> {
        let mut plans = Vec::new();
        let ids: Vec<SeatId> = self.seats.keys().copied().collect();
        for id in ids {
            let steps = match id {
                SeatId::Watcher(w) => self.steps.get(&w).copied(),
                SeatId::Player(_) => None,
            };
            let Some(seat) = self.seats.get(&id) else {
                continue;
            };
            let Link::Connected { outbox, .. } = &seat.link else {
                continue;
            };
            let keeps_up = outbox.queued() < SEAT_BACKLOG;
            let new_side = match (steps, keeps_up) {
                (Some(steps), true) => Some(self.stepped(seat.side, steps)),
                _ => None,
            };
            if let (Some(_), SeatId::Watcher(w)) = (new_side, id) {
                self.steps.remove(&w);
            }
            let Some(seat) = self.seats.get_mut(&id) else {
                continue;
            };
            let Link::Connected { last, pace, .. } = &mut seat.link else {
                continue;
            };
            let send = match new_side {
                Some(side) => {
                    seat.side = side;
                    Some(Sending::Whole)
                }
                None => keeps_up.then(|| {
                    let missed = match pace {
                        Pace::InStep => VecDeque::new(),
                        Pace::Behind { missed } => std::mem::take(missed),
                    };
                    Sending::Change {
                        last: Arc::clone(last),
                        missed,
                    }
                }),
            };
            plans.push(Plan {
                seat: id,
                side: seat.side,
                send,
            });
        }
        plans
    }

    /// Sends each seat its frames of the turn, keeps the events of a seat
    /// that was not sent the turn, and ends the game of seats whose player
    /// is no longer in it. A player whose game is over is sent the turn,
    /// and any it missed, whatever its connection: its end comes after. A
    /// watcher whose player is no longer in the game follows the next one
    /// still in it, or, if none is, has its end, that player's.
    fn deliver(&mut self, host: &Host, deliveries: Vec<Delivery>) {
        let in_game = deliveries
            .first()
            .map(|d| d.turn.message.state.players.clone());
        if let Some(d) = deliveries.first() {
            self.allies.clone_from(&d.turn.message.state.allies);
        }
        for d in deliveries {
            let Some(Seat {
                link: Link::Connected {
                    outbox, last, pace, ..
                },
                ..
            }) = self.seats.get_mut(&d.seat)
            else {
                continue;
            };
            let side_left = !d.turn.message.state.players.contains(&d.side);
            let over = matches!(d.seat, SeatId::Player(_)) && side_left;
            // A watcher's new side left the game in the turn: it is not
            // sent that side, and follows the next below.
            let planned = d.frames.filter(|sent| !(sent.whole && side_left));
            let sent = planned.or_else(|| {
                over.then(|| {
                    let missed = match pace {
                        Pace::Behind { missed } => std::mem::take(missed),
                        Pace::InStep => VecDeque::new(),
                    };
                    frames(d.seat, last, &missed, &d.turn).map(|frames| Sent {
                        frames,
                        whole: false,
                    })
                })
                .flatten()
            });
            match sent {
                Some(sent) => {
                    for frame in sent.frames {
                        outbox.send_binary(frame);
                    }
                    if sent.whole {
                        debug!(room = self.room.0, seat = ?d.seat, side = d.side.0, "follows");
                    }
                    *last = Arc::clone(&d.turn.message);
                    *pace = Pace::InStep;
                }
                None => {
                    if let Pace::InStep = pace {
                        *pace = Pace::Behind {
                            missed: VecDeque::new(),
                        };
                    }
                    if let Pace::Behind { missed } = pace {
                        missed.extend(told_later(&d.turn).cloned());
                        while missed.len() > MAX_MISSED_EVENTS {
                            missed.pop_front();
                        }
                    }
                }
            }
        }
        if let Some(in_game) = in_game {
            self.in_game = in_game;
            let over: Vec<(SeatId, PlayerId)> = self
                .seats
                .iter()
                .filter(|(_, s)| !self.in_game.contains(&s.side))
                .map(|(&id, s)| (id, s.side))
                .collect();
            for (id, side) in over {
                match (id, self.along(Some(side), 1)) {
                    (SeatId::Watcher(_), Some(next)) => self.refollow(host, id, next),
                    _ => self.end_seat(host, id),
                }
            }
        }
    }

    // -- Inputs --------------------------------------------------------------

    fn on_input(&mut self, host: &Host, input: GameInput) {
        match input {
            GameInput::Command {
                slot,
                command,
                bytes,
            } => self.command(host, slot, command, bytes),
            GameInput::Leave { seat } => {
                let Some(side) = self.seats.get(&seat).map(|s| s.side) else {
                    return;
                };
                if let SeatId::Player(slot) = seat {
                    self.quit(side);
                    self.end_seat(host, seat);
                    self.tell_all(&SeatFrame::Seat {
                        slot,
                        change: SeatChange::Left,
                    });
                } else {
                    self.end_seat(host, seat);
                }
            }
            GameInput::Dropped { seat } => self.dropped(seat),
            GameInput::Return {
                conn,
                token,
                outbox,
            } => self.take_back(host, conn, &token, outbox),
            GameInput::Watch(j) => self.seat(host, j),
            GameInput::Follow { watcher, step } => self.follow(watcher, step),
            GameInput::Pause { by } => self.clock_change(by, ClockChange::Paused),
            GameInput::Resume { by } => self.clock_change(by, ClockChange::Resumed),
            GameInput::Speed { by, speed } => {
                self.clock_change(by, ClockChange::Speed { speed });
            }
            GameInput::Remove { by, slot } => self.remove(host, by, slot),
            GameInput::Flare { slot, square } => self.flare(host, slot, square),
        }
    }

    fn command(&mut self, host: &Host, slot: SlotIndex, command: Command, bytes: usize) {
        let Some(seat) = self.seats.get(&SeatId::Player(slot)) else {
            return;
        };
        let Link::Connected { outbox, .. } = &seat.link else {
            return;
        };
        // No cheat in a game with others, and the speed is the room
        // host's to set, for everyone.
        if matches!(command, Command::Cheat(_) | Command::Speed(_)) {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NotNow,
            });
            return;
        }
        // What names a square or player the game does not have comes from
        // a broken or hostile client: the sender hears it now, as the world
        // would refuse it at the turn without telling anyone.
        if host.check(seat.side, &command).is_err() {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::Invalid,
            });
            return;
        }
        let given = self.command_bytes.entry(slot).or_default();
        let total = given.saturating_add(bytes);
        if total > MAX_TURN_ORDER_BYTES {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::SlowDown,
            });
            return;
        }
        *given = total;
        self.commands.push((seat.side, command));
    }

    /// A seat's connection is lost: the seat is held. Everyone hears of a
    /// player's; a watcher's is said only if it does not come back
    /// ([`LobbyInput::WatcherLeft`]).
    fn dropped(&mut self, id: SeatId) {
        let Some(seat) = self.seats.get_mut(&id) else {
            return;
        };
        if let Link::Dropped { .. } = seat.link {
            return;
        }
        info!(room = self.room.0, seat = ?id, login = %seat.login, "seat dropped");
        seat.link = Link::Dropped {
            deadline: after(self.settings.grace),
        };
        if let SeatId::Player(slot) = id {
            self.tell_all(&SeatFrame::Seat {
                slot,
                change: SeatChange::Dropped,
            });
        }
    }

    /// Gives a dropped seat back to the player or watcher who holds its
    /// token, or refuses. The lobby hears of it before the seat is sent
    /// anything, so that whatever is sent after it finds it in the game.
    fn take_back(&mut self, host: &Host, conn: ConnId, token: &SeatToken, outbox: Outbox) {
        let found = self.seats.iter().find_map(|(&id, s)| {
            let held = matches!(s.link, Link::Dropped { deadline } if deadline > Instant::now());
            (held && s.token.matches(token)).then_some(id)
        });
        let Some(id) = found else {
            let _ = self.lobby.send(LobbyInput::ReturnRefused {
                room: self.room,
                conn,
            });
            return;
        };
        let _ = self.lobby.send(LobbyInput::Returned {
            room: self.room,
            conn,
            seat: id,
        });
        if let SeatId::Player(slot) = id {
            self.tell_all(&SeatFrame::Seat {
                slot,
                change: SeatChange::Returned,
            });
        }
        if let Some(seat) = self.seats.get(&id) {
            info!(room = self.room.0, seat = ?id, login = %seat.login, "seat returned");
        }
        self.connect(host, id, conn, outbox);
    }

    fn clock_change(&mut self, by: SlotIndex, change: ClockChange) {
        let Some((login, outbox)) = self.hosting(by) else {
            return;
        };
        let changed = match (change, &self.clock) {
            (ClockChange::Paused, Clock::Running(_)) => {
                self.clock = Clock::Paused;
                true
            }
            (ClockChange::Resumed, Clock::Paused) => {
                self.clock = Clock::running(self.settings.speed);
                true
            }
            (ClockChange::Speed { speed }, _) => {
                self.settings.speed = speed;
                if let Clock::Running(_) = self.clock {
                    self.clock = Clock::running(speed);
                }
                true
            }
            (ClockChange::Paused | ClockChange::Resumed, _) => false,
        };
        if !changed {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NotNow,
            });
            return;
        }
        info!(room = self.room.0, by = %login, ?change, "clock changed");
        self.tell_all(&SeatFrame::Clock { by: login, change });
    }

    /// The room's host removes a player: a human seat, or a computer.
    fn remove(&mut self, host: &Host, by: SlotIndex, slot: SlotIndex) {
        let Some((login, outbox)) = self.hosting(by) else {
            return;
        };
        let Some(&player) = self.players.get(slot) else {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NoSuchPlayer,
            });
            return;
        };
        let is_seat = self.seats.contains_key(&SeatId::Player(slot));
        let is_computer = self
            .slots
            .get(slot)
            .is_some_and(|s| matches!(s.player, soundrts_protocol::SlotPlayer::Computer(_)));
        let playing = self.in_game.contains(&player);
        if slot == by || !playing || !(is_seat || is_computer) {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NoSuchPlayer,
            });
            return;
        }
        info!(room = self.room.0, by = %login, slot, "player removed");
        self.tell_all(&SeatFrame::Seat {
            slot,
            change: SeatChange::Removed { by: login },
        });
        self.quit(player);
        self.end_seat(host, SeatId::Player(slot));
    }

    /// A player still in the game points their allies at a square of the
    /// map: each seat whose side is one of them, the player's own seat
    /// included, is told, and so are those who watch them. Refused for a
    /// square the game's map does not have, and beyond [`MAX_FLARES`] in
    /// [`FLARE_WINDOW`].
    fn flare(&mut self, host: &Host, slot: SlotIndex, square: SquareId) {
        let Some((_, outbox)) = self.connected(slot) else {
            return;
        };
        let Some(from) = self.players.get(slot).copied() else {
            return;
        };
        let refusal = if !self.in_game.contains(&from) {
            Some(Refusal::NotNow)
        } else if host.game_start().map.square(square).is_none() {
            Some(Refusal::Invalid)
        } else if !self
            .flares
            .entry(slot)
            .or_insert_with(|| Window::new(MAX_FLARES, FLARE_WINDOW))
            .allow(Instant::now())
        {
            Some(Refusal::SlowDown)
        } else {
            None
        };
        if let Some(reason) = refusal {
            outbox.send(&ServerMsg::Refused { reason });
            return;
        }
        let allies = usize::try_from(from.0)
            .ok()
            .and_then(|i| self.allies.get(i))
            .map_or(&[][..], Vec::as_slice);
        let Some(bytes) = encoded(&SeatFrame::Flare { from, square }) else {
            return;
        };
        for seat in self.seats.values() {
            if let Link::Connected { outbox, .. } = &seat.link
                && allies.contains(&seat.side)
            {
                outbox.send_binary(bytes.clone());
            }
        }
    }

    // -- The host's powers ---------------------------------------------------

    /// The login and outbox of the player of `slot` if they hold the
    /// host's powers and are connected. Anyone else who asks is refused.
    fn hosting(&self, slot: SlotIndex) -> Option<(Login, Outbox)> {
        let (login, outbox) = self.connected(slot)?;
        if self.host != Some(slot) {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NotNow,
            });
            return None;
        }
        Some((login, outbox))
    }

    /// Keeps the host's powers with a player still in the game. When the
    /// seat of the player who holds them is gone (they left, lost, were
    /// given up), the powers pass to the first player still in the game in
    /// the room's order, as a room passes its host on, never to a watcher.
    /// Everyone in the game hears who, after what happened to the seat; the
    /// lobby makes that player the room's host.
    /// A player away whose seat is held keeps them, and a paused game stays
    /// paused for the next holder. When no player is left to take them, a
    /// paused game runs again: its watchers then have their end at the next
    /// turn, as when the last player leaves a running game.
    fn hand_on_host(&mut self) {
        let Some(slot) = self.host else {
            return;
        };
        if self.seats.contains_key(&SeatId::Player(slot)) {
            return;
        }
        self.host = self.seats.keys().find_map(|seat| match seat {
            SeatId::Player(slot) => Some(*slot),
            SeatId::Watcher(_) => None,
        });
        match self.host {
            Some(slot) => {
                info!(room = self.room.0, slot, "host's powers handed on");
                self.tell_all(&SeatFrame::Host { slot });
                let _ = self.lobby.send(LobbyInput::HostChanged {
                    room: self.room,
                    slot,
                });
            }
            None => {
                info!(room = self.room.0, "no player left to host");
                if let Clock::Paused = self.clock {
                    self.clock = Clock::running(self.settings.speed);
                }
            }
        }
    }

    // -- Helpers -------------------------------------------------------------

    /// The login and outbox of a player's connected seat.
    fn connected(&self, slot: SlotIndex) -> Option<(Login, Outbox)> {
        let seat = self.seats.get(&SeatId::Player(slot))?;
        match &seat.link {
            Link::Connected { outbox, .. } => Some((seat.login.clone(), outbox.clone())),
            Link::Dropped { .. } => None,
        }
    }

    /// Tells every connected seat, players and watchers.
    fn tell_all(&self, frame: &SeatFrame) {
        let Some(bytes) = encoded(frame) else {
            return;
        };
        for seat in self.seats.values() {
            if let Link::Connected { outbox, .. } = &seat.link {
                outbox.send_binary(bytes.clone());
            }
        }
    }
}

/// Faults the tests cause on purpose, to prove that a failing or slow game
/// ends only itself. Only the `fault-injection` feature has them; a game
/// whose rules start with a fault's line has it, while it is built or at
/// every turn.
#[cfg(feature = "fault-injection")]
pub mod fault {
    use std::time::Duration;

    /// The world's turn panics.
    pub const PANIC_IN_TURN: &str = "; fault: panic in the turn\n";
    /// The game's own task panics.
    pub const PANIC_IN_TASK: &str = "; fault: panic in the task\n";
    /// Building the game takes [`SLOW`].
    pub const SLOW_TO_BUILD: &str = "; fault: slow to build\n";
    /// Each turn takes [`SLOW`].
    pub const SLOW_TURN: &str = "; fault: slow turn\n";
    /// How long a slow fault takes.
    pub const SLOW: Duration = Duration::from_millis(500);

    const FAULTS: [&str; 4] = [PANIC_IN_TURN, PANIC_IN_TASK, SLOW_TO_BUILD, SLOW_TURN];

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct Fault(&'static str);

    pub(super) fn of(rules: &str) -> Option<Fault> {
        FAULTS.into_iter().find(|f| rules.starts_with(f)).map(Fault)
    }

    /// On the blocking thread that builds the game.
    pub(super) fn while_building(fault: Option<Fault>) {
        if fault == Some(Fault(SLOW_TO_BUILD)) {
            std::thread::sleep(SLOW);
        }
    }

    /// In the game's task, before a turn.
    pub(super) fn before_turn(fault: Option<Fault>) {
        if fault == Some(Fault(PANIC_IN_TASK)) {
            panic!("fault injected in the game's task");
        }
    }

    /// On the blocking thread that plays a turn.
    pub(super) fn in_turn(fault: Option<Fault>) {
        match fault {
            Some(Fault(PANIC_IN_TURN)) => panic!("fault injected in the turn"),
            Some(Fault(SLOW_TURN)) => std::thread::sleep(SLOW),
            _ => {}
        }
    }
}

// The tests may panic: that is how they fail.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use soundrts_world::Controller;

    fn read(relative: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/game-data")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"))
    }

    /// Andrew and Bob on jl2, with an easy computer.
    fn host() -> Host {
        let setup = GameSetup::multiplayer(
            read("res/rules.txt"),
            read("res/ai.txt"),
            read("res/multi/jl2.txt"),
            7,
            vec![
                Controller::human("andrew", true),
                Controller::human("bob", true),
                Controller::computer("easy"),
            ],
        );
        Host::new(&setup).unwrap()
    }

    /// Andrew's seat, Bob's, and `watchers` watchers following Andrew,
    /// each in step from the last message it was sent.
    fn plans(seats: &[(SeatId, PlayerId, Arc<TurnMessage>)]) -> Vec<Plan> {
        seats
            .iter()
            .map(|(seat, side, last)| Plan {
                seat: *seat,
                side: *side,
                send: Some(Sending::Change {
                    last: Arc::clone(last),
                    missed: VecDeque::new(),
                }),
            })
            .collect()
    }

    fn seats(host: &Host, watchers: u32) -> Vec<(SeatId, PlayerId, Arc<TurnMessage>)> {
        let (a, b) = (host.seats()[0], host.seats()[1]);
        let (last_a, last_b) = (Arc::new(host.message(a)), Arc::new(host.message(b)));
        let mut seats = vec![
            (SeatId::Player(0), a, Arc::clone(&last_a)),
            (SeatId::Player(1), b, last_b),
        ];
        for w in 0..watchers {
            seats.push((SeatId::Watcher(WatcherId(w)), a, Arc::clone(&last_a)));
        }
        seats
    }

    /// Plays `turns` turns, as the game's task does, and gives what each
    /// turn delivered.
    fn play(
        host: &mut Host,
        seats: &mut [(SeatId, PlayerId, Arc<TurnMessage>)],
        turns: usize,
    ) -> Vec<Vec<Delivery>> {
        (0..turns)
            .map(|_| {
                let deliveries = play_turn(host, Vec::new(), plans(seats));
                for (seat, d) in seats.iter_mut().zip(&deliveries) {
                    seat.2 = Arc::clone(&d.turn.message);
                }
                deliveries
            })
            .collect()
    }

    fn bytes(d: &Delivery) -> Vec<*const u8> {
        d.frames
            .as_ref()
            .unwrap()
            .frames
            .iter()
            .map(|b| b.as_ptr())
            .collect()
    }

    #[test]
    fn watchers_in_step_are_sent_the_very_bytes_of_their_player() {
        let mut host = host();
        let mut seats = seats(&host, 8);
        for turn in play(&mut host, &mut seats, 20) {
            let (andrew, bob) = (&turn[0], &turn[1]);
            assert!(!Arc::ptr_eq(&andrew.turn, &bob.turn));
            for w in &turn[2..] {
                // One turn of the world for Andrew's side, and one
                // encoding, whoever is sent it.
                assert!(Arc::ptr_eq(&w.turn, &andrew.turn));
                assert_eq!(bytes(w), bytes(andrew));
            }
        }
    }

    #[test]
    fn a_watcher_that_follows_another_player_is_sent_its_whole_message() {
        let mut host = host();
        let mut seats = seats(&host, 2);
        play(&mut host, &mut seats, 3);
        let bob = host.seats()[1];
        let mut plans = plans(&seats);
        for p in &mut plans[2..] {
            p.side = bob;
            p.send = Some(Sending::Whole);
        }
        let turn = play_turn(&mut host, Vec::new(), plans);
        let (w1, w2) = (&turn[2], &turn[3]);
        assert!(Arc::ptr_eq(&w1.turn, &turn[1].turn), "Bob's turn");
        assert!(w1.frames.as_ref().unwrap().whole);
        assert_eq!(bytes(w1), bytes(w2), "encoded once");
        let Ok(SeatFrame::Whole(m)) =
            soundrts_protocol::game::decode_frame(&w1.frames.as_ref().unwrap().frames[0])
        else {
            panic!("a whole message");
        };
        assert_eq!(m.seat, bob);
        assert!(m.events.is_empty());
        assert_eq!(m.time, turn[1].turn.message.time);
    }

    /// The time of a turn for the players, with and without eight
    /// watchers following Andrew: what eight watchers add beyond the
    /// sending. A measurement, not a check: run it with
    /// `cargo test -p soundrts-server --release --lib -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement"]
    fn measure_eight_watchers() {
        let turns = 600;
        for watchers in [0, 8, 0, 8] {
            let mut host = host();
            let mut seats = seats(&host, watchers);
            let start = std::time::Instant::now();
            play(&mut host, &mut seats, turns);
            let took = start.elapsed();
            eprintln!(
                "{watchers} watchers: {turns} turns in {took:?}, {:?} a turn",
                took / turns as u32
            );
        }
    }

    #[test]
    fn only_the_newest_records_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let name = |i: usize| format!("game-{i:012}-room1.{RECORD_EXTENSION}");
        for i in 0..RECORDS_KEPT + 3 {
            std::fs::write(dir.path().join(name(i)), b"").unwrap();
        }
        std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
        remove_oldest_records(dir.path()).unwrap();
        for i in 0..3 {
            assert!(!dir.path().join(name(i)).exists(), "{i}");
        }
        for i in 3..RECORDS_KEPT + 3 {
            assert!(dir.path().join(name(i)).exists(), "{i}");
        }
        assert!(dir.path().join("notes.txt").exists());
    }
}
