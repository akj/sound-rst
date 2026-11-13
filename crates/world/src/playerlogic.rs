//! What players do (`worldplayerbase.py`): perception and memory, menace,
//! resources, victory and defeat.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::entity::{Entity, EntityId, MemoryRef, Place, PlayerId, SquareId, Target, ZoomTarget};
use crate::events::Event;
use crate::msg::{Msg, msg};
use crate::msgparts as mp;
use crate::nofloat::{PRECISION, floordiv, py_int, square_of_distance, to_int};
use crate::orders::{Keyword, Order};
use crate::player::{PlayerKind, Remembrance, StatEvent, StatTarget};
use crate::rules::{AirGround, EntityClass, MAX_NB_OF_RESOURCE_TYPES};
use crate::world::{Abort, INF, Res, World};

/// The side of the buckets used to find units near a point.
pub const BUCKET_SIDE: i64 = 12 * PRECISION;
/// The slowest a mobile unit can go.
pub const VERY_SLOW: i64 = PRECISION / 100;

/// What `_is_seeing` reads of an entity or of a remembered copy.
#[derive(Clone, Copy)]
struct Seen {
    id: EntityId,
    x: i64,
    y: i64,
    place: Option<Place>,
    hidden: bool,
}

impl Seen {
    fn of(e: &Entity) -> Seen {
        Seen {
            id: e.id,
            x: e.x,
            y: e.y,
            place: e.place,
            hidden: e.is_invisible() || e.is_cloaked,
        }
    }
}

impl World {
    // relations

    /// `Player.is_an_enemy(o)`: the entity belongs to a player who is not an
    /// ally.
    pub(crate) fn player_is_an_enemy_of_entity(&self, p: PlayerId, e: &Entity) -> bool {
        e.player.is_some_and(|op| !self.pl(p).allied.contains(&op))
    }

    /// `Player.player_is_an_enemy(other)`.
    pub fn player_is_an_enemy(&self, p: PlayerId, other: PlayerId) -> bool {
        !self.pl(p).allied.contains(&other)
    }

    /// `observe`: a visible entity revealed by its action stays perceived
    /// for 3 seconds.
    pub(crate) fn observe(&mut self, p: PlayerId, o: EntityId) {
        let e = self.e(o);
        if e.is_invisible() || e.is_cloaked {
            return;
        }
        let t = self.time + 3000;
        self.plm(p).observed_objects.insert(o, t);
    }

    pub(crate) fn get_safest_subsquare(&self, p: PlayerId, place: SquareId) -> (i64, i64) {
        let sw = self.square_width();
        let sq = self.square(place);
        let (x, y) = (floordiv(sq.x * 3, sw), floordiv(sq.y * 3, sw));
        let mut candidates = Vec::new();
        for dx in [0, 1, -1] {
            for dy in [0, 1, -1] {
                candidates.push((x + dx, y + dy));
            }
        }
        let threat = |k: &(i64, i64)| self.pl(p).subsquare_threat.get(k).copied().unwrap_or(0);
        candidates.sort_by_key(threat);
        let sub = candidates[0];
        (
            floordiv(sub.0 * sw, 3) + floordiv(sw, 6),
            floordiv(sub.1 * sw, 3) + floordiv(sw, 6),
        )
    }

    /// `known_enemies`: the perceived, vulnerable enemy units on a square,
    /// computed once per world time. In id order.
    pub fn known_enemies(&mut self, p: PlayerId, place: SquareId) -> Vec<EntityId> {
        let cached = self.pl(p).known_enemies.get(&place).cloned();
        let list: Vec<EntityId> = match cached {
            Some((t, list)) if t == self.time => list
                .into_iter()
                .filter(|&u| self.e(u).place.is_some())
                .collect(),
            _ => {
                // The units of the players in the game who see this player as
                // an enemy, that this player perceives on the square.
                let mut list: Vec<EntityId> = self
                    .square(place)
                    .objects
                    .iter()
                    .copied()
                    .filter(|u| {
                        let e = self.e(*u);
                        e.player.is_some_and(|owner| {
                            self.players.contains(&owner)
                                && self.player_is_an_enemy(owner, p)
                                && self.pl(owner).units.contains(u)
                        }) && self.pl(p).perception.contains(u)
                            && e.stats.is_vulnerable != 0
                            && !e.is_inside()
                    })
                    .collect();
                list.sort();
                list.dedup();
                list
            }
        };
        let time = self.time;
        self.plm(p)
            .known_enemies
            .insert(place, (time, list.clone()));
        list
    }

    pub(crate) fn enemy_menace(&self, p: PlayerId, place: SquareId) -> i64 {
        self.pl(p).enemy_menace.get(&place).copied().unwrap_or(0)
    }

