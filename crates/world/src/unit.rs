//! What units, buildings, deposits and corpses do (`worldunit.py`,
//! `worldaction.py`, `worldresource.py`, `worldupgrade.py`).

use crate::entity::{
    Action, ActionKind, AiMode, Entity, EntityId, Place, PlayerId, SiteData, SquareId, Target,
};
use crate::events::{EntityEvent, Reason};
use crate::nofloat::{
    PRECISION, floordiv, int_angle, int_cos_1000, int_distance, int_sin_1000, py_int,
    square_of_distance,
};
use crate::pathfinding::{Node, Viewer};
use crate::player::StatEvent;
use crate::rules::{AirGround, EntityClass, TypeId, VIRTUAL_TIME_INTERVAL};
use crate::world::{Abort, NotEnoughSpace, Res, TPlace, World};

/// Extra distance allowed when reaching a target, in millimeters.
pub const DISTANCE_MARGIN: i64 = 175;

impl World {
    // creation

    /// Creates a unit or a building of a rules type (`Creature.__init__`).
    pub(crate) fn create_creature(
        &mut self,
        type_id: TypeId,
        player: Option<PlayerId>,
        place: Place,
        x: i64,
        y: i64,
    ) -> Result<EntityId, NotEnoughSpace> {
        let t = self.rules.ty(type_id);
        let class = t.entity_class().ok_or(NotEnoughSpace)?;
        let mut e = Entity::new(class, &t.name);
        e.type_id = Some(type_id);
        e.stats = t.stats.clone();
        e.actual_speed = t.stats.speed;
        e.airground_type = t.airground_type;
        self.create_creature_from(e, player, place, x, y)
    }

    fn create_creature_from(
        &mut self,
        e: Entity,
        player: Option<PlayerId>,
        place: Place,
        x: i64,
        y: i64,
    ) -> Result<EntityId, NotEnoughSpace> {
        let (x, y) = self.initial_position(
            e.stats.collision != 0,
            e.airground_type,
            place,
            Some((x, y)),
        )?;
        let id = self.insert_entity(e, place, x, y, 90);
        self.set_player(id, player);
        let md = self.rules.minimal_damage();
        let time = self.time;
        let e = self.em(id);
        if e.stats.transport_capacity != 0 {
            e.inside = Some(Vec::new());
        }
        e.hp = e.stats.hp_max;
        e.mana = if e.stats.mana_start > 0 {
            e.stats.mana_start
        } else {
            e.stats.mana_max
        };
        e.minimal_damage = md;
        if e.stats.decay != 0 {
            e.time_limit = Some(time + e.stats.decay);
        }
        Ok(id)
    }

    /// An entity of a type that is not in the world (`create_from_nowhere`):
    /// only its type's attributes are set.
    pub fn entity_from_nowhere(&self, type_name: &str) -> Option<Entity> {
        let t = self.rules.unit_type(type_name)?;
        let mut e = Entity::new(t.entity_class()?, &t.name);
        e.type_id = self.rules.type_id(type_name);
        e.stats = t.stats.clone();
        e.airground_type = t.airground_type;
        Some(e)
    }

    /// The damage of an attack between two entities, before armor.
    pub fn base_damage_versus_entities(&self, attacker: &Entity, target: &Entity) -> i64 {
        self.base_damage_versus_of(attacker, target)
    }

    /// The armor of an entity against an attacker.
    pub fn armor_versus_entities(&self, target: &Entity, attacker: &Entity) -> i64 {
        self.armor_versus_of(target, attacker)
    }

    /// Creates a building site for `building_type`, and pays it.
    pub(crate) fn create_building_site(
        &mut self,
        player: PlayerId,
        place: Place,
        x: i64,
        y: i64,
        building_type: TypeId,
    ) -> Result<EntityId, NotEnoughSpace> {
        let e = Entity::new(EntityClass::BuildingSite, "buildingsite");
        let id = self.create_creature_from(e, Some(player), place, x, y)?;
        let bt = self.rules.ty(building_type).clone();
        self.pay(player, &bt.cost);
        let e = self.em(id);
        e.stats.hp_max = bt.stats.hp_max;
        e.stats.is_buildable_anywhere = bt.stats.is_buildable_anywhere;
        e.stats.is_buildable_on_exits_only = bt.stats.is_buildable_on_exits_only;
        e.stats.is_buildable_near_water_only = bt.stats.is_buildable_near_water_only;
        e.stats.is_a_gate = bt.stats.is_a_gate;
        e.stats.time_cost = bt.stats.time_cost;
        let starting_hp = bt.stats.hp_max * 5 / 100;
        e.hp = starting_hp;
        e.site = Some(SiteData {
            building_type,
            starting_hp,
            timer: floordiv(bt.stats.time_cost, VIRTUAL_TIME_INTERVAL),
            damage_during_construction: 0,
        });
        Ok(id)
    }

    /// Creates a deposit with a quantity written as in a map.
    pub(crate) fn create_deposit(
        &mut self,
        type_id: TypeId,
        square: SquareId,
        qty: &str,
    ) -> Res<EntityId> {
        let t = self.rules.ty(type_id);
        let mut e = Entity::new(EntityClass::Deposit, &t.name);
        e.type_id = Some(type_id);
        e.stats = t.stats.clone();
        e.airground_type = t.airground_type;
        let q = crate::nofloat::to_int(qty)
            .map_err(|_| Abort::new(format!("could not convert string to float: {qty:?}")))?;
        e.stats.qty = q;
        e.qty_max = q;
        if t.resource_type == Some(1) {
            e.resource_regen = 10;
        }
        let place = Place::Square(square);
        let (x, y) = self
            .initial_position(e.stats.collision != 0, e.airground_type, place, None)
            .map_err(|_| Abort::new("NotEnoughSpaceError"))?;
        Ok(self.insert_entity(e, place, x, y, 90))
    }

    /// Creates a meadow on a square, at the center or at a point.
    pub(crate) fn create_meadow(&mut self, square: SquareId, xy: Option<(i64, i64)>) -> EntityId {
        let e = Entity::new(EntityClass::Meadow, "meadow");
        let place = Place::Square(square);
        let (x, y) = xy.unwrap_or((self.square(square).x, self.square(square).y));
        self.insert_entity(e, place, x, y, 90)
    }

    /// Leaves a corpse where a unit dies.
    fn create_corpse(&mut self, unit: EntityId) {
        let mut copy = self.e(unit).clone();
        // The original's shallow copy shares the order list and the
        // interior, which the death empties.
        copy.orders.clear();
        if copy.inside.is_some() {
            copy.inside = Some(Vec::new());
        }
        let Some(place) = copy.place else { return };
        let (x, y) = (copy.x, copy.y);
        let mut e = Entity::new(EntityClass::Corpse, "corpse");
        e.corpse_unit = Some(Box::new(copy));
        let id = self.insert_entity(e, place, x, y, 90);
        let t = self.time + 300 * PRECISION;
        self.em(id).time_limit = Some(t);
    }

    // ownership

    /// `set_player`: stops the unit, cancels its orders and gives it to
    /// another player (or to nobody).
    pub(crate) fn set_player(&mut self, id: EntityId, player: Option<PlayerId>) {
        self.stop(id);
        self.cancel_all_orders(id, false);
        if let Some(old) = self.e(id).player {
            self.player_remove(old, id);
        } else if let Some(p) = player
            && let Some(st) = self.e(id).stat_type()
        {
            self.plm(p).stats.add(StatEvent::Produced, st.into(), 1);
        }
        self.em(id).player = player;
        if let Some(p) = player {
            self.player_add(p, id);
        }
        if let Some(inside) = self.e(id).inside.clone() {
            for o in inside {
                self.set_player(o, player);
            }
        }
    }

    fn next_free_number(&self, id: EntityId, p: PlayerId) -> i64 {
        let name = &self.e(id).type_name;
        let numbers: Vec<i64> = self
            .pl(p)
            .units
            .iter()
            .filter(|&&u| u != id && self.e(u).type_name == *name)
            .filter_map(|&u| self.e(u).number)
            .collect();
        let mut n = 1;
        while numbers.contains(&n) {
            n += 1;
        }
        n
    }

    /// `Player.add`.
    fn player_add(&mut self, p: PlayerId, id: EntityId) {
        let n = self.next_free_number(id, p);
        self.em(id).number = Some(n);
        self.plm(p).units.push(id);
        let (fp, fc) = (self.e(id).stats.food_provided, self.e(id).stats.food_cost);
        self.plm(p).food += fp;
        self.plm(p).used_food += fc;
        if let Err(Abort(m)) = self.upgrade_to_player_level(id) {
            self.warn(m);
        }
        for u in self.pl(p).units.clone() {
            if matches!(self.e(u).action_target(), Some(Target::Entity(t)) if *t == id) {
                self.stop(u);
            }
        }
        for a in self.pl(p).allied.clone() {
            self.plm(a).perception.insert(id);
        }
    }

    /// `Player.remove`.
    fn player_remove(&mut self, p: PlayerId, id: EntityId) {
        if let Some(i) = self.pl(p).units.iter().position(|&u| u == id) {
            self.plm(p).units.remove(i);
        }
        let (fp, fc) = (self.e(id).stats.food_provided, self.e(id).stats.food_cost);
        self.plm(p).food -= fp;
        self.plm(p).used_food -= fc;
    }

