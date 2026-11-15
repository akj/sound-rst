//! Game sessions (`game.py`): a training game against computers on a
//! multiplayer map, or a mission of a campaign, which this process runs;
//! or a game a server runs ([`play_on_server`]). A session builds the
//! world, runs the interface, and says the score.

use std::sync::Arc;

use soundrts_protocol::ClientMsg;
use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{Controller, DefaultTrigger, GameSetup, Host, multiplayer_default_triggers};

use crate::campaign::Campaign;
use crate::client::Client;
use crate::game::GameInterface;
use crate::game::stats::score_msgs;
use crate::mapfile::Map;
use crate::menu::Exit;
use crate::message::{Category, News};
use crate::net::Seated;
use crate::net::words::{self, text};
use crate::saves::{SaveInfo, SavedGame};

/// The kind of a session, which decides its rules of victory, whether
/// factions replace the start units, and what happens before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// Against computers, on a multiplayer map.
    Training,
    /// A chapter of a campaign: its intro is played first.
    Mission,
}

impl SessionKind {
    /// The triggers of a map that has none of its own.
    pub fn default_triggers(&self) -> Vec<DefaultTrigger> {
        match self {
            SessionKind::Training => multiplayer_default_triggers(),
            SessionKind::Mission => Vec::new(),
        }
    }

    /// Whether the factions replace the start units with their own.
    pub fn equivalents(&self) -> bool {
        *self != SessionKind::Mission
    }
}

/// A game to play here: map, players, seed.
pub struct Session {
    pub kind: SessionKind,
    pub map: Arc<Map>,
    /// The local player first.
    pub players: Vec<Controller>,
    pub seed: i64,
    pub allow_cheatmode: bool,
    /// Whether the local player won, once the game is over.
    pub has_victory: bool,
    /// What a save of the game says it is.
    pub save: SaveInfo,
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
        let save = SaveInfo {
            game: SavedGame::Training {
                map: map.name.clone(),
            },
            title: map.title.clone(),
        };
        Session {
            kind: SessionKind::Training,
            map,
            players: controllers,
            seed: Self::new_seed(cx),
            allow_cheatmode: true,
            has_victory: false,
            save,
        }
    }

    /// A mission on the map of chapter `chapter` of `campaign`, alone.
    pub fn mission(cx: &Client, map: Arc<Map>, campaign: &Campaign, chapter: usize) -> Session {
        let save = SaveInfo {
            game: SavedGame::Mission {
                campaign: campaign.name.clone(),
                chapter,
            },
            title: [
                campaign.title.clone(),
                nb2msg(chapter as i64),
                map.title.get(1..).unwrap_or_default().to_vec(),
            ]
            .concat(),
        };
        Session {
            kind: SessionKind::Mission,
            map,
            players: vec![Controller::human(cx.config.login.as_str(), true)],
            seed: Self::new_seed(cx),
            allow_cheatmode: true,
            has_victory: false,
            save,
        }
    }

    /// The world's seed: the fixed one if there is one, else drawn from the
    /// client's randomness.
    fn new_seed(cx: &Client) -> i64 {
        cx.game.seed.unwrap_or_else(|| cx.rng.randint(0, 10000))
    }

    /// Plays the game to its end and says the score. The map's resources,
    /// loaded by [`Session::start`], are unloaded however it ends.
    pub fn run(&mut self, cx: &mut Client) -> Result<(), Exit> {
        let result = self.play(cx);
        cx.res.set_map(None);
        result
    }

    fn play(&mut self, cx: &mut Client) -> Result<(), Exit> {
        let Some(mut game) = self.start(cx) else {
            return Ok(());
        };
        if let Err(exit) = game.run_loop(cx) {
            // The window was closed: what the game was is kept.
            if !game.already_asked_to_quit {
                game.save_on_leaving(cx);
            }
            return Err(exit);
        }
        self.has_victory = finish(cx, &mut game);
        cx.input.set_game(None);
        Ok(())
    }

    /// Builds the world and the interface, and tells the intro and the
    /// objective as a story: in single-player the first turn waits until
    /// it is told. `None` if the map cannot be built (a beep and the error
    /// are said).
    pub fn start(&mut self, cx: &mut Client) -> Option<GameInterface> {
        cx.res.set_map(Some(&self.map));
        let setup = GameSetup {
            rules: cx.res.rules_text(),
            ai: cx.res.ai_text(),
            map: self.map.definition.clone(),
            seed: self.seed,
            controllers: self.players.clone(),
            default_triggers: self.kind.default_triggers(),
            allow_cheatmode: self.allow_cheatmode,
            random_starts: true,
            equivalents: self.kind.equivalents(),
        };
        let mut host = match Host::new(&setup) {
            Ok(host) => host,
            Err(e) => {
                let e = format!("map error: {e}");
                cx.log.warning(&e);
                let m: Msg = [msg(mp::BEEP), vec![MsgPart::Text(e)]].concat();
                cx.voice.alert(&m);
                return None;
            }
        };
        for w in host.drain_warnings() {
            cx.log.warning(format!("world: {w}"));
        }
        let me = host.seats()[0];
        let mut game = GameInterface::local(cx, host, me);
        game.set_save_info(self.save.clone());
        let intro = self.intro(&game);
        prepare(cx, &mut game, intro);
        Some(game)
    }

    /// A mission's intro: its sounds or texts, in order.
    fn intro(&self, game: &GameInterface) -> Vec<Msg> {
        if self.kind != SessionKind::Mission {
            return Vec::new();
        }
        game.view()
            .map_intro()
            .iter()
            .map(|n| vec![MsgPart::from(n.to_string().as_str())])
            .collect()
    }

    /// Says what the end of the game left to say, then the score, and
    /// waits until it is said (a key skips ahead).
    pub fn finish(&mut self, cx: &mut Client, game: &mut GameInterface) {
        self.has_victory = finish(cx, game);
    }
}

