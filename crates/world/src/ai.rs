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
//! random numbers. Where the original raises an exception, the work stops
//! with an [`Abort`], which ends the player's update as in the original.
//! Where the original would use the id of an interior (`None`), the AI
//! stops too.

use std::collections::{BTreeMap, BTreeSet};

use crate::abort::{Abort, Res};
use crate::entity::{CreatureKind, Entity, EntityId, Place, SquareId, Target};
use crate::nofloat::{floordiv, py_int, square_of_distance};
use crate::orders::{Keyword, Order, OrderKind, Request, TargetRef};
use crate::player::PlayerKind;
use crate::rules::{AirGround, EntityClass, Role};
use crate::world::TPlace;

mod door;
mod handicap;

use door::{Door, Seen};
pub(crate) use handicap::handicapped_name;
pub use handicap::{Handicap, RANDOM_AI_TYPE, computer_name};

/// The number of workers a scripted computer keeps getting.
const NB_WORKERS_TO_GET: i64 = 10;

/// The deepest the computer player's planning may go. Getting a unit asks
/// for its makers, its requirements, resources and food, and each of these
/// may ask for more. Rules where they ask for each other without end (a farm
/// that costs food) would exhaust the stack; the original stops at Python's
/// recursion limit. The original's rules and scripts go 11 deep.
const MAX_PLANNING_DEPTH: usize = 64;

/// The error at [`MAX_PLANNING_DEPTH`], with Python's message.
const RECURSION_ERROR: &str = "maximum recursion depth exceeded";

/// One step deeper into the planning, or [`RECURSION_ERROR`].
fn deeper(depth: usize) -> Res<usize> {
    if depth >= MAX_PLANNING_DEPTH {
        return Err(Abort::new(RECURSION_ERROR));
    }
    Ok(depth + 1)
}

/// What `check_type` compares a unit or a type with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Sel {
    /// A list of type names: `nb` counts 1 if the first one is an upgrade
    /// the player has.
    List(Vec<String>),
    /// One type name.
    One(String),
    /// The `Worker` class.
    Workers,
}