    // upgrades

    fn upgrade_to_player_level(&mut self, id: EntityId) -> Res {
        let Some(p) = self.e(id).player else {
            return Ok(());
        };
        for upg in self.can_use(id) {
            if self.pl(p).upgrades.contains(&upg) {
                for level in 0..self.pl(p).level(&upg) {
                    self.apply_upgrade_effect(&upg, id, level)?;
                }
            }
        }
        Ok(())
    }

    /// `Upgrade.upgrade_player`: applies the upgrade to the player's units
    /// that can use it, then records it.
    pub(crate) fn upgrade_player(&mut self, upgrade: &str, p: PlayerId) -> Res {
        let level = self.pl(p).level(upgrade);
        for u in self.pl(p).units.clone() {
            if self.can_use(u).iter().any(|c| c == upgrade) {
                self.apply_upgrade_effect(upgrade, u, level)?;
            }
        }
        self.plm(p).upgrades.push(upgrade.to_owned());
        Ok(())
    }

    fn apply_upgrade_effect(&mut self, upgrade: &str, unit: EntityId, start_level: i64) -> Res {
        let effect = self
            .rules
            .unit_type(upgrade)
            .and_then(|t| t.effect.clone())
            .ok_or_else(|| Abort::new(format!("{upgrade} has no effect")))?;
        let arg = |i: usize| -> Res<&String> {
            effect
                .get(i)
                .ok_or_else(|| Abort::new(format!("missing argument in effect of {upgrade}")))
        };
        match effect.first().map(String::as_str) {
            Some("bonus") => {
                let stat = arg(1)?.clone();
                let base = py_int(arg(2)?).map_err(|_| Abort::new("invalid bonus"))?;
                let incr = match effect.get(3) {
                    Some(s) => py_int(s).map_err(|_| Abort::new("invalid bonus"))?,
                    None => 0,
                };
                self.effect_bonus(unit, start_level, &stat, base, incr)
            }
            Some("apply_bonus") => {
                let stat = arg(1)?.clone();
                let base = self
                    .e(unit)
                    .type_id
                    .map_or(0, |t| self.rules.ty(t).bonus(&stat));
                self.effect_bonus(unit, start_level, &stat, base, 0)?;
                if stat == "damage" {
                    self.em(unit).damage_level += 1;
                }
                Ok(())
            }
            other => Err(Abort::new(format!("unknown upgrade effect {other:?}"))),
        }
    }

    fn effect_bonus(
        &mut self,
        unit: EntityId,
        start_level: i64,
        stat: &str,
        base: i64,
        incr: i64,
    ) -> Res {
        let e = self.em(unit);
        let v = e
            .stats
            .get_mut(stat)
            .ok_or_else(|| Abort::new(format!("no attribute {stat}")))?;
        *v += base + incr * start_level;
        if stat == "damage" {
            e.damage_level += 1;
        }
        Ok(())
    }

    // type lists

    fn type_of(&self, id: EntityId) -> Option<&crate::rules::UnitType> {
        self.e(id).type_id.map(|t| self.rules.ty(t))
    }

    pub(crate) fn can_use(&self, id: EntityId) -> Vec<String> {
        self.type_of(id)
            .map(|t| t.can_use.clone())
            .unwrap_or_default()
    }

    /// The list of a type attribute (`can_train`...) of an entity.
    pub(crate) fn type_list(&self, id: EntityId, attr: &str) -> Vec<String> {
        let Some(t) = self.type_of(id) else {
            return Vec::new();
        };
        match attr {
            "can_build" => t.can_build.clone(),
            "can_train" => t.can_train.clone(),
            "can_use" => t.can_use.clone(),
            "can_research" => t.can_research.clone(),
            "can_upgrade_to" => t.can_upgrade_to.clone(),
            _ => Vec::new(),
        }
    }

    /// The cost of an entity's type: zeros for building sites and others.
    pub(crate) fn entity_cost(&self, id: EntityId) -> Vec<i64> {
        match self.type_of(id) {
            Some(t) if !t.cost.is_empty() => t.cost.clone(),
            _ => vec![0; crate::rules::MAX_NB_OF_RESOURCE_TYPES],
        }
    }

    pub(crate) fn expanded_is_a(&self, e: &Entity) -> Vec<String> {
        e.type_id
            .map(|t| self.rules.ty(t).expanded_is_a.clone())
            .unwrap_or_default()
    }

    /// What a unit is busy with, for its noise: `building`, or
    /// `exploiting_<deposit type>`, or nothing.
    pub fn activity(&self, id: EntityId) -> Option<String> {
        let o = self.e(id).orders.first()?;
        match o.mode {
            crate::orders::Mode::Build => Some("building".into()),
            crate::orders::Mode::Gather => {
                let name = match o.target.as_ref()? {
                    Target::Square(s) => self.square(*s).type_name.clone(),
                    t => self.t_entity(t)?.type_name.to_string(),
                };
                Some(format!("exploiting_{name}"))
            }
            _ => None,
        }
    }

    /// The upgrades of the unit's player that apply to it.
    pub fn unit_upgrades(&self, id: EntityId) -> Vec<String> {
        let Some(p) = self.e(id).player else {
            return Vec::new();
        };
        self.can_use(id)
            .into_iter()
            .filter(|u| self.pl(p).upgrades.contains(u))
            .collect()
    }

    // geometry

    pub(crate) fn place_high_ground(&self, place: Place) -> bool {
        match place {
            Place::Square(s) => self.square(s).high_ground,
            Place::Inside(c) => {
                let ce = self.e(c);
                if ce.airground_type == AirGround::Air {
                    return true;
                }
                ce.place.is_some_and(|p| self.place_high_ground(p))
            }
        }
    }

    /// `height`: 2 in the air, else the height of the place plus the
    /// bonus height.
    pub(crate) fn height_of(&self, e: &Entity) -> Res<i64> {
        if e.airground_type == AirGround::Air {
            return Ok(2);
        }
        let ph = match e.place {
            Some(Place::Square(s)) => self.square(s).height(),
            Some(Place::Inside(c)) => self.height_of(self.e(c))?,
            None => return Err(Abort::new("'NoneType' object has no attribute 'height'")),
        };
        Ok(ph + e.stats.bonus_height)
    }

    pub fn height(&self, id: EntityId) -> Option<i64> {
        self.height_of(self.e(id)).ok()
    }

