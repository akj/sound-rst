//! The game interface (`clientgame.py`): what the player hears of a game
//! and what the keys do.
//!
//! The interface owns the game's [`Host`] and gives it the player's
//! commands. It reads the game through the host's view of what its player
//! knows ([`GameInterface::view`]), never the world itself. Its loop steps
//! the host when a turn is due (every 300 ms
//! divided by the speed), then reads what happened as the original's
//! server events: entity events, messages, and a `voila` with the
//! perception, memory and observed squares at the end of the turn. One
//! server event is handled per pass of the loop, after the key presses,
//! as in the original.
//!
//! Nothing said stops the loop. What the keys do depends on one [`Mode`].
//! In single-player the world waits, on purpose, while a story is told
//! and while the game menu or the console is open: the host is not
//! stepped, its events wait and nothing moves, but the keys and the voice
//! go on. In a game on a server the world never waits.
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
pub mod lockstep;
pub mod misc;
pub mod news;
pub mod order;
pub mod questions;
pub mod selection;
pub mod squares;
pub mod stats;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use indexmap::{IndexMap, IndexSet};
use soundrts_world::events::{EntityEvent, Event};
use soundrts_world::msg::{Msg, MsgPart, msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;
use soundrts_world::{
    Command, EntityId, Host, KnownEntity, Place, PlayerId, SquareId, View, Voila,
};

use crate::bindings::{Bindings, game_bindings_text};
use crate::client::Client;
use crate::input::{Event as InputEvent, GameProgress, InputPoint, KeyDown};
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
pub const BEEP_SOUND: &str = "1029";
pub const POSITIONAL_BEEP_SOUND: &str = "1003";

/// Whether an entity event reaches the interface in time for its sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    OnTime,
    /// More than [`EVENT_LIMIT`] seconds late: a sound would be noise.
    Late,
}

/// An error the original raises and catches in its loop
/// (`exception("error in clientgame loop")`): the rest of the pass is
/// skipped and the game goes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Oops {
    pub message: String,
    /// The error is a lookup that found nothing (the original's
    /// `KeyError`). Raised by a command, it is a beep, not a logged error.
    pub missing: bool,
}

impl Oops {
    pub fn new(s: impl Into<String>) -> Oops {
        Oops {
            message: s.into(),
            missing: false,
        }
    }

    /// A lookup of `what` that found nothing.
    pub fn missing(what: impl std::fmt::Display) -> Oops {
        Oops {
            message: format!("KeyError: {what}"),
            missing: true,
        }
    }
}

/// The result of interface work that may fail as the original's does.
pub type R<T = ()> = Result<T, Oops>;

/// Why a pass of the loop stopped early.
pub(crate) enum Stop {
    /// The window was closed.
    Exit,
    Oops(Oops),
}

impl From<Oops> for Stop {
    fn from(o: Oops) -> Stop {
        Stop::Oops(o)
    }
}

impl From<Exit> for Stop {
    fn from(_: Exit) -> Stop {
        Stop::Exit
    }
}

/// What the world tells the interface, in order.
#[derive(Clone, Debug)]
pub enum SrvEvent {
    Entity {
        entity: Box<KnownEntity>,
        event: EntityEvent,
    },
    VoiceImportant(Msg),
    Sequence(Vec<String>),
    Speed(f64),
    Quit,
    /// The turn is over: what the player knows at its end.
    Voila(Box<Voila>),
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
}

