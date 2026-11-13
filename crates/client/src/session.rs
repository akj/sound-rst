//! Game sessions (`game.py`): a training game against computers on a
//! multiplayer map, or a mission of a campaign. A session builds the world,
//! runs the interface, and says the score.

use std::sync::Arc;

use soundrts_protocol::{ClientMsg, Slot, SlotIndex, SlotPlayer, Speed};
use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::player::ControllerKind;
use soundrts_world::{
    AiScripts, Controller, DefaultTrigger, Rules, World, multiplayer_default_triggers,
};

use crate::client::Client;
use crate::game::GameInterface;
use crate::game::lockstep::NetDriver;
use crate::game::stats::score_msgs;
use crate::mapfile::Map;
use crate::menu::Exit;
use crate::net::GameLink;

/// The kind of a session, which decides its rules of victory, whether
/// factions replace the start units, and what happens before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// Against computers, on a multiplayer map.
    Training,
    /// A chapter of a campaign: its intro is played first.
    Mission,
    /// On a server, with other players.
    Multiplayer,
}

/// What a game on a server adds to a session.
#[derive(Clone, Debug)]
pub struct NetSession {
    /// The local player's slot.
    pub you: SlotIndex,
    pub speed: Speed,
}

/// A game to play: map, players, seed.
pub struct Session {
    pub kind: SessionKind,
    pub map: Arc<Map>,
    /// The local player first.
    pub players: Vec<Controller>,
    pub seed: i64,
    pub allow_cheatmode: bool,
    /// Whether the local player won, once the game is over.
    pub has_victory: bool,
    /// For a game on a server.
    pub net: Option<NetSession>,
}

impl Session {
    /// A training game. `players` are the local login then computer types
    /// (`easy`, `aggressive`, `ai2`); `factions` and `alliances` go with
    /// them in order.
    pub fn training(
        cx: &Client,
        map: Arc<Map>,
        players: &[String],
        factions: &[String],
        alliances: &[String],
    ) -> Session {
        let mut controllers = vec![Controller::human(cx.config.login.as_str(), true)];
        controllers.extend(players.iter().skip(1).map(|p| Controller::computer(p)));
        for ((c, f), a) in controllers.iter_mut().zip(factions).zip(alliances) {
            c.faction = f.clone();
            c.alliance = Some(a.clone());
        }
        Session {
            kind: SessionKind::Training,
            map,
            players: controllers,
            seed: Self::new_seed(cx),
            allow_cheatmode: true,
            has_victory: false,
            net: None,
        }
    }

    /// A mission on a campaign chapter's map, alone.
    pub fn mission(cx: &Client, map: Arc<Map>) -> Session {
        Session {
            kind: SessionKind::Mission,
            map,
            players: vec![Controller::human(cx.config.login.as_str(), true)],
            seed: Self::new_seed(cx),
            allow_cheatmode: true,
            has_victory: false,
            net: None,
        }
    }