/// Readies a new game's interface: the order menus, the keys, and the
/// story told before the first turn, `intro` then the objective. In a game
/// run here the first turn waits until the story is told.
pub(crate) fn prepare(cx: &mut Client, game: &mut GameInterface, intro: Vec<Msg>) {
    cx.input.set_game(Some(game.progress()));
    game.update_orders_list(cx);
    let story = intro.into_iter().chain(game.objective()).collect();
    game.tell_story(cx, story);
    game.load_bindings(cx);
}

/// Says what the end of the game left to say, then the score, and waits
/// until it is said (a key skips ahead). Whether the player won.
pub fn finish(cx: &mut Client, game: &mut GameInterface) -> bool {
    cx.voice.flush(true);
    game.end_game();
    for m in score_msgs(game.view(), cx.res.style()) {
        cx.news(News::new(Category::System, m));
    }
    cx.voice.flush(true);
    game.stop_noises(cx);
    game.view().has_victory()
}

/// Joins the game a server runs for the player, on the room's `map`.
/// `None` if it cannot be played: what went wrong is said, and the server
/// told that the player leaves.
pub fn join_server_game(cx: &mut Client, seated: &Seated) -> Option<GameInterface> {
    match crate::game::server::join(cx, seated) {
        Ok(mut game) => {
            prepare(cx, &mut game, Vec::new());
            Some(game)
        }
        Err(why) => {
            if let Some(why) = why {
                cx.log.warning(format!("cannot play the game: {why}"));
                cx.news(News::new(Category::System, text(words::CANNOT_BE_RIGHT)));
            }
            leave_server_game(cx);
            None
        }
    }
}

/// Plays the game a server runs for the player, on the room's `map`, to
/// its end, and says the score. The map's resources are loaded while it
/// is played.
pub fn play_on_server(cx: &mut Client, map: &Map, seated: &Seated) -> Result<(), Exit> {
    cx.res.set_map(Some(map));
    let result = play_server_game(cx, seated);
    cx.res.set_map(None);
    result
}

fn play_server_game(cx: &mut Client, seated: &Seated) -> Result<(), Exit> {
    let Some(mut game) = join_server_game(cx, seated) else {
        return Ok(());
    };
    let result = game.run_loop(cx);
    // Before the score, so that the server knows at once.
    leave_server_game(cx);
    result?;
    finish(cx, &mut game);
    cx.input.set_game(None);
    if let Some(n) = &mut cx.net {
        n.game = None;
    }
    cx.voice
        .menu(&[msg(mp::MENU), msg(mp::MAKE_A_SELECTION)].concat());
    Ok(())
}

/// Tells the server the player has left its game, unless it ended the
/// game for the player.
pub fn leave_server_game(cx: &mut Client) {
    if !crate::net::game_link(cx).is_some_and(|g| g.ended) {
        crate::net::send(cx, ClientMsg::LeaveGame);
    }
}
