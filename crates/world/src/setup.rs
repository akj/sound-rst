//! The description of a game's set-up: everything a world is built from.

use crate::ai::Skill;
use crate::entity::PlayerId;
use crate::player::Controller;
use crate::rules::{AiScripts, Rules};
use crate::world::{DefaultTrigger, World};

/// Everything a game's world is built from. The same set-up builds the same
/// world on any machine: a game, the replay of its record and every
/// client of a game on a server build their world from one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GameSetup {
    /// Every `rules.txt` of the layers (the game, a mod, a campaign, the
    /// map), joined in order.
    pub rules: String,
    /// Every `ai.txt` of the layers, joined in order.
    pub ai: String,
    /// The map's definition.
    pub map: String,
    pub seed: i64,
    /// Who plays each seat, with faction and alliance, in seat order.
    pub controllers: Vec<Controller>,
    /// The triggers of a map that has none of its own.
    pub default_triggers: Vec<DefaultTrigger>,
    /// Whether the players may cheat.
    pub allow_cheatmode: bool,
    /// The level of the computers that the map makes, as in a campaign
    /// mission.
    pub computer_skill: Skill,
    /// The players start at random starting places, not in seat order.
    pub random_starts: bool,
    /// The factions replace the start units with their own.
    pub equivalents: bool,
}

impl GameSetup {
    /// A game with other players on a server: the map's default triggers
    /// if it has none, no cheats, random starting places, and the
    /// factions' own start units. `controllers` are the room's slots in
    /// order; which humans receive events is theirs to say (a client
    /// running the game marks its own player, a host every human).
    pub fn multiplayer(
        rules: String,
        ai: String,
        map: String,
        seed: i64,
        controllers: Vec<Controller>,
    ) -> GameSetup {
        GameSetup {
            rules,
            ai,
            map,
            seed,
            controllers,
            default_triggers: crate::world::multiplayer_default_triggers(),
            allow_cheatmode: false,
            computer_skill: Skill::NORMAL,
            random_starts: true,
            equivalents: true,
        }
    }

    /// Builds the world and its map, and creates the players. Returns the
    /// world and the players of the controllers, in order; the error is the
    /// map's.
    pub(crate) fn build_world(&self) -> Result<(World, Vec<PlayerId>), String> {
        let rules = Rules::load(&[self.rules.as_str()]);
        let ai = AiScripts::load(&[self.ai.as_str()]);
        let mut world = World::new(rules, ai, self.default_triggers.clone(), self.seed);
        world.allow_cheatmode = self.allow_cheatmode;
        world.computer_skill = self.computer_skill;
        world.load_and_build_map(&self.map).map_err(|e| e.0)?;
        let players = world
            .populate_map(&self.controllers, self.random_starts, self.equivalents)
            .map_err(|e| e.0)?;
        Ok((world, players))
    }
}