    /// A game on a server, with the room's slots as players in order;
    /// `you` is the local player's slot.
    pub fn multiplayer(
        map: Arc<Map>,
        slots: &[Slot],
        you: SlotIndex,
        seed: i64,
        speed: Speed,
    ) -> Session {
        let players = slots
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let mut c = match &slot.player {
                    // Said as written by the world too (see `literal`).
                    SlotPlayer::Human(login) => Controller::human(
                        &format!("{}{login}", crate::sound_cache::LITERAL),
                        i == you,
                    ),
                    SlotPlayer::Computer(level) => Controller::computer(level.ai_type()),
                };
                c.faction = slot.faction.as_str().to_owned();
                c.alliance = Some(slot.alliance.to_string());
                c
            })
            .collect();
        Session {
            kind: SessionKind::Multiplayer,
            map,
            players,
            seed,
            allow_cheatmode: false,
            has_victory: false,
            net: Some(NetSession { you, speed }),
        }
    }

    /// The number of human players.
    fn humans(&self) -> usize {
        self.players
            .iter()
            .filter(|c| matches!(c.kind, ControllerKind::Human { .. }))
            .count()
    }

    /// A seed drawn from the client's randomness; a fixed seed replaces it
    /// (the draw still happens, so the noises that follow stay the same).
    fn new_seed(cx: &Client) -> i64 {
        let drawn = cx.rng.randint(0, 10000);
        cx.game.seed.unwrap_or(drawn)
    }

    fn default_triggers(&self) -> Vec<DefaultTrigger> {
        match self.kind {
            SessionKind::Training | SessionKind::Multiplayer => multiplayer_default_triggers(),
            SessionKind::Mission => Vec::new(),
        }
    }

    /// Plays the game to its end and says the score.
    pub fn run(&mut self, cx: &mut Client) -> Result<(), Exit> {
        let Some(mut game) = self.start(cx) else {
            self.leave_server_game(cx);
            return Ok(());
        };
        let result = game.run_loop(cx);
        // Before the score, so that the others' game does not wait.
        self.leave_server_game(cx);
        result?;
        self.finish(cx, &mut game);
        cx.input.set_game(None);
        if self.net.is_some() {
            if let Some(n) = &mut cx.net {
                n.game = None;
            }
            cx.voice
                .menu(&[msg(mp::MENU), msg(mp::MAKE_A_SELECTION)].concat());
        }
        Ok(())
    }

    /// In a game on a server: tells the server the player has left the
    /// game, unless the server ended it.
    fn leave_server_game(&self, cx: &mut Client) {
        if self.net.is_some() && !crate::net::game_link(cx).is_some_and(|g| g.ended) {
            crate::net::send(cx, ClientMsg::LeaveGame);
        }
    }

    /// Builds the world and the interface, plays the intro and says the
    /// objective. `None` if the map cannot be built (a beep and the error
    /// are said).
    pub fn start(&mut self, cx: &mut Client) -> Option<GameInterface> {
        cx.res.set_map(Some(&self.map));
        let rules = Rules::load(&[cx.res.rules_text()]);
        let ai = AiScripts::load(&[cx.res.ai_text()]);
        let mut world = World::new(rules, ai, self.default_triggers(), self.seed);
        let built = world
            .load_and_build_map(&self.map.definition)
            .map_err(|e| e.0)
            .and_then(|()| {
                world
                    .populate_map(&self.players, true, self.kind != SessionKind::Mission)
                    .map_err(|e| e.0)
            });
        let players = match built {
            Ok(p) => p,
            Err(e) => {
                let e = format!("map error: {e}");
                cx.log.warning(&e);
                let m: Msg = [msg(mp::BEEP), vec![MsgPart::Text(e)]].concat();
                cx.voice.alert(&m);
                return None;
            }
        };
        for w in world.drain_warnings() {
            cx.log.warning(format!("world: {w}"));
        }
        let local = self.net.as_ref().map_or(0, |n| n.you);
        let mut game = GameInterface::new(cx, world, players[local], self.allow_cheatmode);
        if let Some(n) = &self.net {
            game.speed = n.speed.factor();
            game.net = Some(NetDriver {
                you: n.you,
                slots: players[..self.players.len()].to_vec(),
                interval: n.speed.turn_interval().as_secs_f64(),
                digests: Vec::new(),
            });
            if let Some(net) = &mut cx.net {
                net.game = Some(GameLink::default());
            }
        }
        cx.input.set_game(Some(game.progress()));
        game.update_orders_list(cx);
        self.pre_run(cx, &game);
        game.say_objective(cx);
        game.load_bindings(cx);
        if self.net.is_some() {
            crate::net::send(cx, ClientMsg::Loaded);
        }
        Some(game)
    }

    fn pre_run(&self, cx: &mut Client, game: &GameInterface) {
        if self.kind == SessionKind::Multiplayer && self.humans() > 1 {
            countdown(cx);
        }
        if self.kind == SessionKind::Mission {
            let intro: Vec<String> = game.world.map.intro.iter().map(i64::to_string).collect();
            if !intro.is_empty() {
                cx.play_sequence(&intro);
            }
        }
    }

    /// Says the score and waits until it is said (a key skips ahead).
    pub fn finish(&mut self, cx: &mut Client, game: &mut GameInterface) {
        for m in score_msgs(&game.world, game.me, cx.res.style()) {
            cx.voice.info(&m);
        }
        cx.voice.flush(true);
        game.stop_noises(cx);
        self.has_victory = game.world.player(game.me).has_victory;
    }
}

/// "The game will start in 5 seconds", then the seconds, as the original
/// says before a game with other players. The server's messages are still
/// read meanwhile.
fn countdown(cx: &mut Client) {
    cx.voice.important(&msg(mp::THE_GAME_WILL_START));
    for n in (0..=5).rev() {
        cx.voice.item(&nb2msg(n));
        for _ in 0..100 {
            crate::net::pump(cx);
            cx.voice.update();
            cx.clock.sleep(0.01);
        }
    }
    cx.input.clear_keydowns();
}
