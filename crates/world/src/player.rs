//! Players and their controllers (`worldplayerbase.py` data,
//! `worldplayerhuman.py`, `worldplayerstats.py`, and the controllers of
//! `worldclient.py`).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::ai::Computer;
use crate::entity::{Entity, EntityId, SquareId, StatType};
use crate::events::Event;
use crate::msg::{Msg, MsgPart, msg};
use crate::msgparts as mp;
use crate::nofloat::PRECISION;
use crate::orders::Order;
use crate::triggers::Expr;

/// Who controls a player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerKind {
    /// A person. `local` is true for the person using this machine: only a
    /// local human receives events.
    Human { login: String, local: bool },
    /// A computer player with an AI type from `ai.txt` (`timers` does
    /// nothing; `ai2` makes a random player).
    Computer { ai_type: String },
    /// A neutral computer player: one is created for each `computer_only`
    /// start of the map.
    Neutral,
}

/// Every field but `local`, which differs between the machines of one
/// game.
impl Hash for ControllerKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            ControllerKind::Human { login, local: _ } => login.hash(state),
            ControllerKind::Computer { ai_type } => ai_type.hash(state),
            ControllerKind::Neutral => {}
        }
    }
}

/// A controller: who plays, with which alliance and faction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Controller {
    pub kind: ControllerKind,
    /// Players with the same alliance are allies. `None` means no alliance.
    pub alliance: Option<String>,
    /// A faction name, or `random_faction`.
    pub faction: String,
}

impl Controller {
    /// A human, without alliance, with a random faction.
    pub fn human(login: &str, local: bool) -> Controller {
        Controller {
            kind: ControllerKind::Human {
                login: login.to_owned(),
                local,
            },
            alliance: None,
            faction: "random_faction".into(),
        }
    }

    /// A computer player, allied with the other computers (alliance `ai`),
    /// with a random faction.
    pub fn computer(ai_type: &str) -> Controller {
        Controller {
            kind: ControllerKind::Computer {
                ai_type: ai_type.to_owned(),
            },
            alliance: Some("ai".into()),
            faction: "random_faction".into(),
        }
    }

    pub(crate) fn neutral() -> Controller {
        Controller {
            kind: ControllerKind::Neutral,
            alliance: Some("ai".into()),
            faction: "random_faction".into(),
        }
    }

    pub fn is_neutral(&self) -> bool {
        self.kind == ControllerKind::Neutral
    }

    /// The login the original uses: the person's login, or `ai_<type>`.
    pub fn login(&self) -> String {
        match &self.kind {
            ControllerKind::Human { login, .. } => login.clone(),
            ControllerKind::Computer { ai_type } => format!("ai_{ai_type}"),
            ControllerKind::Neutral => "ai_timers".into(),
        }
    }
}

/// The kind of player, which decides a few behaviors.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlayerKind {
    Human,
    /// The scripted computer player; `ai_type` is `timers` for one that
    /// does nothing.
    Computer {
        ai_type: String,
    },
    /// The random computer player.
    Computer2,
}

/// A mission objective.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Objective {
    pub number: String,
    pub description: Vec<i64>,
}

/// A trigger of the map: when the condition holds, the action runs once.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Trigger {
    pub condition: Expr,
    pub action: Expr,
}

/// A remembered entity and when it was last seen.
#[derive(Clone, Debug, Hash)]
pub struct Remembrance {
    pub time_stamp: i64,
    pub snapshot: Arc<Entity>,
}

/// What the game statistics count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatEvent {
    Produced,
    Lost,
    Killed,
    Gathered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatTarget {
    Unit,
    Building,
    /// A resource, by index.
    Resource(usize),
}

impl From<StatType> for StatTarget {
    fn from(t: StatType) -> Self {
        match t {
            StatType::Unit => StatTarget::Unit,
            StatType::Building => StatTarget::Building,
        }
    }
}

/// The statistics of a player (`worldplayerstats.Stats`). The client
/// builds the score messages from them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerStats {
    counts: BTreeMap<(StatEvent, StatTarget), i64>,
    game_duration: Option<i64>,
}

impl PlayerStats {
    pub fn add(&mut self, event: StatEvent, target: StatTarget, inc: i64) {
        *self.counts.entry((event, target)).or_insert(0) += inc;
    }

    pub fn get(&self, event: StatEvent, target: StatTarget) -> i64 {
        self.counts.get(&(event, target)).copied().unwrap_or(0)
    }

    /// Stops the game clock of the statistics at `time`.
    pub fn freeze(&mut self, time: i64) {
        self.game_duration = Some(time);
    }

    /// The duration of the game for this player: until victory or defeat,
    /// or `now`.
    pub fn game_duration(&self, now: i64) -> i64 {
        self.game_duration.unwrap_or(now)
    }

    /// Minutes and seconds of [`PlayerStats::game_duration`].
    pub fn game_duration_in_minutes_seconds(&self, now: i64) -> (i64, i64) {
        let t = self.game_duration(now) / 1000;
        let m = t / 60;
        (m, t - m * 60)
    }

