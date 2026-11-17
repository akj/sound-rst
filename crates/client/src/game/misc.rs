//! The other commands: statuses, objectives, the game menu, cheats and the
//! console, the clocks, first person mode, history and help.

use std::cell::Cell;
use std::rc::Rc;

use soundrts_world::msg::{Cue, Msg, MsgPart, Phrase, comma, cue, msg, nb2msg};
use soundrts_world::player::Objective;
use soundrts_world::{Cheat, Command};

use crate::client::Client;
use crate::input::{Event, Key, KeyDown};
use crate::menu::{Action, Exit, Flow, Menu, TextEntry, Typed};
use crate::sound::StereoVolume;
use soundrts_protocol::{Channel, Step};

use super::command::GameCommand;
use super::entity::direction_to_msgpart;
use super::replay::Pace;
use super::source::Speeds;
use super::{GameInterface, Mode, Story, TURN};

/// What the game menu's items do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum GameMenuChoice {
    Quit,
    Speed(f64),
    /// Pauses a game on a server, or resumes it.
    PauseOrResume,
    /// Saves a game run here.
    Save,
    /// What a listener to a recorded game does with its keys.
    Listen(ListenChoice),
}

/// The game menu's items for a listener to a recorded game: what its keys
/// do, for someone who does not know them.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ListenChoice {
    PauseOrResume,
    Pace(Pace),
    /// Minutes forward, or back below 0.
    Jump(i64),
    /// Keeps the record in the records folder.
    Keep,
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

/// The two sounds that stand for the clock's bell, which has no words.
fn bell() -> Msg {
    [cue(Cue::BellStrike), cue(Cue::PositionalBeep)].concat()
}

/// The characters the console accepts.
fn console_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || " .,'@#$%^&*()_?!".contains(c) || ('+'..='=').contains(&c)
}

