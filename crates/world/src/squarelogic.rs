//! What squares do (`worldroom.py`): free space, building lands, terrain,
//! and the map editor's changes.

use crate::entity::{Entity, EntityId, ExitData, Place, PlayerId, SquareId, Target, ZoomTarget};
use crate::nofloat::{floordiv, int_angle, int_cos_1000, int_distance, int_sin_1000};
use crate::rules::{AirGround, EntityClass};
use crate::square::{SPACE_LIMIT, Spiral, spiral_index};
use crate::world::World;

impl World {
    /// `find_free_space`: the point itself if free, else the first free
    /// point of a spiral around it. During world time 0, searches from the
    /// center of a square continue one spiral per layer, so the points
    /// already given are not tried again.
    pub(crate) fn find_free_space(
        &mut self,
        s: SquareId,
        a: AirGround,
        x: i64,
        y: i64,
    ) -> Option<(i64, i64)> {
        let free = |w: &World, x: i64, y: i64| {
            w.square(s).contains(x, y) && !w.collision(a).would_collide(x, y)
        };
        if free(self, x, y) {
            return Some((x, y));
        }
        let center = (self.square(s).x, self.square(s).y);
        if self.time == 0 && (x, y) == center {
            if self.square(s).spirals.is_none() {
                let sp = Spiral::new(x, y);
                self.sqm(s).spirals = Some([sp.clone(), sp.clone(), sp]);
            }
            loop {
                let next =
                    self.sqm(s).spirals.as_mut().expect("created above")[spiral_index(a)].next();
                match next {
                    None => return None,
                    Some((px, py)) if free(self, px, py) => return Some((px, py)),
                    Some(_) => {}
                }
            }
        }
        Spiral::new(x, y).find(|&(px, py)| free(self, px, py))
    }

    /// `can_receive`: fewer than [`SPACE_LIMIT`] colliding units of the
    /// layer, enemies of `player` aside.
    pub(crate) fn can_receive(&self, s: SquareId, a: AirGround, player: Option<PlayerId>) -> bool {
        let n = self
            .square(s)
            .objects
            .iter()
            .map(|&o| self.e(o))
            .filter(|u| {
                u.stats.collision != 0
                    && u.airground_type == a
                    && !player.is_some_and(|p| self.player_is_an_enemy_of_entity(p, u))
            })
            .count();
        n < SPACE_LIMIT
    }

    /// The first building land (meadow) of a square.
    pub fn square_building_land(&self, s: SquareId) -> Option<EntityId> {
        self.square(s)
            .objects
            .iter()
            .copied()
            .find(|&o| self.e(o).is_a_building_land())
    }

    /// The first exit of a square that is not blocked.
    pub fn square_exit(&self, s: SquareId) -> Option<EntityId> {
        self.square(s).exits.iter().copied().find(|&e| {
            self.e(e).is_exit()
                && !self.exit_is_blocked(e, crate::pathfinding::Viewer::Nobody, false, false)
        })
    }

    pub(crate) fn zoom_building_land(&self, z: &ZoomTarget) -> Option<EntityId> {
        self.square(z.place).objects.iter().copied().find(|&o| {
            let e = self.e(o);
            e.is_a_building_land() && self.zoom_contains(z, e.x, e.y)
        })
    }

    pub(crate) fn zoom_exit(&self, z: &ZoomTarget) -> Option<EntityId> {
        self.square(z.place).exits.iter().copied().find(|&o| {
            let e = self.e(o);
            !self.exit_is_blocked(o, crate::pathfinding::Viewer::Nobody, false, false)
                && self.zoom_contains(z, e.x, e.y)
        })
    }

    /// `is_buildable_anywhere` as `getattr(o, ..., False)`: only creatures
    /// have it.
    fn occupies_land(&self, e: &Entity) -> bool {
        e.is_creature() && e.stats.is_buildable_anywhere != 0
    }

    /// `ZoomTarget.any_land`: the subsquare, if no building that can stand
    /// anywhere (or unit) is on it.
    fn zoom_any_land(&self, z: &ZoomTarget) -> Option<ZoomTarget> {
        let taken = self.square(z.place).objects.iter().any(|&o| {
            let e = self.e(o);
            self.occupies_land(e) && self.zoom_contains(z, e.x, e.y)
        });
        (!taken).then(|| z.clone())
    }

    fn subsquares(&self, s: SquareId) -> Vec<ZoomTarget> {
        self.square(s)
            .subsquare_centers()
            .iter()
            .map(|&(x, y)| ZoomTarget {
                place: s,
                x,
                y,
                id: None,
            })
            .collect()
    }

    /// `any_land` of a square, subsquare or entity: a free subsquare for a
    /// building that can stand anywhere.
    pub(crate) fn target_any_land(&self, t: &Target) -> Option<Target> {
        match t {
            Target::Square(s) => self
                .subsquares(*s)
                .iter()
                .find_map(|z| self.zoom_any_land(z))
                .map(Target::Zoom),
            Target::Zoom(z) => self.zoom_any_land(z).map(Target::Zoom),
            _ => {
                let e = self.t_entity(t)?;
                let Some(Place::Square(s)) = e.place else {
                    return None;
                };
                let z = self
                    .subsquares(s)
                    .into_iter()
                    .find(|z| self.zoom_contains(z, e.x, e.y))?;
                self.zoom_any_land(&z).map(Target::Zoom)
            }
        }
    }

