//! The other commands: statuses, objectives, the game menu, cheats and the
//! console, the clocks, first person mode, history and help.

use std::cell::Cell;
use std::rc::Rc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::player::ControllerKind;

use crate::client::Client;
use crate::input::{Event, KeyDown};
use crate::menu::{Action, Flow, Menu, TextEntry, input_string};
use crate::net::words::{self, text};
use crate::sound::StereoVolume;
use soundrts_protocol::Channel;

use super::command::GameCommand;
use super::entity::direction_to_msgpart;
use super::{GameInterface, Oops, POSITIONAL_BEEP_SOUND, R, Stop, TURN, style_msg};

/// What the game menu's items do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameMenuChoice {
    Quit,
    Speed(&'static str),
}

/// The game menu, open in a game on a server while the game goes on, and
/// the choice made in it.
pub(crate) struct GameMenu {
    menu: Menu,
    choice: Rc<Cell<Option<GameMenuChoice>>>,
}

/// The characters the console accepts.
fn console_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || " .,'@#$%^&*()_?!".contains(c) || ('+'..='=').contains(&c)
}

impl GameInterface {
    /// Runs a command of the bindings.
    pub(crate) fn execute(&mut self, cx: &mut Client, c: &GameCommand) -> Result<(), Stop> {
        use GameCommand as C;
        match c {
            C::ResourceStatus(r) => self.cmd_resource_status(cx, *r),
            C::FoodStatus => {
                let m = self.food_msg(cx);
                cx.voice.item(&m);
            }
            C::UnitHpStatus => self.cmd_unit_hp_status(cx),
            C::Examine => self.cmd_examine(cx)?,
            C::UnitStatus => self.cmd_unit_status(cx)?,
            C::SelectSquare {
                dx,
                dy,
                no_collision,
            } => self.cmd_select_square(cx, *dx, *dy, *no_collision)?,
            C::SelectScoutedSquare(i) => self.cmd_select_scouted_square(cx, *i)?,
            C::SelectConflictSquare(i) => self.cmd_select_conflict_square(cx, *i)?,
            C::SelectUnknownSquare(i) => self.cmd_select_unknown_square(cx, *i)?,
            C::SelectResourceSquare(i) => self.cmd_select_resource_square(cx, *i)?,
            C::ToggleZoom => self.cmd_toggle_zoom(cx),
            C::SelectTarget { inc, types } => self.cmd_select_target(cx, *inc, types)?,
            C::SelectOrder { inc, menu } => self.cmd_select_order(cx, *inc, *menu),
            C::OrderShortcut => self.cmd_order_shortcut(cx),
            C::Validate(flags) => self.cmd_validate(cx, *flags)?,
            C::Default(flags) => self.cmd_default(cx, *flags)?,
            C::DoAgain { now, flags } => self.cmd_do_again(cx, *now, *flags)?,
            C::SelectUnit { inc, filter } => self.cmd_select_unit(cx, *inc, filter)?,
            C::SelectUnits(filter) => self.cmd_select_units(cx, filter)?,
            C::CommandUnit => self.cmd_command_unit(cx)?,
            C::Group { portion, filter } => self.cmd_group(cx, *portion, filter)?,
            C::Ungroup => self.cmd_ungroup(cx)?,
            C::RecallGroup(name) => self.cmd_recall_group(cx, name)?,
            C::SetGroup { name, flags } => self.cmd_set_group(cx, name, *flags)?,
            C::AppendGroup { name, flags } => self.cmd_append_group(cx, name, *flags)?,
            C::Escape => self.cmd_escape(cx)?,
            C::Help(incr) => {
                let m = cx.help_msg("game", *incr);
                cx.voice.item(&m);
            }
            C::Fullscreen => cx.toggle_fullscreen(),
            C::ToggleCheatmode => self.cmd_toggle_cheatmode(cx),
            C::SayTime => self.cmd_say_time(cx),
            C::ToggleTick => self.must_play_tick = !self.must_play_tick,
            C::ToggleTalkingClock => self.cmd_toggle_talking_clock(cx),
            C::ChangePlayer => self.cmd_change_player(cx),
            C::HistoryPrevious => cx.voice.previous(),
            C::HistoryNext => cx.voice.say_next(true),
            C::HistoryStop => cx.voice.say_next(false),
            C::Objectives => self.cmd_objectives(cx),
            C::GameMenu => self.cmd_gamemenu(cx)?,
            C::SayPlayers => {
                let mut m = Vec::new();
                for &p in &self.world.players {
                    m.extend(self.world.player(p).name());
                    m.extend(msg(mp::COMMA));
                }
                cx.voice.item(&m);
            }
            C::Say => self.cmd_say(cx, Channel::Game),
            C::SayAllies => self.cmd_say(cx, Channel::Allies),
            C::ChatPrevious => crate::net::chat_previous(cx),
            C::ChatNext => crate::net::chat_next(cx),
            C::Volume(incr) => cx.modify_volume(*incr as i32),
            C::Immersion => {
                if !self.immersion {
                    self.toggle_immersion(cx)?;
                }
            }
            C::RotateLeft => self.cmd_rotate_left(cx)?,
            C::RotateRight => self.cmd_rotate_right(cx)?,
            C::Console => self.cmd_console(cx)?,
            C::ReloadParameters => cx.reload_parameters(),
            C::Cmd(cmd) => {
                if self.world.allow_cheatmode {
                    self.execute_command(cx, cmd)?;
                } else {
                    cx.voice.item(&msg(mp::BEEP));
                }
            }
            C::Broken { name, error } => {
                return Err(Stop::Oops(Oops::new(format!("cmd_{name}: {error}"))));
            }
        }
        Ok(())
    }

