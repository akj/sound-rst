//! The game interface (`clientgame.py`): what the player hears of a game
//! and what the keys do.
//!
//! The turns of a game come from one of three places ([`Source`]): a game
//! run in this process, on a [`Host`] the interface owns (single player, a
//! campaign, training); a server, which sends the seat's stream
//! ([`server`]); or a record replayed for someone who listens to it
//! ([`replay`]). A watcher of a game on a server and a listener to a
//! record are the same watcher (`watching`). Either way the interface
//! reads the game only through what its seat is told
//! ([`GameInterface::view`]): the game's start, then a [`TurnMessage`]
//! for each turn, installed when the turn is taken. Its loop steps a local
//! host when a turn is due (every 300 ms divided by the speed), takes the
//! turns a server sent as they come, or replays the record's at the
//! listener's pace, then handles what happened as
//! the original's server events: the turn's events, then the end of the
//! turn, which brings the interface's own copies of the entities and
//! squares up to date. One server event is handled per pass of the loop,
//! after the key presses, as in the original, so the questions answer for
//! the end of the turn from the moment it is taken, while those copies
//! catch up a few passes later.
//!
//! Nothing said stops the loop. What the keys do depends on one [`Mode`].
//! In single-player the world waits, on purpose, while a story is told
//! and while the game menu or the console is open: the host is not
//! stepped, its events wait and nothing moves, but the keys and the voice
//! go on. In a game on a server, or a record listened to, the world never
//! waits; a listener pauses the record with a key.
//!
//! The submodules follow the original's files and sections: [`entity`]
//! (`clientgameentity.py`), [`focus`] (`clientgamefocus.py`), [`news`],
//! [`order`] (`clientgameorder.py`), [`animation`], [`stats`]
//! (`worldplayerstats.score_msgs`), [`command`] (the bindings' commands),
//! and the interface's own parts: [`squares`], [`selection`], [`alerts`],
//! [`questions`] and [`misc`].

pub mod alerts;
pub mod animation;
pub mod command;
pub mod entity;
pub mod focus;
pub mod misc;
pub mod news;
pub mod order;
pub mod questions;
pub mod replay;
mod saving;
pub mod selection;
pub mod server;
pub mod squares;
pub mod stats;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
mod watching;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use indexmap::{IndexMap, IndexSet};
use soundrts_world::events::{EntityEvent, Event};
use soundrts_world::msg::{Cue, Msg, MsgPart, Phrase, cue, msg};
use soundrts_world::orders::{Keyword, MenuEntry};
use soundrts_world::player::Objective;
use soundrts_world::{
    Command, EntityId, Host, KnownEntity, Place, PlayerId, SeatView, SquareId, TurnMessage,
};

use crate::bindings::{Bindings, Chord, game_bindings_text};
use crate::client::Client;
use crate::input::{Event as InputEvent, GameProgress, InputPoint, Key, KeyDown, Mods};
use crate::menu::{Exit, TextEntry};
use crate::message::{Category, News, Subject};
use crate::sound::Listener;
use crate::style::{Style, StyleValue};
use crate::voice::EntryId;

use self::animation::NoiseSource;
use self::command::GameCommand;
use self::entity::EntityView;
use self::focus::Zoom;
use self::order::OrderTypeView;

/// Seconds of game time in a turn.
pub const TURN: f64 = 0.3;
/// The same alert sound is not played again within this many seconds.
pub const ALERT_LIMIT: f64 = 0.5;
/// Entity events this many seconds late play no sound: their news is
/// still told.
pub const EVENT_LIMIT: f64 = 3.0;

/// Whether an entity event reaches the interface in time for its sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    OnTime,
    /// More than [`EVENT_LIMIT`] seconds late: a sound would be noise.
    Late,
}

/// What the world tells the interface, in order.
#[derive(Clone, Debug)]
pub enum SrvEvent {
    Entity {
        entity: Box<KnownEntity>,
        event: EntityEvent,
    },
    VoiceImportant(Msg),
    Sequence(Msg),
    Speed(f64),
    Quit,
    /// The turn is over: the message of the turn, whose entities and
    /// squares the interface's copies take.
    EndOfTurn(Arc<TurnMessage>),
}

