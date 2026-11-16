//! Players and their controllers (`worldplayerbase.py` data,
//! `worldplayerhuman.py`, `worldplayerstats.py`, and the controllers of
//! `worldclient.py`).

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::ai::{Computer, Handicap, Skill, handicapped_name};
use crate::entity::{Entity, EntityId, Names, SquareId, StatType};
use crate::events::Event;
use crate::message::TurnMessage;
use crate::msg::{Msg, MsgPart};
use crate::nofloat::PRECISION;
use crate::orders::Order;
use crate::rules::{FactionId, TypeId};
use crate::triggers::Expr;

/// Who controls a player.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ControllerKind {
    /// A person. `local` is true for the person using this machine: only a
    /// local human receives events.
    Human { login: String, local: bool },
    /// A computer player with an AI type from `ai.txt` (`timers` does
    /// nothing; `ai2` makes a random player), at a skill level. The random
    /// player has no level: its skill changes nothing.
    Computer { ai_type: String, skill: Skill },
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
            ControllerKind::Computer { ai_type, skill } => {
                ai_type.hash(state);
                skill.hash(state);
            }
            ControllerKind::Neutral => {}
        }
    }
}

/// The share of what a seat's workers gather that the seat receives, in
/// percent: from 25 to 400. A record can come from anyone, so one with an
/// income outside these bounds is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "u16", into = "u16")
)]
pub struct Income(u16);

impl Income {
    /// All that the workers gather, as in the original.
    pub const FULL: Income = Income(100);

    /// The incomes the menus offer.
    pub const CHOICES: [Income; 5] = [
        Income(50),
        Income(75),
        Income::FULL,
        Income(150),
        Income(200),
    ];

    /// An income of `percent`, from 25 to 400.
    pub fn new(percent: u16) -> Option<Income> {
        (25..=400).contains(&percent).then_some(Income(percent))
    }

    pub fn percent(self) -> u16 {
        self.0
    }

    /// The share of a gathered `amount` that the seat receives, rounded
    /// down.
    pub(crate) fn of(self, amount: i64) -> i64 {
        crate::nofloat::floordiv(amount * i64::from(self.0), 100)
    }
}

impl Default for Income {
    fn default() -> Income {
        Income::FULL
    }
}

impl TryFrom<u16> for Income {
    type Error = &'static str;

    fn try_from(percent: u16) -> Result<Income, Self::Error> {
        Income::new(percent).ok_or("an income is from 25 to 400 percent")
    }
}

impl From<Income> for u16 {
    fn from(income: Income) -> u16 {
        income.0
    }
}

/// A controller: who plays, with which alliance and faction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Controller {
    pub kind: ControllerKind,
    /// Players with the same alliance are allies. `None` means no alliance.
    pub alliance: Option<String>,
    /// A faction name, or `random_faction`.
    pub faction: String,
    /// The share of what its workers gather that the seat receives.
    pub income: Income,
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
            income: Income::FULL,
        }
    }

    /// A computer player at the normal level, allied with the other
    /// computers (alliance `ai`), with a random faction.
    pub fn computer(ai_type: &str) -> Controller {
        Controller::computer_at(ai_type, Skill::NORMAL)
    }

    /// A computer player at a skill level, allied with the other computers
    /// (alliance `ai`), with a random faction.
    pub fn computer_at(ai_type: &str, skill: Skill) -> Controller {
        Controller {
            kind: ControllerKind::Computer {
                ai_type: ai_type.to_owned(),
                skill,
            },
            alliance: Some("ai".into()),
            faction: "random_faction".into(),
            income: Income::FULL,
        }
    }

    pub(crate) fn neutral() -> Controller {
        Controller {
            kind: ControllerKind::Neutral,
            alliance: Some("ai".into()),
            faction: "random_faction".into(),
            income: Income::FULL,
        }
    }

    pub fn is_neutral(&self) -> bool {
        self.kind == ControllerKind::Neutral
    }

    /// The login the original uses: the person's login, or `ai_<type>`.
    pub fn login(&self) -> String {
        match &self.kind {
            ControllerKind::Human { login, .. } => login.clone(),
            ControllerKind::Computer { ai_type, .. } => format!("ai_{ai_type}"),
            ControllerKind::Neutral => "ai_timers".into(),
        }
    }
}