    /// The menace of the enemies a player knows on a square, and around it
    /// for ranged enemies.
    pub fn enemy_menace_on(&self, p: PlayerId, place: SquareId) -> i64 {
        self.enemy_menace(p, place)
    }

    pub fn square_is_dangerous(&self, p: PlayerId, s: SquareId) -> bool {
        self.pl(p).enemy_menace.contains_key(&s)
    }

    pub fn square_is_very_dangerous(&self, p: PlayerId, s: SquareId) -> bool {
        self.square_is_dangerous(p, s) && self.pl(p).enemy_presence.contains(&s)
    }

    pub fn exit_is_very_dangerous(&self, p: PlayerId, e: EntityId) -> bool {
        let Some(os) = self.exit_other_side(e) else {
            return false;
        };
        let Some(Place::Square(s)) = self.e(os).place else {
            return false;
        };
        self.pl(p).enemy_menace.contains_key(&s) && self.pl(p).enemy_presence.contains(&s)
    }

    /// `is_very_dangerous` of a square or an exit.
    pub fn is_very_dangerous(&self, p: PlayerId, t: &Target) -> Res<bool> {
        match t {
            Target::Square(s) => Ok(self.square_is_very_dangerous(p, *s)),
            Target::Entity(e) if self.e(*e).is_exit() => Ok(self.exit_is_very_dangerous(p, *e)),
            Target::Memory(m) if m.snapshot.is_exit() => {
                Ok(self.exit_is_very_dangerous(p, m.snapshot.id))
            }
            _ => Err(Abort::new("object has no attribute 'other_side'")),
        }
    }

    /// `balance`: the menace of the player's units on the squares (and of
    /// `add`), times `mult`, divided by the enemy menace on the first
    /// square; 1000 without enemy menace.
    pub fn balance(
        &self,
        p: PlayerId,
        squares: &[Option<Place>],
        add: Option<EntityId>,
        mult: i64,
    ) -> i64 {
        let mut a = 0;
        for &u in &self.pl(p).units {
            let e = self.e(u);
            if squares.contains(&e.place) || Some(u) == add {
                a += e.menace();
            }
        }
        let em = match squares.first() {
            Some(Some(Place::Square(s))) => self.enemy_menace(p, *s),
            _ => 0,
        };
        if em == 0 {
            1000
        } else {
            floordiv(a * mult, em)
        }
    }

    pub(crate) fn unknown_starting_squares(&mut self, p: PlayerId) -> Vec<SquareId> {
        let result: Vec<SquareId> = self
            .map
            .starting_squares
            .iter()
            .filter_map(|n| self.square_by_name(n))
            .filter(|s| !self.pl(p).strictly_observed_before_squares.contains(s))
            .collect();
        let n = result.len();
        self.random.sample(&result, n).unwrap_or_default()
    }

    pub(crate) fn unknown_squares(&mut self, p: PlayerId) -> Vec<SquareId> {
        let result: Vec<SquareId> = (0..self.squares.len() as u32)
            .map(SquareId)
            .filter(|s| !self.pl(p).strictly_observed_before_squares.contains(s))
            .collect();
        let n = result.len();
        self.random.sample(&result, n).unwrap_or_default()
    }

    pub(crate) fn squares_to_watch(&mut self, p: PlayerId) -> Vec<SquareId> {
        let mut squares: BTreeSet<SquareId> = BTreeSet::new();
        for r in self.pl(p).memory.values() {
            let m = &r.snapshot;
            let Some(Place::Square(s)) = m.place else {
                continue;
            };
            if self.player_is_an_enemy_of_entity(p, m) {
                squares.insert(s);
                for &ex in &self.square(s).exits {
                    if let Some(os) = self.exit_other_side(ex)
                        && let Some(Place::Square(o)) = self.e(os).place
                    {
                        squares.insert(o);
                    }
                }
            } else if m.is_deposit() {
                squares.insert(s);
            }
        }
        let mut result: Vec<SquareId> = squares.into_iter().collect();
        result.sort_by(|a, b| self.square(*a).name.cmp(&self.square(*b).name));
        let n = result.len();
        self.random.sample(&result, n).unwrap_or_default()
    }

    // update

    pub(crate) fn player_update(&mut self, p: PlayerId) -> Res {
        self.update_actual_speed(p);
        self.update_storage_bonus(p);
        self.update_allied_upgrades(p)?;
        self.update_perception_and_memory(p)?;
        self.update_menace(p);
        self.update_enemy_menace_and_presence_and_corpses(p);
        self.play(p);
        self.update_drowning(p)?;
        self.update_counterattacks(p)
    }