impl SrvEvent {
    /// The event to handle in its turn. A unit replaced is followed at the
    /// end of its turn instead, when the interface knows the new unit.
    fn of(e: Event) -> Option<SrvEvent> {
        Some(match e {
            Event::Entity { entity, event } => SrvEvent::Entity { entity, event },
            Event::VoiceImportant(m) => SrvEvent::VoiceImportant(m),
            Event::Sequence(s) => SrvEvent::Sequence(s),
            Event::Speed(s) => SrvEvent::Speed(s),
            Event::Quit => SrvEvent::Quit,
            Event::Replaced { .. } => return None,
        })
    }
}

/// What the keys do now.
pub(crate) enum Mode {
    /// The keys are the commands of the bindings.
    Playing,
    /// The next key picks an order by its shortcut.
    OrderShortcut,
    /// A story is told (see [`GameInterface::tell_story`]).
    Story(Story),
    /// The game menu is open.
    GameMenu(misc::GameMenu),
    /// A chat line is being typed.
    Chat(misc::Chat),
    /// A command of the console is being typed.
    Console(TextEntry),
    /// Key help: a key says what it does instead of doing it.
    KeyHelp,
}

impl Mode {
    /// Where the keys of this mode are read.
    fn input_point(&self) -> InputPoint {
        match self {
            Mode::Playing | Mode::OrderShortcut | Mode::Story(_) | Mode::KeyHelp => {
                InputPoint::Game
            }
            Mode::GameMenu(_) => InputPoint::Menu,
            Mode::Chat(_) | Mode::Console(_) => InputPoint::Text,
        }
    }
}

/// A story being told: its messages in the voice's log.
pub(crate) struct Story(Vec<EntryId>);

impl Story {
    /// Whether every message has been said or skipped.
    fn is_told(&self, cx: &Client) -> bool {
        !self.0.iter().any(|&id| cx.voice.is_waiting(id))
    }

    /// Skips the rest of the story, and the message being said: the
    /// story's messages are kept in the log unheard, none of them is
    /// heard, and the news waiting with them is said.
    fn skip(&self, cx: &mut Client) {
        cx.voice.skip_with(&self.0);
    }
}

/// The category of a message the world says now. Objectives and the end of
/// a mission are the story; the rest (players defeated or leaving, answers
/// to the console) is about the game itself.
fn important_category(m: &Msg) -> Category {
    let story = [
        Phrase::NewObjective,
        Phrase::ObjectiveComplete,
        Phrase::MissionComplete,
    ];
    if story.iter().any(|s| m.starts_with(&msg(*s))) {
        Category::Story
    } else {
        Category::System
    }
}

/// The words of `obj.attr` in style as text parts (with a warning, once,
/// if it is missing). For an `if_me` value, the words for the player.
pub fn style_msg(style: &Style, obj: &str, attr: &str) -> Msg {
    style
        .words(obj, attr)
        .iter()
        .filter_map(|w| style_part(w))
        .collect()
}

/// What a word of a style names: a phrase, a sound or a count. `$1` stays
/// as written, for [`order::substitute_args`].
pub fn style_part(word: &str) -> Option<MsgPart> {
    if word.starts_with('$') {
        Some(MsgPart::literal(word))
    } else {
        MsgPart::named(word)
    }
}

/// As [`style_msg`], without the warning.
pub fn style_msg_quiet(style: &Style, obj: &str, attr: &str) -> Msg {
    match style.get_or_warn(obj, attr, false) {
        Some(StyleValue::Words(w)) | Some(StyleValue::IfMe(w, _)) => {
            w.iter().filter_map(|w| style_part(w)).collect()
        }
        _ => Vec::new(),
    }
}

/// The title of a type: the phrases and sounds its style `title` names.
pub fn compute_title(style: &Style, type_name: &str) -> Msg {
    style
        .words(type_name, "title")
        .iter()
        .filter_map(|w| MsgPart::named(w))
        .filter(|part| matches!(part, MsgPart::Name(_)))
        .collect()
}

/// Where the game's turns come from.
pub(crate) enum Source {
    /// A game run in this process: single player, a campaign, training.
    Local(Local),
    /// A server, which sends what the seat is told.
    Server(server::ServerGame),
    /// A record replayed for someone who listens to it.
    Replay(Box<replay::Replay>),
}