impl GameInterface {
    /// Runs a command of the bindings.
    pub(crate) fn execute(&mut self, cx: &mut Client, c: &GameCommand) -> Result<(), Exit> {
        use GameCommand as C;
        match c {
            C::ResourceStatus(r) => self.cmd_resource_status(cx, *r),
            C::AllResourcesStatus => self.cmd_all_resources_status(cx),
            C::FoodStatus => {
                let m = self.food_msg(cx);
                cx.voice.item(&m);
            }
            C::UnitHpStatus => self.cmd_unit_hp_status(cx),
            C::ProductionStatus => self.cmd_production_status(cx),
            C::WorkersStatus => self.cmd_workers_status(cx),
            C::FightStatus => self.cmd_fight_status(cx),
            C::CombatStatus => self.cmd_combat_status(cx),
            C::Examine => self.cmd_examine(cx),
            C::UnitStatus => self.cmd_unit_status(cx),
            C::SelectSquare {
                dx,
                dy,
                no_collision,
            } => self.cmd_select_square(cx, *dx, *dy, *no_collision),
            C::SelectScoutedSquare(i) => self.cmd_select_scouted_square(cx, *i),
            C::SelectConflictSquare(i) => self.cmd_select_conflict_square(cx, *i),
            C::SelectUnknownSquare(i) => self.cmd_select_unknown_square(cx, *i),
            C::SelectResourceSquare(i) => self.cmd_select_resource_square(cx, *i),
            C::ToggleZoom => self.cmd_toggle_zoom(cx),
            C::SelectTarget { inc, types } => self.cmd_select_target(cx, *inc, types),
            C::SelectOrder { inc, menu } => self.cmd_select_order(cx, *inc, *menu),
            C::OrderShortcut => self.cmd_order_shortcut(cx),
            C::Validate(flags) => self.cmd_validate(cx, *flags),
            C::Default(flags) => self.cmd_default(cx, *flags),
            C::DoAgain { now, flags } => self.cmd_do_again(cx, *now, *flags),
            C::SelectUnit { inc, filter } => self.cmd_select_unit(cx, *inc, filter),
            C::SelectUnits(filter) => self.cmd_select_units(cx, filter),
            C::CommandUnit => self.cmd_command_unit(cx),
            C::Group { portion, filter } => self.cmd_group(cx, *portion, filter),
            C::Ungroup => self.cmd_ungroup(cx),
            C::RecallGroup(name) => self.cmd_recall_group(cx, name),
            C::SetGroup { name, flags } => self.cmd_set_group(cx, name, *flags),
            C::AppendGroup { name, flags } => self.cmd_append_group(cx, name, *flags),
            C::Escape => self.cmd_escape(cx),
            C::Help(incr) => {
                let m = cx.help_msg("game", *incr);
                cx.voice.item(&m);
            }
            C::Fullscreen => cx.toggle_fullscreen(),
            C::ToggleCheatmode => self.cmd_toggle_cheatmode(cx),
            C::SayTime => {
                if !self.say_replay_time(cx) {
                    self.cmd_say_time(cx);
                }
            }
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
            C::CopyMessage => {
                if let Some(words) = cx.voice.copy() {
                    cx.desktop.set_clipboard(&words);
                }
            }
            C::JumpToAlert => self.cmd_jump_to_alert(cx),
            C::Objectives(inc) => self.cmd_objectives(cx, *inc),
            C::FollowPlayer(inc) => {
                let step = if *inc < 0 { Step::Previous } else { Step::Next };
                self.follow_player(cx, step);
            }
            C::ListenSpeed(step) => self.listen_speed(cx, *step),
            C::ListenPause => self.listen_pause(cx),
            C::ListenJump(minutes) => self.listen_jump(cx, *minutes),
            C::GameMenu => self.cmd_gamemenu(cx),
            C::SayPlayers => {
                let mut m = Vec::new();
                let view = self.view();
                let factions = self.has_factions();
                for &p in view.players() {
                    m.extend(view.player_name(p));
                    if factions {
                        m.extend(self.faction_title(cx, p));
                    }
                    m.extend(comma());
                }
                // On a server, who watches the game: nobody listens unknown.
                let room = cx.net.as_ref().and_then(|n| n.room.as_ref());
                if self.on_a_server()
                    && let Some(watchers) = room.map(|r| r.watchers())
                {
                    m.extend(watchers);
                }
                cx.voice.item(&m);
            }
            C::SayFaction => {
                let m = self.faction_title(cx, self.me);
                cx.voice.item(&m);
            }
            C::PlayersStatus => self.cmd_players_status(cx),
            C::Say => self.cmd_say(cx, Channel::Game),
            C::SayAllies => self.cmd_say(cx, Channel::Allies),
            C::Flare => self.flare(cx),
            C::ChatPrevious => crate::net::chat_previous(cx),
            C::ChatNext => crate::net::chat_next(cx),
            C::Volume(incr) => cx.modify_volume(*incr as i32),
            C::Immersion => {
                if !self.immersion {
                    self.toggle_immersion(cx);
                }
            }
            C::RotateLeft => self.cmd_rotate_left(cx),
            C::RotateRight => self.cmd_rotate_right(cx),
            C::Console => self.cmd_console(cx),
            C::ReloadParameters => cx.reload_parameters(),
            C::KeyHelp => self.start_key_help(cx),
            C::Cmd(cmd) => {
                if self.cheats_allowed() {
                    self.execute_command(cx, cmd);
                } else {
                    cx.voice.item(&cue(Cue::Beep));
                }
            }
        }
        Ok(())
    }

