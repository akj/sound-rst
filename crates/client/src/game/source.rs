//! Where a game's turns come from, and what each source answers for
//! itself: a game run here ([`Local`]), a game a server runs
//! ([`ServerGame`]), or a record listened to ([`Replay`]).
//!
//! Every question whose answer depends on the source is a method here:
//! how the game starts and how it ends, what the seat may do, and the
//! next turn. The rest of the interface asks these questions and never
//! the kind of source. The modules of one source ([`super::server`],
//! [`super::replay`], and [`super::saving`] for a game run here) reach
//! their own state through [`Source::server`], [`Source::replay_mut`]
//! and [`Source::local`].

use soundrts_protocol::Step;
use soundrts_world::msg::{Msg, Phrase, msg};
use soundrts_world::{Command, EntityId, Host, PlayerId, SeatView, TurnMessage};

use super::replay::Replay;
use super::server::ServerGame;
use super::stats::score_msgs;
use super::watching::Orders;
use super::{GameInterface, Mode, TURN};
use crate::client::Client;
use crate::menu::Exit;
use crate::message::{Category, News};
use crate::saves::SaveInfo;

/// Where the game's turns come from.
pub(crate) enum Source {
    /// A game run in this process: single player, a campaign, training.
    Local(Local),
    /// A server, which sends what the seat is told.
    Server(ServerGame),
    /// A record replayed for someone who listens to it.
    Replay(Box<Replay>),
}

/// The speeds the game menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Speeds {
    /// None: a game run here offers them only to its first player.
    None,
    /// The game's: in a game run here, to its first player; on a server,
    /// to every seat, and only the choice of the room's host goes to the
    /// server.
    Game,
    /// The listener's paces of a record, and whether it is paused.
    Paces { paused: bool },
}

/// A game this process runs.
pub(crate) struct Local {
    pub(super) host: Box<Host>,
    /// When the next turn is due.
    next_update: f64,
    /// A turn has run and its end is not handled yet.
    waiting_for_world_update: bool,
    /// What a save of the game says it is; `None` if it is not saved.
    pub(super) save: Option<SaveInfo>,
    /// The selection of a game loaded, taken at the first end of turn,
    /// once the interface knows the units.
    pub(super) selection_to_restore: Vec<EntityId>,
}

impl Source {
    /// The game run here, for the module that saves it.
    pub(super) fn local(&self) -> Option<&Local> {
        match self {
            Source::Local(l) => Some(l),
            Source::Server(_) | Source::Replay(_) => None,
        }
    }

    /// As [`Source::local`], to change it.
    pub(super) fn local_mut(&mut self) -> Option<&mut Local> {
        match self {
            Source::Local(l) => Some(l),
            Source::Server(_) | Source::Replay(_) => None,
        }
    }

    /// The game a server runs, for the module that plays its stream.
    pub(super) fn server(&self) -> Option<&ServerGame> {
        match self {
            Source::Server(s) => Some(s),
            Source::Local(_) | Source::Replay(_) => None,
        }
    }

    /// As [`Source::server`], to change it.
    pub(super) fn server_mut(&mut self) -> Option<&mut ServerGame> {
        match self {
            Source::Server(s) => Some(s),
            Source::Local(_) | Source::Replay(_) => None,
        }
    }

    /// The record listened to, for the tests of the module that replays
    /// it.
    #[cfg(test)]
    pub(super) fn replay(&self) -> Option<&Replay> {
        match self {
            Source::Replay(r) => Some(r),
            Source::Local(_) | Source::Server(_) => None,
        }
    }

    /// The record listened to, for the module that replays it.
    pub(super) fn replay_mut(&mut self) -> Option<&mut Replay> {
        match self {
            Source::Replay(r) => Some(r),
            Source::Local(_) | Source::Server(_) => None,
        }
    }

    /// The watcher now follows another player, whose whole message brought
    /// `objectives`: the end of a turn of the last one may not come.
    pub(super) fn followed(&mut self, objectives: usize) {
        match self {
            Source::Server(server) => server.followed(objectives),
            Source::Replay(replay) => replay.followed(),
            Source::Local(_) => {}
        }
    }
}

