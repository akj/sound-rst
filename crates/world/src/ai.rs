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
//! The AI reads and changes the world as the original's methods on the
//! player do, in the same order, so that it draws the same random numbers.
//! Where the original raises an exception, the work stops with an
//! [`Abort`], which ends the player's update as in the original. Where the
//! original would use the id of an interior (`None`), the AI stops too.

use std::collections::{BTreeMap, BTreeSet};

use crate::entity::{Entity, EntityId, Place, PlayerId, SquareId, Target};
use crate::nofloat::{PRECISION, floordiv, py_int, square_of_distance};
use crate::orders::{Keyword, Order};
use crate::pathfinding::Viewer;
use crate::player::PlayerKind;
use crate::rules::{AirGround, EntityClass};
use crate::world::{Abort, Res, TPlace, World};

/// The number of workers a scripted computer keeps getting.
const NB_WORKERS_TO_GET: i64 = 10;

/// What `check_type` compares a unit or a type with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Meadow,
    Deposit,
}

impl Kind {
    fn matches(self, e: &Entity) -> bool {
        match self {
            Kind::Meadow => e.class == EntityClass::Meadow,
            Kind::Deposit => e.class == EntityClass::Deposit,
        }
    }
}

/// An order given by `order`, followed until it is done.
#[derive(Clone, Debug)]
struct Tracked {
    unit: EntityId,
    /// The order as last seen in the unit's queue.
    order: Order,
}

/// The comparison key of `_deposit_priority`: the id is text, and the
/// missing deposit has 0 instead.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IdKey {
    Zero,
    Text(String),
}

type DepositPriority = (i64, i64, IdKey);

/// The state of a scripted computer player (`Computer`).
#[derive(Clone, Debug, Default)]
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

impl World {
    // state

    fn cs(&self, p: PlayerId) -> &Computer {
        self.pl(p)
            .ai
            .as_deref()
            .expect("a scripted computer has its state")
    }

    fn csm(&mut self, p: PlayerId) -> &mut Computer {
        self.plm(p)
            .ai
            .as_deref_mut()
            .expect("a scripted computer has its state")
    }

    /// The state of a new scripted computer, with its AI type set.
    pub(crate) fn new_computer(&mut self, p: PlayerId) -> Res {
        self.plm(p).ai = Some(Box::default());
        let t = self.pl(p).ai_type().unwrap_or("timers").to_owned();
        self.computer_set_ai(p, &t)
    }

    /// `set_ai`, after the AI type of the player changed: loads the script
    /// and resets the script's settings.
    pub(crate) fn computer_set_ai(&mut self, p: PlayerId, ai_type: &str) -> Res {
        if ai_type == "timers" || self.pl(p).ai.is_none() {
            return Ok(());
        }
        let name = self.faction_ai_type(p, ai_type);
        let Some(plan) = self.ai_scripts.get(&name).map(<[String]>::to_vec) else {
            return Err(Abort::new(format!("KeyError: '{name}'")));
        };
        let time = self.time;
        let c = self.csm(p);
        c.plan = Some(plan);
        c.line_nb = 0;
        c.previous_linechange = time;
        c.watchdog = 0;
        c.constant_attacks = 0;
        c.research = 0;
        self.update_effect_users_and_workers(p)
    }

    fn set_line_nb(&mut self, p: PlayerId, n: i64) {
        let time = self.time;
        let c = self.csm(p);
        c.line_nb = n;
        c.previous_linechange = time;
    }

    // hooks

    /// `play`: what a computer player does during its update.
    pub(crate) fn ai_play(&mut self, p: PlayerId) -> Res {
        match self.pl(p).kind.clone() {
            PlayerKind::Human => Ok(()),
            PlayerKind::Computer2 => self.computer2_play(p),
            PlayerKind::Computer { ai_type } => {
                if ai_type == "timers" || self.pl(p).ai.is_none() {
                    return Ok(());
                }
                self.computer_play(p)
            }
        }
    }

    /// `Computer.on_unit_attacked`: remembers where to retaliate.
    pub(crate) fn computer_on_unit_attacked(
        &mut self,
        p: PlayerId,
        unit: EntityId,
        attacker: EntityId,
    ) {
        if self.pl(p).ai.is_none() {
            return;
        }
        let a = self.e(attacker);
        if a.player.is_some_and(|ap| self.pl(p).allied.contains(&ap)) || !a.is_vulnerable() {
            return;
        }
        let u = self.e(unit);
        if u.orders.first().map(|o| o.keyword) == Some(Keyword::AutoExplore) {
            return;
        }
        if u.stats.is_a_building != 0 {
            self.csm(p).sensible_building = Some(unit);
        }
        let place = if self.pl(p).perception.contains(&attacker) {
            self.e(attacker).place
        } else {
            self.e(unit).place
        };
        let c = self.csm(p);
        if !c.attacked_places.contains(&place) {
            c.attacked_places.push(place);
        }
    }

    /// `Computer.time_has_come` without the cataclysm check: whether the
    /// units waiting on a square are strong enough.
    pub(crate) fn computer_time_has_come(&self, p: PlayerId, s: SquareId) -> bool {
        let Some(c) = self.pl(p).ai.as_deref() else {
            return false;
        };
        let key = Some(Target::Square(s));
        c.time_has_come
            .iter()
            .find(|(t, _)| same_opt(t, &key))
            .is_some_and(|(_, v)| *v)
    }

    // Computer2