    pub(crate) fn player_slow_update(&mut self, p: PlayerId) -> Res {
        self.free_project_resources_if_no_worker_on_project(p)?;
        self.run_triggers(p)
    }

    fn play(&mut self, p: PlayerId) {
        if matches!(self.pl(p).kind, PlayerKind::Human) {
            return;
        }
        if let Some(mut ai) = self.plm(p).ai.take() {
            ai.play(self, p);
            self.plm(p).ai = Some(ai);
        }
    }

    pub(crate) fn update_actual_speed(&mut self, p: PlayerId) {
        for u in self.pl(p).units.clone() {
            let e = self.e(u);
            let speed = e.stats.speed;
            let actual = (|| -> Option<i64> {
                let Some(Place::Square(s)) = e.place else {
                    return None;
                };
                let sq = self.square(s);
                let sot = e.type_id.map(|t| &self.rules.ty(t).speed_on_terrain)?;
                let mut actual = if let Some(i) = sot.iter().position(|t| *t == sq.type_name) {
                    to_int(sot.get(i + 1)?).ok()?
                } else if e.airground_type == AirGround::Water {
                    speed
                } else {
                    let ts = if e.airground_type == AirGround::Ground {
                        sq.terrain_speed.0
                    } else {
                        sq.terrain_speed.1
                    };
                    floordiv(speed * ts, 100)
                };
                if speed != 0 {
                    actual = actual.max(VERY_SLOW);
                }
                Some(actual)
            })()
            .unwrap_or(speed);
            self.em(u).actual_speed = actual;
        }
        for (_, g) in self.groups.clone() {
            if g.is_empty() {
                continue;
            }
            let min = g
                .iter()
                .map(|&u| self.e(u).actual_speed)
                .min()
                .expect("not empty");
            for u in g {
                self.em(u).actual_speed = min;
            }
        }
    }

    fn update_storage_bonus(&mut self, p: PlayerId) {
        let mut bonus = vec![0; MAX_NB_OF_RESOURCE_TYPES];
        for &u in &self.pl(p).units {
            if let Some(t) = self.e(u).type_id {
                for (r, &b) in self.rules.ty(t).storage_bonus.iter().enumerate() {
                    if r < bonus.len() {
                        bonus[r] = bonus[r].max(b);
                    }
                }
            }
        }
        self.plm(p).storage_bonus = bonus;
    }

    fn update_allied_upgrades(&mut self, p: PlayerId) -> Res {
        for a in self.pl(p).allied.clone() {
            for name in self.pl(a).upgrades.clone() {
                while self.pl(p).level(&name) < self.pl(a).level(&name) {
                    if self.rules.unit_type(&name).is_none_or(|t| !t.is_upgrade()) {
                        return Err(Abort::new(format!(
                            "{name} has no attribute 'upgrade_player'"
                        )));
                    }
                    self.upgrade_player(&name, p)?;
                }
            }
        }
        Ok(())
    }

    fn team_has_lost(&self, p: PlayerId) -> bool {
        self.pl(p)
            .allied
            .iter()
            .all(|&a| self.pl(a).has_been_defeated)
    }

