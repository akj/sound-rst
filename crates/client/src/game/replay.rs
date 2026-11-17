//! Listening to a recorded game: the turns come from a record replayed
//! here ([`Replay`]), and the listener is a watcher ([`super::watching`]):
//! one player's side at a time, the whole of what that player was told,
//! and no orders. Unlike a watcher on a server, a listener may follow a
//! computer, whose side is what its seat is told after each turn: its
//! units, what it sees and its resources, but none of the events of a
//! turn, since the game tells a computer none.
//!
//! The clock is the listener's. The record plays at normal speed, game
//! time as fast as real time; the listener plays it slower or faster, up
//! to the fastest pace, where the turns are replayed silently, as when
//! loading, and only how far it has got is said. A pause stops the turns;
//! the keys still answer. A jump replays silently and fast to a minute or
//! five ahead, or from the start to a minute or five back, then says
//! where the replay is. Nothing of the stretch skipped is said: the news
//! waiting is kept in the log unheard, and the interface starts again
//! from what the player followed knows there.
//!
//! When the record ends, the listener hears it, then the result and the
//! score of the player followed. A turn the record cannot replay stops the
//! replay there, saying so; the listener can still review what came
//! before, go back, or leave.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use soundrts_protocol::Step;
use soundrts_world::events::Event;
use soundrts_world::msg::{Cue, Msg, Phrase, comma, cue, msg, nb2msg};
use soundrts_world::{
    Completeness, ControllerKind, GameEnd, GameStart, PlayerId, Record, ReplayError, Replayer,
    TurnMessage, VIRTUAL_TIME_INTERVAL,
};

use super::watching::Orders;
use super::{GameInterface, Source, TURN};
use crate::client::Client;
use crate::message::{Category, News};
use crate::saves::{InterfaceState, SaveInfo, Slot};

/// How many turns are replayed silently at each pass of the loop, while
/// jumping or at the fastest pace: the keys are read between two passes.
const TURNS_PER_PASS: u64 = 50;

/// How often the listener hears how far a long jump, or the fastest pace,
/// has got, in seconds.
const PROGRESS_EVERY: f64 = 2.0;

/// How fast the record plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Slow,
    Normal,
    Fast2,
    Fast4,
    Fast8,
    /// As fast as the machine replays, silently.
    Fastest,
}

impl Pace {
    const ALL: [Pace; 6] = [
        Pace::Slow,
        Pace::Normal,
        Pace::Fast2,
        Pace::Fast4,
        Pace::Fast8,
        Pace::Fastest,
    ];

    /// How many times faster than real time the game plays; `None` at the
    /// fastest pace, which does not keep time.
    pub fn factor(self) -> Option<f64> {
        match self {
            Pace::Slow => Some(0.5),
            Pace::Normal => Some(1.0),
            Pace::Fast2 => Some(2.0),
            Pace::Fast4 => Some(4.0),
            Pace::Fast8 => Some(8.0),
            Pace::Fastest => None,
        }
    }

    /// The pace one step faster (`step` above 0) or slower; the same at
    /// either end.
    fn step(self, step: i64) -> Pace {
        let i = Pace::ALL.iter().position(|&p| p == self).unwrap_or(1);
        let i = if step < 0 {
            i.saturating_sub(1)
        } else {
            i.saturating_add(1)
        };
        Pace::ALL.get(i).copied().unwrap_or(self)
    }

    /// "speed fast 2".
    pub fn spoken(self) -> Msg {
        let what = match self {
            Pace::Slow => msg(Phrase::Slow),
            Pace::Normal => msg(Phrase::Normal),
            Pace::Fast2 => [msg(Phrase::Fast), nb2msg(2)].concat(),
            Pace::Fast4 => [msg(Phrase::Fast), nb2msg(4)].concat(),
            Pace::Fast8 => [msg(Phrase::Fast), nb2msg(8)].concat(),
            Pace::Fastest => msg(Phrase::Fastest),
        };
        [msg(Phrase::GameSpeed), what].concat()
    }
}