    /// Starts typing a chat line to the game's players or to the allies;
    /// the game goes on meanwhile. A beep without a server.
    fn cmd_say(&mut self, cx: &mut Client, channel: Channel) {
        if self.net.is_none() {
            cx.voice.item(&msg(mp::BEEP));
            return;
        }
        let mut prompt = msg(mp::ENTER_MESSAGE);
        if channel == Channel::Allies {
            prompt.extend(text(words::TO_YOUR_ALLIES));
        }
        let entry = TextEntry::start(cx, &prompt, crate::net::chat_char, "", false);
        self.chat_entry = Some((channel, entry));
    }

    fn cmd_resource_status(&self, cx: &mut Client, r: i64) {
        let resources = self.resources();
        let i = if r < 0 { r + resources.len() as i64 } else { r };
        match usize::try_from(i).ok().and_then(|i| resources.get(i)) {
            Some(&n) => {
                let m = [
                    nb2msg(n),
                    style_msg(cx.res.style(), "parameters", &format!("resource_{r}_title")),
                ]
                .concat();
                cx.voice.item(&m);
            }
            None => cx.voice.item(&msg(mp::BEEP)),
        }
    }

    fn cmd_objectives(&self, cx: &mut Client) {
        let mut m = Vec::new();
        let objective = &self.world.map.objective;
        if !objective.is_empty() {
            m.extend(msg(mp::OBJECTIVE));
            m.extend(objective.iter().map(|&n| MsgPart::Num(n as u64)));
            m.extend(msg(mp::PERIOD));
        }
        let objectives = &self.world.player(self.me).objectives;
        if !objectives.is_empty() {
            m.extend(msg(mp::OBJECTIVE));
            m.extend(msg(mp::COMMA));
            for o in objectives {
                m.extend(o.description.iter().map(|&n| MsgPart::Num(n as u64)));
                m.extend(msg(mp::COMMA));
            }
        }
        cx.voice.item(&m);
    }

    fn cmd_toggle_cheatmode(&mut self, cx: &mut Client) {
        if self.world.allow_cheatmode {
            self.write_line(cx, "toggle_cheatmode");
            let state = if self.world.player(self.me).cheatmode {
                mp::IS_NOW_OFF
            } else {
                mp::IS_NOW_ON
            };
            cx.voice.item(&[msg(mp::CHEATMODE), msg(state)].concat());
        } else {
            cx.voice.item(&msg(mp::BEEP));
        }
    }

