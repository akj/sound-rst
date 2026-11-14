//! The game interface (`clientgame.py`): what the player hears of a game
//! and what the keys do.
//!
//! The interface owns the [`World`]. Its loop asks the world for a turn
//! when one is due (every 300 ms divided by the speed), after giving it
//! the commands written since the last turn, then reads what happened as
//! the original's server events: entity events, messages, and a `voila`
//! with the perception, memory and observed squares at the end of the
//! turn. One server event is handled per pass of the loop, after the key
//! presses, as in the original.
//!
//! The submodules follow the original's files and sections: [`entity`]
//! (`clientgameentity.py`), [`focus`] (`clientgamefocus.py`), [`news`],
//! [`order`] (`clientgameorder.py`), [`animation`], [`stats`]
//! (`worldplayerstats.score_msgs`), [`command`] (the bindings' commands),
//! and the interface's own parts: [`squares`], [`selection`], [`alerts`]
//! and [`misc`].

pub mod alerts;
pub mod animation;
pub mod command;
pub mod entity;
pub mod focus;
pub mod lockstep;
pub mod misc;
pub mod news;
pub mod order;
pub mod selection;
pub mod squares;
pub mod stats;

use std::collections::{BTreeSet, HashSet, VecDeque};

use indexmap::{IndexMap, IndexSet};
use soundrts_world::events::{EntityEvent, Event};
use soundrts_world::msg::{Msg, MsgPart, msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;
use soundrts_world::{Entity, EntityId, Place, PlayerId, SquareId, Voila, World};

use crate::bindings::{Bindings, game_bindings_text};
use crate::client::Client;
use crate::input::{Event as InputEvent, GameProgress, InputPoint, KeyDown};
use crate::menu::{Exit, TextEntry, Typed};
use crate::sound::Listener;
use crate::style::{Style, StyleValue};

use self::animation::NoiseSource;
use self::command::GameCommand;
use self::entity::EntityView;
use self::focus::Zoom;
use self::order::OrderTypeView;

/// Seconds of game time in a turn.
pub const TURN: f64 = 0.3;
/// The same alert sound is not played again within this many seconds.
pub const ALERT_LIMIT: f64 = 0.5;
/// Entity events this many seconds late are dropped.
pub const EVENT_LIMIT: f64 = 3.0;
pub const BEEP_SOUND: &str = "1029";
pub const POSITIONAL_BEEP_SOUND: &str = "1003";

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
        entity: Box<Entity>,
        event: EntityEvent,
    },
    VoiceImportant(Msg),
    Sequence(Vec<String>),
    Speed(f64),
    Quit,
    /// The turn is over: what the player knows at its end.
    Voila(Box<Voila>),
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
    pub world: World,
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
    pub(crate) shortcut_mode: bool,
    pub(crate) previous_order: Option<OrderTypeView>,
    pub(crate) previous_group: Option<Vec<EntityId>>,
    pub(crate) previous_compass: Option<Msg>,
    pub(crate) previous_menus: IndexMap<String, Vec<String>>,
    pub(crate) alert_squares: IndexMap<Place, f64>,
    pub(crate) lost_units: Vec<(Msg, Option<Place>)>,
    pub(crate) neutralized_units: Vec<(Msg, Option<Place>)>,
    pub(crate) new_enemy_units: Vec<(Msg, Option<Place>)>,
    pub(crate) scout_info: IndexSet<Place>,
    pub(crate) known_resource_places: HashSet<Option<Place>>,
    pub(crate) previous_unit_attacked_alert: Option<f64>,
    pub(crate) previous_scout_info: Option<f64>,
    pub(crate) previous_units_alert: Option<f64>,
    pub(crate) previous_squares_alert: Option<f64>,
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
    pub(crate) previous_animation: f64,
    pub(crate) bindings: Bindings<GameCommand>,
    pub(crate) srv_queue: VecDeque<SrvEvent>,
    /// The commands written since the last turn.
    pub(crate) pending_commands: Vec<String>,
    /// The order kinds of the menus, in menu order.
    pub(crate) orders_list: Vec<Keyword>,
    /// In a game on a server: what runs the turns the server sends.
    pub net: Option<lockstep::NetDriver>,
    /// A chat line being typed, and who it is for.
    pub(crate) chat_entry: Option<(soundrts_protocol::Channel, TextEntry)>,
    /// The game menu, while it is open in a game on a server.
    pub(crate) game_menu: Option<misc::GameMenu>,
}