    /// What was gathered of a resource and is not left.
    pub fn consumed(&self, resource: usize, resources: &[i64]) -> i64 {
        self.get(StatEvent::Gathered, StatTarget::Resource(resource)) - resources[resource]
    }

    /// The score: units and buildings produced, lost (negative) and killed,
    /// then resources gathered and consumed.
    pub fn score(&self, resources: &[i64]) -> i64 {
        let mut score = 0;
        for t in [StatTarget::Unit, StatTarget::Building] {
            score += self.get(StatEvent::Produced, t);
            score -= self.get(StatEvent::Lost, t);
            score += self.get(StatEvent::Killed, t);
        }
        for i in 0..resources.len() {
            score += crate::nofloat::floordiv(
                self.get(StatEvent::Gathered, StatTarget::Resource(i))
                    + self.consumed(i, resources),
                PRECISION,
            );
        }
        score
    }
}

/// A player of the game.
pub struct Player {
    /// The id in the text protocol (players share the entity counter).
    pub obj_id: u32,
    pub kind: PlayerKind,
    pub controller: Controller,
    pub neutral: bool,
    pub faction: String,
    /// The allies, the player included. A player in an alliance appears
    /// twice, like in the original, which counts its own units twice when
    /// healing.
    pub allied: Vec<super::entity::PlayerId>,
    /// 1 for the first non-neutral player, and so on.
    pub number: Option<u32>,
    /// The entities the player perceives now.
    pub perception: BTreeSet<EntityId>,
    /// The entities the player remembers, by id.
    pub memory: BTreeMap<EntityId, Remembrance>,
    pub objectives: Vec<Objective>,
    pub units: Vec<EntityId>,
    /// Build orders whose cost is reserved.
    pub budget: Vec<Order>,
    /// Researched upgrades; an upgrade researched twice appears twice.
    pub upgrades: Vec<String>,
    pub forbidden_techs: Vec<String>,
    pub observed_before_squares: BTreeSet<SquareId>,
    pub strictly_observed_before_squares: BTreeSet<SquareId>,
    pub observed_squares: BTreeSet<SquareId>,
    /// Entities revealed by their actions, until a time.
    pub observed_objects: BTreeMap<EntityId, i64>,
    pub detected_units: BTreeSet<EntityId>,
    pub allied_control: Vec<super::entity::PlayerId>,
    pub(crate) known_enemies: BTreeMap<SquareId, (i64, Vec<EntityId>)>,
    pub(crate) enemy_menace: BTreeMap<SquareId, i64>,
    pub(crate) enemy_presence: Vec<SquareId>,
    pub(crate) places_with_corpses: BTreeSet<SquareId>,
    pub(crate) places_with_friends: BTreeSet<SquareId>,
    pub(crate) cataclysmic_places: BTreeSet<SquareId>,
    pub(crate) subsquare_threat: BTreeMap<(i64, i64), i64>,
    pub(crate) counterattack_places: Vec<SquareId>,
    /// Resources with [`PRECISION`].
    pub resources: Vec<i64>,
    pub storage_bonus: Vec<i64>,
    pub food: i64,
    pub used_food: i64,
    pub cheatmode: bool,
    pub has_victory: bool,
    pub has_been_defeated: bool,
    /// After defeat, the player keeps watching instead of quitting.
    pub observer_if_defeated: bool,
    /// The units selected by `control`. `None` until the first `control`.
    pub group: Option<Vec<EntityId>>,
    pub group_had_enough_mana: bool,
    pub stats: PlayerStats,
    pub triggers: Vec<Trigger>,
    /// The menace of the player's mobile soldiers.
    pub menace: i64,
    pub(crate) buckets: BTreeMap<(i64, i64), Vec<EntityId>>,
    /// The squares an exploring unit will visit, shuffled once.
    pub places_to_explore: Option<Vec<SquareId>>,
    /// The random computer player has everything it requires.
    pub cheat_has_all: bool,
    pub(crate) events: Vec<Event>,
    /// The state of a scripted computer player.
    pub(crate) ai: Option<Box<Computer>>,
}