impl GameInterface {
    /// The interface of `me` in the game of `host`, run here, from the
    /// start of the game and what `me` is told before the first turn.
    /// `save` is what a save of the game says it is; a game without one is
    /// not saved. Says nothing: queued messages are dropped.
    pub fn local(cx: &mut Client, host: Host, me: PlayerId, save: Option<SaveInfo>) -> Self {
        let seat = SeatView::new(host.game_start(), host.message(me));
        let local = Local {
            host: Box::new(host),
            next_update: 0.0,
            waiting_for_world_update: false,
            save,
            selection_to_restore: Vec::new(),
        };
        let speed = cx.config.speed;
        Self::with(cx, Source::Local(local), seat, speed)
    }

    /// The interface of a seat of a game a server runs, from what the
    /// seat was sent when it joined. Says nothing: queued messages are
    /// dropped.
    pub(crate) fn on_server(cx: &mut Client, seat: SeatView, server: ServerGame) -> Self {
        let speed = server.speed_factor();
        Self::with(cx, Source::Server(server), seat, speed)
    }

    // how the game starts and ends

    /// Readies a new game's interface, once built from its source: what
    /// the source says first (on a server, the room's incomes, which the
    /// game's start silenced; to a listener, whose side it hears), then
    /// the order menus, the keys, and the story told before the first
    /// turn, `intro` then the objective. In a game run here the first turn
    /// waits until the story is told.
    pub(crate) fn begin(&mut self, cx: &mut Client, intro: Vec<Msg>) {
        match self.source {
            Source::Local(_) => {}
            Source::Server(_) => {
                let incomes = cx
                    .net
                    .as_ref()
                    .and_then(|n| n.room.as_ref())
                    .and_then(|r| r.incomes());
                if let Some(incomes) = incomes {
                    cx.news(News::new(Category::System, incomes));
                }
            }
            Source::Replay(_) => self.now_following(cx),
        }
        cx.input.set_game(Some(self.progress()));
        self.update_orders_list(cx);
        let story = intro.into_iter().chain(self.objective()).collect();
        self.tell_story(cx, story);
        self.load_bindings(cx);
    }

    /// The intro of a mission's map, its sounds or texts in order, to tell
    /// before the first turn.
    pub(crate) fn mission_intro(&self) -> Vec<Msg> {
        self.view()
            .map_intro()
            .iter()
            .map(|part| vec![part.clone()])
            .collect()
    }

    /// The game told the player it is over for them. On a server, the
    /// loop ends once the end has come, with the statistics of the score.
    /// A watcher hears that the player it follows is out of the game: the
    /// server moves it on to another player, or ends its game. A listener
    /// to a record moves on to the next person still in the game, or the
    /// listening ends.
    pub(crate) fn quit_heard(&mut self, cx: &mut Client) {
        match &mut self.source {
            Source::Local(_) => self.srv_quit(cx),
            Source::Server(server) => {
                if server.told_over() {
                    self.srv_quit(cx);
                }
            }
            Source::Replay(_) => self.followed_left(cx),
        }
    }

    /// Takes what the seat is told when the game is over for it: the
    /// statistics of the score. On a server, nothing if the server did
    /// not send them.
    pub fn end_game(&mut self) {
        let end = match &mut self.source {
            Source::Local(l) => Some(l.host.game_end(self.me)),
            Source::Server(s) => s.take_end(),
            Source::Replay(r) => r.game_end(self.me),
        };
        if let Some(end) = end {
            self.known.seat_mut().install_end(end);
        }
    }

    /// Ends the game once its loop has ended, `ran` saying how; whether
    /// the player won. First what the source does: a game run here is kept
    /// as the last finished game in the records folder, or, if the window
    /// was closed before the player asked to quit, saved as the last game;
    /// a server is told the player left, before the score so that it knows
    /// at once. Unless the window was closed, what the end of the game left
    /// to say is said, then the score, until done (a key skips ahead): a
    /// listener hears that the game was not over for a player still in it
    /// when the record ended. After a game on a server, the menus come
    /// back.
    pub fn end(&mut self, cx: &mut Client, ran: Result<(), Exit>) -> Result<bool, Exit> {
        let listening = match self.source {
            Source::Local(_) => {
                if let Err(exit) = ran {
                    if !self.already_asked_to_quit {
                        self.save_on_leaving(cx);
                    }
                    return Err(exit);
                }
                self.keep_finished(cx);
                false
            }
            Source::Server(_) => {
                crate::net::leave_game(cx);
                ran?;
                false
            }
            Source::Replay(_) => {
                ran?;
                true
            }
        };
        cx.voice.flush(true);
        self.end_game();
        let still_on = listening && self.goes_on();
        for m in score_msgs(self.view(), cx.res.style(), still_on) {
            cx.news(News::new(Category::System, m));
        }
        cx.voice.flush(true);
        self.stop_noises(cx);
        cx.input.set_game(None);
        if self.on_a_server() {
            if let Some(n) = &mut cx.net {
                n.game = None;
            }
            cx.voice
                .menu(&[msg(Phrase::Menu), msg(Phrase::MakeASelection)].concat());
        }
        Ok(self.view().has_victory())
    }

