//! The other commands: statuses, objectives, the game menu, cheats and the
//! console, the clocks, first person mode, history and help.

use std::cell::Cell;
use std::rc::Rc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{Cheat, Command};

use crate::client::Client;
use crate::input::{Event, Key, KeyDown};
use crate::menu::{Action, Flow, Menu, TextEntry, Typed};
use crate::net::words::{self, text};
use crate::sound::StereoVolume;
use soundrts_protocol::Channel;

use super::command::GameCommand;
use super::entity::direction_to_msgpart;
use super::{GameInterface, Mode, Oops, POSITIONAL_BEEP_SOUND, R, Stop, Story, TURN, style_msg};

/// What the game menu's items do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GameMenuChoice {
    Quit,
    Speed(f64),
}

/// The game menu, the choice made in it, and the story it was opened
/// during, which goes on once it closes.
pub(crate) struct GameMenu {
    menu: Menu,
    choice: Rc<Cell<Option<GameMenuChoice>>>,
    over: Option<Story>,
}

impl GameMenu {
    /// What the keys do once the menu is closed.
    fn closed(self) -> Mode {
        self.over.map_or(Mode::Playing, Mode::Story)
    }
}

/// A chat line being typed, who it is for, and the game menu it was
/// written from, which stays open.
pub(crate) struct Chat {
    channel: Channel,
    entry: TextEntry,
    over: Option<GameMenu>,
}