    /// Runs a console or `cmd` command.
    pub fn execute_command(&mut self, cx: &mut Client, cmd: &str) -> R {
        let now = cx.clock.now();
        if let Some(s) = cmd.strip_prefix("s ") {
            let s = s.split(' ').next().unwrap_or_default();
            self.speed = soundrts_world::nofloat::py_float(s)
                .map_err(|_| Oops::new(format!("could not convert string to float: '{s}'")))?;
            self.next_update = now;
        } else if cmd == "p" {
            if self.speed >= 1.0 {
                self.speed /= 10000.0;
            } else {
                self.speed *= 10000.0;
                self.next_update = now;
            }
        } else if cmd == "m" {
            for u in self.world.player(self.me).units.clone() {
                if let Some(e) = self.world.entity_mut(u) {
                    e.stats.mana_regen *= 1000;
                }
            }
        } else if cmd == "h" {
            cx.voice.item(&vec![MsgPart::from(
                "p: pause/unpause, s: set speed, r: get 1000 resources, t: get all techs, m: infinite mana, a: add units, v: instant victory",
            )]);
        } else if cmd == "r" {
            let p = self.world.player_mut(self.me);
            for r in &mut p.resources {
                *r += 1000 * super::order::PRECISION;
            }
        } else if cmd == "t" {
            self.world.player_mut(self.me).cheat_has_all = true;
        } else if cmd == "dti" {
            // Shows the target's data on screen: nothing to hear.
        } else if matches!(cmd, "edit" | "sm" | "at")
            || cmd.starts_with("te ")
            || cmd.starts_with("st ")
        {
            // The map editor is not part of this client.
            cx.voice.item(&msg(mp::BEEP));
        } else if !cmd.is_empty() {
            let mut cmd = cmd.to_owned();
            if let Some(rest) = cmd.strip_prefix("a ") {
                let name = self
                    .place
                    .map(|p| self.world.square(p).name.clone())
                    .unwrap_or_default();
                cmd = format!("add_units {name} {rest}");
            }
            if cmd == "v" {
                cmd = "victory".into();
            }
            self.write_line(cx, &format!("cmd {cmd}"));
        }
        Ok(())
    }

    fn cmd_console(&mut self, cx: &mut Client) -> Result<(), Stop> {
        if !self.world.allow_cheatmode {
            cx.voice.item(&msg(mp::BEEP));
            return Ok(());
        }
        if let Some(cmd) = input_string(cx, &msg(mp::ENTER_COMMAND), console_char, "", false)? {
            self.execute_command(cx, &cmd)?;
        }
        Ok(())
    }

    /// Plays as the next player of the game (a cheat): the logins stay with
    /// the players, the events now come from the new one.
    fn cmd_change_player(&mut self, cx: &mut Client) {
        if !self.world.allow_cheatmode {
            cx.voice.item(&msg(mp::BEEP));
            return;
        }
        let players = &self.world.players;
        let i = players.iter().position(|&p| p == self.me).unwrap_or(0);
        let new = players[(i + 1) % players.len()];
        if new != self.me {
            let login = self.world.player(new).controller.login();
            self.world.player_mut(new).controller.kind =
                ControllerKind::Human { login, local: true };
            if let ControllerKind::Human { local, .. } =
                &mut self.world.player_mut(self.me).controller.kind
            {
                *local = false;
            }
            self.me = new;
        }
        // Upstream refreshes the fog of war here from the previous player's
        // last perception: nothing changes until the next turn.
        let m = [msg(mp::YOU_ARE), self.world.player(self.me).name()].concat();
        cx.voice.item(&m);
    }

    fn is_admin(&self) -> bool {
        self.world.players.first() == Some(&self.me)
    }