    fn computer2_play(&mut self, p: PlayerId) -> Res {
        let n = self.pl(p).resources.len();
        let pl = self.plm(p);
        pl.cheat_has_all = true;
        pl.resources = vec![1000 * PRECISION; n];
        let mut targets: Vec<(EntityId, Target)> = Vec::new();
        for &o in &self.pl(p).perception {
            targets.push((o, Target::Entity(o)));
        }
        for r in self.pl(p).memory.values() {
            targets.push((
                r.snapshot.id,
                Target::Memory(crate::entity::MemoryRef {
                    owner: p,
                    snapshot: r.snapshot.clone(),
                }),
            ));
        }
        targets.retain(|(_, t)| !self.t_entity(t).is_some_and(Entity::is_exit));
        targets.sort_by(|a, b| a.0.cmp_as_text(b.0));
        let ids: Vec<EntityId> = targets.into_iter().map(|(i, _)| i).collect();
        let mut i = 0;
        while i < self.pl(p).units.len() {
            let u = self.pl(p).units[i];
            if self.e(u).orders.is_empty()
                && let Some(order) = self.random_order(u, &ids)?
            {
                self.take_order(u, order, true, false, None)?;
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

    fn random_order(&mut self, unit: EntityId, targets: &[EntityId]) -> Res<Option<Vec<String>>> {
        let orders = Self::computer2_orders();
        let i = self
            .random
            .choice_index(orders.len())
            .expect("the order list is not empty");
        let kw = orders[i];
        // `UseOrder.nb_args` is a property, true when read on the class.
        let args = kw == Keyword::Use || kw.nb_args() > 0;
        let menu = self.order_menu(unit, kw, true);
        if menu.is_empty() {
            return Ok(None);
        }
        let i = self.random.choice_index(menu.len()).expect("not empty");
        let mut order: Vec<String> = menu[i].split_whitespace().map(str::to_owned).collect();
        if args {
            let Some(j) = self.random.choice_index(targets.len()) else {
                return Err(Abort::new("Cannot choose from an empty sequence"));
            };
            order.push(targets[j].to_string());
        }
        Ok(Some(order))
    }

    // Computer: the turn

    fn computer_play(&mut self, p: PlayerId) -> Res {
        if !self.should_play_this_turn(p)? {
            return Ok(());
        }
        self.update_effect_users_and_workers(p)?;
        self.update_time_has_come(p);
        self.send_workers_to_forgotten_building_sites(p)?;
        self.idle_workers_gather(p)?;
        self.send_explorer(p)?;
        if !self.cs(p).attacked_places.is_empty() {
            let places = self.cs(p).attacked_places.clone();
            self.eventually_attack(p, places)?;
            self.csm(p).attacked_places.clear();
        } else if self.cs(p).constant_attacks != 0 {
            let places = self
                .pl(p)
                .enemy_presence
                .iter()
                .map(|&s| Some(Place::Square(s)))
                .collect();
            self.eventually_attack(p, places)?;
        } else {
            self.defensive_routine(p)?;
        }
        if self.cs(p).research != 0 {
            self.idle_buildings_research(p)?;
        }
        self.raise_dead(p)?;
        self.build_a_warehouse_if_useful(p)?;
        let peasant = self.equivalent(p, "peasant");
        self.ai_get(p, NB_WORKERS_TO_GET, &peasant)?;
        self.follow_plan(p)
    }

    /// The cpu intensive players play in turns, one tenth of them each turn.
    fn should_play_this_turn(&self, p: PlayerId) -> Res<bool> {
        let players = self.cpu_intensive_players();
        let i = players
            .iter()
            .position(|&x| x == p)
            .ok_or_else(|| Abort::new("x not in list"))?;
        let turn = (i * 10 / players.len()) as i64;
        Ok(self.turn().rem_euclid(10) == turn)
    }

    fn equivalent(&self, p: PlayerId, type_name: &str) -> String {
        self.rules.equivalent_type(type_name, &self.pl(p).faction)
    }

    fn follow_plan(&mut self, p: PlayerId) -> Res {
        let Some(plan) = self.cs(p).plan.clone() else {
            return Ok(());
        };
        if plan.is_empty() {
            return Ok(());
        }
        let c = self.cs(p);
        if c.watchdog != 0 && self.time > c.previous_linechange + c.watchdog * 1000 {
            let n = c.line_nb + 1;
            self.set_line_nb(p, n);
        }
        let n = self.cs(p).line_nb.rem_euclid(plan.len() as i64);
        self.set_line_nb(p, n);
        let line_nb = self.cs(p).line_nb;
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
                self.set_line_nb(p, n);
            }
            "label" => {
                self.set_line_nb(p, line_nb + 1);
                arg(1)?;
            }
            "goto_random" => {
                let choices = &cmd[1..];
                let Some(i) = self.random.choice_index(choices.len()) else {
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
                self.set_line_nb(p, n);
            }
            "attack" => {
                self.csm(p).constant_attacks = 1;
                self.set_line_nb(p, line_nb + 1);
            }
            "watchdog" | "constant_attacks" | "research" => {
                let v = py_int(arg(1)?).map_err(|_| Abort::new("invalid literal for int()"))?;
                let c = self.csm(p);
                match first {
                    "watchdog" => c.watchdog = v,
                    "constant_attacks" => c.constant_attacks = v,
                    _ => c.research = v,
                }
                self.set_line_nb(p, line_nb + 1);
            }
            "get" => {
                let mut n = 1;
                let mut done = true;
                for &w in &cmd[1..] {
                    if is_digits(w) {
                        n = py_int(w).map_err(|_| Abort::new("invalid literal for int()"))?;
                    } else if self.rules.has_class(w) {
                        let t = self.equivalent(p, w);
                        if !self.ai_get(p, n, &t)? {
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
                    let line_nb = self.cs(p).line_nb;
                    self.set_line_nb(p, line_nb + 1);
                }
            }
            other => {
                self.warn(format!("unknown command: '{other}' (in ai.txt)"));
                self.set_line_nb(p, line_nb + 1);
            }
        }
        Ok(())
    }

    // Computer: units and their tasks

    pub(crate) fn update_effect_users_and_workers(&mut self, p: PlayerId) -> Res {
        let mut workers = Vec::new();
        let mut gathered: Vec<(Option<Target>, i64)> = Vec::new();
        let mut sites = Vec::new();
        let mut raise_dead = Vec::new();
        let mut teleportation = Vec::new();
        let mut cataclysm = Vec::new();
        let mut detector = Vec::new();
        let mut summon = Vec::new();
        for u in self.pl(p).units.clone() {
            let e = self.e(u);
            match e.class {
                EntityClass::Worker => {
                    workers.push(u);
                    if let Some(o) = e.orders.first()
                        && o.keyword == Keyword::Gather
                    {
                        add_count(&mut gathered, &o.target, 1);
                    }
                }
                EntityClass::BuildingSite => sites.push(u),
                _ => {}
            }
            for a in self.can_use(u) {
                if !self.is_order_allowed(u, Keyword::Use, std::slice::from_ref(&a))? {
                    continue;
                }
                let Some(effect) = self.rules.get_words(&a, "effect") else {
                    continue;
                };
                match effect.first().map(String::as_str) {
                    None => continue,
                    Some("raise_dead") => raise_dead.push((u, a.clone())),
                    Some("teleportation") => teleportation.push((u, a.clone())),
                    Some("summon") => {
                        for item in &effect[1..] {
                            if self.rules.get_is_truthy(item, "harm_level") {
                                cataclysm.push((u, a.clone()));
                            }
                            if self.rules.get_is_truthy(item, "is_a_detector") {
                                detector.push((u, a.clone()));
                            }
                            if self.rules.get_is_truthy(item, "damage") {
                                summon.push((u, a.clone()));
                            }
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        let c = self.csm(p);
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

    fn update_time_has_come(&mut self, p: PlayerId) {
        // (target, menace, units), in the order the targets first appear
        let mut waiting: Vec<(Option<Target>, i64, Vec<EntityId>)> = Vec::new();
        for &u in &self.pl(p).units {
            let e = self.e(u);
            if let Some(o) = e.orders.first()
                && o.keyword == Keyword::Wait
            {
                match waiting.iter_mut().find(|(t, _, _)| same_opt(t, &o.target)) {
                    Some(w) => {
                        w.1 += e.menace();
                        w.2.push(u);
                    }
                    None => waiting.push((o.target.clone(), e.menace(), vec![u])),
                }
            }
        }
        let thc: Vec<(Option<Target>, bool)> = waiting
            .iter()
            .map(|(t, _, us)| {
                let m = self.target_enemy_menace(p, t.as_ref());
                (t.clone(), self.is_powerful_enough(p, us, m))
            })
            .collect();
        self.csm(p).time_has_come = thc;
        let units = self.pl(p).units.clone();
        let mut cancel: BTreeSet<EntityId> = BTreeSet::new();
        for (place, _, _) in &waiting {
            let m = self.target_enemy_menace(p, place.as_ref());
            if !self.is_powerful_enough(p, &units, m) {
                for &u in &units {
                    let hit = self
                        .e(u)
                        .orders
                        .iter()
                        .any(|o| o.keyword == Keyword::Wait && same_opt(&o.target, place));
                    if hit {
                        cancel.insert(u);
                    }
                }
            }
        }
        // The original walks a set of units: the order does not change the
        // result, so the id order is used.
        for u in cancel {
            self.cancel_all_orders(u, true);
        }
    }

    fn target_enemy_menace(&self, p: PlayerId, t: Option<&Target>) -> i64 {
        match t {
            Some(Target::Square(s)) => self.enemy_menace(p, *s),
            _ => 0,
        }
    }

    fn place_enemy_menace(&self, p: PlayerId, place: Option<Place>) -> i64 {
        match place {
            Some(Place::Square(s)) => self.enemy_menace(p, s),
            _ => 0,
        }
    }

    fn is_powerful_enough(&self, p: PlayerId, units: &[EntityId], enemy_menace: i64) -> bool {
        // sometimes the food limit prevents units with more than 1 food cost
        let ratio = if self.pl(p).used_food < self.food_limit() - 5 {
            180
        } else {
            100
        };
        let menace: i64 = units
            .iter()
            .map(|&u| self.e(u))
            .filter(|e| e.stats.speed > 0 && e.class == EntityClass::Soldier)
            .map(Entity::menace)
            .sum();
        menace > floordiv(enemy_menace * ratio, 100)
    }

    fn send_workers_to_forgotten_building_sites(&mut self, p: PlayerId) -> Res {
        for site in self.cs(p).building_sites.clone() {
            let served = self.cs(p).workers.iter().any(|&u| {
                self.e(u)
                    .orders
                    .first()
                    .is_some_and(|o| matches!(o.target, Some(Target::Entity(t)) if t == site))
            });
            if !served {
                self.order(
                    p,
                    4,
                    Sel::Workers,
                    vec!["repair".into(), site.to_string()],
                    true,
                )?;
                break;
            }
        }
        Ok(())
    }

    fn idle_workers_gather(&mut self, p: PlayerId) -> Res {
        let idle: Vec<EntityId> = self
            .cs(p)
            .workers
            .iter()
            .copied()
            .filter(|&u| self.e(u).orders.is_empty())
            .collect();
        for u in idle {
            let place = self.e(u).place;
            let Some(deposit) = self.choose(p, Kind::Deposit, None, place, true)? else {
                return Ok(());
            };
            let id = self.t_entity(&deposit).expect("an entity").id;
            self.take_order(u, vec!["gather".into(), id.to_string()], true, false, None)?;
            add_count(&mut self.csm(p).gathered_deposits, &Some(deposit), 1);
        }
        Ok(())
    }

    fn value_as_an_explorer(&self, u: EntityId) -> (bool, i64, i64) {
        let e = self.e(u);
        (e.airground_type == AirGround::Air, e.stats.speed, e.hp)
    }

    /// `best_explorers`: the mobile units not upgrading, best first.
    pub(crate) fn best_explorers(&self, p: PlayerId) -> Vec<EntityId> {
        let mut v: Vec<EntityId> = self
            .pl(p)
            .units
            .iter()
            .copied()
            .filter(|&u| {
                let e = self.e(u);
                e.stats.speed > 0 && e.orders.first().map(|o| o.keyword) != Some(Keyword::UpgradeTo)
            })
            .collect();
        v.sort_by(|&a, &b| {
            self.value_as_an_explorer(b)
                .cmp(&self.value_as_an_explorer(a))
                .then_with(|| b.cmp_as_text(a))
        });
        v
    }

    fn send_explorer(&mut self, p: PlayerId) -> Res {
        let Some(&best) = self.best_explorers(p).first() else {
            return Ok(());
        };
        let first_is = |w: &World, u: EntityId| {
            w.e(u).orders.first().map(|o| o.keyword) == Some(Keyword::AutoExplore)
        };
        if first_is(self, best) {
            return Ok(());
        }
        let explorer = self
            .pl(p)
            .units
            .iter()
            .copied()
            .find(|&u| first_is(self, u));
        if let Some(explorer) = explorer {
            if self.value_as_an_explorer(explorer) == self.value_as_an_explorer(best) {
                return Ok(());
            }
            let first = self.pl(p).units[0];
            let id = self.place_id(self.e(first).place)?;
            self.take_order(explorer, vec!["go".into(), id], true, false, None)?;
        }
        self.take_order(best, vec!["auto_explore".into()], true, false, None)
    }

    /// `_idle_fighters`: soldiers without orders, or only going to a square
    /// without enemies.
    fn idle_fighters(&self, p: PlayerId) -> Vec<EntityId> {
        self.pl(p)
            .units
            .iter()
            .copied()
            .filter(|&u| {
                let e = self.e(u);
                e.class == EntityClass::Soldier
                    && (e.orders.is_empty()
                        || e.orders.len() == 1
                            && e.orders[0].keyword == Keyword::Go
                            && !matches!(&e.orders[0].target,
                                Some(Target::Square(s)) if self.pl(p).enemy_presence.contains(s)))
            })
            .collect()
    }

    fn friendly_presence(&self, p: PlayerId, place: Option<Place>) -> bool {
        matches!(place, Some(Place::Square(s)) if self.pl(p).places_with_friends.contains(&s))
    }

    /// The id of a square, as an order argument.
    fn place_id(&self, place: Option<Place>) -> Res<String> {
        match place {
            Some(Place::Square(s)) => Ok(self.square(s).id.to_string()),
            _ => Err(Abort::new("the place has no id")),
        }
    }

    fn square_of(place: Option<Place>) -> Res<SquareId> {
        match place {
            Some(Place::Square(s)) => Ok(s),
            Some(Place::Inside(_)) => Err(Abort::new("'Inside' object has no attribute 'exits'")),
            None => Err(Abort::new("'NoneType' object has no attribute 'exits'")),
        }
    }

    fn eventually_attack(&mut self, p: PlayerId, places: Vec<Option<Place>>) -> Res {
        let units = self.idle_fighters(p);
        if units.is_empty() {
            return Ok(());
        }
        let mut places = places;
        places.sort_by_key(|&pl| self.place_enemy_menace(p, pl));
        for &place in &places {
            if self.units_should_attack(p, &units, place)? {
                return self.send_units(p, units, place);
            }
        }
        let (Some(&first), Some(&last)) = (places.first(), places.last()) else {
            return Ok(());
        };
        let temp: Vec<EntityId> = units
            .iter()
            .copied()
            .filter(|&u| {
                let e = self.e(u);
                e.time_limit.is_some_and(|t| t != 0) && e.stats.speed != 0
            })
            .collect();
        if !temp.is_empty() {
            self.send_units(p, temp, first)?;
        }
        if self.friendly_presence(p, last) {
            return Ok(());
        }
        let mut enemies = Enemies {
            place: last,
            used: false,
        };
        for (u, a) in self.cs(p).cataclysm_users.clone() {
            if !self.e(u).orders.is_empty() || !self.cataclysm_is_efficient(p, &a, &mut enemies) {
                continue;
            }
            let start = Self::square_of(self.e(u).place)?;
            let path = self.shortest_path_places(
                start,
                Self::square_of(last)?,
                None,
                AirGround::Ground,
                false,
            )?;
            if path.len() > 2 {
                let id = self.square(path[path.len() - 2]).id.to_string();
                self.take_order(u, vec!["go".into(), id], false, false, None)?;
            }
            let id = self.place_id(last)?;
            self.take_order(u, vec!["use".into(), a.clone(), id], false, false, None)?;
            if self.e(u).orders.first().is_some_and(|o| !o.is_impossible) {
                let id = self.place_id(self.e(u).place)?;
                self.take_order(u, vec!["go".into(), id], false, false, None)?;
            }
        }
        Ok(())
    }

    /// `_cataclysm_is_efficient`: the ability summons something harmful
    /// to one of the enemies.
    fn cataclysm_is_efficient(&mut self, p: PlayerId, a: &str, enemies: &mut Enemies) -> bool {
        let mut names: BTreeSet<String> = BTreeSet::new();
        if !enemies.used {
            enemies.used = true;
            let pl = self.pl(p);
            for &o in &pl.perception {
                let e = self.e(o);
                if e.place == enemies.place && self.player_is_an_enemy_of_entity(p, e) {
                    names.insert(e.type_name.to_string());
                }
            }
            for r in pl.memory.values() {
                let e = &r.snapshot;
                if e.place == enemies.place && self.player_is_an_enemy_of_entity(p, e) {
                    names.insert(e.type_name.to_string());
                }
            }
        }
        let Some(effect) = self.rules.get_words(a, "effect") else {
            return false;
        };
        if effect.first().map(String::as_str) == Some("summon") {
            for item in &effect[1..] {
                if self.rules.get_is_truthy(item, "harm_level") {
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

    fn units_should_attack(
        &mut self,
        p: PlayerId,
        units: &[EntityId],
        place: Option<Place>,
    ) -> Res<bool> {
        let m = self.place_enemy_menace(p, place);
        if !self.is_powerful_enough(p, units, m) {
            return Ok(false);
        }
        let start = Self::square_of(self.e(units[0]).place)?;
        let path = self.shortest_path_places(
            start,
            Self::square_of(place)?,
            None,
            AirGround::Ground,
            false,
        )?;
        if path.is_empty() {
            Ok(false)
        } else if path.len() <= 2 {
            Ok(true)
        } else {
            let before = path[path.len() - 2];
            Ok(self
                .shortest_path_to(start, before, Some(p), AirGround::Ground, true)?
                .is_some())
        }
    }

    fn send_units(&mut self, p: PlayerId, units: Vec<EntityId>, place: Option<Place>) -> Res {
        let Some(place) = place else {
            return Ok(());
        };
        let units: Vec<EntityId> = units
            .into_iter()
            .filter(|&u| self.e(u).place != Some(place))
            .collect();
        for &u in &units {
            self.cancel_all_orders(u, true);
        }
        let mut used_teleportation = false;
        for (u, a) in self.cs(p).teleportation_users.clone() {
            let id = self.place_id(Some(place))?;
            self.take_order(u, vec!["use".into(), a, id], true, false, None)?;
            if self.e(u).orders.first().is_some_and(|o| !o.is_impossible) {
                used_teleportation = true;
            }
        }
        let mut enemies = Enemies {
            place: Some(place),
            used: false,
        };
        for u in units {
            let start = Self::square_of(self.e(u).place)?;
            let path = self.shortest_path_places(
                start,
                Self::square_of(Some(place))?,
                None,
                AirGround::Ground,
                false,
            )?;
            if !used_teleportation && path.len() > 2 {
                let id = self.square(path[path.len() - 2]).id.to_string();
                self.take_order(u, vec!["go".into(), id], false, false, None)?;
                if !self.friendly_presence(p, Some(place)) {
                    for (u_, a) in self.cs(p).cataclysm_users.clone() {
                        if u_ == u && self.cataclysm_is_efficient(p, &a, &mut enemies) {
                            let id = self.place_id(Some(place))?;
                            self.take_order(u, vec!["use".into(), a, id], false, false, None)?;
                        }
                    }
                }
            }
            for (u_, a) in self.cs(p).summon_users.clone() {
                if u_ == u {
                    let id = self.place_id(Some(place))?;
                    self.take_order(u, vec!["use".into(), a, id], false, false, None)?;
                }
            }
            for (u_, a) in self.cs(p).detector_users.clone() {
                if u_ == u {
                    let id = self.place_id(Some(place))?;
                    self.take_order(u, vec!["use".into(), a, id], false, false, None)?;
                }
            }
            let id = self.place_id(Some(place))?;
            self.take_order(u, vec!["go".into(), id], false, false, None)?;
        }
        Ok(())
    }

    /// `_builders_place`: where most workers are; on a tie, the place with
    /// the greatest id (as text).
    fn builders_place(&self, p: PlayerId) -> Res<Option<Place>> {
        let mut starts: Vec<(Option<Place>, i64)> = Vec::new();
        for &u in &self.cs(p).workers {
            let place = self.e(u).place;
            match starts.iter_mut().find(|(pl, _)| *pl == place) {
                Some(s) => s.1 += 1,
                None => starts.push((place, 1)),
            }
        }
        let Some(max) = starts.iter().map(|s| s.1).max() else {
            return Ok(None);
        };
        let best: Vec<Option<Place>> = starts.iter().filter(|s| s.1 == max).map(|s| s.0).collect();
        if best.len() > 1 && best.iter().any(|pl| !matches!(pl, Some(Place::Square(_)))) {
            return Err(Abort::new(
                "'<' not supported between instances of 'str' and 'NoneType'",
            ));
        }
        Ok(best
            .into_iter()
            .max_by(|a, b| {
                let id = |pl: &Option<Place>| match pl {
                    Some(Place::Square(s)) => self.square(*s).id.to_string(),
                    _ => String::new(),
                };
                id(a).cmp(&id(b))
            })
            .flatten())
    }

    fn defensive_routine(&mut self, p: PlayerId) -> Res {
        if let Some(b) = self.cs(p).sensible_building
            && !self.pl(p).units.contains(&b)
        {
            self.csm(p).sensible_building = None;
        }
        // regroup at a valuable place (and get healed or repaired)
        let fighters = self.idle_fighters(p);
        let place = self.builders_place(p)?;
        self.send_units(p, fighters, place)?;
        // build static defenses
        let gate = self.rules.unit_type("gate").map(|t| t.cost.clone());
        if let (Some(b), Some(gate_cost)) = (self.cs(p).sensible_building, gate) {
            let e = self.e(b);
            let s = Self::square_of(e.place)?;
            let (bx, by) = (e.x, e.y);
            let mut exits = self.square(s).exits.clone();
            exits.sort_by_key(|&x| square_of_distance(bx, by, self.e(x).x, self.e(x).y));
            if let Some(&exit) = exits.first()
                && !self.exit_is_blocked(exit, Viewer::Nobody, false, false)
                && self.ai_gather(p, &gate_cost, 0)?
            {
                for w in self.cs(p).workers.clone() {
                    self.take_order(
                        w,
                        vec!["build".into(), "gate".into(), exit.to_string()],
                        true,
                        false,
                        None,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn idle_buildings_research(&mut self, p: PlayerId) -> Res {
        for u in self.pl(p).units.clone() {
            if !self.e(u).orders.is_empty() {
                continue;
            }
            for t in self.type_list(u, "can_research") {
                let cost = self
                    .rules
                    .unit_type(&t)
                    .map(|c| c.cost.clone())
                    .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'cost'"))?;
                if self.future_nb(p, &Sel::List(vec![t.clone()])) == 0
                    && self.missing_resources(p, &cost)?.is_empty()
                    && self.potential(p, &cost)? > 3
                {
                    self.take_order(u, vec!["research".into(), t], true, false, None)?;
                }
            }
        }
        Ok(())
    }

    fn raise_dead(&mut self, p: PlayerId) -> Res {
        for (u, a) in self.cs(p).raise_dead_users.clone() {
            if let Some(Place::Square(s)) = self.e(u).place
                && self.pl(p).places_with_corpses.contains(&s)
            {
                let id = self.square(s).id.to_string();
                self.take_order(u, vec!["use".into(), a, id], true, false, None)?;
            }
        }
        Ok(())
    }

    // Computer: warehouses

    fn best_warehouse(&self, p: PlayerId) -> Option<(String, Vec<i64>)> {
        let name = self.equivalent(p, "townhall");
        self.rules
            .unit_type(&name)
            .map(|t| (t.name.clone(), t.cost.clone()))
    }

    fn build_a_warehouse_if_useful(&mut self, p: PlayerId) -> Res {
        let Some((_, cost)) = self.best_warehouse(p) else {
            return Ok(());
        };
        if !self.missing_resources(p, &cost)?.is_empty() {
            return Ok(());
        }
        let mut deposits = Vec::new();
        for &u in &self.cs(p).workers {
            for o in &self.e(u).orders {
                if o.keyword == Keyword::Gather
                    && let Some(t) = &o.target
                    && self.t_place(t) != TPlace::Nowhere
                {
                    deposits.push(t.clone());
                }
            }
        }
        for d in deposits {
            self.build_a_warehouse_for(p, &d)?;
        }
        Ok(())
    }

    fn deposit_resource_type(&self, d: &Target) -> Res<i64> {
        self.t_entity(d)
            .and_then(|e| e.type_id)
            .and_then(|t| self.rules.ty(t).resource_type)
            .ok_or_else(|| Abort::new("object has no attribute 'resource_type'"))
    }

    fn build_a_warehouse_for(&mut self, p: PlayerId, deposit: &Target) -> Res {
        let dplace = self.t_place_of(deposit);
        let nearby: Vec<EntityId> = self
            .cs(p)
            .workers
            .iter()
            .copied()
            .filter(|&v| {
                let e = self.e(v);
                e.place == dplace
                    || e.orders.first().is_some_and(|o| {
                        o.keyword == Keyword::Gather
                            && o.target
                                .as_ref()
                                .is_none_or(|t| self.t_place_of(t) == dplace && dplace.is_some())
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
        let wh = self.nearest_warehouse(p, dp, rt, true)?;
        if let Some(wh) = wh
            && self.e(wh).class == EntityClass::BuildingSite
        {
            for v in nearby {
                self.take_order(v, vec!["repair".into(), wh.to_string()], true, false, None)?;
            }
            return Ok(());
        }
        let far = match wh {
            None => true,
            Some(wh) => {
                let start = Self::square_of(Some(dp))?;
                let dest = Self::square_of(self.e(wh).place)?;
                self.shortest_path_distance_to(start, dest, Some(p), AirGround::Ground, true)?
                    > self.square_width()
            }
        };
        if far && let Some(meadow) = self.choose(p, Kind::Meadow, None, Some(dp), false)? {
            let (name, _) = self
                .best_warehouse(p)
                .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'type_name'"))?;
            let id = self.t_entity(&meadow).expect("an entity").id.to_string();
            for v in nearby {
                self.take_order(
                    v,
                    vec!["build".into(), name.clone(), id.clone()],
                    true,
                    false,
                    None,
                )?;
            }
        }
        Ok(())
    }

    // Computer: choosing a place

    /// `is_ok_for_warehouse`: no warehouse for the resource on the square,
    /// and a deposit of it.
    pub(crate) fn is_ok_for_warehouse(&self, z: SquareId, resource_type: i64) -> bool {
        let objects = &self.square(z).objects;
        for &o in objects {
            let stores = self.e(o).type_id.is_some_and(|t| {
                self.rules
                    .ty(t)
                    .storable_resource_types
                    .contains(&resource_type)
            });
            if stores {
                return false;
            }
        }
        objects.iter().any(|&o| {
            let e = self.e(o);
            e.class == EntityClass::Deposit
                && e.type_id.and_then(|t| self.rules.ty(t).resource_type) == Some(resource_type)
        })
    }

    fn choose_is_ok(&self, p: PlayerId, o: &Target, resource_type: Option<i64>) -> bool {
        let Some(place) = self.t_place_of(o) else {
            return false;
        };
        let Place::Square(s) = place else {
            return resource_type.is_none();
        };
        resource_type.is_none_or(|rt| self.is_ok_for_warehouse(s, rt))
            && !self.square_is_dangerous(p, s)
    }

    fn is_known(&self, p: PlayerId, o: &Target) -> bool {
        match o {
            Target::Entity(id) => self.pl(p).perception.contains(id),
            Target::Memory(m) => self.memory_is_current(m),
            _ => false,
        }
    }

    /// `choose`: a known meadow or deposit, safe, near `starting_place`
    /// (the first unit's place by default). With `random`, the deposit of
    /// the nearest place gathered by the fewest workers.
    fn choose(
        &mut self,
        p: PlayerId,
        kind: Kind,
        resource_type: Option<i64>,
        starting_place: Option<Place>,
        random: bool,
    ) -> Res<Option<Target>> {
        let Some(&first_unit) = self.pl(p).units.first() else {
            return Ok(None);
        };
        let key = (kind, resource_type, starting_place);
        if !random && let Some(o) = self.cs(p).previous_choose.get(&key).cloned() {
            if self.is_known(p, &o) && self.choose_is_ok(p, &o, resource_type) {
                return Ok(Some(o));
            }
            self.csm(p).previous_choose.remove(&key);
        }
        let start = starting_place.or(self.e(first_unit).place);
        let mut candidates: Vec<(EntityId, Target)> = Vec::new();
        for &o in &self.pl(p).perception {
            if kind.matches(self.e(o)) {
                candidates.push((o, Target::Entity(o)));
            }
        }
        for r in self.pl(p).memory.values() {
            if kind.matches(&r.snapshot) {
                candidates.push((
                    r.snapshot.id,
                    Target::Memory(crate::entity::MemoryRef {
                        owner: p,
                        snapshot: r.snapshot.clone(),
                    }),
                ));
            }
        }
        candidates.retain(|(_, o)| self.choose_is_ok(p, o, resource_type));
        candidates.sort_by(|a, b| a.0.cmp_as_text(b.0));
        let mut candidates: Vec<Target> = candidates.into_iter().map(|(_, o)| o).collect();
        if candidates.len() > 10 {
            candidates = self.remove_far_candidates(&candidates, start, 10)?;
        } else if !candidates.is_empty() {
            let start = Self::square_of(start)?;
            let mut keyed = Vec::new();
            for o in candidates {
                let dest = Self::square_of(self.t_place_of(&o))?;
                let d =
                    self.shortest_path_distance_to(start, dest, Some(p), AirGround::Ground, true)?;
                keyed.push((d, o));
            }
            keyed.sort_by_key(|(d, _)| *d);
            candidates = keyed.into_iter().map(|(_, o)| o).collect();
        }
        if random && let Some(c0) = candidates.first() {
            let p0 = self.t_place_of(c0);
            let mut keyed = Vec::new();
            for o in candidates.into_iter().filter(|o| self.t_place_of(o) == p0) {
                keyed.push((self.deposit_priority(p, &Some(o.clone()))?, o));
            }
            keyed.sort_by(|a, b| b.0.cmp(&a.0));
            candidates = keyed.into_iter().map(|(_, o)| o).collect();
        }
        let result = candidates.into_iter().next();
        if !random && let Some(o) = &result {
            self.csm(p).previous_choose.insert(key, o.clone());
        }
        Ok(result)
    }

    /// `_remove_far_candidates`: the candidates met first in a breadth
    /// first walk of the squares from `start`. A square met twice before
    /// its exits are walked is counted twice, as in the original.
    fn remove_far_candidates(
        &self,
        candidates: &[Target],
        start: Option<Place>,
        limit: usize,
    ) -> Res<Vec<Target>> {
        let start = Self::square_of(start)?;
        let mut ids: BTreeMap<EntityId, Target> = BTreeMap::new();
        for o in candidates {
            ids.insert(self.t_entity(o).expect("an entity").id, o.clone());
        }
        let mut c = Vec::new();
        let mut queue = std::collections::VecDeque::from([start]);
        let mut done: Vec<SquareId> = Vec::new();
        while c.len() < limit {
            let Some(room) = queue.pop_front() else { break };
            for o in &self.square(room).objects {
                if let Some(t) = ids.get(o) {
                    c.push(t.clone());
                    if c.len() >= limit {
                        break;
                    }
                }
            }
            if done.contains(&room) {
                continue;
            }
            for &e in &self.square(room).exits {
                let os = self
                    .exit_other_side(e)
                    .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'place'"))?;
                let next = Self::square_of(self.e(os).place)?;
                if !done.contains(&next) {
                    queue.push_back(next);
                }
            }
            done.push(room);
        }
        Ok(c)
    }

    fn deposit_priority(&self, p: PlayerId, d: &Option<Target>) -> Res<DepositPriority> {
        let Some(t) = d else {
            return Ok((-100, 0, IdKey::Zero));
        };
        let workers = self
            .cs(p)
            .gathered_deposits
            .iter()
            .find(|(k, _)| same_opt(k, d))
            .map_or(0, |(_, n)| *n);
        let rt = self.deposit_resource_type(t)?;
        let r = self
            .pl(p)
            .resources
            .get(rt as usize)
            .copied()
            .ok_or_else(|| Abort::new("list index out of range"))?;
        let id = self.t_entity(t).expect("an entity").id;
        Ok((floordiv(-r, 10), -workers, IdKey::Text(id.to_string())))
    }

    // Computer: counting and getting units

    fn entity_matches(&self, e: &Entity, sel: &Sel) -> bool {
        match sel {
            Sel::Workers => e.class == EntityClass::Worker,
            _ => sel.matches_type(&e.type_name),
        }
    }

    fn nb(&self, p: PlayerId, sel: &Sel) -> i64 {
        if let Sel::List(l) = sel
            && let Some(first) = l.first()
            && self.pl(p).upgrades.contains(first)
        {
            return 1;
        }
        self.pl(p)
            .units
            .iter()
            .filter(|&&u| self.entity_matches(self.e(u), sel))
            .count() as i64
    }

    fn nb_in_production(&self, p: PlayerId, sel: &Sel) -> i64 {
        let mut n = 0;
        for &u in &self.pl(p).units {
            let e = self.e(u);
            if let Some(site) = &e.site
                && e.class == EntityClass::BuildingSite
                && sel.matches_type(&self.rules.ty(site.building_type).name)
            {
                n += 1;
                continue;
            }
            for o in &e.orders {
                if matches!(
                    o.keyword,
                    Keyword::Build | Keyword::Train | Keyword::UpgradeTo | Keyword::Research
                ) && o
                    .type_name
                    .as_deref()
                    .is_some_and(|t| self.rules.unit_type(t).is_some() && sel.matches_type(t))
                {
                    n += 1;
                }
            }
        }
        n
    }

    fn future_nb(&self, p: PlayerId, sel: &Sel) -> i64 {
        self.nb(p, sel) + self.nb_in_production(p, sel)
    }

    fn missing_resources(&self, p: PlayerId, cost: &[i64]) -> Res<Vec<usize>> {
        let r = &self.pl(p).resources;
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

    fn potential(&self, p: PlayerId, cost: &[i64]) -> Res<i64> {
        let mut result = 9999;
        for (i, &res) in self.pl(p).resources.iter().enumerate() {
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

    /// `nb`: the number of units of the type.
    #[cfg(test)]
    pub(crate) fn computer_nb(&self, p: PlayerId, type_name: &str) -> i64 {
        self.nb(p, &Sel::One(type_name.to_owned()))
    }

    /// `get`: works toward having `nb` units of the type; true if they are
    /// there.
    pub(crate) fn ai_get(&mut self, p: PlayerId, nb: i64, type_name: &str) -> Res<bool> {
        self.csm(p).safe_cnt = 0;
        self.get_types(p, nb, vec![type_name.to_owned()])
    }

    fn get_types(&mut self, p: PlayerId, nb: i64, types: Vec<String>) -> Res<bool> {
        let sel = Sel::List(types.clone());
        if self.nb(p, &sel) >= nb {
            return Ok(true);
        }
        if self.future_nb(p, &sel) >= nb {
            return Ok(false);
        }
        self.csm(p).safe_cnt += 1;
        if self.cs(p).safe_cnt > 10 {
            return Ok(false);
        }
        for t in &types {
            if self.rules.unit_type(t).is_none() {
                continue;
            }
            let makers = self.rules.get_makers(t);
            if self.nb(p, &Sel::List(makers.clone())) > 0 {
                let n = nb - self.future_nb(p, &sel);
                self.build_or_train_or_upgradeto_or_summon(p, t, n)?;
                break;
            } else if let Some(m) = makers.first() {
                self.get_types(p, 1, vec![m.clone()])?;
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// `gather`: true if the resources and the food are there; asks for a
    /// farm when food is missing.
    fn ai_gather(&mut self, p: PlayerId, cost: &[i64], food: i64) -> Res<bool> {
        if !self.missing_resources(p, cost)?.is_empty() {
            return Ok(false);
        }
        let limit = self.food_limit();
        let pl = self.pl(p);
        if food != 0 && food > pl.available_food(limit) - pl.used_food {
            if pl.available_food(limit) < limit {
                let t = Sel::One(self.equivalent(p, "farm"));
                if self.future_nb(p, &t) == self.nb(p, &t) {
                    let Sel::One(name) = t else { unreachable!() };
                    self.build_or_train_or_upgradeto_or_summon(p, &name, 1)?;
                }
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn get_requirements(&mut self, p: PlayerId, requirements: &[String]) -> Res<bool> {
        for r in requirements {
            if !self.has(p, r) {
                return self.get_types(p, 1, vec![r.clone()]);
            }
        }
        Ok(true)
    }

    fn build_or_train_or_upgradeto_or_summon(
        &mut self,
        p: PlayerId,
        type_name: &str,
        nb: i64,
    ) -> Res {
        let t = self
            .rules
            .unit_type(type_name)
            .cloned()
            .ok_or_else(|| Abort::new("'NoneType' object has no attribute '__name__'"))?;
        let makers = self.rules.get_makers(type_name);
        if !self.get_types(p, 1, makers.clone())? || !self.get_requirements(p, &t.requirements)? {
            return Ok(());
        }
        let mut maker = makers.last().cloned().expect("a maker exists");
        for m in &makers {
            if self.nb(p, &Sel::One(m.clone())) != 0 {
                maker = m.clone();
                break;
            }
        }
        let mc = self
            .rules
            .unit_type(&maker)
            .cloned()
            .expect("a maker is a class");
        let maker_sel = Sel::One(maker.clone());
        let name = type_name.to_owned();
        if mc.can_upgrade_to.contains(&name) {
            if self.nb(p, &maker_sel) >= nb {
                let cost: Vec<i64> = (0..t.cost.len())
                    .map(|i| t.cost[i] - mc.cost.get(i).copied().unwrap_or(0))
                    .collect();
                if self.ai_gather(p, &cost, t.stats.food_cost - mc.stats.food_cost)? {
                    self.order(p, nb, maker_sel, vec!["upgrade_to".into(), name], false)?;
                }
            } else {
                self.get_types(p, nb, vec![maker])?;
            }
        } else if mc.can_build.contains(&name) {
            if !self.ai_gather(p, &t.cost, t.stats.food_cost)? {
                return Ok(());
            }
            let meadow = if let Some(&rt) = t.storable_resource_types.first() {
                let meadow = self.choose(p, Kind::Meadow, Some(rt), None, false)?;
                let far = match &meadow {
                    None => true,
                    Some(m) => {
                        let start = Self::square_of(self.t_place_of(m))?;
                        let bp = self.builders_place(p)?;
                        let dest = Self::square_of(bp)?;
                        self.shortest_path_distance_to(start, dest, None, AirGround::Ground, false)?
                            > self.square_width() * 3
                    }
                };
                if far {
                    if self.nb(p, &Sel::One(name.clone())) != 0 {
                        return Ok(());
                    }
                    let bp = self.builders_place(p)?;
                    self.choose(p, Kind::Meadow, None, bp, false)?
                } else {
                    meadow
                }
            } else {
                let bp = self.builders_place(p)?;
                self.choose(p, Kind::Meadow, None, bp, false)?
            };
            if let Some(m) = meadow {
                let id = self.t_entity(&m).expect("an entity").id.to_string();
                self.order(p, 4, maker_sel, vec!["build".into(), name, id], true)?;
            }
        } else if mc.can_train.contains(&name) {
            let nb_maker = self.nb(p, &maker_sel);
            if self.nb(p, &Sel::Workers) != 0
                && nb > nb_maker * 3
                && self.potential(p, &t.cost)? > nb_maker * 100
            {
                // additional production sites
                self.build_or_train_or_upgradeto_or_summon(p, &maker, 1)?;
            }
            if self.ai_gather(p, &t.cost, t.stats.food_cost)? {
                self.order(p, nb, maker_sel, vec!["train".into(), name], false)?;
            }
        } else if mc.can_research.contains(&name) {
            if self.ai_gather(p, &t.cost, t.stats.food_cost)? {
                self.order(p, 1, maker_sel, vec!["research".into(), name], false)?;
            }
        } else {
            for ability in &mc.can_use {
                let Some(effect) = self.rules.get_words(ability, "effect") else {
                    continue;
                };
                if effect.first().map(String::as_str) == Some("summon") && effect.contains(&name) {
                    let mut order = vec!["use".into(), ability.clone()];
                    if self.rules.get_words(ability, "effect_target").as_deref()
                        == Some(&["ask".to_owned()][..])
                    {
                        let first = self.pl(p).units[0];
                        order.push(first.to_string());
                    }
                    self.order(p, 1, maker_sel, order, false)?;
                    break;
                }
            }
        }
        Ok(())
    }

    fn worker_orders_priority(
        &self,
        p: PlayerId,
        u: EntityId,
    ) -> Res<(u8, Option<DepositPriority>)> {
        Ok(match self.e(u).orders.first() {
            None => (0, None),
            Some(o) if o.keyword == Keyword::Gather => {
                (1, Some(self.deposit_priority(p, &o.target)?))
            }
            Some(_) => (2, None),
        })
    }

    /// Whether a followed order is still in progress.
    fn tracked_is_alive(&mut self, t: &mut Tracked) -> bool {
        let unit = self.e(t.unit);
        if let Some(live) = unit.orders.iter().find(|o| o.uid == t.order.uid) {
            t.order = live.clone();
            return !live.is_complete && unit.place.is_some();
        }
        !t.order.is_complete
            && unit.place.is_some()
            && unit.orders.iter().any(|o| self.order_eq(o, &t.order))
    }

    /// `order`: gives the order to units of a type until `nb` of them
    /// follow it. With `requisition`, busy workers may be taken, the least
    /// useful first.
    fn order(
        &mut self,
        p: PlayerId,
        nb: i64,
        sel: Sel,
        order: Vec<String>,
        requisition: bool,
    ) -> Res {
        let key = (sel.clone(), order.clone());
        let mut tracked = self.csm(p).orders.remove(&key).unwrap_or_default();
        tracked.retain_mut(|t| self.tracked_is_alive(t));
        let enough = tracked.len() as i64 >= nb;
        self.csm(p).orders.insert(key.clone(), tracked);
        if enough {
            return Ok(());
        }
        let mut units: Vec<EntityId> = self
            .pl(p)
            .units
            .iter()
            .copied()
            .filter(|&u| self.entity_matches(self.e(u), &sel))
            .collect();
        while !units.is_empty() {
            if requisition {
                let mut keyed = Vec::new();
                for u in units {
                    keyed.push((self.worker_orders_priority(p, u)?, u));
                }
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                units = keyed.into_iter().map(|(_, u)| u).collect();
            }
            let u = units.remove(0);
            if order[0] == "upgrade_to"
                && self.e(u).orders.first().map(|o| o.keyword) == Some(Keyword::AutoExplore)
            {
                self.take_order(u, vec!["stop".into()], true, false, None)?;
            }
            if requisition || self.e(u).orders.is_empty() {
                let first = self
                    .e(u)
                    .orders
                    .first()
                    .map(|o| (o.keyword, o.target.clone()));
                if let Some((kw, target)) = &first {
                    if matches!(kw, Keyword::Build | Keyword::Repair) {
                        continue;
                    }
                    if requisition && *kw == Keyword::Gather {
                        let gd = &mut self.csm(p).gathered_deposits;
                        match gd.iter_mut().find(|(k, _)| same_opt(k, target)) {
                            Some(e) => e.1 -= 1,
                            None => return Err(Abort::new("KeyError: the deposit")),
                        }
                    }
                }
                self.take_order(u, order.clone(), true, false, None)?;
                if let Some(o) = self.e(u).orders.first()
                    && o.keyword.as_str() == order[0]
                {
                    let t = Tracked {
                        unit: u,
                        order: o.clone(),
                    };
                    let list = self.csm(p).orders.get_mut(&key).expect("inserted above");
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
mod tests {
    use super::*;

    #[test]
    fn computer2_chooses_from_the_original_list_of_orders() {
        let names: Vec<&str> = World::computer2_orders()
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