impl Sel {
    /// `check_type(type, sel)` for a type known by its name.
    fn matches_type(&self, name: &str) -> bool {
        match self {
            Sel::List(l) => l.iter().any(|t| t == name),
            Sel::One(t) => t == name,
            Sel::Workers => false,
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

/// An order given by `order`, followed until it is done.
#[derive(Clone, Debug, Hash)]
struct Tracked {
    unit: EntityId,
    /// The order as last seen in the unit's queue.
    order: Order,
}

/// The comparison key of `_deposit_priority`: the deposit's id last, and
/// none for a missing deposit.
type DepositPriority = (i64, i64, Option<EntityId>);

/// The state of a scripted computer player (`Computer`).
#[derive(Clone, Debug, Default, Hash)]
pub(crate) struct Computer {
    /// The lines of the script, once an AI type other than `timers` is set.
    plan: Option<Vec<String>>,
    line_nb: i64,
    previous_linechange: i64,
    watchdog: i64,
    constant_attacks: i64,
    research: i64,
    attacked_places: Vec<Option<Place>>,
    orders: BTreeMap<(Sel, Vec<String>), Vec<Tracked>>,
    previous_choose: BTreeMap<(Kind, Option<i64>, Option<Place>), Target>,
    sensible_building: Option<EntityId>,
    workers: Vec<EntityId>,
    /// Workers gathering from each deposit, by the identity of the target.
    gathered_deposits: Vec<(Option<Target>, i64)>,
    building_sites: Vec<EntityId>,
    raise_dead_users: Vec<(EntityId, String)>,
    teleportation_users: Vec<(EntityId, String)>,
    cataclysm_users: Vec<(EntityId, String)>,
    detector_users: Vec<(EntityId, String)>,
    summon_users: Vec<(EntityId, String)>,
    /// Whether units waiting on a target may go, by target.
    time_has_come: Vec<(Option<Target>, bool)>,
    safe_cnt: i64,
    /// The cost of the first purchase of its turn that lacked a resource.
    lacking: Option<Vec<i64>>,
}

impl Computer {
    /// Whether the units waiting on a square are strong enough.
    fn time_has_come(&self, s: SquareId) -> bool {
        let key = Some(Target::Square(s));
        self.time_has_come
            .iter()
            .find(|(t, _)| same_opt(t, &key))
            .is_some_and(|(_, v)| *v)
    }
}

fn same_opt(a: &Option<Target>, b: &Option<Target>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(b),
        _ => false,
    }
}

/// Lazily lists the type names of the enemies on a square, once: the
/// original passes a generator, which the first use exhausts.
struct Enemies {
    place: Option<Place>,
    used: bool,
}

/// The square of a place, or the original's error for another place.
fn square_of(place: Option<Place>) -> Res<SquareId> {
    match place {
        Some(Place::Square(s)) => Ok(s),
        Some(Place::Inside(_)) => Err(Abort::new("'Inside' object has no attribute 'exits'")),
        None => Err(Abort::new("'NoneType' object has no attribute 'exits'")),
    }
}

/// A place as the target of an order: only a square is one.
fn square_target(place: Option<Place>) -> Res<TargetRef> {
    match place {
        Some(Place::Square(s)) => Ok(TargetRef::Square(s)),
        _ => Err(Abort::new("the place has no id")),
    }
}

impl Door<'_> {
    // state

    /// `set_ai`, after the AI type of the player changed: loads the script
    /// and resets the script's settings.
    fn set_ai(&mut self, ai_type: &str) -> Res {
        if ai_type == "timers" || !self.has_state() {
            return Ok(());
        }
        let name = self.faction_ai_type(ai_type);
        let Some(plan) = self.script(&name).map(<[String]>::to_vec) else {
            return Err(Abort::new(format!("KeyError: '{name}'")));
        };
        let time = self.time();
        let c = self.state_mut();
        c.plan = Some(plan);
        c.line_nb = 0;
        c.previous_linechange = time;
        c.watchdog = 0;
        c.constant_attacks = 0;
        c.research = 0;
        self.update_effect_users_and_workers()
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
            PlayerKind::Computer { ai_type } => {
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
        if u.orders().first().map(Order::keyword) == Some(Keyword::AutoExplore) {
            return;
        }
        let place = u.place;
        if u.stats.is_a_building != 0 {
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
            if self.unit(u).is_some_and(|e| e.orders().is_empty())
                && let Some(order) = self.random_order(u, &targets)?
            {
                self.give(u, order, true)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// The orders `Computer2` chooses from, sorted by keyword.
    fn computer2_orders() -> Vec<Keyword> {
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
        let mut v: Vec<Keyword> = Keyword::ALL
            .iter()
            .copied()
            .filter(|k| !EXCLUDED.contains(k))
            .collect();
        v.sort_by_key(|k| k.as_str());
        v
    }

    fn random_order(&mut self, unit: EntityId, targets: &[EntityId]) -> Res<Option<Request>> {
        let orders = Self::computer2_orders();
        let i = self
            .choice_index(orders.len())
            .ok_or_else(|| Abort::new("Cannot choose from an empty sequence"))?;
        let kw = orders[i];
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
            let Some(j) = self.choice_index(targets.len()) else {
                return Err(Abort::new("Cannot choose from an empty sequence"));
            };
            order = order.at(TargetRef::Entity(targets[j]));
        }
        Ok(Some(order))
    }

    // Computer: the turn

    fn computer_play(&mut self) -> Res {
        if !self.should_play_this_turn()? {
            return Ok(());
        }
        self.state_mut().lacking = None;
        self.update_effect_users_and_workers()?;
        self.update_time_has_come()?;
        self.send_workers_to_forgotten_building_sites()?;
        self.idle_workers_gather()?;
        self.send_explorer()?;
        if !self.state().attacked_places.is_empty() {
            let places = self.state().attacked_places.clone();
            self.eventually_attack(places)?;
            self.state_mut().attacked_places.clear();
        } else if self.state().constant_attacks != 0 {
            let places = self
                .enemy_presence()
                .iter()
                .map(|&s| Some(Place::Square(s)))
                .collect();
            self.eventually_attack(places)?;
        } else {
            self.defensive_routine()?;
        }
        if self.state().research != 0 {
            self.idle_buildings_research()?;
        }
        self.raise_dead()?;
        self.build_a_warehouse_if_useful()?;
        let peasant = self.rules().role_name(self.faction(), Role::Worker);
        // A recursion error here ends the player's update with a warning, as
        // the original's ends at the world's catch.
        self.get(NB_WORKERS_TO_GET, &peasant)?;
        let planned = match self.follow_plan() {
            // The original catches a recursion error of its plan here, and
            // goes on with the next line of the plan at its next turn.
            Err(Abort::Unclassified(m)) if m == RECURSION_ERROR => {
                let ai_type = match self.kind() {
                    PlayerKind::Computer { ai_type } => ai_type.clone(),
                    _ => String::new(),
                };
                let c = self.state();
                let line = usize::try_from(c.line_nb)
                    .ok()
                    .and_then(|i| c.plan.as_ref()?.get(i).cloned())
                    .unwrap_or_default();
                let n = c.line_nb + 1;
                self.warn(format!(
                    "recursion error with {ai_type}; current ai.txt line is: {line}"
                ));
                self.set_line_nb(n);
                Ok(())
            }
            other => other,
        };
        planned?;
        self.gather_what_is_lacking()
    }

    /// The cpu intensive players play in turns, one tenth of them each turn.
    fn should_play_this_turn(&self) -> Res<bool> {
        let players = self.planning_computers();
        let i = players
            .iter()
            .position(|&x| x == self.me())
            .ok_or_else(|| Abort::new("x not in list"))?;
        let turn = (i * 10 / players.len()) as i64;
        Ok(self.turn().rem_euclid(10) == turn)
    }

    /// What the player's faction uses instead of the class `name`.
    fn equivalent(&self, name: &str) -> String {
        let faction = self.faction();
        let rules = self.rules();
        match rules.type_id(name) {
            Some(t) => rules.ty(rules.equivalent(faction, t)).name.clone(),
            None => rules.equivalent_name(faction, name),
        }
    }

    fn follow_plan(&mut self) -> Res {
        let Some(plan) = self.state().plan.clone() else {
            return Ok(());
        };
        if plan.is_empty() {
            return Ok(());
        }
        let c = self.state();
        if c.watchdog != 0 && self.time() > c.previous_linechange + c.watchdog * 1000 {
            let n = c.line_nb + 1;
            self.set_line_nb(n);
        }
        let n = self.state().line_nb.rem_euclid(plan.len() as i64);
        self.set_line_nb(n);
        let line_nb = self.state().line_nb;
        let cmd: Vec<&str> = plan[line_nb as usize].split_whitespace().collect();
        let Some(&first) = cmd.first() else {
            return Ok(());
        };
        let arg = |i: usize| -> Res<&str> {
            cmd.get(i)
                .copied()
                .ok_or_else(|| Abort::new("list index out of range"))
        };
        let label_index = |name: &str| plan.iter().position(|l| *l == format!("label {name}"));
        match first {
            "goto" => {
                let dest = arg(1)?;
                let n = if is_signed_number(dest) {
                    line_nb + py_int(dest).map_err(|_| Abort::new("invalid literal for int()"))?
                } else if let Some(i) = label_index(dest) {
                    i as i64
                } else if is_digits(dest) {
                    py_int(dest).map_err(|_| Abort::new("invalid literal for int()"))?
                } else {
                    self.warn(format!("goto: wrong destination: {dest}"));
                    line_nb + 1
                };
                self.set_line_nb(n);
            }
            "label" => {
                self.set_line_nb(line_nb + 1);
                arg(1)?;
            }
            "goto_random" => {
                let choices = &cmd[1..];
                let Some(i) = self.choice_index(choices.len()) else {
                    return Err(Abort::new("Cannot choose from an empty sequence"));
                };
                let dest = choices[i];
                let n = match label_index(dest) {
                    Some(i) => i as i64,
                    None => {
                        self.warn(format!("goto_random: label not found: {dest}"));
                        line_nb + 1
                    }
                };
                self.set_line_nb(n);
            }
            "attack" => {
                self.state_mut().constant_attacks = 1;
                self.set_line_nb(line_nb + 1);
            }
            "watchdog" | "constant_attacks" | "research" => {
                let v = py_int(arg(1)?).map_err(|_| Abort::new("invalid literal for int()"))?;
                let c = self.state_mut();
                match first {
                    "watchdog" => c.watchdog = v,
                    "constant_attacks" => c.constant_attacks = v,
                    _ => c.research = v,
                }
                self.set_line_nb(line_nb + 1);
            }
            "get" => {
                let mut n = 1;
                let mut done = true;
                for &w in &cmd[1..] {
                    if is_digits(w) {
                        n = py_int(w).map_err(|_| Abort::new("invalid literal for int()"))?;
                    } else if self.rules().has_class(w) {
                        let t = self.equivalent(w);
                        if !self.get(n, &t)? {
                            done = false;
                            break;
                        }
                        n = 1;
                    } else {
                        self.warn(format!("get: unknown unit: '{w}' (in ai.txt)"));
                        n = 1;
                    }
                }
                if done {
                    let line_nb = self.state().line_nb;
                    self.set_line_nb(line_nb + 1);
                }
            }
            other => {
                self.warn(format!("unknown command: '{other}' (in ai.txt)"));
                self.set_line_nb(line_nb + 1);
            }
        }
        Ok(())
    }

    // Computer: units and their tasks

    fn update_effect_users_and_workers(&mut self) -> Res {
        let mut workers = Vec::new();
        let mut gathered: Vec<(Option<Target>, i64)> = Vec::new();
        let mut sites = Vec::new();
        let mut raise_dead = Vec::new();
        let mut teleportation = Vec::new();
        let mut cataclysm = Vec::new();
        let mut detector = Vec::new();
        let mut summon = Vec::new();
        for u in self.units().to_vec() {
            let Some(e) = self.unit(u) else { continue };
            match e.creature().map(|c| &c.kind) {
                Some(CreatureKind::Worker { .. }) => {
                    workers.push(u);
                    if let Some(Order {
                        kind: OrderKind::Gather { aim, .. },
                        ..
                    }) = e.orders().first()
                    {
                        add_count(&mut gathered, &aim.target, 1);
                    }
                }
                Some(CreatureKind::BuildingSite(_)) => sites.push(u),
                Some(
                    CreatureKind::Soldier { .. }
                    | CreatureKind::Effect { .. }
                    | CreatureKind::Building { .. },
                )
                | None => {}
            }
            let abilities = e
                .type_id()
                .map(|t| self.rules().ty(t).can_use.clone())
                .unwrap_or_default();
            for a in abilities {
                if !self.is_order_allowed(u, Keyword::Use, Some(&a))? {
                    continue;
                }
                let rules = self.rules();
                let Some(effect) = rules.get_words(&a, "effect") else {
                    continue;
                };
                match effect.first().map(String::as_str) {
                    None => continue,
                    Some("raise_dead") => raise_dead.push((u, a.clone())),
                    Some("teleportation") => teleportation.push((u, a.clone())),
                    Some("summon") => {
                        for item in &effect[1..] {
                            if rules.get_is_truthy(item, "harm_level") {
                                cataclysm.push((u, a.clone()));
                            }
                            if rules.get_is_truthy(item, "is_a_detector") {
                                detector.push((u, a.clone()));
                            }
                            if rules.get_is_truthy(item, "damage") {
                                summon.push((u, a.clone()));
                            }
                        }
                    }
                    Some(_) => {}
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
        Ok(())
    }

    fn update_time_has_come(&mut self) -> Res {
        // (target, menace, units), in the order the targets first appear
        let mut waiting: Vec<(Option<Target>, i64, Vec<EntityId>)> = Vec::new();
        for &u in self.units() {
            let Some(e) = self.unit(u) else { continue };
            if let Some(Order {
                kind: OrderKind::Wait { aim, .. },
                ..
            }) = e.orders().first()
            {
                match waiting
                    .iter_mut()
                    .find(|(t, _, _)| same_opt(t, &aim.target))
                {
                    Some(w) => {
                        w.1 += e.menace();
                        w.2.push(u);
                    }
                    None => waiting.push((aim.target.clone(), e.menace(), vec![u])),
                }
            }
        }
        let thc: Vec<(Option<Target>, bool)> = waiting
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
                        e.orders().iter().any(|o| matches!(&o.kind, OrderKind::Wait { aim, .. } if same_opt(&aim.target, place)))
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
        if self.is_order_allowed(u, Keyword::Stop, None)? {
            self.give(u, Request::stop(), true)?;
        }
        Ok(())
    }

    fn target_enemy_menace(&self, t: Option<&Target>) -> i64 {
        match t {
            Some(Target::Square(s)) => self.enemy_menace(*s),
            _ => 0,
        }
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
            .filter(|e| e.stats.speed > 0 && e.class() == EntityClass::Soldier)
            .map(Entity::menace)
            .sum();
        menace > floordiv(enemy_menace * ratio, 100)
    }

    fn send_workers_to_forgotten_building_sites(&mut self) -> Res {
        for site in self.state().building_sites.clone() {
            let served = self.state().workers.iter().any(|&u| {
                self.unit(u)
                    .and_then(|e| e.orders().first())
                    .is_some_and(|o| matches!(o.target(), Some(Target::Entity(t)) if *t == site))
            });
            if !served {
                let repair = Request::repair(TargetRef::Entity(site));
                self.order(4, Sel::Workers, repair, true)?;
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
            .filter(|&u| self.unit(u).is_some_and(|e| e.orders().is_empty()))
            .collect();
        for u in idle {
            let place = self.unit(u).and_then(|e| e.place);
            let Some(deposit) = self.choose(Kind::Deposit, None, place, true)? else {
                return Ok(());
            };
            let Some(id) = target_id(&deposit) else {
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
        let lacking = self.missing_resources(&cost)?;
        let Some(&first_lacking) = lacking.first() else {
            return Ok(());
        };
        let mut gatherers: Vec<Vec<(EntityId, Target)>> = vec![Vec::new(); self.resources().len()];
        for &u in &self.state().workers {
            let Some(e) = self.unit(u) else { continue };
            if let Some(Order {
                kind: OrderKind::Gather { aim, .. },
                ..
            }) = e.orders().first()
                && matches!(e.place, Some(Place::Square(_)))
                && let Some(t) = &aim.target
                && let Ok(rt) = self.deposit_resource_type(t)
                && let Some(g) = gatherers.get_mut(rt as usize)
            {
                g.push((u, t.clone()));
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
        let place = self.unit(u).and_then(|e| e.place);
        let Some(deposit) = self.choose(Kind::DepositOf(to as i64), None, place, true)? else {
            return Ok(());
        };
        let start = square_of(place)?;
        let dest = square_of(self.target_place_of(&deposit))?;
        if self.distance(start, dest, true)? == crate::INF {
            return Ok(());
        }
        let Some(id) = target_id(&deposit) else {
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
        let hp = e.creature().map_or(0, |c| c.hp);
        (e.airground_type == AirGround::Air, e.stats.speed, hp)
    }

    /// `best_explorers`: the mobile units not upgrading, best first.
    fn best_explorers(&self) -> Vec<EntityId> {
        let mut v: Vec<EntityId> = self
            .units()
            .iter()
            .copied()
            .filter(|&u| {
                self.unit(u).is_some_and(|e| {
                    e.stats.speed > 0
                        && e.orders().first().map(Order::keyword) != Some(Keyword::UpgradeTo)
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
            .and_then(|e| e.orders().first())
            .map(Order::keyword)
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
            let first = self.units().first().copied();
            let t = square_target(first.and_then(|f| self.unit(f)).and_then(|e| e.place))?;
            self.give(explorer, Request::go(t), true)?;
        }
        self.give(best, Request::auto_explore(), true)
    }

    /// `_idle_fighters`: soldiers without orders, or only going to a square
    /// without enemies.
    fn idle_fighters(&self) -> Vec<EntityId> {
        self.units()
            .iter()
            .copied()
            .filter(|&u| {
                self.unit(u).is_some_and(|e| {
                    e.class() == EntityClass::Soldier
                        && (e.orders().is_empty()
                            || e.orders().len() == 1
                                && matches!(&e.orders()[0].kind, OrderKind::Go { aim, .. }
                                    if !matches!(&aim.target,
                                        Some(Target::Square(s)) if self.enemy_presence().contains(s))))
                })
            })
            .collect()
    }

    fn friendly_presence(&self, place: Option<Place>) -> bool {
        matches!(place, Some(Place::Square(s)) if self.places_with_friends().contains(&s))
    }

    fn eventually_attack(&mut self, places: Vec<Option<Place>>) -> Res {
        let units = self.idle_fighters();
        if units.is_empty() {
            return Ok(());
        }
        let mut places = places;
        places.sort_by_key(|&pl| self.place_enemy_menace(pl));
        for &place in &places {
            if self.units_should_attack(&units, place)? {
                return self.send_units(units, place);
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
                    .is_some_and(|e| e.time_limit().is_some_and(|t| t != 0) && e.stats.speed != 0)
            })
            .collect();
        if !temp.is_empty() {
            self.send_units(temp, first)?;
        }
        if self.friendly_presence(last) {
            return Ok(());
        }
        let mut enemies = Enemies {
            place: last,
            used: false,
        };
        for (u, a) in self.state().cataclysm_users.clone() {
            let busy = self.unit(u).is_none_or(|e| !e.orders().is_empty());
            if busy || !self.cataclysm_is_efficient(&a, &mut enemies) {
                continue;
            }
            let start = square_of(self.unit(u).and_then(|e| e.place))?;
            let path = self.path_through_walls(start, square_of(last)?)?;
            if let [.., before, _] = path[..]
                && path.len() > 2
            {
                self.give(u, Request::go(TargetRef::Square(before)), false)?;
            }
            let t = square_target(last)?;
            let cataclysm = Request::use_ability(a.clone()).at(t);
            self.give(u, cataclysm, false)?;
            if self.first_order_is_possible(u) {
                let t = square_target(self.unit(u).and_then(|e| e.place))?;
                self.give(u, Request::go(t), false)?;
            }
        }
        Ok(())
    }

    /// Whether the first order of the unit has not failed.
    fn first_order_is_possible(&self, u: EntityId) -> bool {
        self.unit(u)
            .and_then(|e| e.orders().first())
            .is_some_and(|o| !o.is_impossible())
    }

    /// `_cataclysm_is_efficient`: the ability summons something harmful
    /// to one of the enemies.
    fn cataclysm_is_efficient(&mut self, a: &str, enemies: &mut Enemies) -> bool {
        let mut names: BTreeSet<String> = BTreeSet::new();
        if !enemies.used {
            enemies.used = true;
            let known = self.perceived().chain(self.remembered().map(|(r, _)| r));
            for e in known {
                if e.place() == enemies.place && self.is_an_enemy(e) {
                    names.insert(e.type_name().to_owned());
                }
            }
        }
        let Some(effect) = self.rules().get_words(a, "effect") else {
            return false;
        };
        if effect.first().map(String::as_str) == Some("summon") {
            for item in &effect[1..] {
                if self.rules().get_is_truthy(item, "harm_level") {
                    for t in &names {
                        if self.can_harm(item, t) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn units_should_attack(&mut self, units: &[EntityId], place: Option<Place>) -> Res<bool> {
        let m = self.place_enemy_menace(place);
        if !self.is_powerful_enough(units, m) {
            return Ok(false);
        }
        let first = units.first().and_then(|&u| self.unit(u));
        let start = square_of(first.and_then(|e| e.place))?;
        let path = self.path_through_walls(start, square_of(place)?)?;
        match path[..] {
            [] => Ok(false),
            [_] | [_, _] => Ok(true),
            [.., before, _] => self.has_a_safe_path(start, before),
        }
    }

    fn send_units(&mut self, units: Vec<EntityId>, place: Option<Place>) -> Res {
        let Some(place) = place else {
            return Ok(());
        };
        let units: Vec<EntityId> = units
            .into_iter()
            .filter(|&u| self.unit(u).and_then(|e| e.place) != Some(place))
            .collect();
        for &u in &units {
            self.stop(u)?;
        }
        let mut used_teleportation = false;
        for (u, a) in self.state().teleportation_users.clone() {
            let t = square_target(Some(place))?;
            self.give(u, Request::use_ability(a).at(t), true)?;
            if self.first_order_is_possible(u) {
                used_teleportation = true;
            }
        }
        let mut enemies = Enemies {
            place: Some(place),
            used: false,
        };
        for u in units {
            let start = square_of(self.unit(u).and_then(|e| e.place))?;
            let path = self.path_through_walls(start, square_of(Some(place))?)?;
            if !used_teleportation
                && let [.., before, _] = path[..]
                && path.len() > 2
            {
                self.give(u, Request::go(TargetRef::Square(before)), false)?;
                if !self.friendly_presence(Some(place)) {
                    for (u_, a) in self.state().cataclysm_users.clone() {
                        if u_ == u && self.cataclysm_is_efficient(&a, &mut enemies) {
                            let t = square_target(Some(place))?;
                            let cataclysm = Request::use_ability(a).at(t);
                            self.give(u, cataclysm, false)?;
                        }
                    }
                }
            }
            for (u_, a) in self.state().summon_users.clone() {
                if u_ == u {
                    let t = square_target(Some(place))?;
                    self.give(u, Request::use_ability(a).at(t), false)?;
                }
            }
            for (u_, a) in self.state().detector_users.clone() {
                if u_ == u {
                    let t = square_target(Some(place))?;
                    self.give(u, Request::use_ability(a).at(t), false)?;
                }
            }
            let t = square_target(Some(place))?;
            self.give(u, Request::go(t), false)?;
        }
        Ok(())
    }

    /// `_builders_place`: where most workers are; on a tie, the square with
    /// the greatest number, before a place inside something.
    fn builders_place(&self) -> Res<Option<Place>> {
        let mut starts: Vec<(Option<Place>, i64)> = Vec::new();
        for &u in &self.state().workers {
            let place = self.unit(u).and_then(|e| e.place);
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
        let place = self.builders_place()?;
        self.send_units(fighters, place)?;
        // build static defenses
        let gate = self.rules().roles.get(Role::Gate);
        if let (Some(b), Some(gate)) = (self.state().sensible_building, gate) {
            let gate_cost = self.rules().ty(gate).cost.clone();
            let Some(e) = self.unit(b) else {
                return Ok(());
            };
            let s = square_of(e.place)?;
            let (bx, by) = (e.x, e.y);
            let mut exits = self.exits(s).to_vec();
            exits.sort_by_key(|&x| {
                let (ex, ey) = self.exit_xy(x);
                square_of_distance(bx, by, ex, ey)
            });
            if let Some(&exit) = exits.first()
                && !self.exit_is_blocked(exit)
                && self.gather(&gate_cost, 0, 0)?
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
            if !e.orders().is_empty() {
                continue;
            }
            let researchable = e
                .type_id()
                .map(|t| self.rules().ty(t).can_research.clone())
                .unwrap_or_default();
            for t in researchable {
                let cost = self
                    .rules()
                    .unit_type(&t)
                    .map(|c| c.cost.clone())
                    .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'cost'"))?;
                if self.future_nb(&Sel::List(vec![t.clone()])) == 0
                    && self.missing_resources(&cost)?.is_empty()
                    && self.potential(&cost)? > 3
                {
                    self.give(u, Request::research(t), true)?;
                }
            }
        }
        Ok(())
    }

    fn raise_dead(&mut self) -> Res {
        for (u, a) in self.state().raise_dead_users.clone() {
            if let Some(Place::Square(s)) = self.unit(u).and_then(|e| e.place)
                && self.places_with_corpses().contains(&s)
            {
                let raise = Request::use_ability(a).at(TargetRef::Square(s));
                self.give(u, raise, true)?;
            }
        }
        Ok(())
    }

    // Computer: warehouses

    fn best_warehouse(&self) -> Option<(String, Vec<i64>)> {
        let name = self.rules().role_name(self.faction(), Role::TownHall);
        self.rules()
            .unit_type(&name)
            .map(|t| (t.name.clone(), t.cost.clone()))
    }

    fn build_a_warehouse_if_useful(&mut self) -> Res {
        let Some((_, cost)) = self.best_warehouse() else {
            return Ok(());
        };
        if !self.missing_resources(&cost)?.is_empty() {
            return Ok(());
        }
        let mut deposits = Vec::new();
        for &u in &self.state().workers {
            let Some(e) = self.unit(u) else { continue };
            for o in e.orders() {
                if let OrderKind::Gather { aim, .. } = &o.kind
                    && let Some(t) = &aim.target
                    && self.target_place(t) != TPlace::Nowhere
                {
                    deposits.push(t.clone());
                }
            }
        }
        for d in deposits {
            self.build_a_warehouse_for(&d)?;
        }
        Ok(())
    }

    fn deposit_resource_type(&self, d: &Target) -> Res<i64> {
        self.target_type(d)
            .and_then(|t| self.rules().ty(t).resource_type)
            .ok_or_else(|| Abort::new("object has no attribute 'resource_type'"))
    }

    fn build_a_warehouse_for(&mut self, deposit: &Target) -> Res {
        let dplace = self.target_place_of(deposit);
        let nearby: Vec<EntityId> = self
            .state()
            .workers
            .iter()
            .copied()
            .filter(|&v| {
                self.unit(v).is_some_and(|e| {
                    e.place == dplace
                        || e.orders().first().is_some_and(|o| {
                            matches!(&o.kind, OrderKind::Gather { aim, .. } if aim
                                .target
                                .as_ref()
                                .is_none_or(|t| self.target_place_of(t) == dplace && dplace.is_some()))
                        })
                })
            })
            .collect();
        if nearby.is_empty() {
            return Ok(());
        }
        let rt = self.deposit_resource_type(deposit)?;
        let Some(dp) = dplace else {
            return Err(Abort::new(
                "'NoneType' object has no attribute 'shortest_path_distance_to'",
            ));
        };
        let wh = self.nearest_warehouse(dp, rt, true)?;
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
        let far = match wh.map(Seen::place) {
            None => true,
            Some(place) => {
                let start = square_of(Some(dp))?;
                let dest = square_of(place)?;
                self.distance(start, dest, true)? > self.square_width()
            }
        };
        if far && let Some(meadow) = self.choose(Kind::Meadow, None, Some(dp), false)? {
            let (name, _) = self
                .best_warehouse()
                .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'type_name'"))?;
            let Some(id) = target_id(&meadow) else {
                return Ok(());
            };
            for v in nearby {
                let build = Request::build(name.clone(), TargetRef::Entity(id));
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

    fn choose_is_ok(&self, o: &Target, resource_type: Option<i64>) -> bool {
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
    ) -> Res<Option<Target>> {
        let Some(first_place) = self
            .units()
            .first()
            .map(|&u| self.unit(u).and_then(|e| e.place))
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
        let mut candidates: Vec<(EntityId, Target)> = self
            .perceived()
            .filter(|&e| kind.matches(rules, e))
            .map(|e| (e.id(), Target::Entity(e.id())))
            .collect();
        candidates.extend(
            self.remembered()
                .filter(|&(e, _)| kind.matches(rules, e))
                .map(|(e, t)| (e.id(), t)),
        );
        candidates.retain(|(_, o)| self.choose_is_ok(o, resource_type));
        candidates.sort_by_key(|c| c.0);
        let mut candidates: Vec<Target> = candidates.into_iter().map(|(_, o)| o).collect();
        if candidates.len() > 10 {
            candidates = self.remove_far_candidates(&candidates, start, 10)?;
        } else if !candidates.is_empty() {
            let start = square_of(start)?;
            let mut keyed = Vec::new();
            for o in candidates {
                let dest = square_of(self.target_place_of(&o))?;
                let d = self.distance(start, dest, true)?;
                keyed.push((d, o));
            }
            keyed.sort_by_key(|(d, _)| *d);
            candidates = keyed.into_iter().map(|(_, o)| o).collect();
        }
        if random && let Some(c0) = candidates.first() {
            let p0 = self.target_place_of(c0);
            let mut keyed = Vec::new();
            for o in candidates
                .into_iter()
                .filter(|o| self.target_place_of(o) == p0)
            {
                keyed.push((self.deposit_priority(&Some(o.clone()))?, o));
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
        candidates: &[Target],
        start: Option<Place>,
        limit: usize,
    ) -> Res<Vec<Target>> {
        let start = square_of(start)?;
        let mut on: BTreeMap<SquareId, BTreeMap<EntityId, Target>> = BTreeMap::new();
        for o in candidates {
            if let (Some(Place::Square(s)), Some(id)) = (self.target_place_of(o), target_id(o)) {
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
            for &e in self.exits(room) {
                let beyond = self
                    .beyond_exit(e)
                    .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'place'"))?;
                let next = square_of(beyond)?;
                if !done.contains(&next) {
                    queue.push_back(next);
                }
            }
            done.push(room);
        }
        Ok(c)
    }

    fn deposit_priority(&self, d: &Option<Target>) -> Res<DepositPriority> {
        let Some(t) = d else {
            return Ok((-100, 0, None));
        };
        let workers = self
            .state()
            .gathered_deposits
            .iter()
            .find(|(k, _)| same_opt(k, d))
            .map_or(0, |(_, n)| *n);
        let rt = self.deposit_resource_type(t)?;
        let r = usize::try_from(rt)
            .ok()
            .and_then(|i| self.resources().get(i))
            .copied()
            .ok_or_else(|| Abort::new("list index out of range"))?;
        Ok((floordiv(-r, 10), -workers, target_id(t)))
    }

    // Computer: counting and getting units

    fn entity_matches(e: &Entity, sel: &Sel) -> bool {
        match sel {
            Sel::Workers => e.class() == EntityClass::Worker,
            _ => sel.matches_type(&e.type_name),
        }
    }

    fn nb(&self, sel: &Sel) -> i64 {
        if let Sel::List(l) = sel
            && let Some(first) = l.first()
            && self
                .rules()
                .type_id(first)
                .is_some_and(|t| self.has_upgrade(t))
        {
            return 1;
        }
        self.units()
            .iter()
            .filter_map(|&u| self.unit(u))
            .filter(|e| Self::entity_matches(e, sel))
            .count() as i64
    }

    fn nb_in_production(&self, sel: &Sel) -> i64 {
        let rules = self.rules();
        let mut n = 0;
        for e in self.units().iter().filter_map(|&u| self.unit(u)) {
            if let Some(site) = e.site()
                && sel.matches_type(&rules.ty(site.building_type).name)
            {
                n += 1;
                continue;
            }
            for o in e.orders() {
                let made = match &o.kind {
                    OrderKind::Production(p) => Some(p.product.type_id()),
                    OrderKind::Build { building, .. } => Some(*building),
                    OrderKind::Go { .. }
                    | OrderKind::Attack { .. }
                    | OrderKind::Patrol { .. }
                    | OrderKind::Gather { .. }
                    | OrderKind::AutoAttack
                    | OrderKind::AutoExplore
                    | OrderKind::Wait { .. }
                    | OrderKind::Block { .. }
                    | OrderKind::Repair { .. }
                    | OrderKind::BuildPhaseTwo { .. }
                    | OrderKind::Use { .. }
                    | OrderKind::Load { .. }
                    | OrderKind::LoadAll { .. }
                    | OrderKind::UnloadAll { .. } => None,
                };
                if made.is_some_and(|t| sel.matches_type(&rules.ty(t).name)) {
                    n += 1;
                }
            }
        }
        n
    }

    fn future_nb(&self, sel: &Sel) -> i64 {
        self.nb(sel) + self.nb_in_production(sel)
    }

    fn missing_resources(&self, cost: &[i64]) -> Res<Vec<usize>> {
        let r = self.resources();
        let mut result = Vec::new();
        for (i, &c) in cost.iter().enumerate() {
            let have = r
                .get(i)
                .copied()
                .ok_or_else(|| Abort::new("list index out of range"))?;
            if c > have {
                result.push(i);
            }
        }
        Ok(result)
    }

    fn potential(&self, cost: &[i64]) -> Res<i64> {
        let mut result = 9999;
        for (i, &res) in self.resources().iter().enumerate() {
            let c = cost
                .get(i)
                .copied()
                .ok_or_else(|| Abort::new("tuple index out of range"))?;
            if c != 0 {
                result = result.min(floordiv(res, c));
            }
        }
        Ok(result)
    }

    /// `get`: works toward having `nb` units of the type; true if they are
    /// there.
    fn get(&mut self, nb: i64, type_name: &str) -> Res<bool> {
        self.state_mut().safe_cnt = 0;
        self.get_types(nb, vec![type_name.to_owned()], 0)
    }

    /// `_get`. `depth` counts the planning calls this one is inside.
    fn get_types(&mut self, nb: i64, types: Vec<String>, depth: usize) -> Res<bool> {
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
        for t in &types {
            if self.rules().unit_type(t).is_none() {
                continue;
            }
            let makers = self.rules().get_makers(t);
            if self.nb(&Sel::List(makers.clone())) > 0 {
                let n = nb - self.future_nb(&sel);
                self.build_or_train_or_upgradeto_or_summon(t, n, depth)?;
                break;
            } else if let Some(m) = makers.first() {
                self.get_types(1, vec![m.clone()], depth)?;
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// `gather`: true if the resources and the food are there; asks for a
    /// farm when food is missing.
    fn gather(&mut self, cost: &[i64], food: i64, depth: usize) -> Res<bool> {
        let depth = deeper(depth)?;
        if !self.missing_resources(cost)?.is_empty() {
            self.state_mut()
                .lacking
                .get_or_insert_with(|| cost.to_vec());
            return Ok(false);
        }
        let limit = self.food_limit();
        let available = self.available_food();
        if food != 0 && food > available - self.used_food() {
            if available < limit {
                let name = self.rules().role_name(self.faction(), Role::Farm);
                let t = Sel::One(name.clone());
                if self.future_nb(&t) == self.nb(&t) {
                    self.build_or_train_or_upgradeto_or_summon(&name, 1, depth)?;
                }
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn get_requirements(&mut self, requirements: &[String], depth: usize) -> Res<bool> {
        let depth = deeper(depth)?;
        for r in requirements {
            if !self.has(r) {
                return self.get_types(1, vec![r.clone()], depth);
            }
        }
        Ok(true)
    }

    fn build_or_train_or_upgradeto_or_summon(
        &mut self,
        type_name: &str,
        nb: i64,
        depth: usize,
    ) -> Res {
        let depth = deeper(depth)?;
        let t = self
            .rules()
            .unit_type(type_name)
            .cloned()
            .ok_or_else(|| Abort::new("'NoneType' object has no attribute '__name__'"))?;
        let makers = self.rules().get_makers(type_name);
        if !self.get_types(1, makers.clone(), depth)?
            || !self.get_requirements(&t.requirements, depth)?
        {
            return Ok(());
        }
        let Some(mut maker) = makers.last().cloned() else {
            return Ok(());
        };
        for m in &makers {
            if self.nb(&Sel::One(m.clone())) != 0 {
                maker = m.clone();
                break;
            }
        }
        let Some(mc) = self.rules().unit_type(&maker).cloned() else {
            return Ok(());
        };
        let maker_sel = Sel::One(maker.clone());
        let name = type_name.to_owned();
        if mc.can_upgrade_to.contains(&name) {
            if self.nb(&maker_sel) >= nb {
                let cost: Vec<i64> = t
                    .cost
                    .iter()
                    .enumerate()
                    .map(|(i, c)| c - mc.cost.get(i).copied().unwrap_or(0))
                    .collect();
                if self.gather(&cost, t.stats.food_cost - mc.stats.food_cost, depth)? {
                    self.order(nb, maker_sel, Request::upgrade_to(name), false)?;
                }
            } else {
                self.get_types(nb, vec![maker], depth)?;
            }
        } else if mc.can_build.contains(&name) {
            if !self.gather(&t.cost, t.stats.food_cost, depth)? {
                return Ok(());
            }
            let meadow = if let Some(&rt) = t.storable_resource_types.first() {
                let meadow = self.choose(Kind::Meadow, Some(rt), None, false)?;
                let far = match &meadow {
                    None => true,
                    Some(m) => {
                        let start = square_of(self.target_place_of(m))?;
                        let bp = self.builders_place()?;
                        let dest = square_of(bp)?;
                        self.distance_through_walls(start, dest)? > self.square_width() * 3
                    }
                };
                if far {
                    if self.nb(&Sel::One(name.clone())) != 0 {
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
            if let Some(id) = meadow.as_ref().and_then(target_id) {
                let build = Request::build(name, TargetRef::Entity(id));
                self.order(4, maker_sel, build, true)?;
            }
        } else if mc.can_train.contains(&name) {
            let nb_maker = self.nb(&maker_sel);
            if self.nb(&Sel::Workers) != 0
                && nb > nb_maker * 3
                && self.potential(&t.cost)? > nb_maker * 100
            {
                // additional production sites
                self.build_or_train_or_upgradeto_or_summon(&maker, 1, depth)?;
            }
            if self.gather(&t.cost, t.stats.food_cost, depth)? {
                self.order(nb, maker_sel, Request::train(name), false)?;
            }
        } else if mc.can_research.contains(&name) {
            if self.gather(&t.cost, t.stats.food_cost, depth)? {
                self.order(1, maker_sel, Request::research(name), false)?;
            }
        } else {
            for ability in &mc.can_use {
                let Some(effect) = self.rules().get_words(ability, "effect") else {
                    continue;
                };
                if effect.first().map(String::as_str) == Some("summon") && effect.contains(&name) {
                    let mut order = Request::use_ability(ability.clone());
                    if self.rules().get_words(ability, "effect_target").as_deref()
                        == Some(&["ask".to_owned()][..])
                    {
                        let first = self.units().first().copied();
                        let first = first.ok_or_else(|| Abort::new("list index out of range"))?;
                        order = order.at(TargetRef::Entity(first));
                    }
                    self.order(1, maker_sel, order, false)?;
                    break;
                }
            }
        }
        Ok(())
    }

    fn worker_orders_priority(&self, u: EntityId) -> Res<(u8, Option<DepositPriority>)> {
        Ok(match self.unit(u).and_then(|e| e.orders().first()) {
            None => (0, None),
            Some(Order {
                kind: OrderKind::Gather { aim, .. },
                ..
            }) => (1, Some(self.deposit_priority(&aim.target)?)),
            Some(_) => (2, None),
        })
    }

    /// Whether a followed order is still in progress.
    fn tracked_is_alive(&self, t: &mut Tracked) -> bool {
        let Some(unit) = self.unit(t.unit) else {
            return false;
        };
        if let Some(live) = unit.orders().iter().find(|o| o.uid == t.order.uid) {
            t.order = live.clone();
            return !live.is_complete() && unit.place.is_some();
        }
        !t.order.is_complete()
            && unit.place.is_some()
            && unit.orders().iter().any(|o| self.order_eq(o, &t.order))
    }

    /// `order`: gives the order to units of a type until `nb` of them
    /// follow it. With `requisition`, busy workers may be taken, the least
    /// useful first.
    fn order(&mut self, nb: i64, sel: Sel, order: Request, requisition: bool) -> Res {
        let key = (sel.clone(), self.words(&order));
        let mut tracked = self.state_mut().orders.remove(&key).unwrap_or_default();
        tracked.retain_mut(|t| self.tracked_is_alive(t));
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
                    keyed.push((self.worker_orders_priority(u)?, u));
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
                .and_then(|e| e.orders().first())
                .map(|o| (o.keyword(), o.target().cloned()));
            if requisition || first.is_none() {
                if let Some((kw, target)) = &first {
                    if matches!(kw, Keyword::Build | Keyword::Repair) {
                        continue;
                    }
                    if requisition && *kw == Keyword::Gather {
                        let gd = &mut self.state_mut().gathered_deposits;
                        match gd.iter_mut().find(|(k, _)| same_opt(k, target)) {
                            Some(e) => e.1 -= 1,
                            None => return Err(Abort::new("KeyError: the deposit")),
                        }
                    }
                }
                self.give(u, order.clone(), true)?;
                if let Some(o) = self.unit(u).and_then(|e| e.orders().first())
                    && o.keyword() == order.keyword()
                {
                    let t = Tracked {
                        unit: u,
                        order: o.clone(),
                    };
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

/// The id of the entity a target of the plan names.
fn target_id(t: &Target) -> Option<EntityId> {
    t.entity_id()
}

fn add_count(counts: &mut Vec<(Option<Target>, i64)>, key: &Option<Target>, n: i64) {
    match counts.iter_mut().find(|(k, _)| same_opt(k, key)) {
        Some(e) => e.1 += n,
        None => counts.push((key.clone(), n)),
    }
}

/// `re.match("^[0-9]+$", s)`.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `re.match("^[+-][0-9]+$", s)`.
fn is_signed_number(s: &str) -> bool {
    matches!(s.as_bytes().first(), Some(b'+' | b'-')) && is_digits(&s[1..])
}

#[cfg(test)]
mod fairness;

#[cfg(test)]
mod order_sites;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::PlayerId;
    use crate::world::World;

    /// A world with the original's rules and this map.
    pub(super) fn world(map: &str) -> World {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
        let read = |f: &str| {
            let bytes = std::fs::read(dir.join(f))
                .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let rules = crate::Rules::load(&[read("res/rules.txt")]);
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
            if let Some(Order {
                kind: OrderKind::Gather { aim, .. },
                ..
            }) = door.unit(u).unwrap().orders().first()
                && let Some(t) = &aim.target
            {
                n[door.deposit_resource_type(t).unwrap() as usize] += 1;
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
        let farm = Sel::One("farm".to_owned());
        let door = w.door(p);
        assert_eq!((door.nb(&farm), door.future_nb(&farm)), (1, 2));
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
        let meadows: Vec<Target> = w
            .square(b2)
            .objects
            .iter()
            .filter(|&&o| w.e(o).is_meadow())
            .map(|&o| Target::Entity(o))
            .collect();
        assert_eq!(meadows.len(), 3);
        for m in meadows.iter().filter_map(Target::entity_id) {
            w.plm(p).perception.insert(m);
        }
        let a1 = w.square_by_name("a1").unwrap();
        let near = w
            .door(p)
            .remove_far_candidates(&meadows, Some(Place::Square(a1)), 10)
            .unwrap();
        let ids: Vec<EntityId> = near.iter().filter_map(Target::entity_id).collect();
        let expected: Vec<EntityId> = meadows.iter().filter_map(Target::entity_id).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn computer2_chooses_from_the_original_list_of_orders() {
        let names: Vec<&str> = Door::computer2_orders()
            .iter()
            .map(|k| k.as_str())
            .collect();
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