    /// `is_near_water` of a square: a ground square, not high, beside
    /// water.
    pub fn square_is_near_water(&self, s: SquareId) -> bool {
        let sq = self.square(s);
        if !sq.is_ground || sq.high_ground {
            return false;
        }
        sq.strict_neighbors.iter().any(|&n| self.square(n).is_water)
    }

    pub(crate) fn target_is_near_water(&self, t: &Target) -> bool {
        match t {
            Target::Square(s) => self.square_is_near_water(*s),
            Target::Zoom(_) | Target::Xy(..) => false,
            _ => match self.t_entity(t).and_then(|e| e.place) {
                Some(Place::Square(s)) => self.square_is_near_water(s),
                _ => false,
            },
        }
    }

    /// `find_nearest_meadow`: the closest meadow of the square to the unit.
    pub(crate) fn find_nearest_meadow(&self, s: SquareId, unit: EntityId) -> Option<EntityId> {
        let u = self.e(unit);
        let mut meadows: Vec<EntityId> = self
            .square(s)
            .objects
            .iter()
            .copied()
            .filter(|&o| self.e(o).is_meadow())
            .collect();
        meadows.sort_by(|&a, &b| {
            let (ea, eb) = (self.e(a), self.e(b));
            int_distance(ea.x, ea.y, u.x, u.y)
                .cmp(&int_distance(eb.x, eb.y, u.x, u.y))
                .then(a.cmp_as_text(b))
        });
        meadows.first().copied()
    }

    /// `find_and_remove_meadow`: the center for a type that can stand
    /// anywhere; else takes the first meadow.
    pub(crate) fn find_and_remove_meadow(
        &mut self,
        s: SquareId,
        buildable_anywhere: bool,
    ) -> (i64, i64, Option<EntityId>) {
        let center = (self.square(s).x, self.square(s).y);
        if buildable_anywhere {
            return (center.0, center.1, None);
        }
        let meadow = self
            .square(s)
            .objects
            .iter()
            .copied()
            .find(|&o| self.e(o).is_meadow());
        match meadow {
            Some(m) => {
                let (x, y) = (self.e(m).x, self.e(m).y);
                self.delete(m);
                (x, y, Some(m))
            }
            None => (center.0, center.1, None),
        }
    }

    /// Places the deposits and meadows of a square on a circle, turned so
    /// that the map stays symmetric around (`xc`, `yc`).
    pub(crate) fn arrange_resources_symmetrically(&mut self, s: SquareId, xc: i64, yc: i64) {
        let sq = self.square(s);
        let things: Vec<EntityId> = sq
            .objects
            .iter()
            .copied()
            .filter(|&o| matches!(self.e(o).class, EntityClass::Deposit | EntityClass::Meadow))
            .collect();
        let width = sq.xmax - sq.xmin;
        let nb = things.len() as i64;
        let shift = int_angle(xc, yc, sq.col * 10 + 5, sq.row * 10 + 5);
        let (cx, cy) = (sq.x, sq.y);
        for (i, o) in things.into_iter().enumerate() {
            let (mut x, mut y) = (cx, cy);
            if nb > 1 {
                let a = floordiv(360 * i as i64, nb) + shift;
                x += floordiv(floordiv(width * 35, 100) * int_cos_1000(a), 1000);
                y += floordiv(floordiv(width * 35, 100) * int_sin_1000(a), 1000);
            }
            let place = self.e(o).place;
            let _ = self.move_to(o, place, Some((x, y)), 90);
        }
    }

    /// The number of meadows of a square, buildings standing on one
    /// included.
    pub fn nb_meadows(&self, s: SquareId) -> usize {
        self.square(s)
            .objects
            .iter()
            .filter(|&&o| {
                let e = self.e(o);
                (e.is_a_building_land() && !e.is_exit())
                    || (e.building_land.is_some() && e.stats.qty == 0)
            })
            .count()
    }

    pub(crate) fn update_terrain_of_squares(&mut self) {
        for i in 0..self.squares.len() {
            let s = SquareId(i as u32);
            if matches!(
                self.square(s).type_name.as_str(),
                "" | "_meadows" | "_forest" | "_dense_forest"
            ) {
                self.update_terrain(s);
            }
        }
    }

    /// `update_terrain`: the terrain style follows the woods and meadows; two
    /// dense forests side by side close the path between them.
    fn update_terrain(&mut self, s: SquareId) {
        let objects = &self.square(s).objects;
        let meadows = objects
            .iter()
            .filter(|&&o| &*self.e(o).type_name == "meadow")
            .count();
        let woods = objects
            .iter()
            .filter(|&&o| &*self.e(o).type_name == "wood")
            .count();
        let t = if woods >= 7 {
            "_dense_forest"
        } else if woods > 0 {
            "_forest"
        } else if meadows > 0 {
            "_meadows"
        } else {
            ""
        };
        self.sqm(s).type_name = t.to_owned();
        let dense = t == "_dense_forest";
        for n in self.square(s).strict_neighbors.clone() {
            let blocked = dense && self.square(n).type_name == "_dense_forest";
            self.set_path_blocked_by_forests(s, n, blocked);
        }
    }