/// What the replay is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    /// The turns come at the pace, unless paused.
    Playing,
    /// The turns are replayed silently up to `target`, a number of turns
    /// run, from `from`.
    Jumping { target: u64, from: u64 },
    /// The record cannot be replayed further: the listener stays here.
    Broken,
    /// The listener heard the end.
    Over,
}

/// What "keep this game's record" keeps.
#[derive(Clone, Debug)]
pub(crate) enum Keep {
    /// A record listened to from where it already is kept.
    AlreadyKept,
    /// A save, or the last finished game: what its first line says.
    Save { info: SaveInfo, saved_at: u64 },
    /// It was kept.
    Kept,
}

/// A record being listened to: the game replayed so far, and the
/// listener's clock.
pub(crate) struct Replay {
    /// The record, which a jump back replays again from its start.
    record: Arc<Record>,
    replayer: Replayer<Arc<Record>>,
    /// Whether the file was cut short: its end is not the game's.
    completeness: Completeness,
    pace: Pace,
    paused: bool,
    run: Run,
    /// When the next turn is due, at a pace that keeps time.
    next_update: f64,
    /// A turn has run and its end is not handled yet.
    waiting_for_world_update: bool,
    /// When how far the replay got was last said.
    progress_said_at: f64,
    /// Whether the listener gave orders, and heard that it gives none.
    pub(super) orders: Orders,
    /// The seats people played, in seat order.
    people: Vec<PlayerId>,
    keep: Keep,
}

impl Replay {
    /// The replay of `record`, before its first turn. `Err` if its game
    /// cannot be built.
    pub(crate) fn new(
        record: Arc<Record>,
        completeness: Completeness,
        keep: Keep,
    ) -> Result<Replay, ReplayError> {
        let replayer = Replayer::new(Arc::clone(&record))?;
        let controllers = &replayer.record().setup().controllers;
        let people = replayer
            .host()
            .seats()
            .iter()
            .zip(controllers)
            .filter(|(_, c)| matches!(c.kind, ControllerKind::Human { .. }))
            .map(|(&p, _)| p)
            .collect();
        Ok(Replay {
            record,
            replayer,
            completeness,
            pace: Pace::Normal,
            paused: false,
            run: Run::Playing,
            next_update: 0.0,
            waiting_for_world_update: false,
            progress_said_at: 0.0,
            orders: Orders::NoneGiven,
            people,
            keep,
        })
    }

    /// The player a listener follows first: the first person of the game,
    /// or its first player.
    pub(crate) fn first_player(&self) -> Option<PlayerId> {
        self.people
            .first()
            .or_else(|| self.replayer.host().seats().first())
            .copied()
    }

    /// What `seat` is told of the game as replayed so far.
    pub(crate) fn message(&mut self, seat: PlayerId) -> Result<TurnMessage, ReplayError> {
        self.replayer.message(seat)
    }

    pub(crate) fn game_start(&self) -> Arc<GameStart> {
        self.replayer.host().game_start()
    }

    /// The end of the game for `seat`, for the score.
    pub(crate) fn game_end(&mut self, seat: PlayerId) -> Option<GameEnd> {
        self.replayer.game_end(seat).ok()
    }

    pub(crate) fn next_update(&self) -> f64 {
        self.next_update
    }

    pub(crate) fn set_next_update(&mut self, at: f64) {
        self.next_update = at;
    }

    pub(crate) fn turn_handled(&mut self) {
        self.waiting_for_world_update = false;
    }

    /// The listener now follows another player: the end of a turn of the
    /// last one may not come.
    pub(crate) fn followed(&mut self) {
        self.waiting_for_world_update = false;
    }

    /// Whether the listener paused the record.
    pub(crate) fn paused(&self) -> bool {
        self.paused
    }

    /// Whether sounds and news are muted: while jumping, and at the
    /// fastest pace.
    pub(crate) fn muted(&self) -> bool {
        matches!(self.run, Run::Jumping { .. })
            || self.run == Run::Playing && self.pace == Pace::Fastest
    }

    /// How many times faster than real time the game plays now, for the
    /// interface's clock.
    pub(crate) fn speed_factor(&self) -> f64 {
        self.pace.factor().unwrap_or(100.0)
    }