    /// `_is_seeing`: an allied unit has the entity in sight range and its
    /// square in view. Invisible and cloaked entities must be detected; a
    /// remembered copy never is.
    fn is_seeing(&mut self, p: PlayerId, u: Seen, is_copy: bool) -> Res<bool> {
        if u.hidden && (is_copy || !self.pl(p).detected_units.contains(&u.id)) {
            return Ok(false);
        }
        let Some(Place::Square(us)) = u.place else {
            // An interior is never among the observed squares, but the
            // original still computes them.
            for avp in self.pl(p).allied.clone() {
                for avu in self.potential_neighbors(avp, u.x, u.y) {
                    let a = self.e(avu);
                    if square_of_distance(a.x, a.y, u.x, u.y)
                        < a.stats.sight_range * a.stats.sight_range
                    {
                        self.get_observed_squares(avu, false)?;
                    }
                }
            }
            return Ok(false);
        };
        for avp in self.pl(p).allied.clone() {
            for avu in self.potential_neighbors(avp, u.x, u.y) {
                let a = self.e(avu);
                let r2 = a.stats.sight_range * a.stats.sight_range;
                if square_of_distance(a.x, a.y, u.x, u.y) < r2
                    && self.get_observed_squares(avu, false)?.contains(&us)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub(crate) fn update_perception_and_memory(&mut self, p: PlayerId) -> Res {
        let previous = self.pl(p).perception.clone();
        // Nothing moves during this update, so the squares a unit observes
        // are computed once; repeating the computation would only reveal the
        // same wall blockers again until the same time.
        self.observed_cache = Some(BTreeMap::new());
        let result = self
            .update_perception(p)
            .and_then(|()| self.update_memory(p, &previous));
        self.observed_cache = None;
        result
    }

    pub(crate) fn update_perception(&mut self, p: PlayerId) -> Res {
        if self.pl(p).cheatmode || self.team_has_lost(p) {
            let all: BTreeSet<SquareId> = (0..self.squares.len() as u32).map(SquareId).collect();
            let mut perception = BTreeSet::new();
            for s in &self.squares {
                perception.extend(s.objects.iter().copied());
            }
            let pl = self.plm(p);
            pl.observed_squares = all;
            pl.perception = perception;
            return Ok(());
        }
        let mut perception: BTreeSet<EntityId> = BTreeSet::new();
        let mut observed: BTreeSet<SquareId> = BTreeSet::new();
        let mut partially: BTreeSet<SquareId> = BTreeSet::new();
        let sw = self.square_width();
        for ap in self.pl(p).allied.clone() {
            let mut done: Vec<(bool, bool, i64, Option<Place>)> = Vec::new();
            for u in self.pl(ap).units.clone() {
                let e = self.e(u);
                let k = (
                    e.is_inside(),
                    e.stats.sight_range < sw,
                    self.height_of(e)?,
                    e.place,
                );
                if done.contains(&k) {
                    continue;
                }
                observed.extend(self.get_observed_squares(u, true)?);
                partially.extend(self.get_observed_squares(u, false)?);
                done.push(k);
            }
        }
        let partially: BTreeSet<SquareId> = partially.difference(&observed).copied().collect();
        {
            let pl = self.plm(p);
            pl.observed_before_squares.extend(partially.iter().copied());
            pl.strictly_observed_before_squares
                .extend(observed.iter().copied());
        }
        for &s in &observed {
            for &o in &self.square(s).objects {
                if self.e(o).player.is_none() {
                    perception.insert(o);
                }
            }
        }
        for &s in &partially {
            for o in self.square(s).objects.clone() {
                if self.e(o).player.is_none() {
                    let seen = Seen::of(self.e(o));
                    if self.is_seeing(p, seen, false)? {
                        perception.insert(o);
                    } else {
                        self.memorize(p, o);
                    }
                }
            }
        }
        for ap in self.pl(p).allied.clone() {
            let keys: Vec<EntityId> = self.pl(ap).observed_objects.keys().copied().collect();
            for o in keys {
                let until = self.pl(ap).observed_objects[&o];
                if until < self.time || self.e(o).place.is_none() {
                    self.plm(ap).observed_objects.remove(&o);
                }
            }
            perception.extend(self.pl(ap).observed_objects.keys().copied());
        }
        for op in self.players.clone() {
            if self.pl(p).allied.contains(&op) {
                perception.extend(self.pl(op).units.iter().copied());
            } else {
                for u in self.pl(op).units.clone() {
                    let seen = Seen::of(self.e(u));
                    if self.is_seeing(p, seen, false)? {
                        perception.insert(u);
                    }
                }
            }
        }
        let units: BTreeSet<EntityId> = self.pl(p).units.iter().copied().collect();
        perception.retain(|o| !(self.e(*o).is_inside() && !units.contains(o)));
        let pl = self.plm(p);
        pl.perception = perception;
        pl.observed_squares = observed;
        Ok(())
    }

    fn update_memory(&mut self, p: PlayerId, previous: &BTreeSet<EntityId>) -> Res {
        let observed = self.pl(p).observed_squares.clone();
        self.plm(p).observed_before_squares.extend(observed);
        let duration = self.pl(p).memory_duration();
        let memory: Vec<(EntityId, Remembrance)> = self
            .pl(p)
            .memory
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (id, r) in memory {
            let live = self.e(id);
            let forget = self.pl(p).perception.contains(&id)
                || live.place.is_none()
                || (live.stats.speed != 0 && r.time_stamp + duration < self.time)
                || self.is_seeing(p, Seen::of(&r.snapshot), true)?;
            if forget {
                self.plm(p).memory.remove(&id);
            }
        }
        let gone: Vec<EntityId> = previous
            .difference(&self.pl(p).perception)
            .copied()
            .collect();
        for o in gone {
            let e = self.e(o);
            if e.is_invisible() || e.is_cloaked {
                continue;
            }
            if e.place.is_some() {
                self.memorize(p, o);
            }
        }
        Ok(())
    }

    fn memorize(&mut self, p: PlayerId, o: EntityId) {
        let time = self.time;
        if let Some(r) = self.plm(p).memory.get_mut(&o) {
            r.time_stamp = time;
            return;
        }
        let snapshot = Arc::new(self.e(o).clone());
        self.plm(p).memory.insert(
            o,
            Remembrance {
                time_stamp: time,
                snapshot,
            },
        );
    }

    /// Whether the player remembers the entity.
    pub fn remembers(&self, p: PlayerId, o: EntityId) -> bool {
        self.pl(p).memory.contains_key(&o)
    }

    fn update_menace(&mut self, p: PlayerId) {
        let m = self
            .pl(p)
            .units
            .iter()
            .map(|&u| self.e(u))
            .filter(|e| e.stats.speed > 0 && e.class == EntityClass::Soldier)
            .map(Entity::menace)
            .sum();
        self.plm(p).menace = m;
    }

    pub(crate) fn update_enemy_menace_and_presence_and_corpses(&mut self, p: PlayerId) {
        let mut menace: BTreeMap<SquareId, i64> = BTreeMap::new();
        let mut presence: Vec<SquareId> = Vec::new();
        let mut corpses = BTreeSet::new();
        let mut friends = BTreeSet::new();
        let mut cataclysmic = BTreeSet::new();
        let allied = self.pl(p).allied.clone();
        let mut perceived: Vec<EntityId> = self.pl(p).perception.iter().copied().collect();
        perceived.sort_by(|a, b| a.cmp_as_text(*b));
        let mut remembered: Vec<Arc<Entity>> = self
            .pl(p)
            .memory
            .values()
            .map(|r| r.snapshot.clone())
            .collect();
        remembered.sort_by(|a, b| a.id.cmp_as_text(b.id));
        let entities = perceived
            .iter()
            .map(|&o| self.e(o))
            .chain(remembered.iter().map(|r| &**r));
        for o in entities {
            let Some(Place::Square(mut place)) = o.place else {
                continue;
            };
            if self.player_is_an_enemy_of_entity(p, o) {
                let m = o.menace();
                if let Some(v) = menace.get_mut(&place) {
                    *v += m;
                } else {
                    menace.insert(place, m);
                    presence.push(place);
                }
                if !o.is_melee() {
                    // The loop variable is `place` itself in the original, so
                    // after the loop `place` is the last neighbor.
                    for n in self.square(place).neighbors.clone() {
                        *menace.entry(n).or_insert(0) += floordiv(m, 10);
                        place = n;
                    }
                }
            } else if o.is_corpse() {
                corpses.insert(place);
            } else if o.player.is_some_and(|op| allied.contains(&op)) && o.stats.is_vulnerable != 0
            {
                friends.insert(place);
            }
            if o.time_limit.is_some_and(|t| t != 0) && o.stats.harm_level != 0 {
                cataclysmic.insert(place);
            }
        }
        let pl = self.plm(p);
        pl.enemy_menace = menace;
        pl.enemy_presence = presence;
        pl.places_with_corpses = corpses;
        pl.places_with_friends = friends;
        pl.cataclysmic_places = cataclysmic;
    }

    fn update_drowning(&mut self, p: PlayerId) -> Res {
        for u in self.pl(p).units.clone() {
            let e = self.e(u);
            let on_ground = match e.place {
                Some(Place::Square(s)) => self.square(s).is_ground,
                _ => true,
            };
            if e.stats.is_vulnerable != 0 && e.airground_type == AirGround::Ground && !on_ground {
                self.die(u, None)?;
            }
        }
        Ok(())
    }

    pub(crate) fn update_counterattacks(&mut self, p: PlayerId) -> Res {
        let places = std::mem::take(&mut self.plm(p).counterattack_places);
        let mut nearby: BTreeMap<SquareId, SquareId> = BTreeMap::new();
        for &ap in &places {
            for &n in &self.square(ap).neighbors {
                if !places.contains(&n) {
                    nearby.insert(n, ap);
                }
            }
        }
        for u in self.pl(p).units.clone() {
            if let Some(Place::Square(s)) = self.e(u).place
                && let Some(&ap) = nearby.get(&s)
            {
                self.counterattack(u, ap)?;
            }
        }
        Ok(())
    }

    /// `on_unit_attacked`: humans and random computers prepare
    /// counterattacks; scripted computers let their AI react.
    pub(crate) fn on_unit_attacked(&mut self, p: PlayerId, unit: EntityId, attacker: EntityId) {
        if matches!(self.pl(p).kind, PlayerKind::Computer { .. }) {
            if let Some(mut ai) = self.plm(p).ai.take() {
                ai.on_unit_attacked(self, p, unit, attacker);
                self.plm(p).ai = Some(ai);
            }
            return;
        }
        if self.pl(p).perception.contains(&attacker)
            && let Some(Place::Square(s)) = self.e(attacker).place
            && !self.pl(p).counterattack_places.contains(&s)
        {
            self.plm(p).counterattack_places.push(s);
        }
    }

    /// The squares where units of the player will counterattack at the next
    /// update.
    pub fn counterattack_places(&self, p: PlayerId) -> &[SquareId] {
        &self.pl(p).counterattack_places
    }

    /// `time_has_come` of a computer player, for `wait` orders.
    pub(crate) fn time_has_come(&mut self, p: PlayerId, target: Option<&Target>) -> Res<bool> {
        if !matches!(self.pl(p).kind, PlayerKind::Computer { .. }) {
            return Err(Abort::new(
                "'Player' object has no attribute 'time_has_come'",
            ));
        }
        let Some(Target::Square(s)) = target else {
            return Ok(false);
        };
        if self.pl(p).cataclysmic_places.contains(s) {
            return Ok(false);
        }
        let ai = self.plm(p).ai.take();
        let r = ai.as_ref().is_some_and(|ai| ai.time_has_come(self, p, *s));
        self.plm(p).ai = ai;
        Ok(r)
    }

    // lookups

    /// `has`: an upgrade, or a unit of the type or derived from it.
    pub fn has(&self, p: PlayerId, type_name: &str) -> bool {
        let pl = self.pl(p);
        if pl.cheat_has_all || pl.upgrades.iter().any(|u| u == type_name) {
            return true;
        }
        pl.units.iter().any(|&u| {
            let e = self.e(u);
            *e.type_name == *type_name || self.expanded_is_a(e).iter().any(|t| t == type_name)
        })
    }

    pub fn has_all(&self, p: PlayerId, type_names: &[String]) -> bool {
        type_names.iter().all(|t| self.has(p, t))
    }

    /// `get_object_by_id`: what an id given in a command means to a player:
    /// a subsquare (`zoom-<square>-<x>-<y>`), a square (by name or id), a
    /// perceived entity, or a remembered one.
    pub fn get_object_by_id(&self, p: PlayerId, i: &str) -> Result<Option<Target>, Abort> {
        if i.starts_with("zoom") {
            let parts: Vec<&str> = i.split('-').collect();
            if parts.len() != 4 {
                return Err(Abort::new("not enough values to unpack"));
            }
            let place = self.get_object_by_id(p, parts[1])?;
            let Some(Target::Square(s)) = place else {
                return Err(Abort::new("object has no attribute 'title'"));
            };
            let x = py_int(parts[2]).map_err(|_| Abort::new("invalid literal for int()"))?;
            let y = py_int(parts[3]).map_err(|_| Abort::new("invalid literal for int()"))?;
            return Ok(Some(Target::Zoom(ZoomTarget {
                place: s,
                x,
                y,
                id: Some(i.to_owned()),
            })));
        }
        if let Some(s) = self.square_by_name(i) {
            return Ok(Some(Target::Square(s)));
        }
        let Ok(n) = i.parse::<u32>() else {
            return Ok(None);
        };
        if n.to_string() != i {
            return Ok(None);
        }
        if let Some(s) = self.square_by_obj_id(n) {
            return Ok(Some(Target::Square(s)));
        }
        let id = EntityId(n);
        if let Some(e) = self.entity(id)
            && e.place.is_some()
            && self.pl(p).perception.contains(&id)
        {
            return Ok(Some(Target::Entity(id)));
        }
        if let Some(r) = self.pl(p).memory.get(&id) {
            return Ok(Some(Target::Memory(MemoryRef {
                owner: p,
                snapshot: r.snapshot.clone(),
            })));
        }
        Ok(None)
    }

    /// `updated_target`: the target if still perceived or remembered, else
    /// what its id means now.
    pub(crate) fn updated_target(&self, p: PlayerId, t: &Target) -> Res<Option<Target>> {
        match t {
            Target::Zoom(_) | Target::Square(_) => return Ok(Some(t.clone())),
            Target::Entity(id) => {
                if self.pl(p).perception.contains(id) && self.e(*id).place.is_some() {
                    return Ok(Some(t.clone()));
                }
            }
            Target::Memory(m) => {
                if m.owner == p && self.memory_is_current(m) && m.snapshot.place.is_some() {
                    return Ok(Some(t.clone()));
                }
            }
            Target::Xy(..) => return Err(Abort::new("'tuple' object has no attribute 'id'")),
        }
        let id = t.entity_id().expect("an entity");
        self.get_object_by_id(p, &id.to_string())
    }

    // resources

    pub(crate) fn pay(&mut self, p: PlayerId, cost: &[i64]) {
        let r = &mut self.plm(p).resources;
        for (i, &c) in cost.iter().enumerate() {
            if let Some(x) = r.get_mut(i) {
                *x -= c;
            }
        }
    }

    pub(crate) fn unpay(&mut self, p: PlayerId, cost: &[i64]) {
        let neg: Vec<i64> = cost.iter().map(|c| -c).collect();
        self.pay(p, &neg);
    }

    /// `store`: adds a gathered quantity, with the storage bonus.
    pub(crate) fn store(&mut self, p: PlayerId, resource: i64, qty: i64) {
        let r = resource as usize;
        let bonus = self.pl(p).storage_bonus.get(r).copied().unwrap_or(0);
        let qty = qty + bonus;
        let pl = self.plm(p);
        if let Some(x) = pl.resources.get_mut(r) {
            *x += qty;
        }
        pl.stats
            .add(StatEvent::Gathered, StatTarget::Resource(r), qty);
    }

    pub(crate) fn free_resources(&mut self, p: PlayerId, project: &Order) -> Res {
        let i = self
            .pl(p)
            .budget
            .iter()
            .position(|b| b.uid == project.uid || self.order_eq(b, project));
        if let Some(i) = i {
            let unit_cost = self
                .rules
                .unit_type(project.type_name.as_deref().unwrap_or(""))
                .map(|t| t.cost.clone());
            let cost =
                unit_cost.ok_or_else(|| Abort::new("'NoneType' object has no attribute 'cost'"))?;
            self.unpay(p, &cost);
            self.plm(p).budget.remove(i);
        }
        Ok(())
    }

    fn free_project_resources_if_no_worker_on_project(&mut self, p: PlayerId) -> Res {
        // The budget shrinks while it is walked, so the project after a freed
        // one is skipped, as in the original.
        let mut i = 0;
        while i < self.pl(p).budget.len() {
            let project = self.pl(p).budget[i].clone();
            let worker = self
                .pl(p)
                .units
                .iter()
                .any(|&u| self.e(u).orders.iter().any(|o| self.order_eq(o, &project)));
            if !worker {
                self.free_resources(p, &project)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// `future_count`: units of a type, building sites for it, and units
    /// being trained or upgraded to it.
    pub fn future_count(&self, p: PlayerId, type_name: &str) -> i64 {
        let mut result = 0;
        for &u in &self.pl(p).units {
            let e = self.e(u);
            let site_of = e
                .site
                .as_ref()
                .map(|s| self.rules.ty(s.building_type).name.as_str());
            if *e.type_name == *type_name
                || (*e.type_name == *"buildingsite" && site_of == Some(type_name))
            {
                result += 1;
            }
            for o in &e.orders {
                if matches!(o.keyword, Keyword::Train | Keyword::UpgradeTo)
                    && o.type_name.as_deref() == Some(type_name)
                {
                    result += 1;
                }
            }
        }
        result
    }

    /// `check_count_limit`: whether the player may have one more of the
    /// type.
    pub fn check_count_limit(&self, p: PlayerId, type_name: &str) -> bool {
        let Some(t) = self.rules.unit_type(type_name) else {
            return false;
        };
        if t.stats.count_limit == 0 {
            return true;
        }
        self.future_count(p, &t.name) < t.stats.count_limit
    }

    /// `nearest_warehouse`: the allied unit storing the resource with the
    /// shortest path from `place`; one without path comes last.
    pub fn nearest_warehouse(
        &mut self,
        p: PlayerId,
        place: Place,
        resource_type: i64,
        include_building_sites: bool,
    ) -> Res<Option<EntityId>> {
        let Place::Square(start) = place else {
            return Err(Abort::new(
                "'Inside' object has no attribute 'shortest_path_distance_to'",
            ));
        };
        let mut warehouses: Vec<(i64, EntityId)> = Vec::new();
        for a in self.pl(p).allied.clone() {
            for u in self.pl(a).units.clone() {
                let e = self.e(u);
                let stores = e.type_id.is_some_and(|t| {
                    self.rules
                        .ty(t)
                        .storable_resource_types
                        .contains(&resource_type)
                });
                let site_stores = include_building_sites
                    && e.site.as_ref().is_some_and(|s| {
                        self.rules
                            .ty(s.building_type)
                            .storable_resource_types
                            .contains(&resource_type)
                    });
                if stores || site_stores {
                    let d = match e.place {
                        Some(Place::Square(s)) => self.shortest_path_distance_to(
                            start,
                            s,
                            Some(p),
                            AirGround::Ground,
                            false,
                        )?,
                        _ => return Err(Abort::new("the warehouse is not on a square")),
                    };
                    if d == 0 {
                        return Ok(Some(u));
                    }
                    warehouses.push((d, u));
                }
            }
        }
        warehouses.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp_as_text(b.1)));
        Ok(warehouses.first().map(|w| w.1))
    }

    /// Distance used when no path exists.
    pub fn no_path() -> i64 {
        INF
    }

    // victory and defeat

    pub(crate) fn send_voice_important(&mut self, p: PlayerId, m: Msg) {
        self.push(p, Event::VoiceImportant(m));
    }

    fn broadcast_to_others_only(&mut self, p: PlayerId, m: &Msg) {
        for op in self.players.clone() {
            if op != p {
                self.send_voice_important(op, m.clone());
            }
        }
    }

    /// `quit_game`: the player leaves; its units disappear.
    pub fn quit_game(&mut self, p: PlayerId) {
        self.push(p, Event::Quit);
        if self.true_players().contains(&p) {
            let mut m = self.pl(p).name();
            m.extend(msg(mp::HAS_JUST_QUIT_GAME));
            self.broadcast_to_others_only(p, &m);
        }
        for u in self.pl(p).units.clone() {
            self.delete(u);
        }
        match self.players.iter().position(|&x| x == p) {
            Some(i) => {
                self.players.remove(i);
                self.ex_players.push(p);
            }
            None => self.warn("list.remove(x): x not in list"),
        }
    }

    /// `defeat`: the player loses its units, then watches or leaves.
    pub fn defeat(&mut self, p: PlayerId, force_quit: bool) {
        let time = self.time;
        self.plm(p).has_been_defeated = true;
        self.plm(p).stats.freeze(time);
        if self.true_players().contains(&p) {
            let mut m = self.pl(p).name();
            m.extend(msg(mp::HAS_BEEN_DEFEATED));
            self.broadcast_to_others_only(p, &m);
        }
        for u in self.pl(p).units.clone() {
            self.delete(u);
        }
        if force_quit {
            self.quit_game(p);
        } else if self.pl(p).observer_if_defeated && !self.true_playing_players().is_empty() {
            let mut m = msg(mp::YOU_HAVE_BEEN_DEFEATED);
            if self.at_least_two_camps() {
                m.extend(msg(mp::YOU_ARE_NOW_IN_OBSERVER_MODE));
            }
            self.send_voice_important(p, m);
        } else {
            self.quit_game(p);
        }
    }

    /// `victory`: the player and its allies win; the others lose.
    pub fn victory(&mut self, p: PlayerId) {
        let time = self.time;
        // The list shrinks while it is walked when a loser quits, so the
        // next player is skipped, as in the original.
        let mut i = 0;
        while i < self.players.len() {
            let op = self.players[i];
            if self.pl(op).is_playing() {
                if self.pl(p).allied.contains(&op) {
                    self.plm(op).has_victory = true;
                    self.plm(op).stats.freeze(time);
                } else {
                    self.defeat(op, false);
                }
            }
            i += 1;
        }
    }

    // creation

    /// `add_unit` of a starting unit: on a meadow if it needs one.
    pub(crate) fn add_start_unit(&mut self, p: PlayerId, type_name: &str, place: SquareId) {
        let Some(tid) = self.rules.type_id(type_name) else {
            return;
        };
        let t = self.rules.ty(tid).clone();
        if t.entity_class().is_none_or(|c| c == EntityClass::Deposit) {
            self.warn(format!("couldn't create an initial unit: {type_name}"));
            return;
        }
        let (x, y, land) = self.find_and_remove_meadow(place, t.stats.is_buildable_anywhere != 0);
        let Some((x, y)) = self.find_free_space(place, t.airground_type, x, y) else {
            return;
        };
        if let Ok(u) = self.create_creature(tid, Some(p), Place::Square(place), x, y) {
            self.em(u).building_land = land;
        }
    }

    /// The menace of a player's mobile soldiers.
    pub fn player_menace(&self, p: PlayerId) -> i64 {
        self.pl(p).menace
    }

    /// Whether `o` is an enemy of the player.
    pub fn is_enemy_entity(&self, p: PlayerId, o: EntityId) -> bool {
        self.player_is_an_enemy_of_entity(p, self.e(o))
    }

    /// The enemy squares the player knows, in discovery order.
    pub fn enemy_presence(&self, p: PlayerId) -> &[SquareId] {
        &self.pl(p).enemy_presence
    }

    pub fn places_with_corpses(&self, p: PlayerId) -> &BTreeSet<SquareId> {
        &self.pl(p).places_with_corpses
    }

    pub fn places_with_friends(&self, p: PlayerId) -> &BTreeSet<SquareId> {
        &self.pl(p).places_with_friends
    }

    pub fn cataclysmic_places(&self, p: PlayerId) -> &BTreeSet<SquareId> {
        &self.pl(p).cataclysmic_places
    }
}
