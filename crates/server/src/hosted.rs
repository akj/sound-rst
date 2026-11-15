//! One task per hosted game: the server runs the game itself and sends
//! each seat only what its player knows.
//!
//! The task builds the game from the set-up the lobby made, owns the
//! [`Host`], steps it on a clock that waits for nobody, and sends each
//! human seat its stream: the start and the whole message when the seat
//! joins or returns, then the change since the last message it was sent,
//! each turn. The world's work (building, and each turn with every seat's
//! message) runs on a blocking thread with a time budget; a panic there
//! ends this game and nothing else.
//!
//! Where watchers would attach: a watcher is a [`Seat`] with no player of
//! its own and a message that follows one; [`Link`], [`Pace`] and
//! [`Game::deliver`] would serve it as they serve a player, and returning
//! is how a watcher would join late. Where the record would attach: the
//! clock's branch of `run` takes every command of a turn, in the order the
//! host runs them, with the turn's number.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use soundrts_protocol::game::{SeatFrame, encode_frame};
use soundrts_protocol::{
    ChatText, ClockChange, Login, MAX_TURN_ORDER_BYTES, MapInfo, Refusal, RoomId, SeatChange,
    SeatToken, ServerMsg, Slot, SlotIndex, Speed, StartRefusal, StopReason,
};
use soundrts_world::{
    Command, Event, GameSetup, Host, Order, OrderTarget, PlayerId, SeatTurn, TurnMessage,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Bytes;
use tracing::{debug, info, warn};

use crate::connection::{ConnId, Outbox, Queued};
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

/// How a hosted game runs: from the server's settings and the room.
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
    /// The server's count of work past its budget.
    pub overruns: Overruns,
}

/// While this many builds or turns of hosted games run on past their
/// budget, the server starts no hosted game.
pub(crate) const MAX_OVERRUNNING: usize = 2;

/// The world's work, of any hosted game on the server, that ran past its
/// budget and still runs. Nothing can stop it, so it is counted until it
/// ends, and while there is too much of it no hosted game starts: a
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

/// A human seat as the lobby hands it over when the game starts.
pub(crate) struct Joining {
    pub slot: SlotIndex,
    pub conn: ConnId,
    pub login: Login,
    pub outbox: Outbox,
}