    /// The game menu: quit, the speeds, continue. In a game on a server,
    /// the game goes on while it is open and the keys go to it; otherwise
    /// the game waits for a choice.
    fn cmd_gamemenu(&mut self, cx: &mut Client) -> Result<(), Stop> {
        if self.net.is_none() {
            cx.voice.silent_flush();
            cx.stop_sound(true);
        }
        let choice: Rc<Cell<Option<GameMenuChoice>>> = Rc::new(Cell::new(None));
        let set = |c: GameMenuChoice| {
            let choice = choice.clone();
            Action::run(move |_| {
                choice.set(Some(c));
                Ok(Flow::Stay)
            })
        };
        let mut menu = Menu::new(msg(mp::MENU), Vec::new());
        menu.append(msg(mp::CANCEL_GAME), set(GameMenuChoice::Quit), None);
        // In a game on a server, the server keeps the time.
        if self.is_admin() && self.net.is_none() {
            menu.append(
                msg(mp::SET_SPEED_TO_SLOW),
                set(GameMenuChoice::Speed("0.5")),
                None,
            );
            menu.append(
                msg(mp::SET_SPEED_TO_NORMAL),
                set(GameMenuChoice::Speed("1.0")),
                None,
            );
            menu.append(
                msg(mp::SET_SPEED_TO_FAST),
                set(GameMenuChoice::Speed("2.0")),
                None,
            );
            menu.append(
                [msg(mp::SET_SPEED_TO_FAST), nb2msg(4)].concat(),
                set(GameMenuChoice::Speed("4.0")),
                None,
            );
        }
        menu.append(msg(mp::CONTINUE_GAME), Action::Nothing, None);
        if self.net.is_some() {
            menu.say_title(cx);
            self.game_menu = Some(GameMenu { menu, choice });
            return Ok(());
        }
        menu.run(cx)?;
        self.game_menu_chosen(cx, choice.get());
        Ok(())
    }

    /// A key pressed while the game menu is open in a game on a server: the
    /// keys bound to writing to the game or to the allies do so as in the
    /// game, the other keys go to the menu, which closes once an item is
    /// chosen.
    pub(crate) fn game_menu_key(&mut self, cx: &mut Client, k: KeyDown) -> Result<(), Stop> {
        let channel = match self.bindings.command_for(&k) {
            Some(GameCommand::Say) => Some(Channel::Game),
            Some(GameCommand::SayAllies) => Some(Channel::Allies),
            _ => None,
        };
        if let Some(channel) = channel {
            self.cmd_say(cx, channel);
            return Ok(());
        }
        let Some(gm) = &mut self.game_menu else {
            return Ok(());
        };
        if gm.menu.handle(cx, Some(Event::KeyDown(k)))? {
            let choice = gm.choice.get();
            self.game_menu = None;
            self.game_menu_chosen(cx, choice);
        }
        Ok(())
    }

    fn game_menu_chosen(&mut self, cx: &mut Client, choice: Option<GameMenuChoice>) {
        match choice {
            Some(GameMenuChoice::Quit) => self.gm_quit(cx),
            Some(GameMenuChoice::Speed(s)) => self.write_line(cx, &format!("speed {s}")),
            None => {}
        }
    }

    /// Asks the world to quit; asked again, quits at once.
    fn gm_quit(&mut self, cx: &mut Client) {
        if !self.already_asked_to_quit {
            self.next_update = cx.clock.now();
            self.write_line(cx, "quit");
            cx.input.clear();
            self.already_asked_to_quit = true;
        } else {
            self.srv_quit(cx);
            self.forced_quit = true;
        }
    }

    // clocks

    fn cmd_toggle_talking_clock(&mut self, cx: &mut Client) {
        self.bell_enabled = !self.bell_enabled;
        if self.bell_enabled {
            cx.voice.item(&[msg(mp::BELL), msg(mp::IS_NOW_ON)].concat());
            self.previous_nb_minutes = (self.last_virtual_time / 60.0) as i64;
        } else {
            cx.voice
                .item(&[msg(mp::BELL), msg(mp::IS_NOW_OFF)].concat());
        }
    }

    /// Rings every game minute.
    pub(crate) fn eventually_play_bell(&mut self, cx: &Client) {
        let nb_minutes = (self.last_virtual_time / 60.0) as i64;
        if self.previous_nb_minutes != nb_minutes {
            cx.audio.play_stereo(
                cx.sounds.get_sound(POSITIONAL_BEEP_SOUND),
                StereoVolume::Both(1.0),
                0.0,
            );
            self.previous_nb_minutes = nb_minutes;
        }
    }