/// Every field but the events, which only a local human records.
impl Hash for Player {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let Player {
            obj_id,
            kind,
            controller,
            neutral,
            faction,
            allied,
            number,
            perception,
            memory,
            objectives,
            units,
            budget,
            upgrades,
            forbidden_techs,
            observed_before_squares,
            strictly_observed_before_squares,
            observed_squares,
            observed_objects,
            detected_units,
            allied_control,
            known_enemies,
            enemy_menace,
            enemy_presence,
            places_with_corpses,
            places_with_friends,
            cataclysmic_places,
            subsquare_threat,
            counterattack_places,
            resources,
            storage_bonus,
            food,
            used_food,
            cheatmode,
            has_victory,
            has_been_defeated,
            observer_if_defeated,
            group,
            group_had_enough_mana,
            stats,
            triggers,
            menace,
            buckets,
            places_to_explore,
            cheat_has_all,
            events: _,
            ai,
        } = self;
        obj_id.hash(state);
        kind.hash(state);
        controller.hash(state);
        neutral.hash(state);
        faction.hash(state);
        allied.hash(state);
        number.hash(state);
        perception.hash(state);
        memory.hash(state);
        objectives.hash(state);
        units.hash(state);
        budget.hash(state);
        upgrades.hash(state);
        forbidden_techs.hash(state);
        observed_before_squares.hash(state);
        strictly_observed_before_squares.hash(state);
        observed_squares.hash(state);
        observed_objects.hash(state);
        detected_units.hash(state);
        allied_control.hash(state);
        known_enemies.hash(state);
        enemy_menace.hash(state);
        enemy_presence.hash(state);
        places_with_corpses.hash(state);
        places_with_friends.hash(state);
        cataclysmic_places.hash(state);
        subsquare_threat.hash(state);
        counterattack_places.hash(state);
        resources.hash(state);
        storage_bonus.hash(state);
        food.hash(state);
        used_food.hash(state);
        cheatmode.hash(state);
        has_victory.hash(state);
        has_been_defeated.hash(state);
        observer_if_defeated.hash(state);
        group.hash(state);
        group_had_enough_mana.hash(state);
        stats.hash(state);
        triggers.hash(state);
        menace.hash(state);
        buckets.hash(state);
        places_to_explore.hash(state);
        cheat_has_all.hash(state);
        ai.hash(state);
    }
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("obj_id", &self.obj_id)
            .field("kind", &self.kind)
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}

impl Player {
    pub(crate) fn new(
        obj_id: u32,
        kind: PlayerKind,
        controller: Controller,
        faction: String,
    ) -> Player {
        Player {
            obj_id,
            neutral: controller.is_neutral(),
            observer_if_defeated: kind == PlayerKind::Human,
            kind,
            controller,
            faction,
            allied: Vec::new(),
            number: None,
            perception: BTreeSet::new(),
            memory: BTreeMap::new(),
            objectives: Vec::new(),
            units: Vec::new(),
            budget: Vec::new(),
            upgrades: Vec::new(),
            forbidden_techs: Vec::new(),
            observed_before_squares: BTreeSet::new(),
            strictly_observed_before_squares: BTreeSet::new(),
            observed_squares: BTreeSet::new(),
            observed_objects: BTreeMap::new(),
            detected_units: BTreeSet::new(),
            allied_control: Vec::new(),
            known_enemies: BTreeMap::new(),
            enemy_menace: BTreeMap::new(),
            enemy_presence: Vec::new(),
            places_with_corpses: BTreeSet::new(),
            places_with_friends: BTreeSet::new(),
            cataclysmic_places: BTreeSet::new(),
            subsquare_threat: BTreeMap::new(),
            counterattack_places: Vec::new(),
            resources: Vec::new(),
            storage_bonus: Vec::new(),
            food: 0,
            used_food: 0,
            cheatmode: false,
            has_victory: false,
            has_been_defeated: false,
            group: None,
            group_had_enough_mana: false,
            stats: PlayerStats::default(),
            triggers: Vec::new(),
            menace: 0,
            buckets: BTreeMap::new(),
            places_to_explore: None,
            cheat_has_all: false,
            events: Vec::new(),
            ai: None,
        }
    }

    pub fn is_human(&self) -> bool {
        self.kind == PlayerKind::Human
    }

    /// Whether events are recorded for this player: a local human.
    pub fn is_local_human(&self) -> bool {
        matches!(
            self.controller.kind,
            ControllerKind::Human { local: true, .. }
        )
    }

    /// `ai_type` of a computer player.
    pub fn ai_type(&self) -> Option<&str> {
        match &self.kind {
            PlayerKind::Computer { ai_type } => Some(ai_type),
            _ => None,
        }
    }

    /// Units move cleverly (flee, regroup): computers other than `timers`.
    pub fn smart_units(&self) -> bool {
        matches!(&self.kind, PlayerKind::Computer { ai_type } if ai_type != "timers")
    }

    pub fn is_cpu_intensive(&self) -> bool {
        self.smart_units()
    }

    /// How long a mobile unit is remembered after it went out of sight.
    pub fn memory_duration(&self) -> i64 {
        match self.kind {
            PlayerKind::Computer { .. } => 36_000_000,
            _ => 3 * 60 * 1000,
        }
    }

    pub fn is_playing(&self) -> bool {
        !(self.has_victory || self.has_been_defeated)
    }

    /// The name said to the other players.
    pub fn name(&self) -> Msg {
        if self.kind == PlayerKind::Computer2 {
            return vec![MsgPart::from("ai2")];
        }
        if self.neutral {
            return Vec::new();
        }
        let login = self.controller.login();
        match login.as_str() {
            "ai_easy" => msg(mp::QUIET_COMPUTER),
            "ai_aggressive" => msg(mp::AGGRESSIVE_COMPUTER),
            _ => vec![MsgPart::Literal(login)],
        }
    }

    /// The number of times `type_name` was researched.
    pub fn level(&self, type_name: &str) -> i64 {
        self.upgrades.iter().filter(|u| *u == type_name).count() as i64
    }

    /// `min(food, food_limit)`.
    pub fn available_food(&self, food_limit: i64) -> i64 {
        self.food.min(food_limit)
    }

    /// The events since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}