    fn set_path_blocked_by_forests(&mut self, s: SquareId, other: SquareId, blocked: bool) {
        for e in self.square(s).exits.clone() {
            if self.exit_other_side(e).and_then(|os| self.e(os).square()) == Some(other)
                && let Some(x) = self.em(e).exit.as_mut()
            {
                x.is_blocked_by_forests = blocked;
            }
        }
    }

    /// `passage`: two exits linking two squares, unless it would lead
    /// ground units into water.
    pub(crate) fn passage(
        &mut self,
        a: (SquareId, i64, i64, i64),
        b: (SquareId, i64, i64, i64),
        is_a_portal: bool,
        exit_type: &str,
    ) {
        let (sa, sb) = (self.square(a.0), self.square(b.0));
        if sa.is_water != sb.is_water && !(sa.is_ground && sb.is_ground) {
            let msg = format!("removed dangerous path between {} and {}", sa.name, sb.name);
            self.warn(msg);
            return;
        }
        let e1 = self.create_exit(a, exit_type, is_a_portal);
        let e2 = self.create_exit(b, exit_type, is_a_portal);
        if let Some(x) = self.em(e1).exit.as_mut() {
            x.other_side = Some(e2);
        }
        if let Some(x) = self.em(e2).exit.as_mut() {
            x.other_side = Some(e1);
        }
    }

    fn create_exit(
        &mut self,
        (s, x, y, o): (SquareId, i64, i64, i64),
        exit_type: &str,
        is_a_portal: bool,
    ) -> EntityId {
        let mut e = Entity::new(EntityClass::Exit, exit_type);
        e.exit = Some(ExitData {
            other_side: None,
            is_a_portal,
            blockers: Vec::new(),
            is_blocked_by_forests: false,
        });
        let id = self.insert_entity(e, Place::Square(s), x, y, o);
        self.sqm(s).exits.push(id);
        id
    }

    // map editor

    fn other_sides(&self, s: SquareId) -> Vec<Option<SquareId>> {
        self.square(s)
            .exits
            .iter()
            .map(|&e| self.exit_other_side(e).and_then(|os| self.e(os).square()))
            .collect()
    }

    /// Adds a path to a neighbor if there is none.
    pub fn ensure_path(&mut self, s: SquareId, other: SquareId) {
        if !self.other_sides(s).contains(&Some(other)) {
            let (a, b) = (self.square(s), self.square(other));
            let x = floordiv(a.x + b.x, 2);
            let y = floordiv(a.y + b.y, 2);
            self.passage((s, x, y, 0), (other, x, y, 0), false, "path");
            self.create_graphs();
        }
    }

    /// Removes the path to a neighbor. As in the original, only the exit on
    /// this side leaves the list of exits, then the removal stops.
    pub fn ensure_nopath(&mut self, s: SquareId, other: SquareId) {
        for e in self.square(s).exits.clone() {
            if self.exit_other_side(e).and_then(|os| self.e(os).square()) == Some(other) {
                self.delete(e);
                break;
            }
        }
    }

    /// `toggle_path`: adds or removes the path toward a neighbor. True when
    /// a path was added; `None` at the border.
    pub fn toggle_path(&mut self, s: SquareId, dc: i64, dr: i64) -> Option<bool> {
        let sq = self.square(s);
        let other = self.square_at(sq.col + dc, sq.row + dr)?;
        if self.other_sides(s).contains(&Some(other)) {
            self.ensure_nopath(s, other);
            Some(false)
        } else {
            self.ensure_path(s, other);
            Some(true)
        }
    }

    /// Sets the number of meadows of a square, then arranges its resources.
    pub fn ensure_meadows(&mut self, s: SquareId, n: usize) {
        for o in self.square(s).objects.clone() {
            if n >= self.nb_meadows(s) {
                break;
            }
            let e = self.e(o);
            if e.is_a_building_land() && !e.is_exit() {
                self.delete(o);
            }
        }
        let have = self.nb_meadows(s);
        for _ in have..n {
            self.create_meadow(s, None);
        }
        let (x, y) = (self.square(s).x, self.square(s).y);
        self.arrange_resources_symmetrically(s, x, y);
    }

    /// Replaces the deposits of a type on a square with `n` deposits of
    /// quantity `q` (written as in a map).
    pub fn ensure_resources(&mut self, s: SquareId, type_name: &str, n: usize, q: &str) {
        for o in self.square(s).objects.clone() {
            if &*self.e(o).type_name == type_name {
                self.delete(o);
            }
        }
        let Some(tid) = self.rules.type_id(type_name) else {
            return;
        };
        for _ in 0..n {
            if let Err(e) = self.create_deposit(tid, s, q) {
                self.warn(e.0);
            }
        }
    }
}