    pub(crate) fn nearest_water(&self, id: EntityId) -> Option<SquareId> {
        let e = self.e(id);
        let s = e.square()?;
        let mut best: Option<(i64, SquareId)> = None;
        for &sq in &self.square(s).strict_neighbors {
            let q = self.square(sq);
            if !q.is_water {
                continue;
            }
            let d = square_of_distance(q.x, q.y, e.x, e.y);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, sq));
            }
        }
        best.map(|(_, s)| s)
    }

    /// `get_observed_squares`: the place, and the neighbors not higher,
    /// unless a wall closes the way. Checking a wall reveals its blockers.
    pub(crate) fn get_observed_squares(
        &mut self,
        id: EntityId,
        strict: bool,
    ) -> Res<Vec<SquareId>> {
        if let Some(cached) = self
            .observed_cache
            .as_ref()
            .and_then(|c| c.get(&(id, strict)))
        {
            return Ok(cached.clone());
        }
        let e = self.e(id);
        let Some(Place::Square(place)) = e.place else {
            return Ok(Vec::new());
        };
        let mut result = vec![place];
        if strict && e.stats.sight_range < self.square_width() {
            return Ok(result);
        }
        let h = self.height_of(self.e(id))?;
        for sq in self.square(place).neighbors.clone() {
            let sh = self.square(sq).height();
            let visible = h > sh
                || (h == sh
                    && (self.can_go(id, Some(Place::Square(sq)), false, true)
                        || self.square(sq).is_water
                        || self.square(place).is_water));
            if visible {
                result.push(sq);
            }
        }
        if let Some(c) = self.observed_cache.as_mut() {
            c.insert((id, strict), result.clone());
        }
        Ok(result)
    }

    // movement

    fn already_walked(&self, id: EntityId, x: i64, y: i64) -> i64 {
        let e = self.e(id);
        let r2 = e.radius() * e.radius();
        e.walked
            .iter()
            .filter(|(lw, xw, yw, _)| *lw == e.place && square_of_distance(x, y, *xw, *yw) < r2)
            .map(|w| w.3)
            .sum()
    }

    fn future_coords(&self, id: EntityId, rotation: i64, target_d: i64) -> (i64, i64) {
        let e = self.e(id);
        let mut d = floordiv(e.actual_speed * VIRTUAL_TIME_INTERVAL, 1000);
        if rotation == 0 {
            d = d.min(target_d);
        }
        let a = e.o + rotation;
        (
            e.x + floordiv(d * int_cos_1000(a), 1000),
            e.y + floordiv(d * int_sin_1000(a), 1000),
        )
    }

    fn heuristic_value(&self, id: EntityId, rotation: i64, target_d: i64) -> i64 {
        let (x, y) = self.future_coords(id, rotation, target_d);
        rotation.abs() + self.already_walked(id, x, y) * 200
    }

    /// `_can_go`: whether the unit can go from its square to `new_place`,
    /// directly or through one square. A blocked exit reveals its blockers.
    pub(crate) fn can_go(
        &mut self,
        id: EntityId,
        new_place: Option<Place>,
        ignore_blockers: bool,
        ignore_forests: bool,
    ) -> bool {
        let Some(np) = new_place else { return false };
        let e = self.e(id);
        if e.airground_type != AirGround::Ground {
            return true;
        }
        if Some(np) == e.place {
            return true;
        }
        let Some(Place::Square(here)) = e.place else {
            return false;
        };
        let player = e.player;
        for ex in self.square(here).exits.clone() {
            let Some(os) = self.exit_other_side(ex) else {
                continue;
            };
            let os_place = self.e(os).place;
            if os_place == Some(np) {
                if ignore_blockers {
                    return true;
                }
                if self.exit_is_blocked(ex, Viewer::Unit(id), false, ignore_forests) {
                    if let Some(p) = player {
                        for b in self.exit_blockers(ex) {
                            self.observe(p, b);
                        }
                    }
                } else {
                    return true;
                }
            } else if let Some(Place::Square(next)) = os_place {
                for e2 in self.square(next).exits.clone() {
                    let Some(os2) = self.exit_other_side(e2) else {
                        continue;
                    };
                    if self.e(os2).place == Some(np)
                        && (ignore_blockers
                            || !self.exit_is_blocked(e2, Viewer::Unit(id), false, ignore_forests))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn mark_the_dead_end(&mut self, id: EntityId) {
        let e = self.em(id);
        let w = (e.place, e.x, e.y, 5);
        e.walked.push(w);
    }

    fn position_contains(&self, t: &Target, x: i64, y: i64) -> bool {
        match t {
            Target::Square(s) => self.square(*s).contains(x, y),
            Target::Zoom(z) => self.zoom_contains(z, x, y),
            _ => false,
        }
    }

    pub(crate) fn zoom_contains(&self, z: &crate::entity::ZoomTarget, x: i64, y: i64) -> bool {
        self.get_subsquare_id_from_xy(z.x, z.y) == self.get_subsquare_id_from_xy(x, y)
    }

    fn must_hold(&self, id: EntityId) -> bool {
        let e = self.e(id);
        let smart = e.player.is_some_and(|p| self.pl(p).smart_units());
        !(smart || e.ai_mode == Some(AiMode::Defensive))
            && e.position_to_hold
                .as_ref()
                .is_some_and(|t| self.position_contains(t, e.x, e.y))
    }

    fn must_not_go_to(&self, id: EntityId, x: i64, y: i64) -> bool {
        self.must_hold(id)
            && !self
                .e(id)
                .position_to_hold
                .as_ref()
                .is_some_and(|t| self.position_contains(t, x, y))
    }

    fn would_collide_if(&self, id: EntityId, x: i64, y: i64) -> Res<bool> {
        let e = self.e(id);
        if e.stats.collision == 0 {
            return Ok(false);
        }
        if !matches!(e.place, Some(Place::Square(_))) {
            return Err(Abort::new(
                "'Inside' object has no attribute 'would_collide'",
            ));
        }
        let mut m = self.collision(e.airground_type).clone();
        m.remove(e.x, e.y);
        Ok(m.would_collide(x, y))
    }

    fn try_move(&mut self, id: EntityId, rotation: i64, target_d: i64) -> Res<bool> {
        let (x, y) = self.future_coords(id, rotation, target_d);
        let new_place = self.get_place_from_xy(x, y);
        if self.must_not_go_to(id, x, y) {
            return Ok(false);
        }
        if self.can_go(id, new_place.map(Place::Square), false, false)
            && !self.would_collide_if(id, x, y)?
        {
            if rotation.abs() >= 90 {
                self.mark_the_dead_end(id);
            }
            let o = self.e(id).o + rotation;
            let _ = self.move_to(id, new_place.map(Place::Square), Some((x, y)), o);
            self.unblock(id);
            return Ok(true);
        }
        Ok(false)
    }

    /// `_reach`: one step toward the current orientation, turning around
    /// obstacles.
    fn reach(&mut self, id: EntityId, target_d: i64) -> Res {
        if !self.e(id).smooth_rotations.is_empty() {
            let rotation = self.em(id).smooth_rotations.remove(0);
            if self.try_move(id, rotation, target_d)? || self.try_move(id, -rotation, target_d)? {
                self.em(id).smooth_rotations.clear();
            }
            return Ok(());
        }
        if self.e(id).rotations.is_empty() {
            let e = self.em(id);
            e.walked = e
                .walked
                .iter()
                .filter(|w| w.3 > 1)
                .map(|w| (w.0, w.1, w.2, w.3 - 1))
                .collect();
            if self.e(id).walked.is_empty() && self.try_move(id, 0, target_d)? {
                return Ok(());
            }
            let mut rotations: Vec<(i64, i64)> = [0, 45, -45, 90, -90, 135, -135, 180]
                .iter()
                .map(|&r| (self.heuristic_value(id, r, target_d), r))
                .collect();
            rotations.sort();
            self.em(id).rotations = rotations;
        }
        let n = self.e(id).rotations.len().min(4);
        for _ in 0..n {
            let (_, rotation) = self.em(id).rotations.remove(0);
            if self.try_move(id, rotation, target_d)? {
                self.em(id).rotations.clear();
                return Ok(());
            }
        }
        if self.e(id).rotations.is_empty() {
            let e = self.em(id);
            e.smooth_rotations = (1..180).collect();
            e.walked.clear();
            self.mark_the_dead_end(id);
            self.notify(id, EntityEvent::Collision, false);
        }
        Ok(())
    }

    // holding a position

    /// `deploy`: goes to the position to hold, or the safest subsquare for
    /// smart units, or the center of the square.
    pub(crate) fn deploy(&mut self, id: EntityId) -> Res {
        let e = self.e(id);
        if let Some(Target::Zoom(z)) = &e.position_to_hold {
            let t = Target::Zoom(z.clone());
            self.set_action_target(id, Some(t));
            return Ok(());
        }
        let Some(Place::Square(place)) = e.place else {
            return Err(Abort::new("'Inside' object has no attribute 'x'"));
        };
        let smart = e.player.is_some_and(|p| self.pl(p).smart_units());
        let t = if smart {
            let p = e.player.expect("smart units have a player");
            let (x, y) = self.get_safest_subsquare(p, place);
            Target::Xy(x, y)
        } else {
            Target::Xy(self.square(place).x, self.square(place).y)
        };
        self.set_action_target(id, Some(t));
        Ok(())
    }

    pub(crate) fn is_in_position(&self, id: EntityId, target: &Target) -> bool {
        let e = self.e(id);
        match target {
            Target::Square(s) => e.place == Some(Place::Square(*s)),
            Target::Zoom(z) => self.zoom_contains(z, e.x, e.y),
            _ => false,
        }
    }

    pub(crate) fn hold(&mut self, id: EntityId, target: Target) -> Res {
        self.em(id).position_to_hold = Some(target);
        self.deploy(id)
    }

    // reaching targets

    /// `_near_enough_to_aim`.
    pub(crate) fn near_enough_to_aim(&mut self, id: EntityId, target: &Target) -> Res<bool> {
        let Some(t) = self.t_entity(target) else {
            return Err(Abort::new("the target is not an entity"));
        };
        let target_blocks = t.blocked_exit.is_some();
        let e = self.e(id);
        if e.is_melee() {
            if let Some(Place::Inside(c)) = e.place {
                if self.e(c).airground_type == AirGround::Air {
                    return Ok(false);
                }
            } else if !self.can_go(id, self.t_place_of(target), false, false) && !target_blocks {
                return Ok(false);
            }
        }
        let (e, t) = (self.e(id), self.t_entity(target).expect("checked above"));
        let d2 = square_of_distance(e.x, e.y, t.x, t.y);
        if e.stats.minimal_range != 0 && d2 < e.stats.minimal_range * e.stats.minimal_range {
            return Ok(false);
        }
        let mut actual_range = e.stats.range;
        if e.stats.is_ballistic != 0 {
            let (h, th) = (self.height_of(e)?, self.height_of(t)?);
            if h > th {
                actual_range += (h - th) * PRECISION;
            }
        }
        let d = (e.radius() + DISTANCE_MARGIN).max(actual_range) + t.radius();
        Ok(d2 < d * d)
    }

    /// `_near_enough`: on the same place and touching. Never for a square.
    pub(crate) fn near_enough(&self, id: EntityId, target: &Target) -> bool {
        let e = self.e(id);
        let TPlace::At(tp) = self.t_place(target) else {
            return false;
        };
        if Some(tp) != e.place {
            return false;
        }
        let (tx, ty) = self.t_xy(target);
        let d = e.radius() + self.t_radius(target) + DISTANCE_MARGIN;
        square_of_distance(e.x, e.y, tx, ty) < d * d
    }

    fn collision_range(&self, id: EntityId, other: &Target) -> i64 {
        let e = self.e(id);
        match self.t_entity(other) {
            Some(o)
                if e.stats.collision != 0
                    && o.stats.collision != 0
                    && o.airground_type == e.airground_type =>
            {
                e.radius() + o.radius()
            }
            _ => 0,
        }
    }

    fn turn_toward_and_reach(&mut self, id: EntityId, target: &Target) -> Res {
        let (tx, ty) = self.t_xy(target);
        let (x, y) = (self.e(id).x, self.e(id).y);
        let d = int_distance(x, y, tx, ty);
        self.em(id).o = int_angle(x, y, tx, ty);
        let cr = self.collision_range(id, target);
        self.reach(id, d - cr)
    }

    fn action_reach_and_stop(&mut self, id: EntityId) -> Res {
        let Some(target) = self.e(id).action_target().cloned() else {
            return Ok(());
        };
        if !self.near_enough(id, &target) {
            self.turn_toward_and_reach(id, &target)
        } else {
            // The original also sets an unused attribute here; the action
            // goes on.
            self.em(id).walked.clear();
            Ok(())
        }
    }

    fn action_reach_and_aim(&mut self, id: EntityId) -> Res {
        let Some(target) = self.e(id).action_target().cloned() else {
            return Ok(());
        };
        if !self.near_enough_to_aim(id, &target)? {
            self.turn_toward_and_reach(id, &target)
        } else {
            self.em(id).walked.clear();
            self.aim(id, &target)
        }
    }

    /// `go_to_xy`: a step toward a point; true when already there.
    fn go_to_xy(&mut self, id: EntityId, x: i64, y: i64) -> Res<bool> {
        let e = self.e(id);
        let d = int_distance(e.x, e.y, x, y);
        if d > e.radius() {
            let o = int_angle(e.x, e.y, x, y);
            self.em(id).o = o;
            self.reach(id, d)?;
            Ok(false)
        } else {
            Ok(true)
        }
    }

    /// Sets the action from a target, as the original's `action_target`
    /// setter: a point or a subsquare to move to, an enemy to attack,
    /// anything else to move to, or no action.
    pub(crate) fn set_action_target(&mut self, id: EntityId, value: Option<Target>) {
        let action = match value {
            Some(Target::Xy(x, y)) => Action {
                kind: ActionKind::MoveXy,
                target: Some(Target::Xy(x, y)),
            },
            Some(Target::Zoom(z)) => Action {
                kind: ActionKind::MoveXy,
                target: Some(Target::Xy(z.x, z.y)),
            },
            Some(t) if self.is_an_enemy(id, &t) => Action {
                kind: ActionKind::Attack,
                target: Some(t),
            },
            Some(t) => Action {
                kind: ActionKind::Move,
                target: Some(t),
            },
            None => Action {
                kind: ActionKind::Plain,
                target: None,
            },
        };
        self.em(id).action = Some(action);
    }

    /// `stop`: no action, no position to hold.
    pub(crate) fn stop(&mut self, id: EntityId) {
        if !self.e(id).is_creature() {
            return;
        }
        self.set_action_target(id, None);
        self.em(id).position_to_hold = None;
    }

    fn complete_action(&mut self, id: EntityId) {
        let e = self.em(id);
        e.walked.clear();
        e.action = None;
    }

    pub(crate) fn action_update(&mut self, id: EntityId) -> Res {
        let Some(action) = self.e(id).action.clone() else {
            return Ok(());
        };
        match action.kind {
            ActionKind::Plain => Ok(()),
            ActionKind::Move => self.move_action_update(id, action.target),
            ActionKind::MoveXy => {
                let Some(Target::Xy(x, y)) = action.target else {
                    return Ok(());
                };
                let (ux, uy) = (self.e(id).x, self.e(id).y);
                if self.get_subsquare_id_from_xy(x, y) != self.get_subsquare_id_from_xy(ux, uy) {
                    if self.go_to_xy(id, x, y)? {
                        self.complete_action(id);
                    }
                } else {
                    let previous_d2 = square_of_distance(x, y, ux, uy);
                    let arrived = self.go_to_xy(id, x, y)?;
                    let e = self.e(id);
                    if arrived || square_of_distance(x, y, e.x, e.y) > previous_d2 {
                        self.complete_action(id);
                    }
                }
                Ok(())
            }
            ActionKind::Attack => {
                let Some(target) = action.target else {
                    return Ok(());
                };
                let e = self.e(id);
                let in_place = match (&target, e.place) {
                    (Target::Entity(t), Some(p)) => self.place_objects(p).contains(t),
                    _ => false,
                };
                if e.stats.speed != 0 && in_place {
                    self.action_reach_and_aim(id)
                } else if self.can_attack(id, &target)? {
                    self.aim(id, &target)
                } else {
                    self.complete_action(id);
                    Ok(())
                }
            }
        }
    }

    fn move_action_update(&mut self, id: EntityId, target: Option<Target>) -> Res {
        // A move action always has a target.
        let Some(target) = target else { return Ok(()) };
        let exit = self
            .t_entity(&target)
            .filter(|e| e.is_exit())
            .and_then(|e| e.exit.clone());
        if let Some(exit) = exit
            && let Some(os) = exit.other_side
        {
            if exit.is_a_portal {
                let (tx, ty) = self.t_xy(&target);
                self.go_to_xy(id, tx, ty)?;
                if self.near_enough(id, &target) {
                    let o = self.e(os);
                    let (osp, osx, osy, oso) = (o.place, o.x, o.y, o.o);
                    let before = self.e(id).place;
                    let _ = self.move_to(id, osp, Some((osx, osy)), oso);
                    if self.e(id).place == before
                        && let Some(Place::Square(s)) = osp
                    {
                        // The unit could not move: the action now aims at the
                        // center of the other side.
                        let c = (self.square(s).x, self.square(s).y);
                        if let Some(a) = self.em(id).action.as_mut() {
                            a.target = Some(Target::Xy(c.0, c.1));
                        }
                    }
                }
            } else if let Some(Place::Square(s)) = self.e(os).place {
                let (x, y) = (self.square(s).x, self.square(s).y);
                self.go_to_xy(id, x, y)?;
            }
            return Ok(());
        }
        let same_place = match self.t_place(&target) {
            TPlace::At(p) => Some(p) == self.e(id).place,
            _ => false,
        };
        if same_place {
            self.action_reach_and_stop(id)
        } else if matches!(self.e(id).airground_type, AirGround::Air | AirGround::Water) {
            if let Target::Xy(..) = target {
                return Err(Abort::new("'tuple' object has no attribute 'x'"));
            }
            let (x, y) = self.t_xy(&target);
            self.go_to_xy(id, x, y)?;
            Ok(())
        } else {
            self.complete_action(id);
            Ok(())
        }
    }

    pub(crate) fn place_objects(&self, p: Place) -> &[EntityId] {
        match p {
            Place::Square(s) => &self.square(s).objects,
            Place::Inside(c) => self.e(c).inside.as_deref().unwrap_or(&[]),
        }
    }

    // update

    pub(crate) fn creature_update(&mut self, id: EntityId) -> Res {
        self.em(id).is_moving = false;
        if self.e(id).player.is_none() {
            return Ok(());
        }
        if self.e(id).stats.heal_level != 0 {
            self.heal_nearby_units(id);
        }
        if self.e(id).stats.harm_level != 0 {
            self.harm_nearby_units(id)?;
        }
        if self.e(id).inside.is_some() {
            let mut i = 0;
            while let Some(&o) = self.e(id).inside.as_ref().and_then(|v| v.get(i)) {
                let (x, y) = (self.e(id).x, self.e(id).y);
                let oe = self.em(o);
                oe.x = x;
                oe.y = y;
                self.creature_update(o)?;
                i += 1;
            }
        }
        if self.e(id).player.is_none() {
            return Ok(());
        }
        if self.e(id).action.is_some() {
            self.action_update(id)?;
        }
        if self.e(id).player.is_none() {
            return Ok(());
        }
        if self.e(id).has_imperative_orders() {
            self.execute_orders(id)?;
        } else {
            self.decide(id)?;
            if !self.e(id).is_attacking() && !self.e(id).orders.is_empty() {
                self.execute_orders(id)?;
            }
        }
        Ok(())
    }

    fn execute_orders(&mut self, id: EntityId) -> Res {
        let first = &self.e(id).orders[0];
        if first.is_complete || first.is_impossible {
            self.em(id).orders.remove(0);
            Ok(())
        } else {
            let uid = first.uid;
            self.order_update(id, uid)
        }
    }

    pub(crate) fn regenerate(&mut self, id: EntityId) {
        let e = self.em(id);
        if e.stats.hp_regen != 0 && e.hp < e.stats.hp_max {
            e.hp = e.stats.hp_max.min(e.hp + e.stats.hp_regen);
        }
        if e.stats.mana_regen != 0 && e.mana < e.stats.mana_max {
            e.mana = e.stats.mana_max.min(e.mana + e.stats.mana_regen);
        }
    }

    // damage

    fn raise_subsquare_threat(&mut self, id: EntityId, delta: i64) {
        let e = self.e(id);
        let sub = self.get_subsquare_id_from_xy(e.x, e.y);
        let Some(p) = e.player else { return };
        for a in self.pl(p).allied.clone() {
            if self.pl(p).allied.contains(&a) {
                *self.plm(a).subsquare_threat.entry(sub).or_insert(0) += delta;
            }
        }
    }

    /// `receive_hit`: loses hit points, and dies below zero.
    pub(crate) fn receive_hit(
        &mut self,
        id: EntityId,
        damage: i64,
        attacker: EntityId,
        notify: bool,
    ) -> Res {
        let Some(p) = self.e(id).player else {
            return Ok(());
        };
        if let Some(site) = self.em(id).site.as_mut() {
            site.damage_during_construction += damage;
        }
        let attacker = match self.e(attacker).place {
            Some(Place::Inside(c)) => c,
            _ => attacker,
        };
        self.observe(p, attacker);
        self.raise_subsquare_threat(id, damage);
        if notify {
            let a = self.e(attacker);
            let ev = EntityEvent::Wounded {
                attacker_type: a.type_name.to_string(),
                attacker,
                attacker_damage_level: a.damage_level,
            };
            self.notify(id, ev, false);
        }
        self.em(id).hp -= damage;
        if self.e(id).hp < 0 {
            self.die(id, Some(attacker))
        } else {
            self.on_unit_attacked(p, id, attacker);
            Ok(())
        }
    }

    /// `die`, with the behavior of the entity's class.
    pub(crate) fn die(&mut self, id: EntityId, attacker: Option<EntityId>) -> Res {
        match self.e(id).class {
            EntityClass::Worker | EntityClass::Soldier => {
                if self.e(id).stats.corpse != 0 {
                    self.create_corpse(id);
                }
                self.creature_die(id, attacker)
            }
            EntityClass::Effect => {
                self.delete(id);
                Ok(())
            }
            EntityClass::Building | EntityClass::BuildingSite => {
                let e = self.e(id);
                let (place, x, y) = (e.place, e.x, e.y);
                self.creature_die(id, attacker)?;
                if let Some(land) = self.e(id).building_land {
                    let _ = self.move_to(land, place, Some((x, y)), 90);
                }
                Ok(())
            }
            EntityClass::Deposit => {
                let e = self.e(id);
                let (place, x, y) = (e.place, e.x, e.y);
                self.notify(id, EntityEvent::Exhausted, false);
                self.delete(id);
                if let Some(land) = self.e(id).building_land {
                    let _ = self.move_to(land, place, Some((x, y)), 90);
                }
                Ok(())
            }
            EntityClass::Corpse => Ok(()),
            EntityClass::Meadow | EntityClass::Exit => {
                Err(Abort::new("this entity has no attribute 'die'"))
            }
        }
    }

    fn creature_die(&mut self, id: EntityId, attacker: Option<EntityId>) -> Res {
        if let Some(inside) = self.e(id).inside.clone() {
            for o in inside {
                let e = self.e(id);
                let (place, x, y, a) = (e.place, e.x, e.y, e.airground_type);
                let _ = self.move_to(o, place, Some((x, y)), 90);
                if self.e(o).place == Some(Place::Inside(id)) {
                    self.em(o).stats.collision = 0;
                    let _ = self.move_to(o, place, Some((x, y)), 90);
                }
                if a != AirGround::Ground {
                    self.die(o, attacker)?;
                }
            }
        }
        self.notify(id, EntityEvent::Death, false);
        let Some(p) = self.e(id).player else {
            return Err(Abort::new("'NoneType' object has no attribute 'stats'"));
        };
        let st = self.e(id).stat_type();
        if let Some(st) = st {
            self.plm(p).stats.add(StatEvent::Lost, st.into(), 1);
        }
        if let Some(a) = attacker {
            self.notify(id, EntityEvent::DeathBy(a), false);
            self.on_unit_attacked(p, id, a);
            match self.e(a).player {
                Some(ap) => {
                    if let Some(st) = st {
                        self.plm(ap).stats.add(StatEvent::Killed, st.into(), 1);
                    }
                }
                None => return Err(Abort::new("'NoneType' object has no attribute 'stats'")),
            }
        }
        self.delete(id);
        Ok(())
    }

    /// `get_objects2`: the units of some players (all by default) within a
    /// radius, found through the buckets.
    pub(crate) fn get_objects2(
        &self,
        x: i64,
        y: i64,
        radius: i64,
        players: &[PlayerId],
        filter: impl Fn(&Entity) -> bool,
    ) -> Vec<EntityId> {
        let players: Vec<PlayerId> = if players.is_empty() {
            self.players.clone()
        } else {
            players.to_vec()
        };
        let r2 = radius * radius;
        let mut out = Vec::new();
        for p in players {
            for o in self.potential_neighbors(p, x, y) {
                let e = self.e(o);
                if e.place.is_some() && filter(e) && square_of_distance(x, y, e.x, e.y) <= r2 {
                    out.push(o);
                }
            }
        }
        out
    }

    /// `get_objects`: the entities on squares within a radius.
    pub(crate) fn get_objects(
        &self,
        x: i64,
        y: i64,
        radius: i64,
        filter: impl Fn(&Entity) -> bool,
    ) -> Vec<EntityId> {
        let r2 = radius * radius;
        let mut out = Vec::new();
        for z in &self.squares {
            for &o in &z.objects {
                let e = self.e(o);
                if filter(e) && square_of_distance(x, y, e.x, e.y) <= r2 {
                    out.push(o);
                }
            }
        }
        out
    }

    fn heal_nearby_units(&mut self, id: EntityId) {
        let e = self.e(id);
        let hp = floordiv(e.stats.heal_level * PRECISION, 25);
        let Some(p) = e.player else { return };
        let allies = self.pl(p).allied.clone();
        let units = self.get_objects2(e.x, e.y, 6 * PRECISION, &allies, |x| {
            x.stats.is_healable != 0 && x.hp < x.stats.hp_max
        });
        for u in units {
            let ue = self.em(u);
            ue.hp = ue.stats.hp_max.min(ue.hp + hp);
        }
    }

    fn harm_nearby_units(&mut self, id: EntityId) -> Res {
        let e = self.e(id);
        let hp = floordiv(e.stats.harm_level * PRECISION, 25);
        let (x, y) = (e.x, e.y);
        let my_type = e.type_name.to_string();
        let candidates =
            self.get_objects2(x, y, 6 * PRECISION, &[], |o| o.stats.is_vulnerable != 0);
        let mut units = Vec::new();
        for u in candidates {
            let t = self.e(u).type_name.to_string();
            if self.can_harm(&my_type, &t) {
                units.push(u);
            }
        }
        for u in units {
            self.receive_hit(u, hp, id, false)?;
        }
        Ok(())
    }

    // enemies

    /// `Creature.is_an_enemy`: a creature of an enemy player, or the target
    /// of the unit's imperative `go` order.
    pub(crate) fn is_an_enemy(&self, id: EntityId, c: &Target) -> bool {
        let Some(ce) = self.t_entity(c) else {
            return false;
        };
        if !ce.is_creature() {
            return false;
        }
        let e = self.e(id);
        if e.has_imperative_orders()
            && e.orders[0].keyword == crate::orders::Keyword::Go
            && e.orders[0].target.as_ref().is_some_and(|t| t.same(c))
        {
            return true;
        }
        match e.player {
            Some(p) => !self.pl(p).allied.contains_opt(ce.player),
            None => false,
        }
    }

    fn can_attack_if_in_range(&self, id: EntityId, other: &Target) -> bool {
        let e = self.e(id);
        if e.stats.damage == 0 {
            return false;
        }
        let Some(p) = e.player else { return false };
        let Target::Entity(oid) = other else {
            return false;
        };
        if !self.pl(p).perception.contains(oid) {
            return false;
        }
        let o = self.e(*oid);
        if o.place.is_none() || o.hp < 0 {
            return false;
        }
        let kind = o.airground_type.ground_or_air();
        let targets = self
            .type_of(id)
            .map(|t| t.target_types.clone())
            .unwrap_or_else(|| vec!["ground".into()]);
        if !targets.iter().any(|t| t == kind) {
            return false;
        }
        o.stats.is_vulnerable != 0
    }

    /// `can_attack`: without moving to another square.
    pub(crate) fn can_attack(&mut self, id: EntityId, other: &Target) -> Res<bool> {
        if !self.can_attack_if_in_range(id, other) {
            return Ok(false);
        }
        if self.e(id).stats.speed != 0 && self.t_place_of(other) == self.e(id).place {
            return Ok(true);
        }
        self.near_enough_to_aim(id, other)
    }

    fn choose_enemy(&mut self, id: EntityId, place: SquareId) -> Res<bool> {
        let Some(p) = self.e(id).player else {
            return Ok(false);
        };
        let mut known = self.known_enemies(p, place);
        if known.is_empty() {
            for n in self.square(place).strict_neighbors.clone() {
                known = self.known_enemies(p, n);
                if !known.is_empty() {
                    break;
                }
            }
        }
        let mut reachable = Vec::new();
        for k in known {
            if self.can_attack(id, &Target::Entity(k))? {
                reachable.push(k);
            }
        }
        if reachable.is_empty() {
            return Ok(false);
        }
        let (x, y) = (self.e(id).x, self.e(id).y);
        reachable.sort_by(|&a, &b| {
            let (ea, eb) = (self.e(a), self.e(b));
            (-ea.menace(), square_of_distance(x, y, ea.x, ea.y))
                .cmp(&(-eb.menace(), square_of_distance(x, y, eb.x, eb.y)))
                .then(a.cmp_as_text(b))
        });
        self.attack(id, Target::Entity(reachable[0]));
        Ok(true)
    }

    fn attack(&mut self, id: EntityId, target: Target) {
        let same = matches!(&self.e(id).action, Some(a) if a.kind == ActionKind::Attack
            && a.target.as_ref().is_some_and(|t| t.same(&target)));
        if !same {
            self.em(id).action = Some(Action {
                kind: ActionKind::Attack,
                target: Some(target),
            });
            self.notify(id, EntityEvent::Attack, false);
        }
    }

    fn flee(&mut self, id: EntityId) -> Res {
        let Some(p) = self.e(id).player else {
            return Ok(());
        };
        let Some(here) = self.e(id).place else {
            return Ok(());
        };
        let mut sl: Vec<Place> = Vec::new();
        if let Place::Square(s) = here {
            for ex in self.square(s).exits.clone() {
                if let Some(os) = self.exit_other_side(ex)
                    && let Some(pl) = self.e(os).place
                {
                    sl.push(pl);
                }
            }
        }
        if let Some(prev) = self.e(id).previous_place {
            sl.insert(0, prev);
        }
        for s in sl {
            if self.balance(p, &[Some(s)], Some(id), 100)
                > self.balance(p, &[Some(here)], None, 100)
            {
                self.notify(id, EntityEvent::Flee, false);
                let sid = self.place_id_text(s);
                self.take_order(id, vec!["go".into(), sid], true, true, None)?;
                break;
            }
        }
        Ok(())
    }

    /// The protocol id of a place: a square's id, or nothing for an
    /// interior (whose id is `None` in the original).
    pub(crate) fn place_id_text(&self, p: Place) -> String {
        match p {
            Place::Square(s) => self.square(s).id.to_string(),
            Place::Inside(_) => "None".into(),
        }
    }

    /// `decide`: flee, or choose an enemy to attack; workers of humans then
    /// repair and gather on their own.
    pub(crate) fn decide(&mut self, id: EntityId) -> Res {
        self.creature_decide(id)?;
        if self.e(id).class != EntityClass::Worker {
            return Ok(());
        }
        let Some(p) = self.e(id).player else {
            return Ok(());
        };
        if !self.pl(p).is_human() {
            return Ok(());
        }
        if self
            .e(id)
            .orders
            .first()
            .is_some_and(|o| o.keyword != crate::orders::Keyword::Gather)
        {
            return Ok(());
        }
        let place = self.e(id).place;
        if self.e(id).auto_repair == Some(true) {
            for a in self.pl(p).allied.clone() {
                for u in self.pl(a).units.clone() {
                    let ue = self.e(u);
                    if ue.place == place
                        && ue.stats.is_repairable != 0
                        && ue.hp < ue.stats.hp_max
                        && !ue.is_building_site()
                    {
                        let cost = self.repair_cost(u);
                        if self.check_if_enough_resources(id, &cost, 0).is_none() {
                            self.take_order(
                                id,
                                vec!["repair".into(), u.to_string()],
                                true,
                                false,
                                None,
                            )?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        if !self.e(id).orders.is_empty() {
            return Ok(());
        }
        if self.e(id).auto_gather == Some(true) {
            let Some(place) = place else { return Ok(()) };
            let allied = self.pl(p).allied.clone();
            let objects = self.place_objects(place).to_vec();
            let mut local: Vec<i64> = Vec::new();
            for &w in &objects {
                let we = self.e(w);
                if we.player.is_some_and(|wp| allied.contains(&wp))
                    && let Some(t) = self.type_of(w)
                {
                    for &r in &t.storable_resource_types {
                        if !local.contains(&r) {
                            local.push(r);
                        }
                    }
                }
            }
            if !local.is_empty() {
                let deposits: Vec<EntityId> = objects
                    .iter()
                    .copied()
                    .filter(|&o| {
                        self.e(o).is_deposit()
                            && self
                                .type_of(o)
                                .and_then(|t| t.resource_type)
                                .is_some_and(|r| local.contains(&r))
                    })
                    .collect();
                if !deposits.is_empty() {
                    if let Some((r, _)) = self.e(id).cargo
                        && !local.contains(&r)
                    {
                        self.em(id).cargo = None;
                    }
                    let i = self.random.choice_index(deposits.len()).expect("not empty");
                    let o = deposits[i];
                    self.take_order(id, vec!["gather".into(), o.to_string()], true, false, None)?;
                }
            }
        }
        Ok(())
    }

    fn creature_decide(&mut self, id: EntityId) -> Res {
        let e = self.e(id);
        let Some(p) = e.player else { return Ok(()) };
        if let Some(Place::Inside(c)) = e.place {
            if let Some(Place::Square(s)) = self.e(c).place {
                self.choose_enemy(id, s)?;
            }
            return Ok(());
        }
        let Some(Place::Square(here)) = e.place else {
            return Ok(());
        };
        if (self.pl(p).smart_units() || e.ai_mode == Some(AiMode::Defensive))
            && e.stats.speed > 0
            && !self.must_hold(id)
            && self.balance(p, &[Some(Place::Square(here)), e.previous_place], None, 10) < 5
        {
            return self.flee(id);
        }
        let e = self.e(id);
        let target_menace = e
            .action_target()
            .and_then(|t| self.t_entity(t))
            .map_or(0, Entity::menace);
        if e.stats.damage == 0 || target_menace != 0 {
            return Ok(());
        }
        if self.choose_enemy(id, here)? {
            return Ok(());
        }
        for n in self.square(here).neighbors.clone() {
            if self.choose_enemy(id, n)? {
                break;
            }
        }
        Ok(())
    }

    // attack

    fn armor_versus_of(&self, target: &Entity, attacker: &Entity) -> i64 {
        if let Some(t) = target.type_id {
            let d = &self.rules.ty(t).armor_vs;
            if let Some((_, v)) = d.iter().find(|(k, _)| **k == *attacker.type_name) {
                return *v;
            }
            for name in self.expanded_is_a(attacker) {
                if let Some((_, v)) = d.iter().find(|(k, _)| *k == name) {
                    return *v;
                }
            }
        }
        target.stats.armor
    }

    fn base_damage_versus_of(&self, attacker: &Entity, target: &Entity) -> i64 {
        if let Some(t) = attacker.type_id {
            let d = &self.rules.ty(t).damage_vs;
            if let Some((_, v)) = d.iter().find(|(k, _)| **k == *target.type_name) {
                return *v;
            }
            for name in self.expanded_is_a(target) {
                if let Some((_, v)) = d.iter().find(|(k, _)| *k == name) {
                    return *v;
                }
            }
        }
        attacker.stats.damage
    }

    /// The damage of an attack before armor (`_base_damage_versus`).
    pub fn base_damage_versus(&self, attacker: EntityId, target: EntityId) -> i64 {
        self.base_damage_versus_of(self.e(attacker), self.e(target))
    }

    /// The armor of `target` against `attacker` (`armor_versus`).
    pub fn armor_versus(&self, target: EntityId, attacker: EntityId) -> i64 {
        self.armor_versus_of(self.e(target), self.e(attacker))
    }

    /// `hit`: deals the damage of an attack.
    pub fn hit(&mut self, id: EntityId, target: EntityId) {
        let (a, t) = (self.e(id), self.e(target));
        let base = self.base_damage_versus_of(a, t);
        let damage = a.minimal_damage.max(base - self.armor_versus_of(t, a));
        if let Err(Abort(m)) = self.receive_hit(target, damage, id, true) {
            self.warn(m);
        }
    }

    fn hit_res(&mut self, id: EntityId, target: EntityId) -> Res {
        let (a, t) = (self.e(id), self.e(target));
        let base = self.base_damage_versus_of(a, t);
        let damage = a.minimal_damage.max(base - self.armor_versus_of(t, a));
        self.receive_hit(target, damage, id, true)
    }

    fn hit_or_miss(&mut self, id: EntityId, target: EntityId) -> Res {
        if self.has_hit(id, target)? {
            self.hit_res(id, target)
        } else {
            self.notify(target, EntityEvent::Missed, false);
            Ok(())
        }
    }

    fn splash_aim(&mut self, id: EntityId, target: EntityId) -> Res {
        let r = self.e(id).stats.damage_radius;
        let r2 = r * r;
        let Some(tp) = self.e(target).place else {
            return Ok(());
        };
        let (tx, ty) = (self.e(target).x, self.e(target).y);
        for o in self.place_objects(tp).to_vec() {
            let ot = Target::Entity(o);
            if !self.is_an_enemy(id, &ot) && o != target {
                continue;
            }
            let oe = self.e(o);
            if oe.is_creature()
                && square_of_distance(oe.x, oe.y, tx, ty) <= r2
                && self.can_attack_if_in_range(id, &ot)
            {
                self.hit_or_miss(id, o)?;
            }
        }
        Ok(())
    }

    /// `chance_to_hit`: 50% for a ranged attack from below a plateau, less
    /// the cover of the target's terrain.
    pub fn chance_to_hit(&self, id: EntityId, target: EntityId) -> Option<i64> {
        self.chance_to_hit_res(id, target).ok()
    }

    fn chance_to_hit_res(&self, id: EntityId, target: EntityId) -> Res<i64> {
        let (e, t) = (self.e(id), self.e(target));
        let (Some(ep), Some(tp)) = (e.place, t.place) else {
            return Err(Abort::new(
                "'NoneType' object has no attribute 'high_ground'",
            ));
        };
        let high_ground = !self.place_high_ground(ep)
            && self.place_high_ground(tp)
            && t.airground_type == AirGround::Ground
            && !e.is_melee()
            && self.height_of(e)? < self.height_of(t)?;
        let mut result = if high_ground { 50 } else { 100 };
        if !e.is_melee() {
            let Place::Square(ts) = tp else {
                return Err(Abort::new(
                    "'Inside' object has no attribute 'terrain_cover'",
                ));
            };
            let cover = self.square(ts).terrain_cover;
            let c = if t.airground_type != AirGround::Air {
                cover.0
            } else {
                cover.1
            };
            result = floordiv(result * (100 - c), 100);
        }
        Ok(result)
    }

    /// `has_hit`: draws a number unless the chance is 100%.
    pub fn has_hit(&mut self, id: EntityId, target: EntityId) -> Res<bool> {
        let chance = self.chance_to_hit_res(id, target)?;
        Ok(chance == 100 || self.random.randint(1, 100) <= chance)
    }

    fn aim(&mut self, id: EntityId, target: &Target) -> Res {
        if self.can_attack(id, target)? && self.time >= self.e(id).next_attack_time {
            let t = self.time + self.e(id).stats.cooldown;
            self.em(id).next_attack_time = t;
            self.notify(id, EntityEvent::LaunchAttack, false);
            let Target::Entity(tid) = target else {
                return Ok(());
            };
            let splash = self.type_of(id).is_some_and(|t| t.splash);
            if splash {
                self.splash_aim(id, *tid)?;
            } else {
                self.hit_or_miss(id, *tid)?;
            }
        }
        Ok(())
    }

    // resources and repairs

    /// `check_if_enough_resources`: the reason the player cannot pay, if
    /// any. Food matters only for a unit without orders.
    pub(crate) fn check_if_enough_resources(
        &self,
        id: EntityId,
        cost: &[i64],
        food: i64,
    ) -> Option<Reason> {
        let p = self.e(id).player?;
        let pl = self.pl(p);
        for (i, &c) in cost.iter().enumerate() {
            if pl.resources.get(i).copied().unwrap_or(0) < c {
                return Some(Reason::NotEnoughResource(i));
            }
        }
        let limit = self.food_limit();
        if self.e(id).orders.is_empty()
            && food > 0
            && pl.available_food(limit) < pl.used_food + food
        {
            if pl.available_food(limit) < limit {
                return Some(Reason::NotEnoughFood);
            }
            return Some(Reason::PopulationLimitReached);
        }
        None
    }

    fn delta(&self, id: EntityId, total: i64, percentage: i64) -> i64 {
        let tc = self.e(id).stats.time_cost;
        let n = total * percentage * VIRTUAL_TIME_INTERVAL;
        if tc == 0 {
            floordiv(n, 100)
        } else {
            floordiv(floordiv(n, tc), 100)
        }
    }

    pub(crate) fn repair_cost(&self, id: EntityId) -> Vec<i64> {
        self.entity_cost(id)
            .iter()
            .map(|&c| self.delta(id, c, 30))
            .collect()
    }

    fn hp_delta(&self, id: EntityId) -> i64 {
        let e = self.e(id);
        match &e.site {
            Some(s) => self.delta(id, e.stats.hp_max - s.starting_hp, 100),
            None => self.delta(id, e.stats.hp_max, 70),
        }
    }

    pub(crate) fn is_fully_repaired(&self, id: EntityId) -> bool {
        let e = self.e(id);
        !e.is_building_site() && e.stats.is_repairable != 0 && e.hp == e.stats.hp_max
    }

    /// `be_built`: a building site grows, another building is repaired at
    /// the actor's player's cost.
    pub(crate) fn be_built(&mut self, id: EntityId, actor: EntityId) -> Res {
        if self.e(id).site.is_some() {
            let delta = self.hp_delta(id);
            let e = self.em(id);
            e.hp = (e.hp + delta).min(e.stats.hp_max);
            let site = e.site.as_mut().expect("checked");
            site.timer -= 1;
            if site.timer == 0 {
                let site = site.clone();
                let e = self.e(id);
                let (player, place, x, y) = (e.player, e.place, e.x, e.y);
                let blocked_exit = e.blocked_exit;
                let land = e.building_land;
                self.delete(id);
                let place = place.ok_or_else(|| Abort::new("the building site has no place"))?;
                let b = self
                    .create_creature(site.building_type, player, place, x, y)
                    .map_err(|_| Abort::new("NotEnoughSpaceError"))?;
                self.em(b).building_land = land;
                if let Some(ex) = blocked_exit {
                    self.block(b, ex);
                }
                let hp = self.rules.ty(site.building_type).stats.hp_max
                    - site.damage_during_construction;
                self.em(b).hp = hp;
                self.notify(b, EntityEvent::Complete, false);
            }
            return Ok(());
        }
        let e = self.e(id);
        if e.hp < e.stats.hp_max {
            let cost = self.repair_cost(id);
            match self.check_if_enough_resources(actor, &cost, 0) {
                Some(reason) => {
                    self.notify(actor, EntityEvent::OrderImpossible(Some(reason)), false);
                    if let Some(o) = self.em(actor).orders.first_mut() {
                        o.is_complete = true;
                    }
                    self.em(actor).distance_to_goal = 0;
                }
                None => {
                    if let Some(p) = self.e(actor).player {
                        self.pay(p, &cost);
                    }
                    let delta = self.hp_delta(id);
                    let e = self.em(id);
                    e.hp = (e.hp + delta).min(e.stats.hp_max);
                }
            }
        }
        Ok(())
    }

    // transport

    /// `have_enough_space`: whether the transport can take `target`.
    pub(crate) fn have_enough_space(&self, id: EntityId, target: &Entity) -> bool {
        let e = self.e(id);
        let Some(inside) = &e.inside else {
            return false;
        };
        let mut capacity = e.stats.transport_capacity;
        for &o in inside {
            capacity -= self.e(o).stats.transport_volume;
        }
        capacity >= target.stats.transport_volume
    }

    pub(crate) fn load(&mut self, id: EntityId, target: EntityId) {
        self.cancel_all_orders(target, true);
        self.notify(target, EntityEvent::Enter, false);
        let _ = self.move_to(target, Some(Place::Inside(id)), Some((0, 0)), 90);
    }

    pub(crate) fn load_all(&mut self, id: EntityId, place: Option<Place>) {
        let place = place.or(self.e(id).place);
        let Some(p) = self.e(id).player else { return };
        let mut units = self.pl(p).units.clone();
        units.sort_by_key(|&u| std::cmp::Reverse(self.e(u).stats.transport_volume));
        for u in units {
            if self.e(u).place == place && self.have_enough_space(id, self.e(u)) {
                self.load(id, u);
            }
        }
    }

    pub(crate) fn unload_all(&mut self, id: EntityId, place: Option<SquareId>) {
        let (place, x, y) = match place {
            None => {
                let e = self.e(id);
                (e.place, e.x, e.y)
            }
            Some(s) => (Some(Place::Square(s)), self.square(s).x, self.square(s).y),
        };
        for o in self.e(id).inside.clone().unwrap_or_default() {
            let _ = self.move_to(o, place, Some((x, y)), 90);
            self.notify(o, EntityEvent::Exit, false);
        }
    }

    /// `counterattack`: an offensive unit without orders goes to fight in a
    /// neighboring square, then comes back.
    pub(crate) fn counterattack(&mut self, id: EntityId, place: SquareId) -> Res {
        let e = self.e(id);
        let Some(Place::Square(here)) = e.place else {
            return Ok(());
        };
        if e.stats.speed != 0
            && e.menace() != 0
            && e.ai_mode == Some(AiMode::Offensive)
            && e.orders.is_empty()
            && !e.is_attacking()
            && self.can_go(id, Some(Place::Square(place)), false, false)
        {
            let pid = self.square(place).id.to_string();
            self.take_order(id, vec!["go".into(), pid], true, false, None)?;
            let e = self.e(id);
            let zoom = format!("zoom-{}-{}-{}", self.square(here).id, e.x, e.y);
            self.take_order(id, vec!["go".into(), zoom], false, false, None)?;
        }
        Ok(())
    }

    // paths

    /// `next_stage`: the exit (or, in the air, the square) to go to next to
    /// reach the target; the target itself on the same square.
    pub(crate) fn next_stage(
        &mut self,
        id: EntityId,
        target: Option<&Target>,
        avoid: bool,
    ) -> Res<Option<Target>> {
        let e = self.e(id);
        if e.is_inside() {
            return Ok(None);
        }
        let Some(target) = target else {
            return Ok(None);
        };
        let place = match target {
            Target::Square(s) => {
                if e.place == Some(Place::Square(*s)) {
                    return Ok(None);
                }
                *s
            }
            Target::Xy(..) => return Err(Abort::new("'tuple' object has no attribute 'place'")),
            _ => match self.t_place(target) {
                TPlace::Nowhere => return Ok(None),
                TPlace::World => return Ok(None),
                TPlace::At(p) => {
                    if e.place == Some(p) {
                        return Ok(Some(target.clone()));
                    }
                    match p {
                        Place::Square(s) => s,
                        Place::Inside(_) => return Ok(None),
                    }
                }
            },
        };
        let Some(Place::Square(here)) = e.place else {
            return Ok(None);
        };
        let (player, plane) = (e.player, e.airground_type);
        let d = self.shortest_path_distance_to(here, place, player, plane, avoid)?;
        self.em(id).distance_to_goal = d;
        let n = self.shortest_path_to(here, place, player, plane, avoid)?;
        Ok(n.map(|n| match n {
            Node::Square(s) => Target::Square(s),
            Node::Exit(x) => Target::Entity(x),
        }))
    }

    /// `next_square`: the square after the next exit.
    pub(crate) fn next_square(
        &mut self,
        id: EntityId,
        target: &Target,
        avoid: bool,
    ) -> Res<Option<Target>> {
        let stage = self.next_stage(id, Some(target), avoid)?;
        if let Some(Target::Entity(x)) = &stage
            && self.e(*x).is_exit()
            && let Some(os) = self.exit_other_side(*x)
            && let Some(Place::Square(s)) = self.e(os).place
        {
            return Ok(Some(Target::Square(s)));
        }
        Ok(stage)
    }

    pub(crate) fn start_moving_to(
        &mut self,
        id: EntityId,
        target: Option<&Target>,
        avoid: bool,
    ) -> Res {
        let stage = self.next_stage(id, target, avoid)?;
        self.set_action_target(id, stage);
        Ok(())
    }

    fn contains_enemy(&self, place: Place, p: PlayerId) -> bool {
        self.place_objects(place)
            .iter()
            .any(|&o| self.player_is_an_enemy_of_entity(p, self.e(o)))
    }

    pub(crate) fn start_moving_to_enemy(&mut self, id: EntityId) -> Res {
        let Some(p) = self.e(id).player else {
            return Ok(());
        };
        let Some(Place::Square(here)) = self.e(id).place else {
            return Ok(());
        };
        if self.contains_enemy(Place::Square(here), p) {
            self.choose_enemy(id, here)?;
            return Ok(());
        }
        for ex in self.square(here).exits.clone() {
            if let Some(os) = self.exit_other_side(ex)
                && let Some(op) = self.e(os).place
                && self.contains_enemy(op, p)
            {
                self.set_action_target(id, Some(Target::Entity(ex)));
                return Ok(());
            }
        }
        let i = self
            .random
            .choice_index(self.squares.len())
            .ok_or_else(|| Abort::new("Cannot choose from an empty sequence"))?;
        let stage = self.next_stage(id, Some(&Target::Square(SquareId(i as u32))), false)?;
        self.set_action_target(id, stage);
        Ok(())
    }

    /// `auto_explore`: goes to unknown starting squares, unknown squares,
    /// squares to watch, then anywhere, avoiding danger.
    pub(crate) fn auto_explore(&mut self, id: EntityId) -> Res {
        let Some(p) = self.e(id).player else {
            return Err(Abort::new("AssertionError"));
        };
        let current = self.e(id).action_target().cloned();
        match current {
            None => {
                let dest = self.e(id).destination;
                if self.e(id).place != dest.map(Place::Square) {
                    let t = dest.map(Target::Square);
                    let stage = self.next_stage(id, t.as_ref(), true)?;
                    self.set_action_target(id, stage);
                    if !self.e(id).is_idle() {
                        return Ok(());
                    }
                }
                // Each list is a fresh random sample, drawn only if the
                // previous lists gave nothing.
                for list_index in 0..3 {
                    let mut list = match list_index {
                        0 => self.unknown_starting_squares(p),
                        1 => self.unknown_squares(p),
                        _ => self.squares_to_watch(p),
                    };
                    if list_index > 0 {
                        list.truncate(10);
                    }
                    for place in list {
                        let stage = self.next_stage(id, Some(&Target::Square(place)), true)?;
                        self.set_action_target(id, stage);
                        if !self.e(id).is_idle() {
                            self.em(id).destination = Some(place);
                            return Ok(());
                        }
                    }
                }
                let i = self
                    .random
                    .choice_index(self.squares.len())
                    .ok_or_else(|| Abort::new("Cannot choose from an empty sequence"))?;
                let dest = SquareId(i as u32);
                self.em(id).destination = Some(dest);
                let stage = self.next_stage(id, Some(&Target::Square(dest)), false)?;
                self.set_action_target(id, stage);
            }
            Some(t) => {
                let here = self.e(id).place;
                if self.is_very_dangerous(p, &t)? {
                    let here_dangerous = match here {
                        Some(Place::Square(s)) => self.square_is_very_dangerous(p, s),
                        _ => {
                            return Err(Abort::new(
                                "'Inside' object has no attribute 'other_side'",
                            ));
                        }
                    };
                    if !here_dangerous {
                        self.set_action_target(id, None);
                    }
                } else if let Some(Place::Square(s)) = here
                    && self.square_is_very_dangerous(p, s)
                    && let Some(prev) = self.e(id).previous_place
                {
                    let prev_dangerous = match prev {
                        Place::Square(ps) => self.square_is_very_dangerous(p, ps),
                        Place::Inside(_) => {
                            return Err(Abort::new(
                                "'Inside' object has no attribute 'other_side'",
                            ));
                        }
                    };
                    if !prev_dangerous {
                        let t = match prev {
                            Place::Square(ps) => Target::Square(ps),
                            Place::Inside(_) => unreachable!("handled above"),
                        };
                        self.start_moving_to(id, Some(&t), false)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn move_on_border(&mut self, id: EntityId, exit: EntityId) {
        let e = self.e(exit);
        let (place, x, y) = (e.place, e.x, e.y);
        let _ = self.move_to(id, place, Some((x, y)), 90);
    }

    // deposits

    pub(crate) fn extract_resource(&mut self, id: EntityId) -> Res {
        let q = self.type_of(id).map_or(0, |t| t.stats.extraction_qty);
        self.em(id).stats.qty -= q;
        if self.e(id).stats.qty <= 0 {
            self.die(id, None)?;
        }
        Ok(())
    }

    /// The default order of a unit on a target (`get_default_order`).
    pub fn get_default_order(
        &mut self,
        id: EntityId,
        target_id: &str,
    ) -> Res<Option<&'static str>> {
        let Some(p) = self.e(id).player else {
            return Err(Abort::new(
                "'NoneType' object has no attribute 'get_object_by_id'",
            ));
        };
        let Some(target) = self.get_object_by_id(p, target_id)? else {
            return Ok(None);
        };
        let te = self.t_entity(&target).cloned();
        if te.as_ref().is_some_and(Entity::is_exit) {
            return Ok(Some("block"));
        }
        let mine = te.as_ref().is_some_and(|t| t.player == Some(p));
        if mine && self.have_enough_space(id, te.as_ref().expect("mine")) {
            return Ok(Some("load"));
        }
        if mine {
            let tid = te.as_ref().expect("mine").id;
            if self.e(tid).inside.is_some() && self.have_enough_space(tid, self.e(id)) {
                return Ok(Some("enter"));
            }
        }
        if self.e(id).basic_abilities().contains(&"gather")
            && te.as_ref().is_some_and(Entity::is_deposit)
        {
            return Ok(Some("gather"));
        }
        if let Some(t) = &te {
            let can_build = self.type_list(id, "can_build");
            let site_ok = t
                .site
                .as_ref()
                .is_some_and(|s| can_build.contains(&self.rules.ty(s.building_type).name));
            let repair_ok =
                t.stats.is_repairable != 0 && t.hp < t.stats.hp_max && !can_build.is_empty();
            if (site_ok || repair_ok) && !self.is_an_enemy(id, &target) {
                return Ok(Some("repair"));
            }
        }
        if self.order_is_allowed(id, crate::orders::Keyword::RallyingPoint, &[]) {
            return Ok(Some("rallying_point"));
        }
        if self.order_is_allowed(id, crate::orders::Keyword::Go, &[]) {
            return Ok(Some("go"));
        }
        Ok(None)
    }
}

/// `x in list` for an optional player, where `None` is never in the list.
pub(crate) trait ContainsOpt {
    fn contains_opt(&self, p: Option<PlayerId>) -> bool;
}

impl ContainsOpt for Vec<PlayerId> {
    fn contains_opt(&self, p: Option<PlayerId>) -> bool {
        p.is_some_and(|p| self.contains(&p))
    }
}