/// The kind of player, which decides a few behaviors.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlayerKind {
    Human,
    /// The scripted computer player, at a skill level; `ai_type` is
    /// `timers` for one that does nothing.
    Computer {
        ai_type: String,
        skill: Skill,
    },
    /// The random computer player.
    Computer2,
}

/// A mission objective.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum StatEvent {
    Produced,
    Lost,
    Killed,
    Gathered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlayerStats {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::pairs"))]
    counts: BTreeMap<(StatEvent, StatTarget), i64>,
    game_duration: Option<i64>,
}

impl PlayerStats {
    /// Every number of the statistics: the counts, and the duration.
    pub(crate) fn numbers(&self) -> impl Iterator<Item = i64> + '_ {
        self.counts.values().copied().chain(self.game_duration)
    }

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

/// The upgrades a player has, each with its level: how many times it was
/// researched or given. In the order the player first got them, which is
/// the order a player's allies get them in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Upgrades(Vec<(TypeId, u32)>);

impl Upgrades {
    /// How many times the player got `upgrade`: 0 if never.
    pub fn level(&self, upgrade: TypeId) -> u32 {
        self.0
            .iter()
            .find(|(t, _)| *t == upgrade)
            .map_or(0, |&(_, n)| n)
    }

    pub fn has(&self, upgrade: TypeId) -> bool {
        self.level(upgrade) > 0
    }

    /// Raises the level of `upgrade` by one.
    pub(crate) fn add(&mut self, upgrade: TypeId) {
        match self.0.iter_mut().find(|(t, _)| *t == upgrade) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => self.0.push((upgrade, 1)),
        }
    }

    /// Each upgrade and its level, in the order the player first got them.
    pub fn iter(&self) -> impl Iterator<Item = (TypeId, u32)> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The end of a local human's turn, as the world records it for the host.
#[derive(Debug)]
pub(crate) struct EndOfTurn {
    /// How many of the player's events came before it.
    pub(crate) after_events: usize,
    /// What the player knew then, without the events.
    pub(crate) message: TurnMessage,
}

/// A player of the game.
pub struct Player {
    /// The id in the text protocol (players share the entity counter).
    pub obj_id: u32,
    pub kind: PlayerKind,
    pub controller: Controller,
    pub neutral: bool,
    /// `None` for a faction the rules do not define, which a controller
    /// may name: the player then uses every type as it is.
    pub faction: Option<FactionId>,
    /// The allies, the player included, each once.
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
    pub upgrades: Upgrades,
    /// The types the player may not build, train, research or use.
    pub forbidden_techs: BTreeSet<TypeId>,
    pub observed_before_squares: BTreeSet<SquareId>,
    pub strictly_observed_before_squares: BTreeSet<SquareId>,
    pub observed_squares: BTreeSet<SquareId>,
    /// The squares seen only partly at the last perception update, from a
    /// unit on the next square: not observed, but their neutral objects in
    /// sight are seen. Made anew from the units each update, so it is not
    /// part of the digest.
    pub partly_observed_squares: BTreeSet<SquareId>,
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
    /// Every requirement is met: a computer's handicap, or a cheat.
    pub cheat_has_all: bool,
    /// What this computer may do that a human may not: nothing for a
    /// human or a fair computer.
    pub handicap: Handicap,
    pub(crate) events: Vec<Event>,
    /// The end of the last turn, for a local human, until the host takes
    /// it.
    pub(crate) end_of_turn: Option<EndOfTurn>,
    /// The state of a scripted computer player.
    pub(crate) ai: Option<Box<Computer>>,
}