    /// The game time of the whole record, in seconds.
    fn total_seconds(&self) -> i64 {
        i64::try_from(self.replayer.record().game_time_ms() / 1000).unwrap_or(i64::MAX)
    }

    /// The number of turns run where the replay is, or goes when jumping.
    fn position(&self) -> u64 {
        match self.run {
            Run::Jumping { target, .. } => target,
            Run::Playing | Run::Broken | Run::Over => self.replayer.turns_run(),
        }
    }
}

/// "3 minutes 20 seconds".
fn time_msg(seconds: i64) -> Msg {
    [
        nb2msg(seconds.div_euclid(60)),
        msg(Phrase::Minutes),
        nb2msg(seconds.rem_euclid(60)),
        msg(Phrase::Seconds),
    ]
    .concat()
}

/// The number of turns `minutes` of game time take.
fn turns_of(minutes: i64) -> i64 {
    minutes
        .saturating_mul(60_000)
        .checked_div(VIRTUAL_TIME_INTERVAL)
        .unwrap_or(0)
}

impl GameInterface {
    /// The interface of a listener to `replay`, following `me`, from what
    /// `me` is told before the first turn. Says nothing: queued messages
    /// are dropped.
    pub(crate) fn listening(
        cx: &mut Client,
        mut replay: Replay,
        me: PlayerId,
    ) -> Result<Self, ReplayError> {
        let start = replay.game_start();
        let first = replay.message(me)?;
        let seat = soundrts_world::SeatView::new(start, first);
        let speed = replay.speed_factor();
        Ok(Self::with(
            cx,
            Source::Replay(Box::new(replay)),
            seat,
            speed,
        ))
    }

    fn replay(&mut self) -> Option<&mut Replay> {
        self.source.replay_mut()
    }

    /// One pass of a listener's loop: the turn due at the pace, or a
    /// stretch replayed silently; then the end, once the record has given
    /// every turn and the last is told.
    pub(crate) fn replay_update(&mut self, cx: &mut Client) {
        self.tell_watcher_no_orders(cx);
        let now = cx.clock.now();
        let Some(r) = self.replay() else {
            return;
        };
        let run = r.run;
        match run {
            Run::Over | Run::Broken => return,
            Run::Jumping { target, .. } => self.replay_silently(cx, target),
            Run::Playing if r.paused => {}
            Run::Playing if r.pace == Pace::Fastest => {
                let end = r.replayer.record().turns_run();
                self.replay_silently(cx, end);
            }
            Run::Playing => {
                if !r.waiting_for_world_update && now >= r.next_update {
                    let factor = r.pace.factor().unwrap_or(1.0);
                    r.next_update = now + TURN / factor;
                    self.replay_turn(cx);
                }
            }
        }
        let ended = self.replay().is_some_and(|r| {
            r.run == Run::Playing
                && !r.paused
                && !r.waiting_for_world_update
                && r.replayer.is_over()
        });
        if ended && self.srv_queue.is_empty() && !self.end_loop {
            self.recording_ended(cx);
        }
    }

    /// Replays the next turn and takes what the player followed was told
    /// of it. A computer, which is told no events, is given the end of its
    /// turn, so that the interface follows its side; one that left the
    /// game is heard to leave, as a person is.
    fn replay_turn(&mut self, cx: &mut Client) {
        let me = self.me;
        let Some(r) = self.replay() else {
            return;
        };
        let turn = match r.replayer.step_seats(&[me]) {
            Ok(mut turns) => turns.pop(),
            Err(e) => return self.recording_broke(cx, &e),
        };
        let Some(turn) = turn else {
            return;
        };
        let in_game = turn.message.state.players.contains(&me);
        let end = turn
            .end_of_turn
            .or_else(|| in_game.then(|| Arc::clone(&turn.message)));
        let mut then = turn.then;
        if !in_game && !then.iter().any(|e| matches!(e, Event::Quit)) {
            then.push(Event::Quit);
        }
        r.waiting_for_world_update = end.is_some();
        self.take_turn(turn.message, end, then);
    }