impl Mode {
    /// Where the keys of this mode are read.
    fn input_point(&self) -> InputPoint {
        match self {
            Mode::Playing | Mode::OrderShortcut | Mode::Story(_) => InputPoint::Game,
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
}

/// The category of a message the world says now. Objectives and the end of
/// a mission are the story; the rest (players defeated or leaving, answers
/// to the console) is about the game itself.
fn important_category(m: &Msg) -> Category {
    let story = [
        mp::NEW_OBJECTIVE,
        mp::OBJECTIVE_COMPLETE,
        mp::MISSION_COMPLETE,
    ];
    if story.iter().any(|s| m.starts_with(&msg(s))) {
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
        .into_iter()
        .map(MsgPart::Text)
        .collect()
}

/// As [`style_msg`], without the warning.
pub fn style_msg_quiet(style: &Style, obj: &str, attr: &str) -> Msg {
    match style.get_or_warn(obj, attr, false) {
        Some(StyleValue::Words(w)) | Some(StyleValue::IfMe(w, _)) => {
            w.iter().cloned().map(MsgPart::Text).collect()
        }
        _ => Vec::new(),
    }
}

/// The title of a type: its style `title` as sound numbers.
pub fn compute_title(style: &Style, type_name: &str) -> Msg {
    style
        .words(type_name, "title")
        .iter()
        .filter_map(|w| w.parse::<u64>().ok())
        .map(MsgPart::Num)
        .collect()
}

/// The interface of a running game.
pub struct GameInterface {
    /// The game.
    pub host: Host,
    /// The player this interface plays.
    pub me: PlayerId,
    /// The game speed: 1 is normal.
    pub speed: f64,
    /// What the player perceives or remembers, by id, in arrival order.
    pub dobjets: IndexMap<EntityId, EntityView>,
    /// The squares observed now.
    pub scouted_squares: BTreeSet<SquareId>,
    /// The squares observed before (all of them in cheat mode).
    pub scouted_before_squares: BTreeSet<SquareId>,
    /// The game time of the last turn, in seconds.
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
    pub(crate) previous_menus: IndexMap<String, Vec<String>>,
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
    pub(crate) previous_resources: Option<Vec<i64>>,
    pub(crate) previous_available_food: i64,
    pub(crate) previous_used_food: i64,
    pub(crate) bell_enabled: bool,
    pub(crate) previous_nb_minutes: i64,
    pub(crate) must_play_tick: bool,
    pub(crate) average_turn_duration: f64,
    pub(crate) previous_update_time: Option<f64>,
    /// When the next turn is due.
    pub next_update: f64,
    pub(crate) waiting_for_world_update: bool,
    /// The loop ends at its next pass.
    pub end_loop: bool,
    pub(crate) already_asked_to_quit: bool,
    pub forced_quit: bool,
    pub(crate) terrain_noises: Vec<(SquareId, NoiseSource)>,
    /// The terrain of each square when the player last observed it.
    pub(crate) seen_terrain: HashMap<SquareId, String>,
    /// The upgrades of each known creature when the player last perceived
    /// it.
    pub(crate) seen_upgrades: HashMap<EntityId, Vec<String>>,
    pub(crate) previous_animation: f64,
    pub(crate) bindings: Bindings<GameCommand>,
    pub(crate) srv_queue: VecDeque<SrvEvent>,
    /// The commands given since the last turn, or in a game on a server,
    /// not yet sent.
    pub(crate) pending_commands: Vec<Command>,
    /// The order kinds of the menus, in menu order.
    pub(crate) orders_list: Vec<Keyword>,
    /// In a game on a server: what runs the turns the server sends.
    pub net: Option<lockstep::NetDriver>,
}

impl GameInterface {
    /// The interface of `me` in the game of `host`. Says nothing: queued
    /// messages are dropped.
    pub fn new(cx: &mut Client, host: Host, me: PlayerId) -> Self {
        cx.voice.silent_flush();
        GameInterface {
            host,
            me,
            speed: cx.config.speed,
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
            previous_resources: None,
            previous_available_food: 0,
            previous_used_food: 0,
            bell_enabled: false,
            previous_nb_minutes: 0,
            must_play_tick: false,
            average_turn_duration: 0.0,
            previous_update_time: None,
            next_update: 0.0,
            waiting_for_world_update: false,
            end_loop: false,
            already_asked_to_quit: false,
            forced_quit: false,
            terrain_noises: Vec::new(),
            seen_terrain: HashMap::new(),
            seen_upgrades: HashMap::new(),
            previous_animation: 0.0,
            bindings: Bindings::new(),
            srv_queue: VecDeque::new(),
            pending_commands: Vec::new(),
            orders_list: Vec::new(),
            net: None,
        }
    }

    /// What the player knows of the game.
    pub fn view(&self) -> View<'_> {
        self.host.view(self.me)
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
    /// with the next orders sent. A player out of the game sends nothing,
    /// and a game on a server takes no command without a text line (a
    /// cheat the host carries out itself).
    pub fn send_command(&mut self, cx: &Client, command: Command) {
        if !self.view().players().contains(&self.me) {
            cx.log.info(format!(
                "didn't send the order for player {}: {command:?}",
                self.me.0
            ));
        } else if self.net.is_some() && self.host.line(&command).is_none() {
            cx.log
                .warning(format!("a game on a server cannot send {command:?}"));
        } else {
            self.pending_commands.push(command);
        }
    }

    // the loop

    /// Runs the game until it ends.
    pub fn run_loop(&mut self, cx: &mut Client) -> Result<(), Exit> {
        cx.input.clear();
        self.next_update = cx.clock.now();
        self.end_loop = false;
        while !self.end_loop {
            self.step(cx)?;
        }
        Ok(())
    }

    /// One pass of the loop: a turn if one is due, the animations, the key
    /// presses, one server event, the voice, then a millisecond's rest.
    /// While the world waits, only the keys and the voice. Nothing said
    /// waits meanwhile ([`crate::voice::Voice::set_in_game`]). Errors are
    /// logged, as the original's loop catches them, and cut the pass
    /// short, rest included.
    pub fn step(&mut self, cx: &mut Client) -> Result<(), Exit> {
        cx.voice.set_in_game(true);
        let result = self.try_step(cx);
        cx.voice.set_in_game(false);
        match result {
            Ok(()) => cx.clock.sleep(0.001),
            Err(Stop::Exit) => return Err(Exit),
            Err(Stop::Oops(e)) => cx
                .log
                .error(format!("error in clientgame loop: {}", e.message)),
        }
        Ok(())
    }

    fn try_step(&mut self, cx: &mut Client) -> Result<(), Stop> {
        crate::net::pump(cx);
        if let Mode::Story(s) = &self.mode
            && s.is_told(cx)
        {
            self.mode = Mode::Playing;
        }
        let waits = self.world_waits();
        if self.net.is_some() {
            self.net_update(cx);
        } else if !waits && self.time_to_ask_for_next_update(cx) {
            self.ask_for_update(cx);
        }
        if !waits {
            self.animate_objects(cx);
        }
        self.process_events(cx)?;
        if let Some(hook) = cx.game.hook.clone() {
            hook(self, cx);
        }
        if !self.world_waits() {
            self.process_srv_events(cx)?;
        }
        cx.voice.update();
        Ok(())
    }

    /// Whether the world waits for the player: in single-player, while a
    /// story is told or the game menu or the console is open.
    pub fn world_waits(&self) -> bool {
        self.net.is_none()
            && match self.mode {
                Mode::Playing | Mode::OrderShortcut | Mode::Chat(_) => false,
                Mode::Story(_) | Mode::GameMenu(_) | Mode::Console(_) => true,
            }
    }

    /// Tells a story: each message as story news, said before the other
    /// news waiting. In single-player the world waits until every message
    /// has been said or skipped, and its sounds stop; in a game on a
    /// server, nothing waits.
    pub(crate) fn tell_story(&mut self, cx: &mut Client, messages: Vec<Msg>) {
        let policy = cx.config.news.of(Category::Story);
        let story = Story(
            messages
                .into_iter()
                .filter_map(|m| cx.voice.news(News::new(Category::Story, m), policy))
                .collect(),
        );
        if self.net.is_some() || story.is_told(cx) {
            return;
        }
        cx.stop_sound(false);
        match &mut self.mode {
            Mode::Story(s) => s.0.extend(story.0),
            Mode::Playing | Mode::OrderShortcut => self.mode = Mode::Story(story),
            // The world waits while these are open, and chat is only typed
            // on a server: no story comes meanwhile.
            Mode::GameMenu(_) | Mode::Chat(_) | Mode::Console(_) => {}
        }
    }

    fn time_to_ask_for_next_update(&self, cx: &Client) -> bool {
        !self.waiting_for_world_update && cx.clock.now() >= self.next_update
    }

    /// Gives the pending commands to the host as the player's, runs a
    /// turn, and queues what happened.
    fn ask_for_update(&mut self, cx: &mut Client) {
        for c in std::mem::take(&mut self.pending_commands) {
            self.host.queue(self.me, c);
        }
        self.next_update = cx.clock.now() + TURN / self.speed;
        self.run_turn(cx);
    }

    /// Runs a turn of the game with the commands queued, and queues what
    /// happened.
    pub(crate) fn run_turn(&mut self, cx: &mut Client) {
        self.waiting_for_world_update = true;
        let report = self.host.step(self.me);
        for w in report.warnings {
            cx.log.warning(format!("world: {w}"));
        }
        for e in report.events {
            self.srv_queue.push_back(match e {
                Event::Entity { entity, event } => SrvEvent::Entity { entity, event },
                Event::VoiceImportant(m) => SrvEvent::VoiceImportant(m),
                Event::Sequence(s) => SrvEvent::Sequence(s),
                Event::Speed(s) => SrvEvent::Speed(s),
                Event::Quit => SrvEvent::Quit,
                Event::Voila(v) => SrvEvent::Voila(v),
            });
        }
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
    fn process_events(&mut self, cx: &mut Client) -> Result<(), Stop> {
        cx.input.set_game(Some(self.progress()));
        cx.input
            .input_point(self.mode.input_point(), cx.voice.is_idle());
        for e in cx.input.get_all() {
            match e {
                InputEvent::Quit => return Err(Stop::Exit),
                InputEvent::KeyDown(k) => self.route_key(cx, k)?,
                InputEvent::KeyUp(_) => {}
            }
        }
        Ok(())
    }

    /// Gives a key press to the mode, which may change.
    fn route_key(&mut self, cx: &mut Client, k: KeyDown) -> Result<(), Stop> {
        match std::mem::replace(&mut self.mode, Mode::Playing) {
            Mode::Playing => self.process_keydown(cx, &k),
            Mode::OrderShortcut => Ok(self.execute_order_shortcut(cx, &k)?),
            Mode::Story(s) => {
                self.mode = Mode::Story(s);
                self.story_key(cx, &k)
            }
            Mode::GameMenu(gm) => self.game_menu_key(cx, gm, k),
            Mode::Chat(chat) => {
                self.chat_key(cx, chat, &k);
                Ok(())
            }
            Mode::Console(entry) => Ok(self.console_key(cx, entry, &k)?),
        }
    }

    /// A key pressed while a story is told: the keys of the game menu,
    /// the history, the news, help and the volume do as in the game; any
    /// other key skips the message being said.
    fn story_key(&mut self, cx: &mut Client, k: &KeyDown) -> Result<(), Stop> {
        use GameCommand as C;
        match self.bindings.command_for(k).cloned() {
            Some(
                c @ (C::GameMenu
                | C::HistoryPrevious
                | C::HistoryNext
                | C::Unheard
                | C::ReviewPrevious
                | C::ReviewNext
                | C::ReviewCategory
                | C::Help(_)
                | C::Volume(_)),
            ) => self.execute(cx, &c),
            _ => {
                cx.voice.say_next(false);
                Ok(())
            }
        }
    }

    fn process_keydown(&mut self, cx: &mut Client, k: &KeyDown) -> Result<(), Stop> {
        let result = match self.bindings.command_for(k).cloned() {
            Some(c) => self.execute(cx, &c),
            None => Err(Stop::Oops(Oops::missing("binding"))),
        };
        match result {
            Err(Stop::Oops(e)) if e.missing => {
                cx.voice.item(&msg(mp::BEEP));
                Ok(())
            }
            other => other,
        }
    }

    fn process_srv_events(&mut self, cx: &mut Client) -> Result<(), Stop> {
        if let Some(e) = self.srv_queue.pop_front() {
            self.process_srv_event(cx, e)?;
        }
        Ok(())
    }

    /// Handles a server event now.
    pub(crate) fn process_srv_event(&mut self, cx: &mut Client, e: SrvEvent) -> Result<(), Stop> {
        match e {
            SrvEvent::Entity { entity, event } => self.srv_event(cx, *entity, &event),
            SrvEvent::VoiceImportant(m) => cx.news(News::new(important_category(&m), m)),
            SrvEvent::Sequence(names) => {
                let story = names
                    .iter()
                    .map(|n| vec![MsgPart::from(n.as_str())])
                    .collect();
                self.tell_story(cx, story);
            }
            // In a game on a server, the server keeps the time.
            SrvEvent::Speed(s) => {
                if self.net.is_none() {
                    self.speed = s;
                }
            }
            SrvEvent::Quit => self.srv_quit(cx),
            SrvEvent::Voila(v) => self.srv_voila(cx, *v)?,
        }
        Ok(())
    }

    fn srv_event(&mut self, cx: &mut Client, entity: KnownEntity, event: &EntityEvent) {
        let timing = if cx.clock.now() > self.next_update + EVENT_LIMIT {
            Timing::Late
        } else {
            Timing::OnTime
        };
        let view = EntityView::new(entity, &cx.rng);
        if let Err(Oops { message: e, .. }) = self.notify(cx, &view, event, timing) {
            cx.log.error(format!("problem during srv_event: {e}"));
        }
    }

    /// The game is over for the player: the sounds stop, and of the news
    /// waiting, only what the end of a game is about is still said.
    pub(crate) fn srv_quit(&mut self, cx: &mut Client) {
        cx.voice.end_game();
        cx.stop_sound(false);
        self.end_loop = true;
    }

    fn srv_voila(&mut self, cx: &mut Client, v: Voila) -> R {
        self.turns_handled += 1;
        self.last_virtual_time = v.time as f64 / 1000.0;
        self.waiting_for_world_update = false;
        self.scouted_squares = v.observed_squares;
        self.scouted_before_squares = v.observed_before_squares;
        self.note_terrain();
        self.send_resource_alerts_if_needed(cx);
        if self.previous_menus.is_empty() {
            self.send_menu_alerts_if_needed(cx);
        }
        self.units_alert_if_needed(cx, None);
        self.scout_info_if_needed(cx);
        self.update_fog_of_war(cx, v.memory, v.perception);
        self.note_upgrades();
        self.update_group();
        if self.bell_enabled {
            self.eventually_play_bell(cx);
        }
        if self.must_play_tick {
            self.play_tick(cx);
        }
        self.record_update_time(cx);
        Ok(())
    }

    // animation

    fn animate_objects(&mut self, cx: &mut Client) {
        let now = cx.clock.now();
        if now < self.previous_animation + cx.params.number("animation_delay", 0.1) {
            return;
        }
        if let Err(Oops { message: e, .. }) = self.set_obs_pos(cx) {
            cx.log
                .error(format!("couldn't set user interface position: {e}"));
        }
        let ids: Vec<EntityId> = self.dobjets.keys().copied().collect();
        for id in ids {
            if let Err(Oops { message: e, .. }) = self.animate(cx, id) {
                cx.log.error(format!("couldn't animate object: {e}"));
            }
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
            let mut m = msg(mp::OBJECTIVE);
            m.extend(objective.iter().map(|&n| MsgPart::Num(n as u64)));
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