/// A game this process runs.
pub(crate) struct Local {
    host: Box<Host>,
    /// When the next turn is due.
    next_update: f64,
    /// A turn has run and its end is not handled yet.
    waiting_for_world_update: bool,
    /// What a save of the game says it is; `None` if it is not saved.
    save: Option<crate::saves::SaveInfo>,
    /// The selection of a game loaded, taken at the first end of turn,
    /// once the interface knows the units.
    selection_to_restore: Vec<EntityId>,
}

/// The interface of a running game.
pub struct GameInterface {
    /// Where the game runs.
    pub(crate) source: Source,
    /// The player this interface plays.
    pub me: PlayerId,
    /// What the player knows of the game: everything the interface reads
    /// of it.
    pub(crate) seat: SeatView,
    /// The game speed: 1 is normal.
    pub speed: f64,
    /// What the player perceives or remembers, by id, in arrival order, as
    /// of the last end of turn handled.
    pub dobjets: IndexMap<EntityId, EntityView>,
    /// The squares observed at the last end of turn handled.
    pub scouted_squares: BTreeSet<SquareId>,
    /// The squares observed before then (all of them in cheat mode).
    pub scouted_before_squares: BTreeSet<SquareId>,
    /// The game time of the last end of turn handled, in seconds.
    pub last_virtual_time: f64,
    /// The number of turns whose results have been handled.
    pub(crate) turns_handled: u64,
    /// The selected square.
    pub place: Option<SquareId>,
    /// The selected object: always one of `dobjets`.
    pub target: Option<EntityId>,
    /// The zoom, when zoom mode is on.
    pub zoom: Option<Zoom>,
    /// The selected order, waiting for a target or a confirmation.
    pub order: Option<OrderTypeView>,
    /// The units controlled.
    pub group: Vec<EntityId>,
    pub immersion: bool,
    pub follow_mode: bool,
    /// The listener, in meters, and its facing in degrees.
    pub x: f64,
    pub y: f64,
    pub o: f64,
    /// What the keys do now.
    pub(crate) mode: Mode,
    pub(crate) previous_order: Option<OrderTypeView>,
    pub(crate) previous_group: Option<Vec<EntityId>>,
    pub(crate) previous_compass: Option<Msg>,
    pub(crate) previous_menus: IndexMap<String, Vec<MenuEntry>>,
    /// When the player's units were last attacked at each place.
    pub(crate) attacks: HashMap<Place, f64>,
    pub(crate) lost_units: Vec<(Msg, Option<Place>)>,
    pub(crate) neutralized_units: Vec<(Msg, Option<Place>)>,
    pub(crate) new_enemy_units: Vec<(Msg, Option<Place>)>,
    pub(crate) scout_info: IndexSet<Place>,
    pub(crate) known_resource_places: HashSet<Option<Place>>,
    /// The units each piece of news about units counts.
    pub(crate) told_units: HashMap<Subject, Vec<Msg>>,
    /// The first message of the log that is about this game.
    pub(crate) log_start: EntryId,
    /// The alert the cursor last jumped to, and its square.
    pub(crate) alert_jump: Option<(EntryId, SquareId)>,
    /// Every objective of the player's seen at the end of a turn, in the
    /// order they came: one no longer among them is done.
    pub(crate) objectives_seen: Vec<Objective>,
    /// The step of the objectives said last.
    pub(crate) objective_at: Option<usize>,
    pub(crate) previous_resources: Option<Vec<i64>>,
    pub(crate) previous_available_food: i64,
    pub(crate) previous_used_food: i64,
    pub(crate) bell_enabled: bool,
    pub(crate) previous_nb_minutes: i64,
    pub(crate) must_play_tick: bool,
    pub(crate) average_turn_duration: f64,
    pub(crate) previous_update_time: Option<f64>,
    /// The loop ends at its next pass.
    pub end_loop: bool,
    pub(crate) already_asked_to_quit: bool,
    pub forced_quit: bool,
    pub(crate) terrain_noises: Vec<(SquareId, NoiseSource)>,
    pub(crate) previous_animation: f64,
    pub(crate) bindings: Bindings<GameCommand>,
    pub(crate) srv_queue: VecDeque<SrvEvent>,
    /// The commands given since the last turn, or in a game on a server,
    /// not yet sent.
    pub(crate) pending_commands: Vec<Command>,
    /// The order kinds of the menus, in menu order.
    pub(crate) orders_list: Vec<Keyword>,
}