    /// Replays turns silently towards `target`, a number of turns run, a
    /// pass's worth at most; then the interface starts again from where
    /// the replay is, without telling anything of the stretch. Once there,
    /// a jump says where the replay is.
    fn replay_silently(&mut self, cx: &mut Client, target: u64) {
        let now = cx.clock.now();
        let Some(r) = self.replay() else {
            return;
        };
        let mut broke = None;
        for _ in 0..TURNS_PER_PASS {
            if r.replayer.turns_run() >= target || r.replayer.is_over() {
                break;
            }
            if let Err(e) = r.replayer.step_seats(&[]) {
                broke = Some(e);
                break;
            }
        }
        let done = r.replayer.turns_run();
        let arrived = done >= target || r.replayer.is_over();
        let (from, total) = match r.run {
            Run::Jumping { from, .. } => (from, target.saturating_sub(from)),
            _ => (0, r.replayer.record().turns_run()),
        };
        let progress = (now - r.progress_said_at >= PROGRESS_EVERY && !arrived).then(|| {
            r.progress_said_at = now;
            done.saturating_sub(from)
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(100)
        });
        if let Some(e) = broke {
            return self.recording_broke(cx, &e);
        }
        self.catch_up(cx);
        if let Some(percent) = progress {
            let m = [
                nb2msg(i64::try_from(percent).unwrap_or(100)),
                msg(Phrase::Percent),
            ]
            .concat();
            cx.voice.item(&m);
        }
        let Some(r) = self.replay() else {
            return;
        };
        if arrived && matches!(r.run, Run::Jumping { .. }) {
            r.run = Run::Playing;
            r.next_update = now;
            let m = self.where_the_replay_is();
            cx.voice.item(&m);
        }
    }

    /// The interface starts again, silently, from what the player followed
    /// is told where the replay is: nothing of the turns skipped is said,
    /// and what the end of that turn would say of the game so far is not
    /// news. If that player left the game meanwhile, the listener follows
    /// another, as when a player leaves.
    fn catch_up(&mut self, cx: &mut Client) {
        let me = self.me;
        let Some(r) = self.replay() else {
            return;
        };
        let m = match r.message(me) {
            Ok(m) => m,
            Err(e) => return self.recording_broke(cx, &e),
        };
        if !m.state.players.contains(&me) {
            // What the player knew when it left: its result and score.
            // What waited of its turns is not heard.
            self.known.seat_mut().install(m);
            self.srv_queue.clear();
            return self.followed_left(cx);
        }
        let minutes = m.time.div_euclid(60_000);
        let start = r.game_start();
        self.restart_from(cx, start, Arc::new(m));
        self.previous_available_food = self.available_food();
        self.previous_used_food = self.used_food();
        self.previous_nb_minutes = minutes;
        while let Some(e) = self.srv_queue.pop_front() {
            self.process_srv_event(cx, e);
        }
        self.scout_info.clear();
        cx.voice.give_up_held();
        if self.place.is_none() {
            let square = self
                .view()
                .own_units()
                .find_map(|u| self.outer_square(u.place));
            self.place = square;
        }
    }

    /// The player followed left the game: the listener follows the next
    /// person still in it, as a watcher does; with nobody left, the
    /// listening is over, with the result and the score of the player
    /// followed last.
    pub(crate) fn followed_left(&mut self, cx: &mut Client) {
        let me = self.me;
        let Some(r) = self.replay() else {
            return;
        };
        let in_game = r
            .replayer
            .host()
            .seats()
            .first()
            .copied()
            .and_then(|s| r.message(s).ok())
            .map(|m| m.state.players)
            .unwrap_or_default();
        let next = after(&r.people, me, Step::Next)
            .into_iter()
            .find(|p| in_game.contains(p));
        let over = r.replayer.is_over();
        match next.map(|p| r.message(p)) {
            Some(Ok(m)) => {
                let start = r.game_start();
                self.follow(cx, start, Arc::new(m));
            }
            Some(Err(e)) => self.recording_broke(cx, &e),
            // The game's last turn: the recording ends with it.
            None if over => self.recording_ended(cx),
            None => {
                if let Some(r) = self.replay() {
                    r.run = Run::Over;
                }
                self.srv_quit(cx);
            }
        }
    }