    // what the seat may do

    /// Whether the world waits, on purpose, while the player reads a story
    /// or a menu: in a game run here, not on a server or in a record.
    pub(crate) fn waits_for_the_player(&self) -> bool {
        match self.source {
            Source::Local(_) => true,
            Source::Server(_) | Source::Replay(_) => false,
        }
    }

    /// Whether the world waits for the player now: in a game run here,
    /// while a story is told or the game menu or the console is open. The
    /// keys and the voice go on meanwhile.
    pub fn world_waits(&self) -> bool {
        self.waits_for_the_player()
            && match self.mode {
                Mode::Playing | Mode::OrderShortcut | Mode::Chat(_) | Mode::KeyHelp => false,
                Mode::Story(_) | Mode::GameMenu(_) | Mode::Console(_) => true,
            }
    }

    /// Whether the game takes cheats: a game run here does, if its setup
    /// allows them. Nobody cheats in a game on a server, nor in a record
    /// listened to.
    pub(crate) fn takes_cheats(&self) -> bool {
        match self.source {
            Source::Local(_) => true,
            Source::Server(_) | Source::Replay(_) => false,
        }
    }

    /// Whether the game's speed is the world's: in a game run here, the
    /// world and the game menu set it, and the next single-player game
    /// starts at the speed chosen. A server keeps the time of its games,
    /// and a listener the pace of a record.
    pub(crate) fn speed_is_the_worlds(&self) -> bool {
        match self.source {
            Source::Local(_) => true,
            Source::Server(_) | Source::Replay(_) => false,
        }
    }

    /// Whether the player only watches: a watcher of a game on a server,
    /// or a listener to a recorded game.
    pub(crate) fn is_watching(&self) -> bool {
        match &self.source {
            Source::Server(s) => s.watching(),
            Source::Replay(_) => true,
            Source::Local(_) => false,
        }
    }

    /// Whether the watcher gave orders, and heard so; `None` for a player.
    pub(super) fn watcher_orders(&mut self) -> Option<&mut Orders> {
        match &mut self.source {
            Source::Server(s) if s.watching() => Some(s.watcher_orders()),
            Source::Replay(r) => Some(&mut r.orders),
            Source::Server(_) | Source::Local(_) => None,
        }
    }

    /// A watcher follows the next player still in the game, or the
    /// previous one: on a server, the server sends that player's whole
    /// message; in a recorded game, the replay gives it. Anyone else hears
    /// that only a watcher can.
    pub(crate) fn follow_player(&mut self, cx: &mut Client, step: Step) {
        match &self.source {
            Source::Server(s) if s.watching() => self.ask_server_to_follow(cx, step),
            Source::Replay(_) => self.follow_in_record(cx, step),
            Source::Server(_) | Source::Local(_) => {
                cx.voice.item(&msg(Phrase::OnlyAWatcherFollows));
            }
        }
    }

    /// Whether the game runs on a server: its players can write to each
    /// other and point their allies at a square, and people may watch it.
    pub fn on_a_server(&self) -> bool {
        match self.source {
            Source::Server(_) => true,
            Source::Local(_) | Source::Replay(_) => false,
        }
    }

    /// The speeds the game menu offers.
    pub(crate) fn speeds_offered(&self) -> Speeds {
        match &self.source {
            Source::Local(_) if self.view().players().first() == Some(&self.me) => Speeds::Game,
            Source::Local(_) => Speeds::None,
            Source::Server(_) => Speeds::Game,
            Source::Replay(r) => Speeds::Paces { paused: r.paused() },
        }
    }

    /// Whether sounds are muted: while a listener jumps, or plays the
    /// record at its fastest.
    pub(crate) fn muted(&self) -> bool {
        match &self.source {
            Source::Replay(r) => r.muted(),
            Source::Local(_) | Source::Server(_) => false,
        }
    }

    // the next turn

