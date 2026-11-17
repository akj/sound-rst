//! The computer players: `Computer`, scripted by `ai.txt`
//! (`worldplayercomputer.py`), and `Computer2`, which gives random orders
//! (`worldplayercomputer2.py`).
//!
//! The kind of a player selects its AI. A [`PlayerKind::Computer`] player
//! keeps a [`Computer`] state; its AI type `timers` does nothing. A
//! [`PlayerKind::Computer2`] player needs no state. The world calls the AI
//! during the player's update (`play`), when one of its units is attacked,
//! when a unit waits for the right time, and when a map trigger changes the
//! AI type.
//!
//! A computer reads and acts only through its [`Door`], which offers what a
//! human in its seat could know and do: outside its tests, this module
//! never names the world.
//! It reads and acts in the original's order, so that it draws the same
//! random numbers. Where the original raises an exception for what a mod,
//! a map or the turn of a game can bring (a unit inside another, a deposit
//! gone, rules without a farm, a purchase that asks for itself), it skips
//! that one step and goes on with its turn. Its turn stops only for a
//! broken invariant of the simulation, from an order it gives or a path it
//! asks for.

use std::collections::{BTreeMap, BTreeSet};

use crate::abort::{Abort, Res};
use crate::entity::{EntityId, Place, SquareId};
use crate::nofloat::{floordiv, square_of_distance};
use crate::orders::{Keyword, Request, TargetRef};
use crate::player::PlayerKind;
use crate::rules::{AirGround, ClassId, Effect, EffectTarget, EntityClass, Role, TypeId};
use crate::world::TPlace;

mod door;
mod handicap;
pub(crate) mod script;
mod skill;
mod tactical;

use door::{Door, Followed, Mark, Own, OwnOrder, Seen};
pub(crate) use handicap::handicapped_name;
pub use handicap::{Handicap, RANDOM_AI_TYPE, computer_name};
use script::{Plan, Step};
pub use skill::Skill;
pub(crate) use tactical::TacticalKnowledge;

/// The deepest the computer player's planning may go. Getting a unit asks
/// for its makers, its requirements, resources and food, and each of these
/// may ask for more. Rules where they ask for each other without end (a farm
/// that costs food) would exhaust the stack; the original stops at Python's
/// recursion limit. The original's rules and scripts go 11 deep.
const MAX_PLANNING_DEPTH: usize = 64;

/// Why the planning of a purchase stops before it is done.
#[derive(Debug)]
enum Unplanned {
    /// The purchase asked for others, one inside another, deeper than
    /// [`MAX_PLANNING_DEPTH`]: rules where a unit asks for itself, through
    /// its makers, its requirements or its food. The purchase waits for a
    /// later turn, and the computer goes on with the rest of its turn.
    TooDeep,
    /// The work stops.
    Stop(Abort),
}

impl From<Abort> for Unplanned {
    fn from(a: Abort) -> Unplanned {
        Unplanned::Stop(a)
    }
}

/// The result of planning a purchase.
type Planned<T = ()> = Result<T, Unplanned>;

/// One step deeper into the planning, or [`Unplanned::TooDeep`].
fn deeper(depth: usize) -> Planned<usize> {
    if depth >= MAX_PLANNING_DEPTH {
        return Err(Unplanned::TooDeep);
    }
    Ok(depth + 1)
}

/// What `check_type` compares a unit or a type with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Sel {
    /// A list of types: `nb` counts 1 if the first one is an upgrade the
    /// player has.
    List(Vec<TypeId>),
    One(TypeId),
    /// The `Worker` class.
    Workers,
}

impl Sel {
    /// `check_type(type, sel)` for a type.
    fn matches_type(&self, t: TypeId) -> bool {
        match self {
            Sel::List(l) => l.contains(&t),
            Sel::One(o) => *o == t,
            Sel::Workers => false,
        }
    }
}

/// An order the computer gives to units of a kind, and follows until
/// enough of them carry it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Task {
    /// Repair a building site.
    Repair(EntityId),
    /// Build a building on a meadow.
    Build(TypeId, EntityId),
    Train(TypeId),
    Research(TypeId),
    UpgradeTo(TypeId),
    /// Use an ability, aimed at a unit if it asks for a target.
    Use(TypeId, Option<EntityId>),
}

impl Task {
    /// The order, as a unit takes it.
    fn request(self, rules: &crate::Rules) -> Request {
        let name = |t: TypeId| rules.ty(t).name.clone();
        match self {
            Task::Repair(site) => Request::repair(TargetRef::Entity(site)),
            Task::Build(t, meadow) => Request::build(name(t), TargetRef::Entity(meadow)),
            Task::Train(t) => Request::train(name(t)),
            Task::Research(t) => Request::research(name(t)),
            Task::UpgradeTo(t) => Request::upgrade_to(name(t)),
            Task::Use(a, None) => Request::use_ability(name(a)),
            Task::Use(a, Some(u)) => Request::use_ability(name(a)).at(TargetRef::Entity(u)),
        }
    }
}

/// The classes `choose` looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    Meadow,
    Deposit,
    /// A deposit of this resource type.
    DepositOf(i64),
}

impl Kind {
    fn matches(self, rules: &crate::Rules, e: Seen) -> bool {
        match self {
            Kind::Meadow => e.is_meadow(),
            Kind::Deposit => e.is_deposit(),
            Kind::DepositOf(rt) => e
                .deposit_type()
                .is_some_and(|t| rules.ty(t).resource_type == Some(rt)),
        }
    }
}

/// The comparison key of `_deposit_priority`: the deposit's id last, and
/// none for a missing deposit.
type DepositPriority = (i64, i64, Option<EntityId>);

/// The state of a scripted computer player (`Computer`).
#[derive(Clone, Debug, Default, Hash)]
pub(crate) struct Computer {
    /// The script, once an AI type other than `timers` is set.
    plan: Option<Plan>,
    line_nb: i64,
    previous_linechange: i64,
    watchdog: i64,
    constant_attacks: i64,
    research: i64,
    attacked_places: Vec<Option<Place>>,
    orders: BTreeMap<(Sel, Task), Vec<Followed>>,
    previous_choose: BTreeMap<(Kind, Option<i64>, Option<Place>), Mark>,
    sensible_building: Option<EntityId>,
    workers: Vec<EntityId>,
    /// Workers gathering from each deposit, by the identity of the target.
    gathered_deposits: Vec<(Option<Mark>, i64)>,
    building_sites: Vec<EntityId>,
    /// The units able to use an ability of each kind, and the ability.
    raise_dead_users: Vec<(EntityId, TypeId)>,
    teleportation_users: Vec<(EntityId, TypeId)>,
    cataclysm_users: Vec<(EntityId, TypeId)>,
    detector_users: Vec<(EntityId, TypeId)>,
    summon_users: Vec<(EntityId, TypeId)>,
    /// Whether units waiting on a target may go, by target.
    time_has_come: Vec<(Option<Mark>, bool)>,
    safe_cnt: i64,
    /// The cost of the first purchase of its turn that lacked a resource.
    lacking: Option<Vec<i64>>,
}

impl Computer {
    /// Whether the units waiting on a square are strong enough.
    fn time_has_come(&self, s: SquareId) -> bool {
        let key = Some(Mark::square(s));
        self.time_has_come
            .iter()
            .find(|(t, _)| same_opt(t, &key))
            .is_some_and(|(_, v)| *v)
    }
}

fn same_opt(a: &Option<Mark>, b: &Option<Mark>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(b),
        _ => false,
    }
}

/// The orders `Computer2` chooses from, sorted by keyword: every order but
/// `attack`, `auto_attack`, `auto_explore`, `block`, the cancels, `enter`,
/// `join_group`, `patrol`, `rallying_point`, `stop` and `wait`.
const COMPUTER2_ORDERS: [Keyword; 18] = [
    Keyword::Build,
    Keyword::BuildPhaseTwo,
    Keyword::DisableAutoGather,
    Keyword::DisableAutoRepair,
    Keyword::EnableAutoGather,
    Keyword::EnableAutoRepair,
    Keyword::Gather,
    Keyword::Go,
    Keyword::Load,
    Keyword::LoadAll,
    Keyword::ModeDefensive,
    Keyword::ModeOffensive,
    Keyword::Repair,
    Keyword::Research,
    Keyword::Train,
    Keyword::UnloadAll,
    Keyword::UpgradeTo,
    Keyword::Use,
];

/// Lazily lists the type names of the enemies on a square, once: the
/// original passes a generator, which the first use exhausts.
struct Enemies {
    place: Option<Place>,
    used: bool,
}

/// The square of a place: none for the place inside a unit, or none.
fn square_of(place: Option<Place>) -> Option<SquareId> {
    match place {
        Some(Place::Square(s)) => Some(s),
        Some(Place::Inside(_)) | None => None,
    }
}