impl GameInterface {
    /// The interface of `me` in `world`. Says nothing: queued messages are
    /// dropped.
    pub fn new(cx: &mut Client, world: World, me: PlayerId) -> Self {
        cx.voice.silent_flush();
        GameInterface {
            world,
            me,
            speed: cx.config.speed as f64,
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
            shortcut_mode: false,
            previous_order: None,
            previous_group: None,
            previous_compass: None,
            previous_menus: IndexMap::new(),
            alert_squares: IndexMap::new(),
            lost_units: Vec::new(),
            neutralized_units: Vec::new(),
            new_enemy_units: Vec::new(),
            scout_info: IndexSet::new(),
            known_resource_places: HashSet::new(),
            previous_unit_attacked_alert: None,
            previous_scout_info: None,
            previous_units_alert: None,
            previous_squares_alert: None,
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
            previous_animation: 0.0,
            bindings: Bindings::new(),
            srv_queue: VecDeque::new(),
            pending_commands: Vec::new(),
            orders_list: Vec::new(),
            net: None,
            chat_entry: None,
            game_menu: None,
        }
    }

    /// The square width in meters.
    pub fn square_width(&self) -> f64 {
        self.world.square_width() as f64 / 1000.0
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

    /// Sends a command to the world at the next turn, for this player
    /// (`server.write_line`).
    pub fn write_line(&mut self, cx: &Client, s: &str) {
        if self.world.players.contains(&self.me) {
            self.pending_commands.push(s.to_owned());
        } else {
            cx.log.info(format!(
                "didn't send the order for player {}: {s}",
                self.me.0
            ));
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
    /// Errors are logged, as the original's loop catches them, and cut the
    /// pass short, rest included.
    pub fn step(&mut self, cx: &mut Client) -> Result<(), Exit> {
        match self.try_step(cx) {
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
        if self.net.is_some() {
            self.net_update(cx);
        } else if self.time_to_ask_for_next_update(cx) {
            self.ask_for_update(cx);
        }
        self.animate_objects(cx);
        self.process_events(cx)?;
        if let Some(hook) = cx.game.hook.clone() {
            hook(self, cx);
        }
        self.process_srv_events(cx)?;
        cx.voice.update();
        Ok(())
    }

    fn time_to_ask_for_next_update(&self, cx: &Client) -> bool {
        !self.waiting_for_world_update && cx.clock.now() >= self.next_update
    }

    /// Gives the pending commands to the world, runs a turn, and queues
    /// what happened.
    fn ask_for_update(&mut self, cx: &mut Client) {
        for c in std::mem::take(&mut self.pending_commands) {
            self.world.queue_command(self.me, &c);
        }
        self.next_update = cx.clock.now() + TURN / self.speed;
        self.run_turn(cx);
    }

    /// Runs a turn of the world with the commands queued, and queues what
    /// happened.
    pub(crate) fn run_turn(&mut self, cx: &mut Client) {
        self.waiting_for_world_update = true;
        self.world.update();
        for w in self.world.drain_warnings() {
            cx.log.warning(format!("world: {w}"));
        }
        for e in self.world.drain_events(self.me) {
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
            turn: self.world.turn(),
            events_waiting: !self.srv_queue.is_empty(),
            turns_handled: self.turns_handled,
        }
    }

    /// Handles the key presses waiting.
    fn process_events(&mut self, cx: &mut Client) -> Result<(), Stop> {
        cx.input.set_game(Some(self.progress()));
        cx.input.input_point(InputPoint::Game, cx.voice.is_idle());
        for e in cx.input.get_all() {
            match e {
                InputEvent::Quit => return Err(Stop::Exit),
                InputEvent::KeyDown(k) if self.chat_entry.is_some() => {
                    let (channel, entry) = self.chat_entry.as_mut().expect("checked");
                    let channel = *channel;
                    match entry.key(cx, &k) {
                        Typed::Done(line) => {
                            self.chat_entry = None;
                            crate::net::send_chat(cx, channel, &line);
                        }
                        Typed::Cancelled => self.chat_entry = None,
                        Typed::Typing | Typed::Ignored => {}
                    }
                }
                InputEvent::KeyDown(k) if self.game_menu.is_some() => {
                    self.game_menu_key(cx, k)?;
                }
                InputEvent::KeyDown(k) => {
                    if self.shortcut_mode {
                        self.shortcut_mode = false;
                        self.execute_order_shortcut(cx, &k)?;
                    } else {
                        self.process_keydown(cx, &k)?;
                    }
                }
                InputEvent::KeyUp(_) => {}
            }
        }
        Ok(())
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
            SrvEvent::VoiceImportant(m) => cx.voice.confirmation(&m),
            SrvEvent::Sequence(parts) => cx.play_sequence(&parts),
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

    fn srv_event(&mut self, cx: &mut Client, entity: Entity, event: &EntityEvent) {
        if cx.clock.now() > self.next_update + EVENT_LIMIT {
            return;
        }
        let view = EntityView::new(std::sync::Arc::new(entity), None, &cx.rng);
        if let Err(Oops { message: e, .. }) = self.notify(cx, &view, event) {
            cx.log.error(format!("problem during srv_event: {e}"));
        }
    }

    pub(crate) fn srv_quit(&mut self, cx: &mut Client) {
        cx.voice.silent_flush();
        cx.stop_sound(true);
        self.end_loop = true;
    }

    fn srv_voila(&mut self, cx: &mut Client, v: Voila) -> R {
        self.turns_handled += 1;
        self.last_virtual_time = v.time as f64 / 1000.0;
        self.waiting_for_world_update = false;
        self.scouted_squares = v.observed_squares;
        self.scouted_before_squares = v.observed_before_squares;
        self.send_resource_alerts_if_needed(cx);
        if self.previous_menus.is_empty() {
            self.send_menu_alerts_if_needed(cx);
        }
        self.units_alert_if_needed(cx, None)?;
        self.squares_alert_if_needed(cx);
        self.scout_info_if_needed(cx)?;
        self.update_fog_of_war(cx, v.memory, v.perception);
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
    pub fn is_an_enemy(&self, e: &Entity) -> bool {
        e.player
            .is_some_and(|p| !self.world.player(self.me).allied.contains(&p))
    }

    /// Whether `p` is the player or an ally.
    pub fn is_allied(&self, p: Option<PlayerId>) -> bool {
        p.is_some_and(|p| self.world.player(self.me).allied.contains(&p))
    }

    /// The title of a place: the square's, or the container's type title.
    pub fn place_title(&self, cx: &Client, place: Place) -> Msg {
        match place {
            Place::Square(s) => self.world.square(s).title.clone(),
            Place::Inside(c) => self
                .world
                .entity(c)
                .map(|e| style_msg(cx.res.style(), &e.type_name, "title"))
                .unwrap_or_default(),
        }
    }

    /// The square of a place, through containers.
    pub fn outer_square(&self, place: Option<Place>) -> Option<SquareId> {
        match place? {
            Place::Square(s) => Some(s),
            Place::Inside(c) => self.outer_square(self.world.entity(c)?.place),
        }
    }

    /// Says the objective of the map, if any.
    pub fn say_objective(&self, cx: &mut Client) {
        let objective = &self.world.map.objective;
        if !objective.is_empty() {
            let mut m = msg(mp::OBJECTIVE);
            m.extend(objective.iter().map(|&n| MsgPart::Num(n as u64)));
            cx.voice.confirmation(&m);
        }
    }

    /// Whether server events wait to be handled.
    pub fn has_pending_server_events(&self) -> bool {
        !self.srv_queue.is_empty()
    }

    /// The commands not yet given to the world.
    pub fn pending_commands(&self) -> &[String] {
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
