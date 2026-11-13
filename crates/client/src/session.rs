//! Game sessions (`game.py`): a training game against computers on a
//! multiplayer map, or a mission of a campaign. A session builds the world,
//! runs the interface, and says the score.

use std::sync::Arc;

use soundrts_world::msg::{Msg, MsgPart, msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{
    AiScripts, Controller, DefaultTrigger, Rules, World, multiplayer_default_triggers,
};

use crate::client::Client;
use crate::game::GameInterface;
use crate::game::stats::score_msgs;
use crate::mapfile::Map;
use crate::menu::Exit;

/// The kind of a session, which decides its rules of victory, whether
/// factions replace the start units, and what happens before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// Against computers, on a multiplayer map.
    Training,
    /// A chapter of a campaign: its intro is played first.
    Mission,
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
        }
    }

    /// A seed drawn from the client's randomness; a fixed seed replaces it
    /// (the draw still happens, so the noises that follow stay the same).
    fn new_seed(cx: &Client) -> i64 {
        let drawn = cx.rng.randint(0, 10000);
        cx.game.seed.unwrap_or(drawn)
    }

    fn default_triggers(&self) -> Vec<DefaultTrigger> {
        match self.kind {
            SessionKind::Training => multiplayer_default_triggers(),
            SessionKind::Mission => Vec::new(),
        }
    }

    /// Plays the game to its end and says the score.
    pub fn run(&mut self, cx: &mut Client) -> Result<(), Exit> {
        let Some(mut game) = self.start(cx) else {
            return Ok(());
        };
        game.run_loop(cx)?;
        self.finish(cx, &mut game);
        cx.input.set_game(None);
        Ok(())
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
                    .populate_map(&self.players, true, self.kind == SessionKind::Training)
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
        let mut game = GameInterface::new(cx, world, players[0], self.allow_cheatmode);
        cx.input.set_game(Some(game.progress()));
        game.update_orders_list(cx);
        self.pre_run(cx, &game);
        game.say_objective(cx);
        game.load_bindings(cx);
        Some(game)
    }

    fn pre_run(&self, cx: &mut Client, game: &GameInterface) {
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