impl GameInterface {
    /// The interface of `me` in the game of `host`, run here, from the
    /// start of the game and what `me` is told before the first turn. Says
    /// nothing: queued messages are dropped.
    pub fn local(cx: &mut Client, host: Host, me: PlayerId) -> Self {
        let seat = SeatView::new(host.game_start(), host.message(me));
        let local = Local {
            host: Box::new(host),
            next_update: 0.0,
            waiting_for_world_update: false,
            save: None,
            selection_to_restore: Vec::new(),
        };
        let speed = cx.config.speed;
        Self::with(cx, Source::Local(local), seat, speed)
    }

    /// The interface of a seat of a game a server runs, from what the
    /// seat was sent when it joined. Says nothing: queued messages are
    /// dropped.
    pub(crate) fn on_server(cx: &mut Client, seat: SeatView, server: server::ServerGame) -> Self {
        let speed = server.speed_factor();
        Self::with(cx, Source::Server(server), seat, speed)
    }

    fn with(cx: &mut Client, source: Source, seat: SeatView, speed: f64) -> Self {
        cx.voice.silent_flush();
        GameInterface {
            source,
            me: seat.player(),
            seat,
            speed,
            dobjets: IndexMap::new(),
            scouted_squares: BTreeSet::new(),
            scouted_before_squares: BTreeSet::new(),
            last_virtual_time: 0.0,
            turns_handled: 0,
            place: None,
            target: None,
            zoom: None,
            order: None,
            group: Vec::new(),
            immersion: false,
            follow_mode: false,
            x: 0.0,
            y: 0.0,
            o: 0.0,
            mode: Mode::Playing,
            previous_order: None,
            previous_group: None,
            previous_compass: None,
            previous_menus: IndexMap::new(),
            attacks: HashMap::new(),
            lost_units: Vec::new(),
            neutralized_units: Vec::new(),
            new_enemy_units: Vec::new(),
            scout_info: IndexSet::new(),
            known_resource_places: HashSet::new(),
            told_units: HashMap::new(),
            log_start: cx.voice.start_game(),
            alert_jump: None,
            objectives_seen: Vec::new(),
            objective_at: None,
            previous_resources: None,
            previous_available_food: 0,
            previous_used_food: 0,
            bell_enabled: false,
            previous_nb_minutes: 0,
            must_play_tick: false,
            average_turn_duration: 0.0,
            previous_update_time: None,
            end_loop: false,
            already_asked_to_quit: false,
            forced_quit: false,
            terrain_noises: Vec::new(),
            previous_animation: 0.0,
            bindings: Bindings::new(),
            srv_queue: VecDeque::new(),
            pending_commands: Vec::new(),
            orders_list: Vec::new(),
        }
    }

    /// What the player knows of the game.
    pub fn view(&self) -> &SeatView {
        &self.seat
    }