    /// The record has no more turns: the listener hears it, and the
    /// listening ends with the result and the score of the player followed.
    fn recording_ended(&mut self, cx: &mut Client) {
        let Some(r) = self.replay() else {
            return;
        };
        r.run = Run::Over;
        let words = match r.completeness {
            Completeness::Whole => Phrase::RecordingEnded,
            Completeness::CutShort => Phrase::RecordingDamaged,
        };
        cx.log.info(format!(
            "the recording ended after {} turns",
            r.replayer.turns_run()
        ));
        cx.news(News::new(Category::System, msg(words)));
        self.srv_quit(cx);
    }

    /// A turn of the record cannot be replayed: the replay stops here,
    /// saying so. The listener can still review what came before, go
    /// back, or leave.
    fn recording_broke(&mut self, cx: &mut Client, e: &ReplayError) {
        cx.log.warning(format!("the recording cannot go on: {e}"));
        if let Some(r) = self.replay() {
            r.run = Run::Broken;
        }
        cx.news(News::new(Category::System, msg(Phrase::RecordingDamaged)));
    }

    /// "3 minutes 20 seconds of 12 minutes 0 seconds": where the replay
    /// is, of the whole record.
    fn where_the_replay_is(&mut self) -> Msg {
        let now = self.view().time() / 1000;
        let total = self.replay().map_or(0, |r| r.total_seconds());
        [time_msg(now), msg(Phrase::OutOf), time_msg(total)].concat()
    }

    /// F3 while listening: where the replay is, of the whole record, then
    /// the pace, and whether it is paused.
    pub(crate) fn say_replay_time(&mut self, cx: &mut Client) -> bool {
        let Some(r) = self.replay() else {
            return false;
        };
        let (pace, paused) = (r.pace, r.paused);
        let mut m = self.where_the_replay_is();
        m.extend(comma());
        m.extend(pace.spoken());
        if paused {
            m.extend(comma());
            m.extend(msg(Phrase::Paused));
        }
        cx.voice.item(&m);
        true
    }

    /// The listener plays the record one step faster (`step` above 0) or
    /// slower, and hears the pace. Anyone else hears that only a listener
    /// can.
    pub(crate) fn listen_speed(&mut self, cx: &mut Client, step: i64) {
        let Some(r) = self.replay() else {
            cx.voice.item(&msg(Phrase::OnlyWhileListening));
            return;
        };
        let pace = r.pace.step(step);
        self.set_pace(cx, pace);
    }

    /// The listener plays the record at `pace`, and hears it. Going to the
    /// fastest pace gives up the news waiting, which stays in the log
    /// unheard, and the sounds stop.
    pub(crate) fn set_pace(&mut self, cx: &mut Client, pace: Pace) {
        let now = cx.clock.now();
        let Some(r) = self.replay() else {
            return;
        };
        let was = r.pace;
        r.pace = pace;
        r.next_update = now;
        r.progress_said_at = now;
        let factor = r.speed_factor();
        let playing = r.run == Run::Playing;
        self.speed = factor;
        if pace == Pace::Fastest && was != Pace::Fastest {
            cx.voice.silent_flush();
            self.silence_noises(cx);
            // The events of the last turn waiting would be heard.
            if playing {
                self.catch_up(cx);
            }
        }
        cx.voice.item(&pace.spoken());
    }

    /// The listener pauses the record, or resumes it, and hears which.
    /// Anyone else hears that only a listener can.
    pub(crate) fn listen_pause(&mut self, cx: &mut Client) {
        let now = cx.clock.now();
        let Some(r) = self.replay() else {
            cx.voice.item(&msg(Phrase::OnlyWhileListening));
            return;
        };
        r.paused = !r.paused;
        r.next_update = now;
        let m = if r.paused {
            msg(Phrase::Paused)
        } else {
            r.pace.spoken()
        };
        cx.voice.item(&m);
    }