    /// Takes the next turn if it has come: a game run here runs it once it
    /// is due, unless the world waits for the player (`waits`); a server's
    /// turns are taken as they come, and a record's at the listener's
    /// pace, whatever the player reads.
    pub(super) fn next_turn(&mut self, cx: &mut Client, waits: bool) {
        match &self.source {
            Source::Server(_) => self.server_update(cx),
            Source::Replay(_) => self.replay_update(cx),
            Source::Local(l) => {
                let due = !l.waiting_for_world_update && cx.clock.now() >= l.next_update;
                if !waits && due {
                    self.ask_for_update(cx);
                }
            }
        }
    }

    /// Makes the next turn of a game run here, or of a record listened
    /// to, due at `now`. A server keeps the time of its own games.
    pub(crate) fn turn_due_at(&mut self, now: f64) {
        match &mut self.source {
            Source::Local(l) => l.next_update = now,
            Source::Replay(r) => r.set_next_update(now),
            Source::Server(_) => {}
        }
    }

    /// When the turn after the last one taken is due: a local game's next
    /// step, the next turn of the server's clock as of the last turn it
    /// sent, or the next turn of a record at the listener's pace.
    pub(super) fn turn_due(&self) -> f64 {
        match &self.source {
            Source::Local(l) => l.next_update,
            Source::Server(s) => s.next_turn(),
            Source::Replay(r) => r.next_update(),
        }
    }

    /// The end of the last turn taken is handled: a game run here may run
    /// the next, and the selection of a game loaded is taken back; a
    /// record may replay the next. A server sends its turns as they come.
    pub(super) fn turn_handled(&mut self) {
        match &mut self.source {
            Source::Local(l) => {
                l.waiting_for_world_update = false;
                if !l.selection_to_restore.is_empty() {
                    self.group = std::mem::take(&mut l.selection_to_restore);
                }
            }
            Source::Replay(r) => r.turn_handled(),
            Source::Server(_) => {}
        }
    }

    /// Gives the pending commands to the host as the player's, runs a
    /// turn, and queues what happened.
    fn ask_for_update(&mut self, cx: &mut Client) {
        for c in std::mem::take(&mut self.pending_commands) {
            self.queue_at_once(c);
        }
        self.turn_due_at(cx.clock.now() + TURN / self.speed);
        self.run_turn(cx);
    }

    /// Runs a turn of the game here with the commands queued, takes the
    /// turn's message, and queues what happened.
    pub(crate) fn run_turn(&mut self, cx: &mut Client) {
        let Source::Local(l) = &mut self.source else {
            return;
        };
        l.waiting_for_world_update = true;
        let report = l.host.step(self.me);
        for w in report.warnings {
            cx.log.warning(format!("world: {w}"));
        }
        self.take_turn(report.message, report.end_of_turn, report.then);
    }

    /// Gives a command to the host of a game run here, which carries out
    /// at once a cheat that changes the game between turns: what the
    /// player hears next is told by the host at once. A game on a server
    /// takes no cheat, nor a record.
    pub(crate) fn queue_at_once(&mut self, command: Command) {
        let Source::Local(l) = &mut self.source else {
            return;
        };
        if let Some(told) = l.host.queue(self.me, command) {
            self.install_at_once(told);
        }
    }

    /// Takes what the host tells the seat between turns, after a cheat
    /// that changed the game at once: the answers are from it from now
    /// on. After playing as another player, the seat is that player's, and
    /// what the interface remembered of the game starts again. An end of
    /// turn of the previous seat still waiting in the queue brings the
    /// interface's copies up to date with that seat's entities and squares;
    /// the new seat notes only what it sees itself.
    fn install_at_once(&mut self, told: TurnMessage) {
        let Source::Local(l) = &self.source else {
            return;
        };
        if told.seat == self.me {
            self.known.seat_mut().install(told);
        } else {
            self.me = told.seat;
            *self.known.seat_mut() = SeatView::new(l.host.game_start(), told);
            self.objectives_seen.clear();
            self.objective_at = None;
        }
    }

    /// The host of a game run here, for tests that check its results.
    /// Panics in a game on a server, which has no host here, or a record.
    #[cfg(any(test, feature = "testkit"))]
    pub fn host(&self) -> &Host {
        match &self.source {
            Source::Local(l) => &l.host,
            Source::Server(_) | Source::Replay(_) => panic!("no host played here"),
        }
    }

    /// As [`GameInterface::host`], to change the game.
    #[cfg(any(test, feature = "testkit"))]
    pub fn host_mut(&mut self) -> &mut Host {
        match &mut self.source {
            Source::Local(l) => &mut l.host,
            Source::Server(_) | Source::Replay(_) => panic!("no host played here"),
        }
    }
}

#[cfg(test)]
mod tests;