    /// Moves the cursor to the square of the last alert of this game and
    /// says it. Pressed again on that square, goes to the alert before,
    /// and stays on the oldest.
    fn cmd_jump_to_alert(&mut self, cx: &mut Client) {
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
            cx.voice.item(&msg(Phrase::Nothing));
            return;
        };
        self.leave_zoom_and_immersion(cx);
        self.select_and_say_square(cx, square, &Vec::new());
        self.alert_jump = Some((id, square));
    }

    /// Starts typing a chat line to the game's players or to the allies,
    /// over the game menu if it is open; the game goes on meanwhile. A
    /// beep without a server.
    fn cmd_say(&mut self, cx: &mut Client, channel: Channel) {
        if !self.on_a_server() {
            cx.voice.item(&cue(Cue::Beep));
            return;
        }
        let mut prompt = msg(Phrase::EnterMessage);
        if channel == Channel::Allies {
            prompt.extend(msg(Phrase::ToYourAllies));
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
                self.send_chat(cx, channel, &line);
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
                    cx.res.style().get("parameters").resource_title_msg(r),
                ]
                .concat();
                cx.voice.item(&m);
            }
            None => cx.voice.item(&cue(Cue::Beep)),
        }
    }

    /// Notes the player's objectives at the end of a turn.
    pub(crate) fn note_objectives(&mut self, objectives: &[Objective]) {
        for o in objectives {
            if !self.objectives_seen.contains(o) {
                self.objectives_seen.push(o.clone());
            }
        }
    }

    /// Says the next (`inc` 1) or previous (-1) objective, after the last
    /// the first: "build 1 farm , not done , 2 of 3". The map's objective
    /// comes first, without whether it is done. Every objective the player
    /// has had is a step, those done too.
    fn cmd_objectives(&mut self, cx: &mut Client, inc: i64) {
        let view = self.view();
        let now = view.objectives();
        let mut steps: Vec<Msg> = Vec::new();
        let map_objective = view.map_objective();
        if !map_objective.is_empty() {
            steps.push(map_objective.to_vec());
        }
        let seen = self.objectives_seen.iter();
        let new = now.iter().filter(|o| !self.objectives_seen.contains(o));
        for o in seen.chain(new) {
            let done = if now.contains(o) {
                Phrase::ObjectiveNotDone
            } else {
                Phrase::ObjectiveDone
            };
            steps.push([o.description.clone(), comma(), msg(done)].concat());
        }
        let n = steps.len();
        if n == 0 {
            cx.voice.item(&msg(Phrase::Nothing));
            return;
        }
        let i = match self.objective_at {
            Some(i) if i < n => (i as i64 + inc).rem_euclid(n as i64) as usize,
            _ if inc < 0 => n - 1,
            _ => 0,
        };
        self.objective_at = Some(i);
        let m = [
            steps.swap_remove(i),
            comma(),
            nb2msg(i as i64 + 1),
            msg(Phrase::OutOf),
            nb2msg(n as i64),
        ]
        .concat();
        cx.voice.item(&m);
    }

    /// Whether the player may cheat: in a game that takes cheats and
    /// allows them.
    fn cheats_allowed(&self) -> bool {
        self.takes_cheats() && self.view().cheats_allowed()
    }

    fn cmd_toggle_cheatmode(&mut self, cx: &mut Client) {
        if self.cheats_allowed() {
            self.send_command(cx, Command::Cheat(Cheat::ToggleMode));
            let state = if self.view().cheat_mode() {
                Phrase::IsNowOff
            } else {
                Phrase::IsNowOn
            };
            cx.voice
                .item(&[msg(Phrase::CheatMode), msg(state)].concat());
        } else {
            cx.voice.item(&cue(Cue::Beep));
        }
    }

    /// Runs a console or `cmd` command.
    pub fn execute_command(&mut self, cx: &mut Client, cmd: &str) {
        let now = cx.clock.now();
        if let Some(s) = cmd.strip_prefix("s ") {
            let s = s.split(' ').next().unwrap_or_default();
            match soundrts_world::nofloat::py_float(s) {
                Ok(speed) => {
                    self.speed = speed;
                    self.turn_due_at(now);
                }
                // Not a number: the speed stays.
                Err(_) => cx.voice.item(&cue(Cue::Beep)),
            }
        } else if cmd == "p" {
            if self.speed >= 1.0 {
                self.speed /= 10000.0;
            } else {
                self.speed *= 10000.0;
                self.turn_due_at(now);
            }
        } else if cmd == "m" {
            self.cheat_at_once(Cheat::InfiniteMana);
        } else if cmd == "h" {
            cx.voice.item(&msg(Phrase::ConsoleHelp));
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
            cx.voice.item(&cue(Cue::Beep));
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
    }

    /// A cheat that changes the game at once, even while the game waits:
    /// what the player hears next is told by the host at once. It is the
    /// player's even out of the game, as the console's always were; the
    /// console is open only in a game that allows cheats.
    pub(crate) fn cheat_at_once(&mut self, cheat: Cheat) {
        self.queue_at_once(Command::Cheat(cheat));
    }

    /// Starts typing a command of the console, in a game that allows
    /// cheats.
    fn cmd_console(&mut self, cx: &mut Client) {
        if !self.cheats_allowed() {
            cx.voice.item(&cue(Cue::Beep));
            return;
        }
        let entry = TextEntry::start(cx, &msg(Phrase::EnterCommand), console_char, "", false);
        self.mode = Mode::Console(entry);
    }

    /// A key pressed while a command of the console is typed: Enter runs
    /// it.
    pub(crate) fn console_key(&mut self, cx: &mut Client, mut entry: TextEntry, k: &KeyDown) {
        match entry.key(cx, k) {
            Typed::Done(cmd) => self.execute_command(cx, &cmd),
            Typed::Cancelled => {}
            Typed::Typing | Typed::Ignored => self.mode = Mode::Console(entry),
        }
    }

    /// Plays as the next player of the game (a cheat): the logins stay with
    /// the players, the events now come from the new one.
    fn cmd_change_player(&mut self, cx: &mut Client) {
        if !self.cheats_allowed() {
            cx.voice.item(&cue(Cue::Beep));
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
        let m = [msg(Phrase::YouAre), self.view().player_name(self.me)].concat();
        cx.voice.item(&m);
    }

    /// Opens the game menu: quit, the speeds, continue; on a server, for
    /// the room's host, pause or resume; in a game run here, save; for a
    /// listener to a record, what the listener's keys do. The keys go to
    /// it until a choice is made. In single-player the world waits
    /// meanwhile, and its sounds stop; in a game on a server or a record,
    /// the game goes on.
    fn cmd_gamemenu(&mut self, cx: &mut Client) {
        if self.waits_for_the_player() {
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
        let mut menu = Menu::new(msg(Phrase::Menu), Vec::new());
        menu.append(msg(Phrase::CancelGame), set(GameMenuChoice::Quit), None);
        let speeds = self.speeds_offered();
        if let Speeds::Paces { paused } = speeds {
            let item = if paused {
                Phrase::Resume
            } else {
                Phrase::Pause
            };
            let listen = |c| set(GameMenuChoice::Listen(c));
            menu.append(msg(item), listen(ListenChoice::PauseOrResume), None);
            let paces = [
                (msg(Phrase::SetSpeedToSlow), Pace::Slow),
                (msg(Phrase::SetSpeedToNormal), Pace::Normal),
                (msg(Phrase::SetSpeedToFast), Pace::Fast2),
                (
                    [msg(Phrase::SetSpeedToFast), nb2msg(4)].concat(),
                    Pace::Fast4,
                ),
                (
                    [msg(Phrase::SetSpeedToFast), nb2msg(8)].concat(),
                    Pace::Fast8,
                ),
                (Pace::Fastest.spoken(), Pace::Fastest),
            ];
            for (label, pace) in paces {
                menu.append(label, listen(ListenChoice::Pace(pace)), None);
            }
            let jumps = [
                (Phrase::JumpForward, Phrase::OneMinute, 1),
                (Phrase::JumpForward, Phrase::FiveMinutes, 5),
                (Phrase::JumpBack, Phrase::OneMinute, -1),
                (Phrase::JumpBack, Phrase::FiveMinutes, -5),
            ];
            for (way, how_long, minutes) in jumps {
                let label = [msg(way), msg(how_long)].concat();
                menu.append(label, listen(ListenChoice::Jump(minutes)), None);
            }
            menu.append(
                msg(Phrase::KeepThisRecord),
                listen(ListenChoice::Keep),
                None,
            );
        }
        // In a game on a server, the room's host sets the server's speed.
        if speeds == Speeds::Game {
            menu.append(
                msg(Phrase::SetSpeedToSlow),
                set(GameMenuChoice::Speed(0.5)),
                None,
            );
            menu.append(
                msg(Phrase::SetSpeedToNormal),
                set(GameMenuChoice::Speed(1.0)),
                None,
            );
            menu.append(
                msg(Phrase::SetSpeedToFast),
                set(GameMenuChoice::Speed(2.0)),
                None,
            );
            menu.append(
                [msg(Phrase::SetSpeedToFast), nb2msg(4)].concat(),
                set(GameMenuChoice::Speed(4.0)),
                None,
            );
        }
        if let Some(paused) = self.paused_for_the_host(cx) {
            let item = if paused {
                Phrase::Resume
            } else {
                Phrase::Pause
            };
            menu.append(msg(item), set(GameMenuChoice::PauseOrResume), None);
        }
        if self.can_be_saved() {
            menu.append(msg(Phrase::SaveGame), set(GameMenuChoice::Save), None);
        }
        menu.append(msg(Phrase::ContinueGame), Action::Nothing, None);
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
    ) -> Result<(), Exit> {
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
            Some(GameMenuChoice::PauseOrResume) => self.pause_or_resume(cx),
            Some(GameMenuChoice::Save) => self.cmd_save_game(cx),
            Some(GameMenuChoice::Speed(s)) => {
                self.send_command(cx, Command::Speed(s));
                // The next single-player game starts at this speed.
                if self.speed_is_the_worlds() {
                    cx.config.speed = s;
                    cx.save_config();
                }
            }
            Some(GameMenuChoice::Listen(c)) => match c {
                ListenChoice::PauseOrResume => self.listen_pause(cx),
                ListenChoice::Pace(pace) => self.set_pace(cx, pace),
                ListenChoice::Jump(minutes) => self.listen_jump(cx, minutes),
                ListenChoice::Keep => self.keep_record(cx),
            },
            None => {}
        }
    }

    /// Asks the world to quit; asked again, while the connection is lost,
    /// or while a game on a server is paused, quits at once. A game run
    /// here that is not over is saved first, as the last game.
    fn gm_quit(&mut self, cx: &mut Client) {
        if self.leave_while_returning(cx)
            || self.leave_while_watching(cx)
            || self.leave_while_paused(cx)
        {
            return;
        }
        if !self.already_asked_to_quit {
            self.save_on_leaving(cx);
            self.turn_due_at(cx.clock.now());
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
            cx.voice.item(&[bell(), msg(Phrase::IsNowOn)].concat());
            self.previous_nb_minutes = (self.known.time() / 60.0) as i64;
        } else {
            cx.voice.item(&[bell(), msg(Phrase::IsNowOff)].concat());
        }
    }

    /// Rings every game minute.
    pub(crate) fn eventually_play_bell(&mut self, cx: &Client) {
        let nb_minutes = (self.known.time() / 60.0) as i64;
        if self.previous_nb_minutes != nb_minutes {
            cx.audio.play_stereo(
                cx.sounds.cue(Cue::PositionalBeep),
                StereoVolume::Both(1.0),
                0.0,
            );
            self.previous_nb_minutes = nb_minutes;
        }
    }

    /// Ticks every turn, quietly.
    pub(crate) fn play_tick(&self, cx: &Client) {
        cx.audio.play_stereo(
            cx.sounds.cue(Cue::PositionalBeep),
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
        let t = self.known.time() as i64;
        let (m, s) = (t.div_euclid(60), t.rem_euclid(60));
        let m = [
            nb2msg(m),
            msg(Phrase::Minutes),
            nb2msg(s),
            msg(Phrase::Seconds),
            comma(),
            msg(Phrase::Speed),
            vec![MsgPart::literal(format!("{:.1}", self.real_speed()))],
        ]
        .concat();
        cx.voice.item(&m);
    }

    // first person mode

    pub(crate) fn toggle_immersion(&mut self, cx: &mut Client) {
        self.immersion = !self.immersion;
        if self.immersion {
            self.cmd_unit_status(cx);
            cx.voice.item(&msg(Phrase::FirstPersonMode));
        } else {
            cx.voice.item(&msg(Phrase::MapMode));
        }
        self.follow_mode = self.immersion;
    }

    /// Cancels the order being given, or leaves first person mode, zoom
    /// mode, or the object selected. While the connection is lost, leaves
    /// the game at once.
    fn cmd_escape(&mut self, cx: &mut Client) {
        if self.leave_while_returning(cx) {
            return;
        }
        if self.order.is_some() {
            cx.voice.item(&msg(Phrase::Cancel));
            self.order = None;
        } else if self.immersion {
            self.toggle_immersion(cx);
        } else if self.zoom.is_some() {
            self.cmd_toggle_zoom(cx);
        } else if self.target.is_some()
            && let Some(p) = self.place
        {
            self.select_and_say_square(cx, p, &Vec::new());
        }
    }

    fn say_compass(&mut self, cx: &mut Client) {
        let compass: Msg = msg(direction_to_msgpart(self.o));
        if self.previous_compass.as_ref() != Some(&compass) {
            cx.voice.item(&compass);
            self.previous_compass = Some(compass);
        }
    }

    /// Turns the first unit controlled, and the listener with it.
    fn rotate(&mut self, cx: &mut Client, degrees: f64) {
        let Some(v) = self.group.first().and_then(|&g| self.known.unit_mut(g)) else {
            return;
        };
        v.o_override = Some(v.o() + degrees);
        self.o += degrees;
        self.say_compass(cx);
        cx.audio.update();
    }

    pub(crate) fn cmd_rotate_left(&mut self, cx: &mut Client) {
        self.rotate(cx, 45.0)
    }

    pub(crate) fn cmd_rotate_right(&mut self, cx: &mut Client) {
        self.rotate(cx, -45.0)
    }
}

#[cfg(test)]
mod tests {
    use super::console_char;
    use crate::game::testing::{cheat, training};
    use crate::input::{Key, Mods};

    #[test]
    fn the_objectives_are_said_one_at_a_time_with_whether_each_is_done() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        cheat(&mut g, me, "add_objective 1 build one_f farm");
        cheat(&mut g, me, "add_objective 2 build one_f barracks");
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        cheat(&mut g, me, "objective_complete 1");
        h.run_until_turn(&mut g, turn + 2);
        h.run_until_voice_idle(&mut g);
        let mut ask = |key, mods| h.key(&mut g, key, mods).remove(0);
        // jl1's own objective comes first.
        assert_eq!(ask(Key::F9, Mods::NONE), "neutralized enemy , 1 of 3");
        assert_eq!(ask(Key::F9, Mods::NONE), "build 1 farm , done , 2 of 3");
        assert_eq!(
            ask(Key::F9, Mods::NONE),
            "build 1 barracks , not done , 3 of 3"
        );
        assert_eq!(ask(Key::F9, Mods::NONE), "neutralized enemy , 1 of 3");
        assert_eq!(
            ask(Key::F9, Mods::SHIFT),
            "build 1 barracks , not done , 3 of 3"
        );
    }

    #[test]
    fn a_console_speed_that_is_not_a_number_beeps_and_keeps_the_speed() {
        let (mut h, mut g) = training();
        h.run_until_voice_idle(&mut g);
        let speed = g.game.speed;
        let from = h.utterances().len();
        g.game.execute_command(&mut h.client, "s fast");
        assert_eq!(h.said_since(from), ["<ui/beep>"]);
        assert_eq!(g.game.speed, speed);
        g.game.execute_command(&mut h.client, "s 2");
        assert_eq!(g.game.speed, 2.0);
    }

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