impl Door<'_> {
    // state

    /// `set_ai`, after the AI type of the player changed: loads the script
    /// and resets the script's settings. Without a script of that name, it
    /// plays as with an empty script: its routine, and no plan.
    fn set_ai(&mut self, ai_type: &str) {
        if ai_type == "timers" || !self.has_state() {
            return;
        }
        let name = self.faction_ai_type(ai_type);
        let mut warnings = Vec::new();
        let plan = match self.script(&name) {
            Some(script) => Some(script.plan(self.rules(), &mut warnings)),
            None => {
                warnings.push(format!(
                    "no script '{name}' in ai.txt: the computer plays without one"
                ));
                None
            }
        };
        for w in warnings {
            self.warn(w);
        }
        let time = self.time();
        let c = self.state_mut();
        c.plan = plan;
        c.line_nb = 0;
        c.previous_linechange = time;
        c.watchdog = 0;
        c.constant_attacks = 0;
        c.research = 0;
        self.update_effect_users_and_workers();
    }

    fn set_line_nb(&mut self, n: i64) {
        let time = self.time();
        let c = self.state_mut();
        c.line_nb = n;
        c.previous_linechange = time;
    }

    // hooks

    /// `play`: what a computer player does during its update.
    fn play(&mut self) -> Res {
        self.grant_handicap();
        match self.kind().clone() {
            PlayerKind::Human => Ok(()),
            PlayerKind::Computer2 => self.computer2_play(),
            PlayerKind::Computer { ai_type, .. } => {
                if ai_type == "timers" || !self.has_state() {
                    return Ok(());
                }
                self.computer_play()
            }
        }
    }

    /// `Computer.on_unit_attacked`: remembers where to retaliate.
    fn on_unit_attacked(&mut self, unit: EntityId, attacker: EntityId) {
        if !self.has_state() {
            return;
        }
        let a = self.attacker(attacker);
        if a.owner.is_some_and(|ap| self.allied().contains(&ap)) || !a.is_vulnerable {
            return;
        }
        let Some(u) = self.unit(unit) else {
            return;
        };
        if u.first_order().map(OwnOrder::keyword) == Some(Keyword::AutoExplore) {
            return;
        }
        let place = u.place();
        if u.is_a_building() {
            self.state_mut().sensible_building = Some(unit);
        }
        let place = match self.known(attacker).filter(|_| self.perceives(attacker)) {
            Some(a) => a.place(),
            None => place,
        };
        let c = self.state_mut();
        if !c.attacked_places.contains(&place) {
            c.attacked_places.push(place);
        }
    }

    // Computer2

    fn computer2_play(&mut self) -> Res {
        let mut targets: Vec<EntityId> = self.perceived().map(Seen::id).collect();
        targets.extend(self.remembered().map(|(r, _)| r.id()));
        // Every id is perceived or remembered: the entity it shows is known.
        targets.retain(|&o| !self.known(o).is_some_and(Seen::is_exit));
        targets.sort();
        let mut i = 0;
        while let Some(&u) = self.units().get(i) {
            if self.unit(u).is_some_and(|e| !e.has_orders())
                && let Some(order) = self.random_order(u, &targets)?
            {
                self.give(u, order, true)?;
            }
            i += 1;
        }
        Ok(())
    }

    fn random_order(&mut self, unit: EntityId, targets: &[EntityId]) -> Res<Option<Request>> {
        let [first, rest @ ..] = &COMPUTER2_ORDERS;
        let kw = self.pick(*first, rest);
        let mut menu = self.order_menu(unit, kw, true);
        let Some(i) = self.choice_index(menu.len()) else {
            return Ok(None);
        };
        let entry = menu.swap_remove(i);
        let mut order = Request::listed(self.rules(), &entry);
        // Of the abilities, only those that ask for a target take one.
        let args = if kw == Keyword::Use {
            self.order_info(unit, kw, entry.type_id)
                .is_some_and(|info| info.nb_args > 0)
        } else {
            kw.nb_args() > 0
        };
        if args {
            // Knowing nothing to aim at, it gives the unit no order.
            let Some(j) = self.choice_index(targets.len()) else {
                return Ok(None);
            };
            order = order.at(TargetRef::Entity(targets[j]));
        }
        Ok(Some(order))
    }

    // Computer: the turn

    fn computer_play(&mut self) -> Res {
        if !self.should_play_this_turn() {
            return Ok(());
        }
        self.state_mut().lacking = None;
        self.update_effect_users_and_workers();
        self.update_time_has_come()?;
        self.send_workers_to_forgotten_building_sites()?;
        self.idle_workers_gather()?;
        self.send_explorer()?;
        if !self.state().attacked_places.is_empty() {
            let places = self.state().attacked_places.clone();
            self.eventually_attack(places)?;
            self.state_mut().attacked_places.clear();
        } else if self.state().constant_attacks != 0 {
            // During its grace, it starts no attack.
            if self.time() >= self.skill().settings().grace {
                let places = self
                    .enemy_presence()
                    .iter()
                    .map(|&s| Some(Place::Square(s)))
                    .collect();
                self.eventually_attack(places)?;
            }
        } else {
            self.defensive_routine()?;
        }
        if self.state().research != 0 {
            self.idle_buildings_research()?;
        }
        self.raise_dead()?;
        self.build_a_warehouse_if_useful()?;
        let worker = self.role_type(Role::Worker);
        match self.get(self.skill().settings().workers.kept(), worker) {
            Ok(_) => {}
            Err(Unplanned::TooDeep) => {
                let ai_type = self.ai_type();
                self.warn(format!(
                    "recursion error with {ai_type}; getting its workers"
                ));
            }
            Err(Unplanned::Stop(a)) => return Err(a),
        }
        match self.follow_plan() {
            Ok(()) => {}
            // As the original, it goes on with the next line of the plan at
            // its next turn.
            Err(Unplanned::TooDeep) => {
                let ai_type = self.ai_type();
                let c = self.state();
                let line = usize::try_from(c.line_nb)
                    .ok()
                    .and_then(|i| c.plan.as_ref()?.lines().get(i).cloned())
                    .unwrap_or_default();
                let n = c.line_nb + 1;
                self.warn(format!(
                    "recursion error with {ai_type}; current ai.txt line is: {line}"
                ));
                self.set_line_nb(n);
            }
            Err(Unplanned::Stop(a)) => return Err(a),
        }
        self.gather_what_is_lacking()
    }

    /// The name of its AI type, as a warning names it.
    fn ai_type(&self) -> String {
        match self.kind() {
            PlayerKind::Computer { ai_type, .. } => ai_type.clone(),
            PlayerKind::Human | PlayerKind::Computer2 => String::new(),
        }
    }

    /// The cpu intensive players play in turns: at the normal level, one
    /// tenth of them each turn. A scripted computer that plays is one of
    /// them.
    fn should_play_this_turn(&self) -> bool {
        let every = self.skill().settings().turns_between_decisions;
        let players = self.planning_computers();
        let i = players.iter().take_while(|&&x| x != self.me()).count() as i64;
        let turn = i * every / (players.len() as i64).max(1);
        self.turn().rem_euclid(every) == turn
    }

    fn follow_plan(&mut self) -> Planned {
        let Some(plan) = self.state().plan.clone() else {
            return Ok(());
        };
        if plan.steps().is_empty() {
            return Ok(());
        }
        let c = self.state();
        let deadline = c
            .previous_linechange
            .saturating_add(c.watchdog.saturating_mul(1000));
        if c.watchdog != 0 && self.time() > deadline {
            let n = c.line_nb + 1;
            self.set_line_nb(n);
        }
        let n = self.state().line_nb.rem_euclid(plan.steps().len() as i64);
        self.set_line_nb(n);
        let line_nb = self.state().line_nb;
        let next = line_nb + 1;
        let Some(step) = plan.steps().get(n as usize) else {
            return Ok(());
        };
        match step {
            &Step::Goto(n) => self.set_line_nb(n),
            Step::Label => self.set_line_nb(next),
            Step::GotoRandom(first, rest) => {
                let n = self.pick(*first, rest);
                self.set_line_nb(n);
            }
            Step::Attack => {
                self.state_mut().constant_attacks = 1;
                self.set_line_nb(next);
            }
            &Step::Watchdog(v) => {
                self.state_mut().watchdog = v;
                self.set_line_nb(next);
            }
            &Step::ConstantAttacks(v) => {
                self.state_mut().constant_attacks = v;
                self.set_line_nb(next);
            }
            &Step::Research(v) => {
                self.state_mut().research = v;
                self.set_line_nb(next);
            }
            Step::Get(items) => {
                let worker = self.role_type(Role::Worker);
                let workers = self.skill().settings().workers;
                for &(n, t) in items {
                    // What its faction uses instead, as it is now.
                    let t = t.map(|t| self.rules().equivalent(self.faction(), t));
                    let n = if t.is_some() && t == worker {
                        workers.asked(n)
                    } else {
                        n
                    };
                    if !self.get(n, t)? {
                        return Ok(());
                    }
                }
                self.set_line_nb(next);
            }
            Step::Skip => self.set_line_nb(next),
        }
        Ok(())
    }

    // Computer: units and their tasks

    fn update_effect_users_and_workers(&mut self) {
        let mut workers = Vec::new();
        let mut gathered: Vec<(Option<Mark>, i64)> = Vec::new();
        let mut sites = Vec::new();
        let mut raise_dead = Vec::new();
        let mut teleportation = Vec::new();
        let mut cataclysm = Vec::new();
        let mut detector = Vec::new();
        let mut summon = Vec::new();
        for u in self.units().to_vec() {
            let Some(e) = self.unit(u) else { continue };
            match e.class() {
                EntityClass::Worker => {
                    workers.push(u);
                    if let Some(o) = e.first_order()
                        && o.keyword() == Keyword::Gather
                    {
                        add_count(&mut gathered, &o.target(), 1);
                    }
                }
                EntityClass::BuildingSite => sites.push(u),
                EntityClass::Soldier
                | EntityClass::Effect
                | EntityClass::Building
                | EntityClass::Deposit
                | EntityClass::Meadow
                | EntityClass::Corpse
                | EntityClass::Exit => {}
            }
            let abilities = self.listed(e.type_id(), |t| &t.can_use);
            for a in abilities {
                // A unit may use only an ability the rules have.
                if !self.is_order_allowed(u, Keyword::Use, Some(a)) {
                    continue;
                }
                let rules = self.rules();
                match &rules.ty(a).effect {
                    Some(Effect::RaiseDead { .. }) => raise_dead.push((u, a)),
                    Some(Effect::Teleportation) => teleportation.push((u, a)),
                    Some(Effect::Summon { items, .. }) => {
                        for item in items {
                            let stats = &rules.ty(item.ty).stats;
                            if stats.harm_level != 0 {
                                cataclysm.push((u, a));
                            }
                            if stats.is_a_detector {
                                detector.push((u, a));
                            }
                            if stats.damage != 0 {
                                summon.push((u, a));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let c = self.state_mut();
        c.workers = workers;
        c.gathered_deposits = gathered;
        c.building_sites = sites;
        c.raise_dead_users = raise_dead;
        c.teleportation_users = teleportation;
        c.cataclysm_users = cataclysm;
        c.detector_users = detector;
        c.summon_users = summon;
    }

    fn update_time_has_come(&mut self) -> Res {
        // (target, menace, units), in the order the targets first appear
        let mut waiting: Vec<(Option<Mark>, i64, Vec<EntityId>)> = Vec::new();
        for &u in self.units() {
            let Some(e) = self.unit(u) else { continue };
            if let Some(o) = e.first_order()
                && o.keyword() == Keyword::Wait
            {
                let target = o.target();
                match waiting.iter_mut().find(|(t, _, _)| same_opt(t, &target)) {
                    Some(w) => {
                        w.1 += e.menace();
                        w.2.push(u);
                    }
                    None => waiting.push((target, e.menace(), vec![u])),
                }
            }
        }
        let thc: Vec<(Option<Mark>, bool)> = waiting
            .iter()
            .map(|(t, _, us)| {
                let m = self.target_enemy_menace(t.as_ref());
                (t.clone(), self.is_powerful_enough(us, m))
            })
            .collect();
        self.state_mut().time_has_come = thc;
        let units = self.units().to_vec();
        let mut cancel: BTreeSet<EntityId> = BTreeSet::new();
        for (place, _, _) in &waiting {
            let m = self.target_enemy_menace(place.as_ref());
            if !self.is_powerful_enough(&units, m) {
                for &u in &units {
                    let hit = self.unit(u).is_some_and(|e| {
                        e.orders()
                            .any(|o| o.keyword() == Keyword::Wait && same_opt(&o.target(), place))
                    });
                    if hit {
                        cancel.insert(u);
                    }
                }
            }
        }
        // The original walks a set of units: the order does not change the
        // result, so the id order is used. The original cancels their
        // orders; a human stops them.
        for u in cancel {
            self.stop(u)?;
        }
        Ok(())
    }

    /// Stops a unit with the stop order, if it has something to stop: a
    /// human gives no stop order to a unit standing idle.
    fn stop(&mut self, u: EntityId) -> Res {
        if self.is_order_allowed(u, Keyword::Stop, None) {
            self.give(u, Request::stop(), true)?;
        }
        Ok(())
    }

    fn target_enemy_menace(&self, t: Option<&Mark>) -> i64 {
        t.and_then(Mark::as_square)
            .map_or(0, |s| self.enemy_menace(s))
    }

    fn place_enemy_menace(&self, place: Option<Place>) -> i64 {
        match place {
            Some(Place::Square(s)) => self.enemy_menace(s),
            _ => 0,
        }
    }

    fn is_powerful_enough(&self, units: &[EntityId], enemy_menace: i64) -> bool {
        // sometimes the food limit prevents units with more than 1 food cost
        let ratio = if self.used_food() < self.food_limit() - 5 {
            180
        } else {
            100
        };
        let menace: i64 = units
            .iter()
            .filter_map(|&u| self.unit(u))
            .filter(|e| e.speed() > 0 && e.class() == EntityClass::Soldier)
            .map(Own::menace)
            .sum();
        menace > floordiv(enemy_menace * ratio, 100)
    }

    fn send_workers_to_forgotten_building_sites(&mut self) -> Res {
        for site in self.state().building_sites.clone() {
            let served = self.state().workers.iter().any(|&u| {
                self.unit(u)
                    .and_then(Own::first_order)
                    .and_then(OwnOrder::target)
                    .is_some_and(|t| t.is_entity(site))
            });
            if !served {
                self.order(4, Sel::Workers, Task::Repair(site), true)?;
                break;
            }
        }
        Ok(())
    }

    fn idle_workers_gather(&mut self) -> Res {
        let idle: Vec<EntityId> = self
            .state()
            .workers
            .iter()
            .copied()
            .filter(|&u| self.unit(u).is_some_and(|e| !e.has_orders()))
            .collect();
        for u in idle {
            let place = self.unit(u).and_then(Own::place);
            // A worker inside a unit gathers nothing.
            if square_of(place).is_none() {
                continue;
            }
            let Some(deposit) = self.choose(Kind::Deposit, None, place, true)? else {
                return Ok(());
            };
            let Some(id) = deposit.entity_id() else {
                return Ok(());
            };
            let gather = Request::gather(TargetRef::Entity(id));
            self.give(u, gather, true)?;
            add_count(&mut self.state_mut().gathered_deposits, &Some(deposit), 1);
        }
        Ok(())
    }

    /// Moves a worker to the resource that the first purchase of the turn
    /// lacks and the fewest workers gather, from the resource it does not
    /// lack that the most workers gather, leaving that one a gatherer: one
    /// worker a turn, to a deposit it knows and can walk to safely.
    fn gather_what_is_lacking(&mut self) -> Res {
        let Some(cost) = self.state().lacking.clone() else {
            return Ok(());
        };
        let lacking = self.missing_resources(&cost);
        let Some(&first_lacking) = lacking.first() else {
            return Ok(());
        };
        let mut gatherers: Vec<Vec<(EntityId, Mark)>> = vec![Vec::new(); self.resources().len()];
        for &u in &self.state().workers {
            let Some(e) = self.unit(u) else { continue };
            if let Some(o) = e.first_order()
                && o.keyword() == Keyword::Gather
                && matches!(e.place(), Some(Place::Square(_)))
                && let Some(t) = o.target()
                && let Some(rt) = self.deposit_resource_type(&t)
                && let Some(g) = gatherers.get_mut(rt as usize)
            {
                g.push((u, t));
            }
        }
        let count = |r: usize| gatherers.get(r).map_or(0, Vec::len);
        let to = lacking
            .iter()
            .copied()
            .min_by_key(|&r| count(r))
            .unwrap_or(first_lacking);
        let Some(from) = (0..gatherers.len())
            .filter(|r| !lacking.contains(r))
            .max_by_key(|&r| (count(r), std::cmp::Reverse(r)))
        else {
            return Ok(());
        };
        let Some(&[(u, ref old), _, ..]) = gatherers.get(from).map(Vec::as_slice) else {
            return Ok(());
        };
        let old = old.clone();
        let place = self.unit(u).and_then(Own::place);
        let Some(deposit) = self.choose(Kind::DepositOf(to as i64), None, place, true)? else {
            return Ok(());
        };
        // The worker is on a square, and so is a deposit.
        let (Some(start), Some(dest)) =
            (square_of(place), square_of(self.target_place_of(&deposit)))
        else {
            return Ok(());
        };
        if self.distance(start, dest, true)? == crate::INF {
            return Ok(());
        }
        let Some(id) = deposit.entity_id() else {
            return Ok(());
        };
        self.give(u, Request::gather(TargetRef::Entity(id)), true)?;
        let gathered = &mut self.state_mut().gathered_deposits;
        add_count(gathered, &Some(old), -1);
        add_count(gathered, &Some(deposit), 1);
        Ok(())
    }

    fn value_as_an_explorer(&self, u: EntityId) -> (bool, i64, i64) {
        let Some(e) = self.unit(u) else {
            return (false, 0, 0);
        };
        (e.airground_type() == AirGround::Air, e.speed(), e.hp())
    }

    /// `best_explorers`: the mobile units not upgrading, best first.
    fn best_explorers(&self) -> Vec<EntityId> {
        let mut v: Vec<EntityId> = self
            .units()
            .iter()
            .copied()
            .filter(|&u| {
                self.unit(u).is_some_and(|e| {
                    e.speed() > 0
                        && e.first_order().map(OwnOrder::keyword) != Some(Keyword::UpgradeTo)
                })
            })
            .collect();
        v.sort_by(|&a, &b| {
            self.value_as_an_explorer(b)
                .cmp(&self.value_as_an_explorer(a))
                .then_with(|| b.cmp(&a))
        });
        v
    }

    fn is_exploring(&self, u: EntityId) -> bool {
        self.unit(u)
            .and_then(Own::first_order)
            .map(OwnOrder::keyword)
            == Some(Keyword::AutoExplore)
    }

    fn send_explorer(&mut self) -> Res {
        let Some(&best) = self.best_explorers().first() else {
            return Ok(());
        };
        if self.is_exploring(best) {
            return Ok(());
        }
        let explorer = self.units().iter().copied().find(|&u| self.is_exploring(u));
        if let Some(explorer) = explorer {
            if self.value_as_an_explorer(explorer) == self.value_as_an_explorer(best) {
                return Ok(());
            }
            // The former explorer goes back to the square of its first
            // unit; with the first unit inside another, it explores on.
            let first = self.units().first().copied();
            let place = first.and_then(|f| self.unit(f)).and_then(Own::place);
            if let Some(s) = square_of(place) {
                self.give(explorer, Request::go(TargetRef::Square(s)), true)?;
            }
        }
        self.give(best, Request::auto_explore(), true)
    }

    /// `_idle_fighters`: soldiers on a square without orders, or only going
    /// to a square without enemies. A unit inside another takes no order.
    fn idle_fighters(&self) -> Vec<EntityId> {
        self.units()
            .iter()
            .copied()
            .filter(|&u| {
                self.unit(u).is_some_and(|e| {
                    e.class() == EntityClass::Soldier
                        && square_of(e.place()).is_some()
                        && (!e.has_orders()
                            || e.orders().len() == 1
                                && e.first_order().is_some_and(|o| {
                                    o.keyword() == Keyword::Go
                                        && !o
                                            .target()
                                            .and_then(|t| t.as_square())
                                            .is_some_and(|s| self.enemy_presence().contains(&s))
                                }))
                })
            })
            .collect()
    }

    fn friendly_presence(&self, place: Option<Place>) -> bool {
        matches!(place, Some(Place::Square(s)) if self.has_friends_on(s))
    }

    fn eventually_attack(&mut self, places: Vec<Option<Place>>) -> Res {
        let units = self.idle_fighters();
        if units.is_empty() {
            return Ok(());
        }
        let mut places = places;
        places.sort_by_key(|&pl| self.place_enemy_menace(pl));
        // A place that is not a square, such as a unit inside another, is
        // not attacked.
        for &place in &places {
            if let Some(s) = square_of(place)
                && self.units_should_attack(&units, s)?
            {
                return self.send_units(units, s);
            }
        }
        let (Some(&first), Some(&last)) = (places.first(), places.last()) else {
            return Ok(());
        };
        let temp: Vec<EntityId> = units
            .iter()
            .copied()
            .filter(|&u| {
                self.unit(u)
                    .is_some_and(|e| e.time_limit().is_some_and(|t| t != 0) && e.speed() != 0)
            })
            .collect();
        if !temp.is_empty()
            && let Some(first) = square_of(first)
        {
            self.send_units(temp, first)?;
        }
        if self.friendly_presence(last) {
            return Ok(());
        }
        let Some(last_square) = square_of(last) else {
            return Ok(());
        };
        let mut enemies = Enemies {
            place: last,
            used: false,
        };
        for (u, a) in self.state().cataclysm_users.clone() {
            let busy = self.unit(u).is_none_or(Own::has_orders);
            if busy || !self.cataclysm_is_efficient(a, &mut enemies) {
                continue;
            }
            // A user inside another stays there.
            let Some(start) = square_of(self.unit(u).and_then(Own::place)) else {
                continue;
            };
            let path = self.path_through_walls(start, last_square)?;
            if let [.., before, _] = path[..]
                && path.len() > 2
            {
                self.give(u, Request::go(TargetRef::Square(before)), false)?;
            }
            let t = TargetRef::Square(last_square);
            let cataclysm = Request::use_ability(self.name_of(a)).at(t);
            self.give(u, cataclysm, false)?;
            if self.first_order_is_possible(u)
                && let Some(s) = square_of(self.unit(u).and_then(Own::place))
            {
                self.give(u, Request::go(TargetRef::Square(s)), false)?;
            }
        }
        Ok(())
    }

    /// Whether the first order of the unit has not failed.
    fn first_order_is_possible(&self, u: EntityId) -> bool {
        self.unit(u)
            .and_then(Own::first_order)
            .is_some_and(|o| !o.is_impossible())
    }

    /// `_cataclysm_is_efficient`: the ability summons something harmful
    /// to one of the enemies.
    fn cataclysm_is_efficient(&mut self, a: TypeId, enemies: &mut Enemies) -> bool {
        let mut types: BTreeSet<TypeId> = BTreeSet::new();
        if !enemies.used {
            enemies.used = true;
            let known = self.perceived().chain(self.remembered().map(|(r, _)| r));
            for e in known {
                // A building site has no type, and nothing harms it.
                if e.place() == enemies.place
                    && self.is_an_enemy(e)
                    && let Some(t) = e.type_id()
                {
                    types.insert(t);
                }
            }
        }
        if let Some(Effect::Summon { items, .. }) = self.rules().ty(a).effect.clone() {
            for item in &items {
                if self.rules().ty(item.ty).stats.harm_level != 0 {
                    for &t in &types {
                        if self.can_harm(item.ty, t) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn units_should_attack(&mut self, units: &[EntityId], place: SquareId) -> Res<bool> {
        let m = self.enemy_menace(place);
        if !self.is_powerful_enough(units, m) {
            return Ok(false);
        }
        let first = units.first().and_then(|&u| self.unit(u));
        let Some(start) = square_of(first.and_then(Own::place)) else {
            return Ok(false);
        };
        let path = self.path_through_walls(start, place)?;
        match path[..] {
            [] => Ok(false),
            [_] | [_, _] => Ok(true),
            [.., before, _] => self.has_a_safe_path(start, before),
        }
    }

    /// Sends units to a square. Those on it already stay, and a unit inside
    /// another, which takes no order, stays inside.
    fn send_units(&mut self, units: Vec<EntityId>, place: SquareId) -> Res {
        let units: Vec<(EntityId, SquareId)> = units
            .into_iter()
            .filter_map(|u| square_of(self.unit(u).and_then(Own::place)).map(|s| (u, s)))
            .filter(|&(_, s)| s != place)
            .collect();
        for &(u, _) in &units {
            self.stop(u)?;
        }
        let t = TargetRef::Square(place);
        let mut used_teleportation = false;
        for (u, a) in self.state().teleportation_users.clone() {
            self.give(u, Request::use_ability(self.name_of(a)).at(t), true)?;
            if self.first_order_is_possible(u) {
                used_teleportation = true;
            }
        }
        let place_ = Some(Place::Square(place));
        let mut enemies = Enemies {
            place: place_,
            used: false,
        };
        for (u, start) in units {
            let path = self.path_through_walls(start, place)?;
            if !used_teleportation
                && let [.., before, _] = path[..]
                && path.len() > 2
            {
                self.give(u, Request::go(TargetRef::Square(before)), false)?;
                if !self.friendly_presence(place_) {
                    for (u_, a) in self.state().cataclysm_users.clone() {
                        if u_ == u && self.cataclysm_is_efficient(a, &mut enemies) {
                            let cataclysm = Request::use_ability(self.name_of(a)).at(t);
                            self.give(u, cataclysm, false)?;
                        }
                    }
                }
            }
            for (u_, a) in self.state().summon_users.clone() {
                if u_ == u {
                    let summon = Request::use_ability(self.name_of(a)).at(t);
                    self.give(u, summon, false)?;
                }
            }
            for (u_, a) in self.state().detector_users.clone() {
                if u_ == u {
                    let detector = Request::use_ability(self.name_of(a)).at(t);
                    self.give(u, detector, false)?;
                }
            }
            self.give(u, Request::go(t), false)?;
        }
        Ok(())
    }

    /// `_builders_place`: where most workers are; on a tie, the square with
    /// the greatest number, before a place inside something.
    fn builders_place(&self) -> Res<Option<Place>> {
        let mut starts: Vec<(Option<Place>, i64)> = Vec::new();
        for &u in &self.state().workers {
            let place = self.unit(u).and_then(Own::place);
            match starts.iter_mut().find(|(pl, _)| *pl == place) {
                Some(s) => s.1 += 1,
                None => starts.push((place, 1)),
            }
        }
        let Some(max) = starts.iter().map(|s| s.1).max() else {
            return Ok(None);
        };
        let best: Vec<Option<Place>> = starts.iter().filter(|s| s.1 == max).map(|s| s.0).collect();
        Ok(best
            .into_iter()
            .max_by(|a, b| {
                let id = |pl: &Option<Place>| match pl {
                    Some(Place::Square(s)) => Some(self.square_number(*s)),
                    _ => None,
                };
                id(a).cmp(&id(b))
            })
            .flatten())
    }

    fn defensive_routine(&mut self) -> Res {
        if let Some(b) = self.state().sensible_building
            && !self.units().contains(&b)
        {
            self.state_mut().sensible_building = None;
        }
        // regroup at a valuable place (and get healed or repaired)
        let fighters = self.idle_fighters();
        // Builders inside a unit are not a place to regroup at.
        if let Some(place) = square_of(self.builders_place()?) {
            self.send_units(fighters, place)?;
        }
        // build static defenses
        let gate = self.rules().roles.get(Role::Gate);
        if let (Some(b), Some(gate)) = (self.state().sensible_building, gate) {
            let gate_cost = self.rules().ty(gate).cost.clone();
            let Some(e) = self.unit(b) else {
                return Ok(());
            };
            // A building stands on a square.
            let Some(s) = square_of(e.place()) else {
                return Ok(());
            };
            let (bx, by) = e.xy();
            let mut exits = self.exits(s).to_vec();
            exits.sort_by_key(|&x| {
                let (ex, ey) = self.exit_xy(x);
                square_of_distance(bx, by, ex, ey)
            });
            if let Some(&exit) = exits.first()
                && !self.exit_is_blocked(exit)
                && self.has_resources(&gate_cost)
            {
                for w in self.state().workers.clone() {
                    let name = self.rules().ty(gate).name.clone();
                    let gate = Request::build(name, TargetRef::Entity(exit));
                    self.give(w, gate, true)?;
                }
            }
        }
        Ok(())
    }

    fn idle_buildings_research(&mut self) -> Res {
        for u in self.units().to_vec() {
            let Some(e) = self.unit(u) else { continue };
            if e.has_orders() {
                continue;
            }
            let researchable = e
                .type_id()
                .map(|t| self.rules().ty(t).can_research.clone())
                .unwrap_or_default();
            for t in researchable {
                let cost = self.rules().ty(t).cost.clone();
                if self.future_nb(&Sel::List(vec![t])) == 0
                    && self.missing_resources(&cost).is_empty()
                    && self.potential(&cost) > 3
                {
                    self.give(u, Request::research(self.name_of(t)), true)?;
                }
            }
        }
        Ok(())
    }

    fn raise_dead(&mut self) -> Res {
        for (u, a) in self.state().raise_dead_users.clone() {
            if let Some(Place::Square(s)) = self.unit(u).and_then(Own::place)
                && self.has_corpses_on(s)
            {
                let raise = Request::use_ability(self.name_of(a)).at(TargetRef::Square(s));
                self.give(u, raise, true)?;
            }
        }
        Ok(())
    }

    // Computer: warehouses

    fn build_a_warehouse_if_useful(&mut self) -> Res {
        let Some(warehouse) = self.role_type(Role::TownHall) else {
            return Ok(());
        };
        let cost = self.rules().ty(warehouse).cost.clone();
        if !self.missing_resources(&cost).is_empty() {
            return Ok(());
        }
        let mut deposits = Vec::new();
        for &u in &self.state().workers {
            let Some(e) = self.unit(u) else { continue };
            for o in e.orders() {
                if o.keyword() == Keyword::Gather
                    && let Some(t) = o.target()
                    && self.target_place(&t) != TPlace::Nowhere
                {
                    deposits.push(t);
                }
            }
        }
        for d in deposits {
            self.build_a_warehouse_for(&d, warehouse)?;
        }
        Ok(())
    }

    /// The resource of the deposit a mark names, by its type as the player
    /// knows it: none for what is no deposit it knows, such as one gone
    /// from the map.
    fn deposit_resource_type(&self, d: &Mark) -> Option<i64> {
        self.target_type(d)
            .and_then(|t| self.rules().ty(t).resource_type)
    }

    /// Has the workers near a deposit repair a warehouse being built near
    /// it, or build `warehouse` there if none stands near.
    fn build_a_warehouse_for(&mut self, deposit: &Mark, warehouse: TypeId) -> Res {
        let dplace = self.target_place_of(deposit);
        let nearby: Vec<EntityId> = self
            .state()
            .workers
            .iter()
            .copied()
            .filter(|&v| {
                self.unit(v).is_some_and(|e| {
                    e.place() == dplace
                        || e.first_order().is_some_and(|o| {
                            o.keyword() == Keyword::Gather
                                && o.target().as_ref().is_none_or(|t| {
                                    self.target_place_of(t) == dplace && dplace.is_some()
                                })
                        })
                })
            })
            .collect();
        if nearby.is_empty() {
            return Ok(());
        }
        // A deposit it knows no more, or whose square it does not know,
        // gets no warehouse.
        let (Some(rt), Some(Place::Square(dp_square))) =
            (self.deposit_resource_type(deposit), dplace)
        else {
            return Ok(());
        };
        let dp = Place::Square(dp_square);
        let wh = self.nearest_warehouse(dp_square, rt, true)?;
        let wh = wh.and_then(|w| self.known(w));
        if let Some(wh) = wh
            && wh.class() == EntityClass::BuildingSite
        {
            let wh = wh.id();
            for v in nearby {
                let repair = Request::repair(TargetRef::Entity(wh));
                self.give(v, repair, true)?;
            }
            return Ok(());
        }
        let far = match wh.map(Seen::place).map(square_of) {
            Some(Some(dest)) => self.distance(dp_square, dest, true)? > self.square_width(),
            Some(None) | None => true,
        };
        if far && let Some(meadow) = self.choose(Kind::Meadow, None, Some(dp), false)? {
            let Some(id) = meadow.entity_id() else {
                return Ok(());
            };
            for v in nearby {
                let build = Request::build(self.name_of(warehouse), TargetRef::Entity(id));
                self.give(v, build, true)?;
            }
        }
        Ok(())
    }

    // Computer: choosing a place

    /// `is_ok_for_warehouse`: as far as the player knows, no warehouse for
    /// the resource on the square, and a deposit of it.
    fn is_ok_for_warehouse(&self, z: SquareId, resource_type: i64) -> bool {
        let here = Some(Place::Square(z));
        let known: Vec<Seen> = self
            .perceived()
            .chain(self.remembered().map(|(r, _)| r))
            .filter(|e| e.place() == here)
            .collect();
        let rules = self.rules();
        let stores = |e: &Seen| {
            e.type_id()
                .is_some_and(|t| rules.ty(t).storable_resource_types.contains(&resource_type))
        };
        if known.iter().any(stores) {
            return false;
        }
        known.iter().any(|e| {
            e.deposit_type()
                .is_some_and(|t| rules.ty(t).resource_type == Some(resource_type))
        })
    }

    fn choose_is_ok(&self, o: &Mark, resource_type: Option<i64>) -> bool {
        let Some(place) = self.target_place_of(o) else {
            return false;
        };
        let Place::Square(s) = place else {
            return resource_type.is_none();
        };
        resource_type.is_none_or(|rt| self.is_ok_for_warehouse(s, rt))
            && !self.square_is_dangerous(s)
    }

    /// `choose`: a known meadow or deposit, safe, near `starting_place`
    /// (the first unit's place by default). With `random`, the deposit of
    /// the nearest place gathered by the fewest workers.
    fn choose(
        &mut self,
        kind: Kind,
        resource_type: Option<i64>,
        starting_place: Option<Place>,
        random: bool,
    ) -> Res<Option<Mark>> {
        let Some(first_place) = self
            .units()
            .first()
            .map(|&u| self.unit(u).and_then(Own::place))
        else {
            return Ok(None);
        };
        let key = (kind, resource_type, starting_place);
        if !random && let Some(o) = self.state().previous_choose.get(&key).cloned() {
            if self.is_known(&o) && self.choose_is_ok(&o, resource_type) {
                return Ok(Some(o));
            }
            self.state_mut().previous_choose.remove(&key);
        }
        let start = starting_place.or(first_place);
        let rules = self.rules();
        let mut candidates: Vec<(EntityId, Mark)> = self
            .perceived()
            .filter(|&e| kind.matches(rules, e))
            .map(|e| (e.id(), Door::mark(e)))
            .collect();
        candidates.extend(
            self.remembered()
                .filter(|&(e, _)| kind.matches(rules, e))
                .map(|(e, t)| (e.id(), t)),
        );
        candidates.retain(|(_, o)| self.choose_is_ok(o, resource_type));
        candidates.sort_by_key(|c| c.0);
        let mut candidates: Vec<Mark> = candidates.into_iter().map(|(_, o)| o).collect();
        if !candidates.is_empty() {
            // Nothing is near a start inside a unit, and meadows and
            // deposits are on squares.
            let Some(start) = square_of(start) else {
                return Ok(None);
            };
            if candidates.len() > 10 {
                candidates = self.remove_far_candidates(&candidates, start, 10);
            } else {
                let mut keyed = Vec::new();
                for o in candidates {
                    if let Some(dest) = square_of(self.target_place_of(&o)) {
                        let d = self.distance(start, dest, true)?;
                        keyed.push((d, o));
                    }
                }
                keyed.sort_by_key(|(d, _)| *d);
                candidates = keyed.into_iter().map(|(_, o)| o).collect();
            }
        }
        if random && let Some(c0) = candidates.first() {
            let p0 = self.target_place_of(c0);
            let mut keyed = Vec::new();
            for o in candidates
                .into_iter()
                .filter(|o| self.target_place_of(o) == p0)
            {
                keyed.push((self.deposit_priority(&Some(o.clone())), o));
            }
            keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
            candidates = keyed.into_iter().map(|(_, o)| o).collect();
        }
        let result = candidates.into_iter().next();
        if !random && let Some(o) = &result {
            self.state_mut().previous_choose.insert(key, o.clone());
        }
        Ok(result)
    }

    /// `_remove_far_candidates`: the candidates met first in a breadth
    /// first walk of the squares from `start`, each on the square where the
    /// player knows it; on one square, in id order.
    fn remove_far_candidates(
        &self,
        candidates: &[Mark],
        start: SquareId,
        limit: usize,
    ) -> Vec<Mark> {
        let mut on: BTreeMap<SquareId, BTreeMap<EntityId, Mark>> = BTreeMap::new();
        for o in candidates {
            if let (Some(Place::Square(s)), Some(id)) = (self.target_place_of(o), o.entity_id()) {
                on.entry(s).or_default().insert(id, o.clone());
            }
        }
        let mut c = Vec::new();
        let mut queue = std::collections::VecDeque::from([start]);
        let mut done: Vec<SquareId> = Vec::new();
        while c.len() < limit {
            let Some(room) = queue.pop_front() else { break };
            // A square may be queued twice before it is walked.
            if done.contains(&room) {
                continue;
            }
            for t in on.get(&room).into_iter().flat_map(BTreeMap::values) {
                c.push(t.clone());
                if c.len() >= limit {
                    break;
                }
            }
            for next in self.squares_beyond(room) {
                if !done.contains(&next) {
                    queue.push_back(next);
                }
            }
            done.push(room);
        }
        c
    }

    /// `_deposit_priority`. A deposit it knows no more counts as none, and
    /// of a resource the game does not have, the player has none.
    fn deposit_priority(&self, d: &Option<Mark>) -> DepositPriority {
        let Some((t, rt)) = d
            .as_ref()
            .and_then(|t| Some((t, self.deposit_resource_type(t)?)))
        else {
            return (-100, 0, None);
        };
        let workers = self
            .state()
            .gathered_deposits
            .iter()
            .find(|(k, _)| same_opt(k, d))
            .map_or(0, |(_, n)| *n);
        let r = usize::try_from(rt)
            .ok()
            .and_then(|i| self.resources().get(i))
            .copied()
            .unwrap_or(0);
        (floordiv(-r, 10), -workers, t.entity_id())
    }

    // Computer: counting and getting units

    fn entity_matches(e: Own, sel: &Sel) -> bool {
        match sel {
            Sel::Workers => e.class() == EntityClass::Worker,
            _ => e.type_id().is_some_and(|t| sel.matches_type(t)),
        }
    }

    fn nb(&self, sel: &Sel) -> i64 {
        if let Sel::List(l) = sel
            && let Some(&first) = l.first()
            && self.has_upgrade(first)
        {
            return 1;
        }
        self.units()
            .iter()
            .filter_map(|&u| self.unit(u))
            .filter(|&e| Self::entity_matches(e, sel))
            .count() as i64
    }

    fn nb_in_production(&self, sel: &Sel) -> i64 {
        let mut n = 0;
        for e in self.units().iter().filter_map(|&u| self.unit(u)) {
            if e.site_type().is_some_and(|t| sel.matches_type(t)) {
                n += 1;
                continue;
            }
            for o in e.orders() {
                if o.made().is_some_and(|t| sel.matches_type(t)) {
                    n += 1;
                }
            }
        }
        n
    }

    fn future_nb(&self, sel: &Sel) -> i64 {
        self.nb(sel) + self.nb_in_production(sel)
    }

    // A cost has a value for each of the player's resources: the rules
    // give a cost, and the map the resources, for each resource type of
    // the rules.

    /// The resources, by index, of which the player has less than `cost`.
    fn missing_resources(&self, cost: &[i64]) -> Vec<usize> {
        cost.iter()
            .zip(self.resources())
            .enumerate()
            .filter(|(_, (c, have))| c > have)
            .map(|(i, _)| i)
            .collect()
    }

    /// How many times over the player could pay `cost`, 9999 at most.
    fn potential(&self, cost: &[i64]) -> i64 {
        self.resources()
            .iter()
            .zip(cost)
            .filter(|&(_, &c)| c != 0)
            .map(|(&res, &c)| floordiv(res, c))
            .fold(9999, i64::min)
    }

    /// `get`: works toward having `nb` units of the type; true if they are
    /// there. Of a name that is no type, it never has any.
    fn get(&mut self, nb: i64, t: Option<TypeId>) -> Planned<bool> {
        self.state_mut().safe_cnt = 0;
        self.get_types(nb, t.into_iter().collect(), 0)
    }

    /// `_get`. `depth` counts the planning calls this one is inside.
    fn get_types(&mut self, nb: i64, types: Vec<TypeId>, depth: usize) -> Planned<bool> {
        let depth = deeper(depth)?;
        let sel = Sel::List(types.clone());
        if self.nb(&sel) >= nb {
            return Ok(true);
        }
        if self.future_nb(&sel) >= nb {
            return Ok(false);
        }
        self.state_mut().safe_cnt += 1;
        if self.state().safe_cnt > 10 {
            return Ok(false);
        }
        for &t in &types {
            let makers = self.makers(t);
            if self.nb(&Sel::List(makers.clone())) > 0 {
                let n = nb - self.future_nb(&sel);
                self.build_or_train_or_upgradeto_or_summon(t, n, depth)?;
                break;
            } else if let Some(&m) = makers.first() {
                self.get_types(1, vec![m], depth)?;
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// `gather`: true if the resources and the food are there; asks for a
    /// farm when food is missing.
    fn gather(&mut self, cost: &[i64], food: i64, depth: usize) -> Planned<bool> {
        let depth = deeper(depth)?;
        if !self.has_resources(cost) {
            return Ok(false);
        }
        let limit = self.food_limit();
        let available = self.available_food();
        if food != 0 && food > available - self.used_food() {
            if available < limit {
                // With rules that have no farm, it cannot raise its food.
                let Some(farm) = self.role_type(Role::Farm) else {
                    return Ok(false);
                };
                let t = Sel::One(farm);
                if self.future_nb(&t) == self.nb(&t) {
                    self.build_or_train_or_upgradeto_or_summon(farm, 1, depth)?;
                }
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }

    /// Whether the player has the resources of `cost`. If not, the cost is
    /// that of the first purchase of its turn that lacked a resource,
    /// unless one lacked one before.
    fn has_resources(&mut self, cost: &[i64]) -> bool {
        if self.missing_resources(cost).is_empty() {
            return true;
        }
        self.state_mut()
            .lacking
            .get_or_insert_with(|| cost.to_vec());
        false
    }

    fn get_requirements(&mut self, requirements: &[ClassId], depth: usize) -> Planned<bool> {
        let depth = deeper(depth)?;
        for &r in requirements {
            if !self.has(r) {
                let t = self.rules().class_type(r);
                return self.get_types(1, t.into_iter().collect(), depth);
            }
        }
        Ok(true)
    }

    fn build_or_train_or_upgradeto_or_summon(
        &mut self,
        made: TypeId,
        nb: i64,
        depth: usize,
    ) -> Planned {
        let depth = deeper(depth)?;
        let t = self.rules().ty(made).clone();
        let makers = self.makers(made);
        if !self.get_types(1, makers.clone(), depth)?
            || !self.get_requirements(&t.requirements, depth)?
        {
            return Ok(());
        }
        let Some(mut maker) = makers.last().copied() else {
            return Ok(());
        };
        for &m in &makers {
            if self.nb(&Sel::One(m)) != 0 {
                maker = m;
                break;
            }
        }
        let mc = self.rules().ty(maker).clone();
        let maker_sel = Sel::One(maker);
        if mc.can_upgrade_to.contains(&made) {
            if self.nb(&maker_sel) >= nb {
                let cost: Vec<i64> = t
                    .cost
                    .iter()
                    .enumerate()
                    .map(|(i, c)| c - mc.cost.get(i).copied().unwrap_or(0))
                    .collect();
                if self.gather(&cost, t.stats.food_cost - mc.stats.food_cost, depth)? {
                    self.order(nb, maker_sel, Task::UpgradeTo(made), false)?;
                }
            } else {
                self.get_types(nb, vec![maker], depth)?;
            }
        } else if mc.can_build.contains(&made) {
            if !self.gather(&t.cost, t.stats.food_cost, depth)? {
                return Ok(());
            }
            let meadow = if let Some(&rt) = t.storable_resource_types.first() {
                let meadow = self.choose(Kind::Meadow, Some(rt), None, false)?;
                // Builders inside a unit are far from any meadow.
                let far = match &meadow {
                    None => true,
                    Some(m) => {
                        let start = square_of(self.target_place_of(m));
                        let dest = square_of(self.builders_place()?);
                        match (start, dest) {
                            (Some(start), Some(dest)) => {
                                self.distance_through_walls(start, dest)? > self.square_width() * 3
                            }
                            _ => true,
                        }
                    }
                };
                if far {
                    if self.nb(&Sel::One(made)) != 0 {
                        return Ok(());
                    }
                    let bp = self.builders_place()?;
                    self.choose(Kind::Meadow, None, bp, false)?
                } else {
                    meadow
                }
            } else {
                let bp = self.builders_place()?;
                self.choose(Kind::Meadow, None, bp, false)?
            };
            if let Some(id) = meadow.as_ref().and_then(Mark::entity_id) {
                self.order(4, maker_sel, Task::Build(made, id), true)?;
            }
        } else if mc.can_train.contains(&made) {
            let nb_maker = self.nb(&maker_sel);
            if self.nb(&Sel::Workers) != 0
                && nb > nb_maker * 3
                && self.potential(&t.cost) > nb_maker * 100
            {
                // additional production sites
                self.build_or_train_or_upgradeto_or_summon(maker, 1, depth)?;
            }
            if self.gather(&t.cost, t.stats.food_cost, depth)? {
                self.order(nb, maker_sel, Task::Train(made), false)?;
            }
        } else if mc.can_research.contains(&made) {
            if self.gather(&t.cost, t.stats.food_cost, depth)? {
                self.order(1, maker_sel, Task::Research(made), false)?;
            }
        } else {
            for a in self.listed(Some(maker), |t| &t.can_use) {
                let summons = matches!(
                    &self.rules().ty(a).effect,
                    Some(Effect::Summon { items, .. }) if items.iter().any(|s| s.ty == made)
                );
                if summons {
                    let mut target = None;
                    if self.rules().ty(a).effect_target == Some(EffectTarget::Ask) {
                        // The maker is one of its units: an upgrade, which a
                        // player has without a unit, makes nothing.
                        let Some(&first) = self.units().first() else {
                            break;
                        };
                        target = Some(first);
                    }
                    self.order(1, maker_sel, Task::Use(a, target), false)?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// The types able to build, train, upgrade to, research or summon `t`,
    /// in definition order.
    fn makers(&self, t: TypeId) -> Vec<TypeId> {
        self.rules().get_makers(t)
    }

    /// A list of a unit's type, such as what it can use.
    fn listed(
        &self,
        t: Option<TypeId>,
        list: impl Fn(&crate::rules::UnitType) -> &Vec<TypeId>,
    ) -> Vec<TypeId> {
        t.map(|t| list(self.rules().ty(t)).clone())
            .unwrap_or_default()
    }

    /// The type that plays a role for the player's faction, if the rules
    /// have one.
    fn role_type(&self, role: Role) -> Option<TypeId> {
        let rules = self.rules();
        rules.type_id(&rules.role_name(self.faction(), role))
    }

    /// The name of a type, as an order names it.
    fn name_of(&self, t: TypeId) -> String {
        self.rules().ty(t).name.clone()
    }

    fn worker_orders_priority(&self, u: EntityId) -> (u8, Option<DepositPriority>) {
        match self.unit(u).and_then(Own::first_order) {
            None => (0, None),
            Some(o) if o.keyword() == Keyword::Gather => {
                (1, Some(self.deposit_priority(&o.target())))
            }
            Some(_) => (2, None),
        }
    }

    /// `order`: gives the order to units of a type until `nb` of them
    /// follow it. With `requisition`, busy workers may be taken, the least
    /// useful first.
    fn order(&mut self, nb: i64, sel: Sel, task: Task, requisition: bool) -> Res {
        let key = (sel.clone(), task);
        let order = task.request(self.rules());
        let mut tracked = self.state_mut().orders.remove(&key).unwrap_or_default();
        tracked.retain_mut(|t| self.still_followed(t));
        let enough = tracked.len() as i64 >= nb;
        self.state_mut().orders.insert(key.clone(), tracked);
        if enough {
            return Ok(());
        }
        let mut units: Vec<EntityId> = self
            .units()
            .iter()
            .copied()
            .filter(|&u| self.unit(u).is_some_and(|e| Self::entity_matches(e, &sel)))
            .collect();
        while !units.is_empty() {
            if requisition {
                let mut keyed = Vec::new();
                for u in units {
                    keyed.push((self.worker_orders_priority(u), u));
                }
                keyed.sort_by_key(|k| k.0);
                units = keyed.into_iter().map(|(_, u)| u).collect();
            }
            let u = units.remove(0);
            if order.keyword() == Keyword::UpgradeTo && self.is_exploring(u) {
                self.give(u, Request::stop(), true)?;
            }
            let first = self
                .unit(u)
                .and_then(Own::first_order)
                .map(|o| (o.keyword(), o.target()));
            if requisition || first.is_none() {
                if let Some((kw, target)) = &first {
                    if matches!(kw, Keyword::Build | Keyword::Repair) {
                        continue;
                    }
                    // A worker whose deposit changed during the turn may
                    // not be counted on it yet.
                    if requisition && *kw == Keyword::Gather {
                        add_count(&mut self.state_mut().gathered_deposits, target, -1);
                    }
                }
                self.give(u, order.clone(), true)?;
                if let Some(t) = self.follow_first(u, order.keyword()) {
                    let list = self.state_mut().orders.entry(key.clone()).or_default();
                    list.push(t);
                    if list.len() as i64 >= nb {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}

fn add_count(counts: &mut Vec<(Option<Mark>, i64)>, key: &Option<Mark>, n: i64) {
    match counts.iter_mut().find(|(k, _)| same_opt(k, key)) {
        Some(e) => e.1 += n,
        None => counts.push((key.clone(), n)),
    }
}

#[cfg(test)]
mod fairness;

#[cfg(test)]
mod order_sites;

#[cfg(test)]
mod levels;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::PlayerId;
    use crate::world::World;

    /// A world with the original's rules and this map.
    /// A file of the original game.
    fn read(f: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
        let bytes = std::fs::read(dir.join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// A world with the original's rules and this map.
    pub(super) fn world(map: &str) -> World {
        world_with_rules(&read("res/rules.txt"), map)
    }

    /// A world of the game's rules, then of `extra`, as a mod adds to them.
    pub(super) fn world_with_extra_rules(extra: &str, map: &str) -> World {
        let rules = crate::Rules::load(&[read("res/rules.txt"), extra.to_owned()]);
        let ai = crate::AiScripts::load(&[read("res/ai.txt")]);
        let mut w = World::new(rules, ai, Vec::new(), 0);
        w.load_and_build_map(map).unwrap();
        w
    }

    fn world_with_rules(rules: &str, map: &str) -> World {
        let rules = crate::Rules::load(&[rules]);
        let ai = crate::AiScripts::load(&[read("res/ai.txt")]);
        let mut w = World::new(rules, ai, Vec::new(), 0);
        w.load_and_build_map(map).unwrap();
        w
    }

    #[test]
    fn on_a_tie_the_builders_place_is_the_square_and_not_the_transport() {
        let mut w = world(
            "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 0
nb_players_min 1
nb_players_max 1
starting_squares a1
",
        );
        let p = w
            .populate_map(&[crate::Controller::computer("easy")], false, false)
            .unwrap()[0];
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let create = |w: &mut World, t: &str| {
            let tid = w.rules.type_id(t).unwrap();
            w.create_creature(tid, Some(p), Place::Square(a1), x, y)
                .unwrap()
        };
        let outside = create(&mut w, "peasant");
        let inside = create(&mut w, "peasant");
        let transport = create(&mut w, "new_flyingmachine");
        w.load(transport, inside);
        w.computer_state(p).workers = vec![inside, outside];
        assert_eq!(w.door(p).builders_place().unwrap(), Some(Place::Square(a1)));
        w.computer_state(p).workers = vec![outside, inside];
        assert_eq!(w.door(p).builders_place().unwrap(), Some(Place::Square(a1)));
    }

    /// The workers of `p` gathering each resource: gold, then wood.
    fn gatherers(w: &mut World, p: PlayerId) -> Vec<usize> {
        let mut n = vec![0; w.pl(p).resources.len()];
        let door = w.door(p);
        for &u in door.units() {
            if let Some(o) = door.unit(u).unwrap().first_order()
                && o.keyword() == Keyword::Gather
                && let Some(t) = o.target()
            {
                n[door.deposit_resource_type(&t).unwrap() as usize] += 1;
            }
        }
        n
    }

    #[test]
    fn at_its_limit_without_gold_for_a_farm_the_computer_sends_workers_from_wood_to_gold() {
        // 9 peasants fill the food of a townhall and a farm. A farm costs 5
        // gold and 5 wood: the computer has no gold and 50 wood, and every
        // peasant gathers wood.
        let mut w = world(
            "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 4
goldmines 1000 a1
woods 1000 a1
nb_players_min 1
nb_players_max 1
starting_squares a1
starting_units townhall farm 9 peasant
starting_resources 0 50
",
        );
        let p = w
            .populate_map(&[crate::Controller::computer("easy")], false, false)
            .unwrap()[0];
        let a1 = w.square_by_name("a1").unwrap();
        let wood = w.square(a1).objects.clone().into_iter().find(|&o| {
            w.e(o).class() == EntityClass::Deposit
                && w.e(o).type_id().and_then(|t| w.rules.ty(t).resource_type) == Some(1)
        });
        let wood = wood.unwrap();
        w.update();
        for u in w.pl(p).units.clone() {
            if w.e(u).class() == EntityClass::Worker {
                let gather = Request::gather(TargetRef::Entity(wood));
                w.take_order(u, gather, true, false, None).unwrap();
            }
        }
        let limit = w.food_limit();
        assert_eq!((w.pl(p).used_food, w.pl(p).available_food(limit)), (9, 9));
        w.update();
        assert_eq!(gatherers(&mut w, p), [0, 9]);
        // The computer plays one turn in 10. One of its peasants explores.
        // Each turn, one peasant goes from wood to gold, until one is left
        // on wood; with the gold, the farm is ordered.
        let mut seen = Vec::new();
        for _ in 0..8 {
            for _ in 0..10 {
                w.update();
            }
            seen.push(gatherers(&mut w, p));
        }
        assert_eq!(
            seen,
            [
                [1, 7],
                [2, 6],
                [3, 5],
                [4, 4],
                [5, 3],
                [6, 2],
                [7, 1],
                [7, 1]
            ]
        );
        for _ in 0..20 {
            w.update();
        }
        let farm = Sel::One(w.rules.type_id("farm").unwrap());
        let door = w.door(p);
        assert_eq!((door.nb(&farm), door.future_nb(&farm)), (1, 2));
    }

    /// A computer of the script `easy` alone on a square with these units.
    pub(super) fn scripted(script: &str, units: &str) -> (World, PlayerId) {
        scripted_with_rules(&read("res/rules.txt"), script, units)
    }

    fn scripted_with_rules(rules: &str, script: &str, units: &str) -> (World, PlayerId) {
        let mut w = world_with_rules(
            rules,
            &format!(
                "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 4
goldmines 1000 a1
woods 1000 a1
nb_players_min 1
nb_players_max 1
starting_squares a1
starting_units {units}
starting_resources 100 100
"
            ),
        );
        w.ai_scripts = crate::AiScripts::load(&[script]);
        let p = w
            .populate_map(&[crate::Controller::computer("easy")], false, false)
            .unwrap()[0];
        (w, p)
    }

    #[test]
    fn a_script_line_with_a_mistake_is_skipped_and_the_computer_plays_on() {
        let (mut w, p) = scripted(
            "def easy\nwatchdog soon\ngoto_random\nattack\n",
            "townhall 2 peasant",
        );
        assert_eq!(
            w.ai_scripts.warnings,
            [
                "in ai.txt, easy, \"watchdog soon\": watchdog needs a number",
                "in ai.txt, easy, \"goto_random\": goto_random needs labels",
            ]
        );
        // The computer plays one turn in 10.
        for _ in 0..30 {
            w.update();
        }
        assert_eq!(w.drain_warnings(), Vec::<String>::new());
        assert_eq!(w.computer_state(p).constant_attacks, 1);
    }

    #[test]
    fn a_purchase_that_asks_for_itself_waits_and_the_computer_plays_on() {
        // A farm that costs food asks for a farm for its food, and so on.
        // Its food is full: getting more workers asks for a farm, and the
        // planning goes too deep.
        let (mut w, p) = scripted("def easy\nattack\n", "townhall farm 9 peasant");
        w.rules.unit_type_mut("farm").unwrap().stats.food_cost = 100;
        for _ in 0..30 {
            w.update();
        }
        let warning = "recursion error with easy; getting its workers";
        assert_eq!(w.drain_warnings(), [warning; 3]);
        // It still followed its plan.
        assert_eq!(w.computer_state(p).constant_attacks, 1);
    }

    #[test]
    fn a_soldier_inside_a_transport_does_not_stop_the_computer() {
        let (mut w, p) = scripted("def easy\nattack\n", "townhall 2 peasant");
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let create = |w: &mut World, t: &str| {
            let tid = w.rules.type_id(t).unwrap();
            w.create_creature(tid, Some(p), Place::Square(a1), x, y)
                .unwrap()
        };
        let footman = create(&mut w, "footman");
        let transport = create(&mut w, "new_flyingmachine");
        w.load(transport, footman);
        // It was attacked on its square: its idle soldiers would fight.
        for _ in 0..30 {
            w.computer_state(p).attacked_places = vec![Some(Place::Square(a1))];
            w.update();
        }
        assert_eq!(w.drain_warnings(), Vec::<String>::new());
        assert_eq!(w.computer_state(p).constant_attacks, 1);
    }

    #[test]
    fn an_order_to_a_unit_no_longer_its_own_is_not_given() {
        let (mut w, p) = scripted("def easy\n", "townhall 2 peasant");
        let theirs = w.pl(p).units[0];
        w.plm(p).units.retain(|&u| u != theirs);
        w.cm(theirs).unwrap().player = None;
        let before = w.e(theirs).orders().len();
        assert!(w.door(p).give(theirs, Request::stop(), true).is_ok());
        assert_eq!(w.e(theirs).orders().len(), before);
    }

    #[test]
    fn an_ai_type_without_a_script_plays_without_one() {
        let (mut w, p) = scripted("def easy\nattack\n", "townhall 2 peasant");
        for _ in 0..10 {
            w.update();
        }
        assert_eq!(w.computer_state(p).constant_attacks, 1);
        // A map's trigger changes its AI type to one without a script.
        w.computer_set_ai(p, "dance");
        assert_eq!(
            w.drain_warnings(),
            ["no script 'dance' in ai.txt: the computer plays without one"]
        );
        assert!(w.computer_state(p).plan.is_none());
        assert_eq!(w.computer_state(p).constant_attacks, 0);
    }

    #[test]
    fn rules_without_a_farm_leave_the_computer_short_of_food() {
        // The farm is called otherwise, where it is defined and where the
        // peasant builds it.
        let rules = read("res/rules.txt").replace(" farm", " barn");
        // A town hall gives the food of one peasant: getting more asks
        // for a farm.
        let (mut w, p) = scripted_with_rules(&rules, "def easy\nattack\n", "townhall 1 peasant");
        for _ in 0..30 {
            w.update();
        }
        assert_eq!(w.drain_warnings(), Vec::<String>::new());
        assert_eq!(w.computer_state(p).constant_attacks, 1);
    }

    #[test]
    fn computer2_knowing_nothing_to_aim_at_gives_no_aimed_order() {
        let mut w = world(
            "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 0
nb_players_min 1
nb_players_max 1
starting_squares a1
",
        );
        let p = w
            .populate_map(&[crate::Controller::computer("ai2")], false, false)
            .unwrap()[0];
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let tid = w.rules.type_id("peasant").unwrap();
        let u = w
            .create_creature(tid, Some(p), Place::Square(a1), x, y)
            .unwrap();
        for _ in 0..200 {
            let order = w.door(p).random_order(u, &[]);
            assert!(
                order
                    .as_ref()
                    .is_ok_and(|o| o.as_ref().is_none_or(|o| o.target().is_none()))
            );
        }
    }

    /// The id of the first entity on the only square of `w` that `f`
    /// accepts.
    fn on_a1(w: &World, f: impl Fn(&crate::entity::Entity) -> bool) -> EntityId {
        let a1 = w.square_by_name("a1").unwrap();
        w.square(a1)
            .objects
            .iter()
            .copied()
            .find(|&o| f(w.e(o)))
            .unwrap()
    }

    #[test]
    fn what_is_no_deposit_it_knows_gets_no_warehouse() {
        let (mut w, p) = scripted("def easy\n", "townhall 2 peasant");
        w.update_effect_users_and_workers(p);
        w.update_perception_and_memory(p).unwrap();
        let meadow = on_a1(&w, |e| e.is_meadow());
        let townhall = w.rules.type_id("townhall").unwrap();
        let not_a_deposit = Mark::of(crate::entity::Target::Entity(meadow));
        let r = w.door(p).build_a_warehouse_for(&not_a_deposit, townhall);
        assert!(r.is_ok());
    }

    #[test]
    fn of_a_resource_the_game_lacks_the_player_has_none() {
        let (mut w, p) = scripted("def easy\n", "townhall 2 peasant");
        let gold = on_a1(&w, |e| e.is_deposit());
        let goldmine = w.e(gold).type_name.to_string();
        w.rules.unit_type_mut(&goldmine).unwrap().resource_type = Some(7);
        let d = Some(Mark::of(crate::entity::Target::Entity(gold)));
        let priority = format!("{:?}", w.door(p).deposit_priority(&d));
        assert_eq!(priority, format!("(0, 0, Some({gold:?}))"));
    }

    #[test]
    fn a_worker_not_counted_on_its_deposit_is_requisitioned() {
        let (mut w, p) = scripted("def easy\n", "townhall 2 peasant");
        let gold = on_a1(&w, |e| e.is_deposit());
        let workers: Vec<EntityId> = w.pl(p).units[1..].to_vec();
        for &u in &workers {
            let gather = Request::gather(TargetRef::Entity(gold));
            w.take_order(u, gather, true, false, None).unwrap();
        }
        w.update_effect_users_and_workers(p);
        w.computer_state(p).gathered_deposits.clear();
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let farm = w.rules.type_id("farm").unwrap();
        let site = w
            .create_building_site(p, Place::Square(a1), x, y, farm)
            .unwrap();
        let r = w.door(p).order(1, Sel::Workers, Task::Repair(site), true);
        assert!(r.is_ok());
        let repairs = workers
            .iter()
            .filter(|&&u| {
                w.e(u).orders().first().map(crate::orders::Order::keyword) == Some(Keyword::Repair)
            })
            .count();
        assert_eq!(repairs, 1);
    }

    #[test]
    fn a_warehouse_inside_a_unit_is_not_reached() {
        let (mut w, p) = scripted("def easy\n", "2 peasant");
        let footman = w.rules.type_id("footman").unwrap();
        w.rules
            .unit_type_mut("footman")
            .unwrap()
            .storable_resource_types = vec![0];
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let store = w
            .create_creature(footman, Some(p), Place::Square(a1), x, y)
            .unwrap();
        let flyer = w.rules.type_id("new_flyingmachine").unwrap();
        let transport = w
            .create_creature(flyer, Some(p), Place::Square(a1), x, y)
            .unwrap();
        w.load(transport, store);
        let r = w.door(p).nearest_warehouse(a1, 0, true);
        assert_eq!(r.ok(), Some(None));
    }

    #[test]
    fn a_script_gets_the_units_of_the_faction_the_computer_has_now() {
        // A map's trigger makes it orc after it took up its script, as in
        // the orc campaign's first chapter: its footman is a grunt.
        let rules = format!(
            "{}\n{}",
            read("res/rules.txt"),
            read("single/orc/rules.txt")
        );
        let (mut w, p) = scripted_with_rules(
            &rules,
            "def easy\nget 1 footman\nattack\n",
            "townhall 2 peasant grunt",
        );
        w.plm(p).faction = w.rules.faction_id("human_faction");
        w.computer_set_ai(p, "easy");
        w.plm(p).faction = w.rules.faction_id("orc_faction");
        for _ in 0..30 {
            w.update();
        }
        assert_eq!(w.computer_state(p).constant_attacks, 1);
    }

    #[test]
    fn a_worker_inside_a_unit_leaves_the_others_to_gather() {
        let (mut w, p) = scripted("def easy\n", "townhall 3 peasant");
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let flyer = w.rules.type_id("new_flyingmachine").unwrap();
        let transport = w
            .create_creature(flyer, Some(p), Place::Square(a1), x, y)
            .unwrap();
        let peasants: Vec<EntityId> = w.pl(p).units[1..4].to_vec();
        w.load(transport, peasants[0]);
        w.update_effect_users_and_workers(p);
        w.update_perception_and_memory(p).unwrap();
        w.door(p).idle_workers_gather().unwrap();
        let gathering = peasants[1..]
            .iter()
            .filter(|&&u| {
                w.e(u).orders().first().map(crate::orders::Order::keyword) == Some(Keyword::Gather)
            })
            .count();
        assert_eq!(gathering, 2);
    }

    #[test]
    fn computer2_aims_only_the_abilities_that_ask_for_a_target() {
        let mut w = world(
            "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 0
nb_players_min 1
nb_players_max 1
starting_squares a1
",
        );
        let p = w
            .populate_map(&[crate::Controller::computer("ai2")], false, false)
            .unwrap()[0];
        let a1 = w.square_by_name("a1").unwrap();
        let (x, y) = (w.square(a1).x, w.square(a1).y);
        let tid = w.rules.type_id("necromancer").unwrap();
        let u = w
            .create_creature(tid, Some(p), Place::Square(a1), x, y)
            .unwrap();
        let mut uses = BTreeMap::new();
        for _ in 0..500 {
            if let Some(o) = w.door(p).random_order(u, &[u]).unwrap()
                && o.keyword() == Keyword::Use
            {
                let ability = o.type_name().unwrap().to_owned();
                uses.insert(ability, o.target().is_some());
            }
        }
        // `a_raise_dead` acts around the necromancer.
        assert_eq!(uses, [("a_raise_dead".to_owned(), false)].into());
    }

    #[test]
    fn a_square_reached_by_two_ways_gives_its_candidates_once() {
        // a1 leads to b1 and a2, which both lead to b2.
        let mut w = world(
            "
square_width 12
nb_columns 2
nb_lines 2
west_east_paths a1 a2
south_north_paths a1 b1
nb_meadows_by_square 3
nb_players_min 1
nb_players_max 1
starting_squares a1
",
        );
        let p = w
            .populate_map(&[crate::Controller::computer("easy")], false, false)
            .unwrap()[0];
        let b2 = w.square_by_name("b2").unwrap();
        let meadows: Vec<Mark> = w
            .square(b2)
            .objects
            .iter()
            .filter(|&&o| w.e(o).is_meadow())
            .map(|&o| Mark::of(crate::entity::Target::Entity(o)))
            .collect();
        assert_eq!(meadows.len(), 3);
        for m in meadows.iter().filter_map(Mark::entity_id) {
            w.plm(p).perception.insert(m);
        }
        let a1 = w.square_by_name("a1").unwrap();
        let near = w.door(p).remove_far_candidates(&meadows, a1, 10);
        let ids: Vec<EntityId> = near.iter().filter_map(Mark::entity_id).collect();
        let expected: Vec<EntityId> = meadows.iter().filter_map(Mark::entity_id).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn computer2_chooses_from_the_original_list_of_orders() {
        const EXCLUDED: &[Keyword] = &[
            Keyword::Enter,
            Keyword::Stop,
            Keyword::Attack,
            Keyword::Patrol,
            Keyword::Wait,
            Keyword::AutoExplore,
            Keyword::AutoAttack,
            Keyword::Block,
            Keyword::JoinGroup,
            Keyword::CancelBuilding,
            Keyword::CancelTraining,
            Keyword::CancelUpgrading,
            Keyword::RallyingPoint,
        ];
        let mut all: Vec<Keyword> = Keyword::ALL
            .iter()
            .copied()
            .filter(|k| !EXCLUDED.contains(k))
            .collect();
        all.sort_by_key(|k| k.as_str());
        assert_eq!(all, COMPUTER2_ORDERS);
        let names: Vec<&str> = COMPUTER2_ORDERS.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "build",
                "build_phase_two",
                "disable_auto_gather",
                "disable_auto_repair",
                "enable_auto_gather",
                "enable_auto_repair",
                "gather",
                "go",
                "load",
                "load_all",
                "mode_defensive",
                "mode_offensive",
                "repair",
                "research",
                "train",
                "unload_all",
                "upgrade_to",
                "use",
            ]
        );
    }
}