impl Chat {
    /// What the keys do once the line is sent or given up.
    fn closed(self) -> Mode {
        self.over.map_or(Mode::Playing, Mode::GameMenu)
    }
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
            C::AllResourcesStatus => self.cmd_all_resources_status(cx),
            C::FoodStatus => {
                let m = self.food_msg(cx);
                cx.voice.item(&m);
            }
            C::UnitHpStatus => self.cmd_unit_hp_status(cx),
            C::ProductionStatus => self.cmd_production_status(cx)?,
            C::WorkersStatus => self.cmd_workers_status(cx),
            C::FightStatus => self.cmd_fight_status(cx),
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
            C::Unheard => cx.voice.unheard(),
            C::ReviewPrevious => cx.voice.review_previous(),
            C::ReviewNext => cx.voice.review_next(),
            C::ReviewCategory => cx.voice.review_category(),
            C::JumpToAlert => self.cmd_jump_to_alert(cx)?,
            C::Objectives => self.cmd_objectives(cx),
            C::GameMenu => self.cmd_gamemenu(cx),
            C::SayPlayers => {
                let mut m = Vec::new();
                let view = self.view();
                for &p in view.players() {
                    m.extend(view.player_name(p));
                    m.extend(msg(mp::COMMA));
                }
                cx.voice.item(&m);
            }
            C::PlayersStatus => self.cmd_players_status(cx),
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
            C::Console => self.cmd_console(cx),
            C::ReloadParameters => cx.reload_parameters(),
            C::KeyHelp => self.start_key_help(cx),
            C::Cmd(cmd) => {
                if self.view().cheats_allowed() {
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

    /// Moves the cursor to the square of the last alert of this game and
    /// says it. Pressed again on that square, goes to the alert before,
    /// and stays on the oldest.
    fn cmd_jump_to_alert(&mut self, cx: &mut Client) -> R {
        let before = match self.alert_jump {
            Some((id, square)) if self.place == Some(square) => Some(id),
            _ => None,
        };
        let Some((id, square)) = cx
            .voice
            .alert_before(before)
            .filter(|&(id, _)| id >= self.log_start)
            .or(before.and(self.alert_jump))
        else {
            cx.voice.item(&msg(mp::NOTHING));
            return Ok(());
        };
        self.leave_zoom_and_immersion(cx)?;
        self.select_and_say_square(cx, square, &Vec::new());
        self.alert_jump = Some((id, square));
        Ok(())
    }

    /// Starts typing a chat line to the game's players or to the allies,
    /// over the game menu if it is open; the game goes on meanwhile. A
    /// beep without a server.
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
        let over = match std::mem::replace(&mut self.mode, Mode::Playing) {
            Mode::GameMenu(gm) => Some(gm),
            _ => None,
        };
        self.mode = Mode::Chat(Chat {
            channel,
            entry,
            over,
        });
    }

    /// A key pressed while a chat line is typed.
    pub(crate) fn chat_key(&mut self, cx: &mut Client, mut chat: Chat, k: &KeyDown) {
        match chat.entry.key(cx, k) {
            Typed::Done(line) => {
                let channel = chat.channel;
                self.mode = chat.closed();
                crate::net::send_chat(cx, channel, &line);
            }
            Typed::Cancelled => self.mode = chat.closed(),
            Typed::Typing | Typed::Ignored => self.mode = Mode::Chat(chat),
        }
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
        let view = self.view();
        let objective = view.map_objective();
        if !objective.is_empty() {
            m.extend(msg(mp::OBJECTIVE));
            m.extend(objective.iter().map(|&n| MsgPart::Num(n as u64)));
            m.extend(msg(mp::PERIOD));
        }
        let objectives = view.objectives();
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
        if self.view().cheats_allowed() {
            self.send_command(cx, Command::Cheat(Cheat::ToggleMode));
            let state = if self.view().cheat_mode() {
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
            self.cheat_at_once(Cheat::InfiniteMana);
        } else if cmd == "h" {
            cx.voice.item(&vec![MsgPart::from(
                "p: pause/unpause, s: set speed, r: get 1000 resources, t: get all techs, m: infinite mana, a: add units, v: instant victory",
            )]);
        } else if cmd == "r" {
            self.cheat_at_once(Cheat::Resources);
        } else if cmd == "t" {
            self.cheat_at_once(Cheat::AllTechs);
        } else if cmd == "dti" {
            // Shows the target's data on screen: nothing to hear.
        } else if matches!(cmd, "edit" | "sm" | "at")
            || cmd.starts_with("te ")
            || cmd.starts_with("st ")
        {
            // The map editor is not part of this client.
            cx.voice.item(&msg(mp::BEEP));
        } else if !cmd.is_empty() {
            let cheat = if let Some(rest) = cmd.strip_prefix("a ") {
                Cheat::AddUnits {
                    square: self.place,
                    units: rest.to_owned(),
                }
            } else if cmd == "v" {
                Cheat::Victory
            } else {
                Cheat::Trigger(cmd.to_owned())
            };
            self.send_command(cx, Command::Cheat(cheat));
        }
        Ok(())
    }

    /// A cheat that changes the game at once, even while the game waits:
    /// what the player hears next is told by the host at once. It is the
    /// player's even out of the game, as the console's always were; the
    /// console is open only in a game that allows cheats.
    pub(crate) fn cheat_at_once(&mut self, cheat: Cheat) {
        if let Some(told) = self.host.queue(self.me, Command::Cheat(cheat)) {
            self.install_at_once(told);
        }
    }

    /// Starts typing a command of the console, in a game that allows
    /// cheats.
    fn cmd_console(&mut self, cx: &mut Client) {
        if !self.view().cheats_allowed() {
            cx.voice.item(&msg(mp::BEEP));
            return;
        }
        let entry = TextEntry::start(cx, &msg(mp::ENTER_COMMAND), console_char, "", false);
        self.mode = Mode::Console(entry);
    }

    /// A key pressed while a command of the console is typed: Enter runs
    /// it.
    pub(crate) fn console_key(&mut self, cx: &mut Client, mut entry: TextEntry, k: &KeyDown) -> R {
        match entry.key(cx, k) {
            Typed::Done(cmd) => self.execute_command(cx, &cmd)?,
            Typed::Cancelled => {}
            Typed::Typing | Typed::Ignored => self.mode = Mode::Console(entry),
        }
        Ok(())
    }

    /// Plays as the next player of the game (a cheat): the logins stay with
    /// the players, the events now come from the new one.
    fn cmd_change_player(&mut self, cx: &mut Client) {
        if !self.view().cheats_allowed() {
            cx.voice.item(&msg(mp::BEEP));
            return;
        }
        let players = self.view().players();
        let i = players.iter().position(|&p| p == self.me).unwrap_or(0);
        let new = players[(i + 1) % players.len()];
        if new != self.me {
            self.cheat_at_once(Cheat::PlayAs(new));
        }
        // Upstream refreshes the fog of war here from the previous player's
        // last perception: nothing changes until the next turn.
        let m = [msg(mp::YOU_ARE), self.view().player_name(self.me)].concat();
        cx.voice.item(&m);
    }

    fn is_admin(&self) -> bool {
        self.view().players().first() == Some(&self.me)
    }

    /// Opens the game menu: quit, the speeds, continue. The keys go to it
    /// until a choice is made. In single-player the world waits meanwhile,
    /// and its sounds stop; in a game on a server, the game goes on.
    fn cmd_gamemenu(&mut self, cx: &mut Client) {
        if self.net.is_none() {
            cx.stop_sound(false);
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
                set(GameMenuChoice::Speed(0.5)),
                None,
            );
            menu.append(
                msg(mp::SET_SPEED_TO_NORMAL),
                set(GameMenuChoice::Speed(1.0)),
                None,
            );
            menu.append(
                msg(mp::SET_SPEED_TO_FAST),
                set(GameMenuChoice::Speed(2.0)),
                None,
            );
            menu.append(
                [msg(mp::SET_SPEED_TO_FAST), nb2msg(4)].concat(),
                set(GameMenuChoice::Speed(4.0)),
                None,
            );
        }
        menu.append(msg(mp::CONTINUE_GAME), Action::Nothing, None);
        menu.say_title(cx);
        let over = match std::mem::replace(&mut self.mode, Mode::Playing) {
            Mode::Story(s) => Some(s),
            _ => None,
        };
        self.mode = Mode::GameMenu(GameMenu { menu, choice, over });
    }

    /// A key pressed while the game menu is open: the keys bound to
    /// writing to the game or to the allies do so as in the game, and so
    /// does the menus' own chat key; the other keys go to the menu, which
    /// closes once an item is chosen.
    pub(crate) fn game_menu_key(
        &mut self,
        cx: &mut Client,
        mut gm: GameMenu,
        k: KeyDown,
    ) -> Result<(), Stop> {
        let channel = match self.bindings.command_for(&k) {
            Some(GameCommand::Say) => Some(Channel::Game),
            Some(GameCommand::SayAllies) => Some(Channel::Allies),
            // The menus' chat key, with other modifiers.
            _ if k.key == Key::F7 => Some(Channel::Game),
            _ => None,
        };
        if let Some(channel) = channel {
            self.mode = Mode::GameMenu(gm);
            self.cmd_say(cx, channel);
            return Ok(());
        }
        if gm.menu.handle(cx, Some(Event::KeyDown(k)))? {
            let choice = gm.choice.get();
            // Quitting ends the story too, so that the world takes the
            // command; the story's messages stay in the queue.
            self.mode = match choice {
                Some(GameMenuChoice::Quit) => Mode::Playing,
                _ => gm.closed(),
            };
            self.game_menu_chosen(cx, choice);
        } else {
            self.mode = Mode::GameMenu(gm);
        }
        Ok(())
    }

    fn game_menu_chosen(&mut self, cx: &mut Client, choice: Option<GameMenuChoice>) {
        match choice {
            Some(GameMenuChoice::Quit) => self.gm_quit(cx),
            Some(GameMenuChoice::Speed(s)) => {
                self.send_command(cx, Command::Speed(s));
                // The next single-player game starts at this speed.
                cx.config.speed = s;
                cx.save_config();
            }
            None => {}
        }
    }

    /// Asks the world to quit; asked again, quits at once.
    fn gm_quit(&mut self, cx: &mut Client) {
        if !self.already_asked_to_quit {
            self.next_update = cx.clock.now();
            self.send_command(cx, Command::Quit);
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