    /// Ticks every turn, quietly.
    pub(crate) fn play_tick(&self, cx: &Client) {
        cx.audio.play_stereo(
            cx.sounds.get_sound(POSITIONAL_BEEP_SOUND),
            StereoVolume::Both(0.1),
            0.0,
        );
    }

    pub(crate) fn record_update_time(&mut self, cx: &Client) {
        let now = cx.clock.now();
        let interval = TURN / self.speed.min(10.0);
        let nb_samples = (1.0 / interval).max(1.0);
        let turn_duration = match self.previous_update_time {
            None => interval,
            Some(t) => now - t,
        };
        self.average_turn_duration =
            (self.average_turn_duration * (nb_samples - 1.0) + turn_duration) / nb_samples;
        self.previous_update_time = Some(now);
    }

    /// Turns per second, measured.
    fn tps(&self) -> f64 {
        if self.average_turn_duration == 0.0 {
            100.0
        } else {
            1.0 / self.average_turn_duration
        }
    }

    /// The measured speed, 1 being normal.
    pub fn real_speed(&self) -> f64 {
        self.tps() / (1.0 / TURN)
    }

    fn cmd_say_time(&self, cx: &mut Client) {
        let t = self.last_virtual_time as i64;
        let (m, s) = (t.div_euclid(60), t.rem_euclid(60));
        let m = [
            nb2msg(m),
            msg(mp::MINUTES),
            nb2msg(s),
            msg(mp::SECONDS),
            msg(mp::COMMA),
            msg(mp::SPEED),
            vec![MsgPart::Text(format!("{:.1}", self.real_speed()))],
        ]
        .concat();
        cx.voice.item(&m);
    }

    // first person mode

    pub(crate) fn toggle_immersion(&mut self, cx: &mut Client) -> R {
        self.immersion = !self.immersion;
        if self.immersion {
            self.cmd_unit_status(cx)?;
            cx.voice.item(&msg(mp::FIRST_PERSON_MODE));
        } else {
            cx.voice.item(&msg(mp::MAP_MODE));
        }
        self.follow_mode = self.immersion;
        Ok(())
    }

    fn cmd_escape(&mut self, cx: &mut Client) -> R {
        if self.order.is_some() {
            cx.voice.item(&msg(mp::CANCEL));
            self.order = None;
        } else if self.immersion {
            self.toggle_immersion(cx)?;
        } else if self.zoom.is_some() {
            self.cmd_toggle_zoom(cx);
        } else if self.target.is_some()
            && let Some(p) = self.place
        {
            self.select_and_say_square(cx, p, &Vec::new());
        }
        Ok(())
    }

    fn say_compass(&mut self, cx: &mut Client) {
        let compass: Msg = msg(direction_to_msgpart(self.o));
        if self.previous_compass.as_ref() != Some(&compass) {
            cx.voice.item(&compass);
            self.previous_compass = Some(compass);
        }
    }

    fn rotate(&mut self, cx: &mut Client, degrees: f64) -> R {
        let Some(&g) = self.group.first() else {
            return Ok(());
        };
        let v = self.dobjets.get_mut(&g).ok_or_else(|| Oops::missing(g.0))?;
        v.o_override = Some(v.o() + degrees);
        self.o += degrees;
        self.say_compass(cx);
        cx.audio.update();
        Ok(())
    }

    pub(crate) fn cmd_rotate_left(&mut self, cx: &mut Client) -> R {
        self.rotate(cx, 45.0)
    }

    pub(crate) fn cmd_rotate_right(&mut self, cx: &mut Client) -> R {
        self.rotate(cx, -45.0)
    }
}

#[cfg(test)]
mod tests {
    use super::console_char;

    #[test]
    fn the_console_accepts_upstreams_characters() {
        for c in "aZ09 .,'@#$%^&*()_+-=?!/:;<".chars() {
            assert!(console_char(c), "{c}");
        }
        for c in "\"[]{}~`|\\>".chars() {
            assert!(!console_char(c), "{c}");
        }
    }
}