    /// The listener goes `minutes` of game time forward, or back below 0,
    /// from where the replay is or is going: forward by replaying silently
    /// and fast, back by replaying again from the start. The news waiting
    /// is kept in the log unheard; once there, the listener hears where
    /// the replay is. Anyone else hears that only a listener can.
    pub(crate) fn listen_jump(&mut self, cx: &mut Client, minutes: i64) {
        let now = cx.clock.now();
        let Some(r) = self.replay() else {
            cx.voice.item(&msg(Phrase::OnlyWhileListening));
            return;
        };
        let end = r.replayer.record().turns_run();
        let at = i64::try_from(r.position()).unwrap_or(i64::MAX);
        let target = u64::try_from(at.saturating_add(turns_of(minutes)).max(0))
            .unwrap_or(0)
            .min(end);
        if r.run == Run::Broken && target >= r.replayer.turns_run() {
            cx.voice.item(&msg(Phrase::RecordingDamaged));
            return;
        }
        if target < r.replayer.turns_run() {
            match Replayer::new(Arc::clone(&r.record)) {
                Ok(replayer) => r.replayer = replayer,
                Err(e) => return self.recording_broke(cx, &e),
            }
        }
        let from = r.replayer.turns_run();
        r.run = Run::Jumping { target, from };
        r.progress_said_at = now;
        r.waiting_for_world_update = false;
        cx.voice.silent_flush();
        self.srv_queue.clear();
        self.silence_noises(cx);
    }

    /// The listener follows the next player still in the game, or the
    /// previous one, a computer too, and hears whose side it now is.
    pub(crate) fn follow_in_record(&mut self, cx: &mut Client, step: Step) {
        let me = self.me;
        let players = self.view().players().to_vec();
        let Some(next) = after(&players, me, step).into_iter().next() else {
            cx.voice.item(&cue(Cue::Beep));
            return;
        };
        let Some(r) = self.replay() else {
            return;
        };
        match r.message(next) {
            Ok(m) => {
                let start = r.game_start();
                self.follow(cx, start, Arc::new(m));
            }
            Err(_) => cx.voice.item(&msg(Phrase::RecordingDamaged)),
        }
    }

    /// Keeps the record listened to in the records folder, under a new
    /// name, as its save says it; "kept", or why not.
    pub(crate) fn keep_record(&mut self, cx: &mut Client) {
        let dir = cx.paths.records();
        let Some(r) = self.replay() else {
            return;
        };
        let (info, saved_at) = match &r.keep {
            Keep::AlreadyKept | Keep::Kept => {
                cx.voice.item(&msg(Phrase::KeptInTheRecordsFolder));
                return;
            }
            Keep::Save { info, saved_at } => (info.clone(), *saved_at),
        };
        let written = crate::saves::write_save(
            &dir,
            &info,
            r.replayer.record(),
            &InterfaceState::default(),
            saved_at,
            Slot::New,
        );
        match written {
            Ok(path) => {
                r.keep = Keep::Kept;
                cx.log
                    .info(format!("kept the record in {}", path.display()));
                cx.voice.item(&msg(Phrase::KeptInTheRecordsFolder));
            }
            Err(e) => {
                cx.log.warning(format!("cannot keep the record: {e}"));
                cx.voice.item(&msg(Phrase::CannotKeepIt));
            }
        }
    }
}

/// The players of `players` after `me`, in the order `step` goes, round
/// to `me` last; all of them from the first (or last) if `me` is not
/// among them.
fn after(players: &[PlayerId], me: PlayerId, step: Step) -> Vec<PlayerId> {
    let at = players.iter().position(|&p| p == me);
    let mut list: Vec<PlayerId> = match at {
        Some(i) => players
            .iter()
            .skip(i.saturating_add(1))
            .chain(players.iter().take(i.saturating_add(1)))
            .copied()
            .collect(),
        None => players.to_vec(),
    };
    if step == Step::Previous {
        if let Some(i) = at {
            list = players
                .iter()
                .take(i)
                .rev()
                .chain(players.iter().skip(i).rev())
                .copied()
                .collect();
        } else {
            list.reverse();
        }
    }
    list
}

#[cfg(test)]
mod tests;