/// What the lobby passes to a hosted game. The lobby has checked who may
/// send it: a seat's own player for a command, and the room's host for
/// the clock and removals.
pub(crate) enum HostedInput {
    Command {
        slot: SlotIndex,
        command: Command,
        bytes: usize,
    },
    /// The player asked to leave.
    Leave {
        slot: SlotIndex,
    },
    /// The player's connection is gone.
    Dropped {
        slot: SlotIndex,
    },
    /// A player in the lobby asks for a dropped seat back.
    Return {
        conn: ConnId,
        token: SeatToken,
        outbox: Outbox,
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
}

/// Where the lobby passes inputs to a hosted game. An input from a client
/// comes with its bytes, counted against the client's allowance until the
/// game has handled it.
pub(crate) type HostedTx = mpsc::UnboundedSender<(HostedInput, Option<Queued>)>;

/// How a hosted game's task ended, for the lobby.
#[derive(Debug)]
pub(crate) enum Ending {
    /// The game could not be built: the room goes back to gathering.
    NotBuilt(StartRefusal),
    /// No seat is left: each was told its end and reported to the lobby.
    Over,
    /// The server stopped the game for everyone in it.
    Stopped(StopReason),
}

/// Starts a hosted game's task. The game reports to the lobby each seat
/// whose game is over ([`LobbyInput::LeftGame`]), returns accepted or
/// refused, and how it ended ([`LobbyInput::HostedEnded`]), even if it
/// panics.
pub(crate) fn spawn(
    room: RoomId,
    setup: GameSetup,
    slots: Vec<Slot>,
    map: MapInfo,
    seats: Vec<Joining>,
    settings: Settings,
    lobby: LobbyTx,
) -> HostedTx {
    let (tx, rx) = mpsc::unbounded_channel();
    let reporter = lobby.clone();
    let task = tokio::spawn(run(room, setup, slots, map, seats, settings, lobby, rx));
    tokio::spawn(async move {
        let ending = task.await.unwrap_or_else(|e| {
            warn!(room = room.0, error = %e, "hosted game failed");
            Ending::Stopped(StopReason::Crashed)
        });
        info!(room = room.0, ?ending, "hosted game ended");
        let _ = reporter.send(LobbyInput::HostedEnded { room, ending });
    });
    tx
}

/// A human seat still in the game.
struct Seat {
    player: PlayerId,
    login: Login,
    token: SeatToken,
    link: Link,
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
    seats: BTreeMap<SlotIndex, Seat>,
    /// The commands for the next turn, in the order they came.
    commands: Vec<(PlayerId, Command)>,
    /// The bytes of commands each seat has given for the next turn.
    command_bytes: BTreeMap<SlotIndex, usize>,
    clock: Clock,
    turn: u64,
    /// The players still in the game, as of the last turn a seat was sent.
    in_game: Vec<PlayerId>,
    /// How many squares the map has.
    squares: usize,
}

/// A connected seat's share of a turn: whether it is sent the turn, and
/// from which message.
struct Plan {
    slot: SlotIndex,
    player: PlayerId,
    send: Option<(Arc<TurnMessage>, VecDeque<Event>)>,
}

/// What a turn gave a connected seat, and the frames that send it, if it
/// is sent the turn.
struct Delivery {
    slot: SlotIndex,
    turn: SeatTurn,
    frames: Option<Vec<Bytes>>,
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
    mut inputs: mpsc::UnboundedReceiver<(HostedInput, Option<Queued>)>,
) -> Ending {
    info!(room = room.0, "hosted game building");
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
    let start = match encode_frame(&SeatFrame::Start(host.game_start().message())) {
        Ok(bytes) => Bytes::from(bytes),
        Err(e) => {
            warn!(room = room.0, error = %e, "cannot encode the start");
            return Ending::NotBuilt(StartRefusal::Crashed);
        }
    };
    let players = host.seats().to_vec();
    let mut game = Game {
        room,
        clock: Clock::running(settings.speed),
        settings,
        lobby,
        slots,
        map,
        start,
        seats: BTreeMap::new(),
        players,
        commands: Vec::new(),
        command_bytes: BTreeMap::new(),
        turn: 0,
        in_game: host.seats().to_vec(),
        squares: host.game_start().map.squares.len(),
    };
    for j in joining {
        let Some(&player) = game.players.get(j.slot) else {
            continue;
        };
        let seat = Seat {
            player,
            login: j.login,
            token: SeatToken::from_bits(rand::random()),
            link: Link::Dropped {
                deadline: Instant::now(),
            },
        };
        game.seats.insert(j.slot, seat);
        game.connect(&host, j.slot, j.conn, j.outbox);
    }
    info!(
        room = room.0,
        seats = game.seats.len(),
        "hosted game started"
    );

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
                let work = move || {
                    #[cfg(feature = "fault-injection")]
                    fault::in_turn(fault);
                    let deliveries = play_turn(&mut host, commands, plans);
                    (host, deliveries)
                };
                match within(budget, &game.settings.overruns, work).await {
                    Ok((h, deliveries)) => {
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
            _ = sleep_until(deadline) => game.give_up_expired(Instant::now()),
            input = inputs.recv() => match input {
                Some((input, _queued)) => game.on_input(&host, input),
                None => break,
            },
        }
    }
    info!(room = room.0, turns = game.turn, "hosted game over");
    Ending::Over
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
    let players: Vec<PlayerId> = plans.iter().map(|p| p.player).collect();
    let turns = host.step_seats(&players);
    plans
        .into_iter()
        .zip(turns)
        .map(|(plan, turn)| Delivery {
            slot: plan.slot,
            frames: plan
                .send
                .and_then(|(last, missed)| frames(plan.slot, &last, &missed, &turn)),
            turn,
        })
        .collect()
}

/// The changes of [`SeatTurn::changes`], encoded; `None` if one cannot
/// be.
fn frames(
    slot: SlotIndex,
    last: &TurnMessage,
    missed: &VecDeque<Event>,
    turn: &SeatTurn,
) -> Option<Vec<Bytes>> {
    turn.changes(last, missed)
        .into_iter()
        .map(|c| match encode_frame(&SeatFrame::Change(c)) {
            Ok(bytes) => Some(Bytes::from(bytes)),
            Err(e) => {
                warn!(slot, error = %e, "cannot encode a change");
                None
            }
        })
        .collect()
}

/// Whether the square a command's order names, if any, is one of the
/// map's `squares`: the world takes it as one of its own.
fn on_the_map(command: &Command, squares: usize) -> bool {
    let target = match command {
        Command::Order {
            order: Order::Keyword(order),
            ..
        } => order.target(),
        Command::Order {
            order: Order::Default(target),
            ..
        } => Some(target),
        _ => None,
    };
    match target {
        Some(OrderTarget::Square(q) | OrderTarget::Point { square: q, .. }) => {
            usize::try_from(q.0).is_ok_and(|q| q < squares)
        }
        Some(OrderTarget::Entity(_)) | None => true,
    }
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

    /// Connects a seat that is in `seats` to `conn`, and sends it the game
    /// and its whole message as it is now.
    fn connect(&mut self, host: &Host, slot: SlotIndex, conn: ConnId, outbox: Outbox) {
        let paused = matches!(self.clock, Clock::Paused);
        let Some(seat) = self.seats.get_mut(&slot) else {
            return;
        };
        let message = Arc::new(host.message(seat.player));
        tell(
            &outbox,
            &SeatFrame::Seated {
                you: slot,
                map: self.map.clone(),
                speed: self.settings.speed,
                paused,
                token: seat.token.clone(),
            },
        );
        outbox.send_binary(self.start.clone());
        match encode_frame(&SeatFrame::Whole((*message).clone())) {
            Ok(bytes) => outbox.send_binary(Bytes::from(bytes)),
            Err(e) => warn!(room = self.room.0, slot, error = %e, "cannot encode a whole message"),
        }
        let over = !message.state.players.contains(&seat.player);
        seat.link = Link::Connected {
            conn,
            outbox,
            last: message,
            pace: Pace::InStep,
        };
        if over {
            self.end_seat(host, slot);
        }
    }

    /// Ends the game for a seat: if connected, it is sent its end, and its
    /// player goes back to the lobby. The seat is gone.
    fn end_seat(&mut self, host: &Host, slot: SlotIndex) {
        let Some(seat) = self.seats.remove(&slot) else {
            return;
        };
        if let Link::Connected { conn, outbox, .. } = seat.link {
            match encode_frame(&SeatFrame::End(host.game_end(seat.player))) {
                Ok(bytes) => outbox.send_binary(Bytes::from(bytes)),
                Err(e) => warn!(room = self.room.0, slot, error = %e, "cannot encode an end"),
            }
            let _ = self.lobby.send(LobbyInput::LeftGame {
                room: self.room,
                conn,
            });
        }
        info!(room = self.room.0, slot, login = %seat.login, "seat left the game");
    }

    /// Takes a seat's player out of the game, as a player who leaves.
    fn quit(&mut self, player: PlayerId) {
        self.commands.push((player, Command::Quit));
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.seats
            .values()
            .filter_map(|s| match s.link {
                Link::Dropped { deadline } => Some(deadline),
                Link::Connected { .. } => None,
            })
            .min()
    }

    fn give_up_expired(&mut self, now: Instant) {
        let expired: Vec<SlotIndex> = self
            .seats
            .iter()
            .filter(|(_, s)| matches!(s.link, Link::Dropped { deadline } if deadline <= now))
            .map(|(&slot, _)| slot)
            .collect();
        for slot in expired {
            if let Some(seat) = self.seats.remove(&slot) {
                info!(room = self.room.0, slot, login = %seat.login, "seat given up");
                self.quit(seat.player);
                self.tell_all(&SeatFrame::Seat {
                    slot,
                    change: SeatChange::GivenUp,
                });
            }
        }
    }

    // -- Turns ---------------------------------------------------------------

    /// What each connected seat is sent of the coming turn: nothing if its
    /// connection is not keeping up.
    fn plans(&mut self) -> Vec<Plan> {
        self.seats
            .iter_mut()
            .filter_map(|(&slot, seat)| match &mut seat.link {
                Link::Connected {
                    outbox, last, pace, ..
                } => {
                    let send = (outbox.queued() < SEAT_BACKLOG).then(|| {
                        let missed = match pace {
                            Pace::InStep => VecDeque::new(),
                            Pace::Behind { missed } => std::mem::take(missed),
                        };
                        (Arc::clone(last), missed)
                    });
                    Some(Plan {
                        slot,
                        player: seat.player,
                        send,
                    })
                }
                Link::Dropped { .. } => None,
            })
            .collect()
    }

    /// Sends each seat its frames of the turn, keeps the events of a seat
    /// that was not sent the turn, and ends the game of seats whose player
    /// is no longer in it. A seat whose game is over is sent the turn, and
    /// any it missed, whatever its connection: its end comes after.
    fn deliver(&mut self, host: &Host, deliveries: Vec<Delivery>) {
        let in_game = deliveries
            .first()
            .map(|d| d.turn.message.state.players.clone());
        for d in deliveries {
            let Some(Seat {
                player,
                link: Link::Connected {
                    outbox, last, pace, ..
                },
                ..
            }) = self.seats.get_mut(&d.slot)
            else {
                continue;
            };
            let over = !d.turn.message.state.players.contains(player);
            let frames = d.frames.or_else(|| {
                over.then(|| {
                    let missed = match pace {
                        Pace::Behind { missed } => std::mem::take(missed),
                        Pace::InStep => VecDeque::new(),
                    };
                    frames(d.slot, last, &missed, &d.turn)
                })
                .flatten()
            });
            match frames {
                Some(frames) => {
                    for frame in frames {
                        outbox.send_binary(frame);
                    }
                    *last = d.turn.message;
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
            let over: Vec<SlotIndex> = self
                .seats
                .iter()
                .filter(|(_, s)| !self.in_game.contains(&s.player))
                .map(|(&slot, _)| slot)
                .collect();
            for slot in over {
                self.end_seat(host, slot);
            }
        }
    }

    // -- Inputs --------------------------------------------------------------

    fn on_input(&mut self, host: &Host, input: HostedInput) {
        match input {
            HostedInput::Command {
                slot,
                command,
                bytes,
            } => self.command(slot, command, bytes),
            HostedInput::Leave { slot } => {
                if let Some(seat) = self.seats.get(&slot) {
                    self.quit(seat.player);
                    self.end_seat(host, slot);
                    self.tell_all(&SeatFrame::Seat {
                        slot,
                        change: SeatChange::Left,
                    });
                }
            }
            HostedInput::Dropped { slot } => self.dropped(slot),
            HostedInput::Return {
                conn,
                token,
                outbox,
            } => self.take_back(host, conn, &token, outbox),
            HostedInput::Pause { by } => self.clock_change(by, ClockChange::Paused),
            HostedInput::Resume { by } => self.clock_change(by, ClockChange::Resumed),
            HostedInput::Speed { by, speed } => {
                self.clock_change(by, ClockChange::Speed { speed });
            }
            HostedInput::Remove { by, slot } => self.remove(host, by, slot),
        }
    }

    fn command(&mut self, slot: SlotIndex, command: Command, bytes: usize) {
        let Some(seat) = self.seats.get(&slot) else {
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
        if !on_the_map(&command, self.squares) {
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
        self.commands.push((seat.player, command));
    }

    fn dropped(&mut self, slot: SlotIndex) {
        let Some(seat) = self.seats.get_mut(&slot) else {
            return;
        };
        if let Link::Dropped { .. } = seat.link {
            return;
        }
        info!(room = self.room.0, slot, login = %seat.login, "seat dropped");
        seat.link = Link::Dropped {
            deadline: after(self.settings.grace),
        };
        self.tell_all(&SeatFrame::Seat {
            slot,
            change: SeatChange::Dropped,
        });
    }

    /// Gives a dropped seat back to the player who holds its token, or
    /// refuses. The lobby hears of it before the player is sent anything,
    /// so that whatever the player sends after it finds them in the game.
    fn take_back(&mut self, host: &Host, conn: ConnId, token: &SeatToken, outbox: Outbox) {
        let found = self.seats.iter().find_map(|(&slot, s)| {
            (matches!(s.link, Link::Dropped { .. }) && s.token.matches(token)).then_some(slot)
        });
        let Some(slot) = found else {
            let _ = self.lobby.send(LobbyInput::ReturnRefused {
                room: self.room,
                conn,
            });
            return;
        };
        let _ = self.lobby.send(LobbyInput::Returned {
            room: self.room,
            conn,
            slot,
        });
        self.tell_all(&SeatFrame::Seat {
            slot,
            change: SeatChange::Returned,
        });
        if let Some(seat) = self.seats.get(&slot) {
            info!(room = self.room.0, slot, login = %seat.login, "seat returned");
        }
        self.connect(host, slot, conn, outbox);
    }

    fn clock_change(&mut self, by: SlotIndex, change: ClockChange) {
        let Some((login, outbox)) = self.connected(by) else {
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
        let Some((login, outbox)) = self.connected(by) else {
            return;
        };
        let Some(&player) = self.players.get(slot) else {
            outbox.send(&ServerMsg::Refused {
                reason: Refusal::NoSuchPlayer,
            });
            return;
        };
        let is_seat = self.seats.contains_key(&slot);
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
        self.end_seat(host, slot);
    }

    // -- Helpers -------------------------------------------------------------

    /// The login and outbox of a connected seat.
    fn connected(&self, slot: SlotIndex) -> Option<(Login, Outbox)> {
        let seat = self.seats.get(&slot)?;
        match &seat.link {
            Link::Connected { outbox, .. } => Some((seat.login.clone(), outbox.clone())),
            Link::Dropped { .. } => None,
        }
    }

    /// Tells every connected seat.
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