/// Every field but the events and the end of the turn, which only a
/// local human records.
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
            partly_observed_squares: _,
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
            // It follows from the controller.
            handicap: _,
            events: _,
            end_of_turn: _,
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
        faction: Option<FactionId>,
    ) -> Player {
        let handicap = match &controller.kind {
            ControllerKind::Computer { ai_type, .. } => Handicap::of(ai_type),
            ControllerKind::Human { .. } | ControllerKind::Neutral => Handicap::NONE,
        };
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
            upgrades: Upgrades::default(),
            forbidden_techs: BTreeSet::new(),
            observed_before_squares: BTreeSet::new(),
            strictly_observed_before_squares: BTreeSet::new(),
            observed_squares: BTreeSet::new(),
            partly_observed_squares: BTreeSet::new(),
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
            handicap,
            events: Vec::new(),
            end_of_turn: None,
            ai: None,
        }
    }

    /// Calls `f` with each id the player's state names (see [`Names`]):
    /// what it perceives and remembers, its units, the orders of its
    /// budget, what its units revealed until a time not yet past, the
    /// enemies it found this turn, its selection and its computer's
    /// bookkeeping. Its buckets name its units only when `buckets_read`:
    /// an allied player in the game looks through them. A remembered copy
    /// is read as it was, so what it holds is not named; which units were
    /// detected is only asked about, never looked up.
    pub(crate) fn names(&self, now: i64, buckets_read: bool, f: &mut impl FnMut(EntityId)) {
        let Player {
            obj_id: _,
            kind: _,
            controller: _,
            neutral: _,
            faction: _,
            allied: _,
            number: _,
            perception,
            memory,
            objectives: _,
            units,
            budget,
            upgrades: _,
            forbidden_techs: _,
            observed_before_squares: _,
            strictly_observed_before_squares: _,
            observed_squares: _,
            partly_observed_squares: _,
            observed_objects,
            detected_units: _,
            allied_control: _,
            known_enemies,
            enemy_menace: _,
            enemy_presence: _,
            places_with_corpses: _,
            places_with_friends: _,
            cataclysmic_places: _,
            subsquare_threat: _,
            counterattack_places: _,
            resources: _,
            storage_bonus: _,
            food: _,
            used_food: _,
            cheatmode: _,
            has_victory: _,
            has_been_defeated: _,
            observer_if_defeated: _,
            group,
            group_had_enough_mana: _,
            stats: _,
            triggers: _,
            menace: _,
            buckets,
            places_to_explore: _,
            cheat_has_all: _,
            handicap: _,
            events: _,
            end_of_turn: _,
            ai,
        } = self;
        perception.iter().for_each(|&o| f(o));
        memory.keys().for_each(|&o| f(o));
        units.names(f);
        budget.names(f);
        for (&o, &until) in observed_objects {
            if until >= now {
                f(o);
            }
        }
        for (t, list) in known_enemies.values() {
            if *t == now {
                list.names(f);
            }
        }
        group.names(f);
        if buckets_read {
            buckets.values().for_each(|b| b.names(f));
        }
        if let Some(ai) = ai {
            ai.names(f);
        }
    }

    /// Forgets, once out of the game, what nothing reads any more: what
    /// it perceived and remembered, its budget, which units were detected,
    /// the enemies it found, its selection and its computer's
    /// bookkeeping. Its statistics, resources, alliances, upgrades and
    /// squares stay, for the score and its allies; so do its buckets and
    /// what its units revealed, which its allies still look through.
    pub(crate) fn forget_after_leaving(&mut self) {
        self.perception.clear();
        self.memory.clear();
        self.budget.clear();
        self.detected_units.clear();
        self.known_enemies.clear();
        self.group = None;
        self.ai = None;
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
            PlayerKind::Computer { ai_type, .. } => Some(ai_type),
            _ => None,
        }
    }

    /// Units move cleverly (flee, regroup): computers other than `timers`.
    pub fn smart_units(&self) -> bool {
        matches!(&self.kind, PlayerKind::Computer { ai_type, .. } if ai_type != "timers")
    }

    pub fn is_cpu_intensive(&self) -> bool {
        self.smart_units()
    }

    /// How long a mobile unit is remembered after it went out of sight: 3
    /// minutes, for a computer as for a human.
    pub fn memory_duration(&self) -> i64 {
        3 * 60 * 1000
    }

    pub fn is_playing(&self) -> bool {
        !(self.has_victory || self.has_been_defeated)
    }

    /// The name said to the other players: a person's login, or the name
    /// of a computer's type, which says its level and its handicap.
    pub fn name(&self) -> Msg {
        match &self.controller.kind {
            ControllerKind::Neutral => Vec::new(),
            ControllerKind::Computer { ai_type, skill } => {
                handicapped_name(ai_type, *skill, self.handicap)
            }
            ControllerKind::Human { login, .. } => vec![MsgPart::Literal(login.clone())],
        }
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