    /// Whether the game runs on a server.
    pub fn on_a_server(&self) -> bool {
        matches!(self.source, Source::Server(_))
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
            self.seat.install_end(end);
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
            self.seat.install(told);
        } else {
            self.me = told.seat;
            self.seat = SeatView::new(l.host.game_start(), told);
            self.objectives_seen.clear();
            self.objective_at = None;
        }
    }

    /// Gives a command to the host of a game run here, which carries out
    /// at once a cheat that changes the game between turns: what the
    /// player hears next is told by the host at once. A game on a server
    /// takes no cheat.
    pub(crate) fn queue_at_once(&mut self, command: Command) {
        let Source::Local(l) = &mut self.source else {
            return;
        };
        if let Some(told) = l.host.queue(self.me, command) {
            self.install_at_once(told);
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
    fn turn_due(&self) -> f64 {
        match &self.source {
            Source::Local(l) => l.next_update,
            Source::Server(s) => s.next_turn(),
            Source::Replay(r) => r.next_update(),
        }
    }

    /// The square width in meters.
    pub fn square_width(&self) -> f64 {
        self.view().square_width() as f64 / 1000.0
    }

    /// The order kinds for the menus, from the current style.
    pub fn update_orders_list(&mut self, cx: &Client) {
        self.orders_list = order::orders_list(cx.res.style(), &cx.log);
    }

    /// Reads the bindings of the game and the player's own.
    pub fn load_bindings(&mut self, cx: &Client) {
        let text = game_bindings_text(&cx.res, &cx.paths.custom_bindings());
        let mut b = Bindings::new();
        b.load(&text, &GameCommand::parse, &cx.log);
        self.bindings = b;
    }

    /// Gives a command of the player to the game at the next turn
    /// (`server.write_line`); in a game on a server, it goes to the server
    /// as soon as the server takes it. A player out of the game sends
    /// nothing. A server takes no cheat, and sets its speed only when the
    /// room's host asks (see [`GameInterface::server_update`]). While the
    /// connection is lost, a command is not taken, and the player hears it
    /// once; a watcher, or a listener to a record, gives none, and hears
    /// it once.
    pub fn send_command(&mut self, cx: &Client, command: Command) {
        if self.refuse_while_watching() {
            cx.log.info(format!("watching: didn't send {command:?}"));
        } else if !self.view().players().contains(&self.me) {
            cx.log.info(format!(
                "didn't send the order for player {}: {command:?}",
                self.me.0
            ));
        } else if self.on_a_server() && matches!(command, Command::Cheat(_)) {
            cx.log
                .warning(format!("a game on a server takes no cheat: {command:?}"));
        } else if self.refuse_while_returning() {
            cx.log
                .info(format!("not connected: didn't send {command:?}"));
        } else {
            self.pending_commands.push(command);
        }
    }

    // the loop

    /// Runs the game until it ends.
    pub fn run_loop(&mut self, cx: &mut Client) -> Result<(), Exit> {
        cx.input.clear();
        self.turn_due_at(cx.clock.now());
        self.end_loop = false;
        while !self.end_loop {
            self.step(cx)?;
        }
        Ok(())
    }

    /// One pass of the loop: a turn if one is due, the animations, the key
    /// presses, one server event, the voice, then a millisecond's rest.
    /// While the world waits, only the keys and the voice. Nothing said
    /// waits meanwhile ([`crate::voice::Voice::set_in_game`]).
    pub fn step(&mut self, cx: &mut Client) -> Result<(), Exit> {
        cx.voice.set_in_game(true);
        let result = self.try_step(cx);
        cx.voice.set_in_game(false);
        result?;
        cx.clock.sleep(0.001);
        Ok(())
    }

    fn try_step(&mut self, cx: &mut Client) -> Result<(), Exit> {
        crate::net::pump(cx);
        self.end_story_if_told(cx);
        let waits = self.world_waits();
        match self.source {
            Source::Server(_) => self.server_update(cx),
            Source::Replay(_) => self.replay_update(cx),
            Source::Local(_) => {
                if !waits && self.time_to_ask_for_next_update(cx) {
                    self.ask_for_update(cx);
                }
            }
        }
        // While returning, nothing is heard moving: the view is old; while
        // a listener jumps, or plays at the fastest pace, nothing is heard.
        if !waits && !self.returning() && !self.muted() {
            self.animate_objects(cx);
        }
        self.process_events(cx)?;
        if let Some(hook) = cx.game.hook.clone() {
            hook(self, cx);
        }
        if !self.world_waits() {
            self.process_srv_events(cx);
        }
        cx.voice.update();
        Ok(())
    }

    /// Goes back to playing once the story being told is told.
    fn end_story_if_told(&mut self, cx: &Client) {
        if let Mode::Story(s) = &self.mode
            && s.is_told(cx)
        {
            self.mode = Mode::Playing;
        }
    }

    /// Whether the world waits for the player: in single-player, while a
    /// story is told or the game menu or the console is open.
    pub fn world_waits(&self) -> bool {
        self.waits_for_the_player()
            && match self.mode {
                Mode::Playing | Mode::OrderShortcut | Mode::Chat(_) | Mode::KeyHelp => false,
                Mode::Story(_) | Mode::GameMenu(_) | Mode::Console(_) => true,
            }
    }

    /// Whether the world waits, on purpose, while the player reads a story
    /// or a menu: in a game run here, not on a server or in a record.
    pub(crate) fn waits_for_the_player(&self) -> bool {
        matches!(self.source, Source::Local(_))
    }

    /// Tells a story: each message as story news, said before the other
    /// news waiting. In single-player the world waits until every message
    /// has been said or skipped, and its sounds stop; in a game on a
    /// server, a record listened to, or while key help is on, nothing
    /// waits.
    pub(crate) fn tell_story(&mut self, cx: &mut Client, messages: Vec<Msg>) {
        let policy = cx.config.news.of(Category::Story);
        let story = Story(
            messages
                .into_iter()
                .filter_map(|m| cx.voice.news(News::new(Category::Story, m), policy))
                .collect(),
        );
        if !self.waits_for_the_player() || matches!(self.mode, Mode::KeyHelp) || story.is_told(cx) {
            return;
        }
        cx.stop_sound(false);
        match &mut self.mode {
            Mode::Story(s) => s.0.extend(story.0),
            Mode::Playing | Mode::OrderShortcut => self.mode = Mode::Story(story),
            // The world waits while these are open, and chat is only typed
            // on a server: no story comes meanwhile.
            Mode::GameMenu(_) | Mode::Chat(_) | Mode::Console(_) | Mode::KeyHelp => {}
        }
    }

    fn time_to_ask_for_next_update(&self, cx: &Client) -> bool {
        match &self.source {
            Source::Local(l) => !l.waiting_for_world_update && cx.clock.now() >= l.next_update,
            Source::Server(_) | Source::Replay(_) => false,
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

    /// Takes a turn, here or from a server: its message is current from
    /// now on, and what happened waits in the queue: the turn's events,
    /// its end, then what followed when the turn ended the game.
    fn take_turn(
        &mut self,
        message: Arc<TurnMessage>,
        end_of_turn: Option<Arc<TurnMessage>>,
        then: Vec<Event>,
    ) {
        self.seat.install(message);
        if let Some(end) = end_of_turn {
            self.follow_replaced_in_commands(&end);
            let events = end.events.iter().cloned().filter_map(SrvEvent::of);
            self.srv_queue.extend(events);
            self.srv_queue.push_back(SrvEvent::EndOfTurn(end));
        }
        self.srv_queue
            .extend(then.into_iter().filter_map(SrvEvent::of));
    }

    /// How far the game has got.
    pub fn progress(&self) -> GameProgress {
        GameProgress {
            turn: self.view().turn(),
            events_waiting: !self.srv_queue.is_empty(),
            turns_handled: self.turns_handled,
        }
    }

    /// Handles the key presses waiting.
    fn process_events(&mut self, cx: &mut Client) -> Result<(), Exit> {
        cx.input.set_game(Some(self.progress()));
        cx.input
            .input_point(self.mode.input_point(), cx.voice.is_idle());
        for e in cx.input.get_all() {
            match e {
                InputEvent::Quit => return Err(Exit),
                InputEvent::KeyDown(k) => self.route_key(cx, k)?,
                InputEvent::KeyUp(_) => {}
            }
        }
        Ok(())
    }

    /// Gives a key press to the mode, which may change.
    fn route_key(&mut self, cx: &mut Client, k: KeyDown) -> Result<(), Exit> {
        // The key before may have skipped the story's last message.
        self.end_story_if_told(cx);
        match std::mem::replace(&mut self.mode, Mode::Playing) {
            Mode::Playing => self.process_keydown(cx, &k),
            Mode::OrderShortcut => {
                self.execute_order_shortcut(cx, &k);
                Ok(())
            }
            Mode::Story(s) => {
                self.mode = Mode::Story(s);
                self.story_key(cx, &k)
            }
            Mode::GameMenu(gm) => self.game_menu_key(cx, gm, k),
            Mode::Chat(chat) => {
                self.chat_key(cx, chat, &k);
                Ok(())
            }
            Mode::Console(entry) => {
                self.console_key(cx, entry, &k);
                Ok(())
            }
            Mode::KeyHelp => {
                self.key_help_key(cx, &k);
                Ok(())
            }
        }
    }

    /// Turns key help on.
    pub(crate) fn start_key_help(&mut self, cx: &mut Client) {
        self.mode = Mode::KeyHelp;
        cx.voice.item(&msg(Phrase::KeyHelpOn));
    }

    /// A key pressed while key help is on: Escape, or the key that turns
    /// key help on, turns it off; any other key or chord says its name and
    /// what it does, without doing it. A modifier pressed alone comes when
    /// it is released, as it acts then.
    fn key_help_key(&mut self, cx: &mut Client, k: &KeyDown) {
        let chord = Chord::of(k);
        let command = self.bindings.get(&chord);
        let escape = Chord {
            mods: Mods::NONE,
            key: Key::Escape,
        };
        if chord == escape || command == Some(&GameCommand::KeyHelp) {
            cx.voice.item(&msg(Phrase::KeyHelpOff));
            return;
        }
        self.mode = Mode::KeyHelp;
        let what = command.map_or_else(
            || msg(Phrase::NoCommand),
            |c| vec![MsgPart::literal(c.description())],
        );
        let key = vec![MsgPart::literal(format!("{}:", chord.spoken()))];
        cx.voice.item(&[key, what].concat());
    }

    /// A key pressed while a story is told: the keys of the game menu,
    /// the history, the news, copying a message, help, key help and the
    /// volume do as in the game; the escape key skips the rest of the story; any other key
    /// skips the message being said.
    fn story_key(&mut self, cx: &mut Client, k: &KeyDown) -> Result<(), Exit> {
        use GameCommand as C;
        match self.bindings.command_for(k).cloned() {
            Some(C::Escape) => {
                if let Mode::Story(s) = &self.mode {
                    s.skip(cx);
                }
                Ok(())
            }
            Some(
                c @ (C::GameMenu
                | C::HistoryPrevious
                | C::HistoryNext
                | C::Unheard
                | C::ReviewPrevious
                | C::ReviewNext
                | C::ReviewCategory
                | C::CopyMessage
                | C::Help(_)
                | C::KeyHelp
                | C::Volume(_)),
            ) => self.execute(cx, &c),
            _ => {
                cx.voice.say_next(false);
                Ok(())
            }
        }
    }

    fn process_keydown(&mut self, cx: &mut Client, k: &KeyDown) -> Result<(), Exit> {
        match self.bindings.command_for(k).cloned() {
            Some(c) => self.execute(cx, &c),
            // A key bound to nothing.
            None => {
                cx.voice.item(&cue(Cue::Beep));
                Ok(())
            }
        }
    }

    fn process_srv_events(&mut self, cx: &mut Client) {
        if let Some(e) = self.srv_queue.pop_front() {
            self.process_srv_event(cx, e);
        }
    }

    /// Handles a server event now.
    pub(crate) fn process_srv_event(&mut self, cx: &mut Client, e: SrvEvent) {
        match e {
            SrvEvent::Entity { entity, event } => self.srv_event(cx, *entity, &event),
            SrvEvent::VoiceImportant(m) => cx.news(News::new(important_category(&m), m)),
            SrvEvent::Sequence(parts) => {
                let story = parts.into_iter().map(|part| vec![part]).collect();
                self.tell_story(cx, story);
            }
            // On a server, the server keeps the time; in a record, the
            // listener does.
            SrvEvent::Speed(s) => {
                if self.waits_for_the_player() {
                    self.speed = s;
                }
            }
            SrvEvent::Quit => self.quit_heard(cx),
            SrvEvent::EndOfTurn(m) => self.srv_end_of_turn(cx, &m),
        }
    }

    fn srv_event(&mut self, cx: &mut Client, entity: KnownEntity, event: &EntityEvent) {
        let timing = if cx.clock.now() > self.turn_due() + EVENT_LIMIT {
            Timing::Late
        } else {
            Timing::OnTime
        };
        let view = EntityView::new(entity);
        self.notify(cx, &view, event, timing);
    }

    /// The game is over for the player: the sounds stop, and of the news
    /// waiting, only what the end of a game is about is still said.
    pub(crate) fn srv_quit(&mut self, cx: &mut Client) {
        cx.voice.end_game();
        cx.stop_sound(false);
        self.end_loop = true;
    }

    /// The end of a turn: the interface's copies of the squares and the
    /// entities take the turn's, and the seat takes note of what it sees.
    fn srv_end_of_turn(&mut self, cx: &mut Client, m: &TurnMessage) {
        self.turns_handled += 1;
        self.last_virtual_time = m.time as f64 / 1000.0;
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
        self.scouted_squares = m.squares.observed.clone();
        self.scouted_before_squares = m.squares.observed_before.clone();
        self.seat.remember();
        self.note_objectives(&m.state.objectives);
        self.send_resource_alerts_if_needed(cx);
        if self.previous_menus.is_empty() {
            self.send_menu_alerts_if_needed(cx);
        }
        self.units_alert_if_needed(cx, None);
        self.scout_info_if_needed(cx);
        self.follow_replaced_units(m);
        self.update_fog_of_war(cx, m.remembered.clone(), m.perceived.clone());
        self.update_group();
        if self.bell_enabled && !self.muted() {
            self.eventually_play_bell(cx);
        }
        if self.must_play_tick && !self.muted() {
            self.play_tick(cx);
        }
        self.record_update_time(cx);
    }

    // animation

    fn animate_objects(&mut self, cx: &mut Client) {
        let now = cx.clock.now();
        if now < self.previous_animation + cx.config.parameters.animation_delay {
            return;
        }
        self.set_obs_pos(cx);
        let ids: Vec<EntityId> = self.dobjets.keys().copied().collect();
        for id in ids {
            self.animate(cx, id);
        }
        self.animate_terrain(cx);
        self.previous_animation = cx.clock.now();
    }

    /// Places the listener and moves the sounds with it.
    pub(crate) fn update_listener(&self, cx: &Client) {
        cx.audio.set_listener(Listener {
            x: self.x,
            y: self.y,
            o: self.o,
            immersion: self.immersion,
        });
        cx.audio.update();
    }

    // players and objects

    /// Whether the entity belongs to an enemy of the player.
    pub fn is_an_enemy(&self, e: &KnownEntity) -> bool {
        e.player.is_some_and(|p| !self.view().is_ally(p))
    }

    /// Whether `p` is the player or an ally.
    pub fn is_allied(&self, p: Option<PlayerId>) -> bool {
        p.is_some_and(|p| self.view().is_ally(p))
    }

    /// The title of a place: the square's, or the container's type title.
    /// Nothing for a container the player knows nothing of.
    pub fn place_title(&self, cx: &Client, place: Place) -> Msg {
        match place {
            Place::Square(s) => self.view().square(s).title().clone(),
            Place::Inside(c) => self
                .view()
                .known(c)
                .map(|e| style_msg(cx.res.style(), &e.type_name, "title"))
                .unwrap_or_default(),
        }
    }

    /// The place of a container, as the player perceives or remembers it.
    /// `None` if the player knows nothing of it.
    pub fn container_place(&self, container: EntityId) -> Option<Place> {
        self.view().known(container).and_then(|e| e.place)
    }

    /// The square of a place, through containers as the player knows
    /// them.
    pub fn outer_square(&self, place: Option<Place>) -> Option<SquareId> {
        match place? {
            Place::Square(s) => Some(s),
            Place::Inside(c) => self.outer_square(self.container_place(c)),
        }
    }

    /// The objective of the map, if any, as said when the game starts.
    pub fn objective(&self) -> Option<Msg> {
        let objective = self.view().map_objective();
        (!objective.is_empty()).then(|| {
            let mut m = msg(Phrase::Objective);
            m.extend(objective.iter().cloned());
            m
        })
    }

    /// Whether server events wait to be handled.
    pub fn has_pending_server_events(&self) -> bool {
        !self.srv_queue.is_empty()
    }

    /// The commands not yet given to the host, or in a game on a server,
    /// not yet sent to the server.
    pub fn pending_commands(&self) -> &[Command] {
        &self.pending_commands
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

    /// Makes the next turn of a game run here due at `at`.
    #[cfg(any(test, feature = "testkit"))]
    pub fn set_turn_due(&mut self, at: f64) {
        self.turn_due_at(at);
    }

    /// Stops every noise of the game and forgets it: the next animation
    /// starts each again where its object then is.
    pub(crate) fn silence_noises(&mut self, cx: &Client) {
        for v in self.dobjets.values_mut() {
            if let Some(n) = v.anim.noise.take() {
                n.stop(&cx.audio);
            }
        }
        for (_, n) in std::mem::take(&mut self.terrain_noises) {
            n.stop(&cx.audio);
        }
    }

    /// Stops every noise of the game.
    pub fn stop_noises(&mut self, cx: &Client) {
        for v in self.dobjets.values() {
            v.stop(&cx.audio);
        }
        for (_, n) in &self.terrain_noises {
            n.stop(&cx.audio);
        }
    }
}
