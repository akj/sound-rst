//! Moving through the squares of the map, and what a square says: its
//! name, terrain, fog, and the summary of what is on it.

use soundrts_world::msg::{Msg, Phrase, comma, msg};
use soundrts_world::rules::AirGround;
use soundrts_world::{INF, Place, SquareId};

use crate::audio::{angle, distance};
use crate::client::Client;

use super::GameInterface;
use super::animation::{NoisePlayer, NoiseSite, NoiseSource};
use super::entity::EntityView;
use super::focus::Zoom;
use crate::message::summary;
use crate::style::mine;

/// Keeps the first of each equal item, in order.
pub fn remove_duplicates<T: PartialEq + Clone>(l: &[T]) -> Vec<T> {
    let mut m: Vec<T> = Vec::new();
    for i in l {
        if !m.contains(i) {
            m.push(i.clone());
        }
    }
    m
}

/// The square `dxc` columns and `dyc` rows away, wrapping around the map's
/// edges (one wrap at most).
pub fn wrapped_coords(
    (xc, yc): (i64, i64),
    (dxc, dyc): (i64, i64),
    (xcmax, ycmax): (i64, i64),
) -> (i64, i64) {
    let mut xc = xc + dxc;
    if xc < 0 {
        xc = xcmax;
    }
    if xc > xcmax {
        xc = 0;
    }
    let mut yc = yc + dyc;
    if yc < 0 {
        yc = ycmax;
    }
    if yc > ycmax {
        yc = 0;
    }
    (xc, yc)
}

impl GameInterface {
    /// The last column and row.
    pub fn grid_max(&self) -> (i64, i64) {
        self.view().grid_max()
    }

    /// The column and row of a square, or (-1, -1).
    pub fn coords_in_map(&self, square: Option<SquareId>) -> (i64, i64) {
        match square {
            Some(s) => {
                let s = self.view().square(s);
                (s.col(), s.row())
            }
            None => (-1, -1),
        }
    }

    /// The square `dxc` columns and `dyc` rows from `from`, across an edge
    /// of the map to the other side. None if the map has no square there.
    pub(crate) fn compute_move(&self, from: SquareId, dxc: i64, dyc: i64) -> Option<SquareId> {
        let (xc, yc) = wrapped_coords(self.coords_in_map(Some(from)), (dxc, dyc), self.grid_max());
        self.view().square_at(xc, yc)
    }

    /// Selects a square; the noises of the previous one stop.
    pub(crate) fn move_to_square(&mut self, cx: &Client, square: SquareId) {
        if self.place != Some(square) {
            self.place = Some(square);
            if cx.config.parameters.silence_previous_square {
                self.silence_square(cx);
            }
        }
    }

    fn silence_square(&self, cx: &Client) {
        for v in self.dobjets.values() {
            if !self.is_in(v, self.place) {
                v.stop(&cx.audio);
            }
        }
        cx.audio.stop(false);
    }

    /// The terrain of a square as the player knows it, and whether it is
    /// high ground.
    fn square_terrain(&self, cx: &Client, place: SquareId) -> Msg {
        let mut result = Vec::new();
        let view = self.view();
        let terrain = view.terrain(place);
        if !terrain.is_empty() {
            let title = cx.res.style().get(terrain).title_msg();
            if !title.is_empty() {
                result.extend(comma());
                result.extend(title);
            }
        }
        if view.square(place).high_ground() {
            result.extend(comma());
            result.extend(msg(Phrase::Plateau));
        }
        result
    }

    /// The terrain, and whether the square is in the fog or unknown.
    pub fn square_postfix(&self, cx: &Client, place: SquareId) -> Msg {
        let mut postfix = Vec::new();
        if self.scouted_squares.contains(&place) {
            postfix.extend(self.square_terrain(cx, place));
        } else if self.scouted_before_squares.contains(&place) {
            postfix.extend(self.square_terrain(cx, place));
            postfix.extend(comma());
            postfix.extend(msg(Phrase::InTheFog));
        } else {
            postfix.extend(comma());
            postfix.extend(msg(Phrase::Unknown));
        }
        postfix
    }

    /// The enemies, allies, own units and resources on a place (or in the
    /// zoom).
    pub fn place_summary(
        &self,
        cx: &Client,
        place: Option<SquareId>,
        me: bool,
        zoom: Option<&Zoom>,
        brief: bool,
    ) -> Msg {
        let mut enemies = Vec::new();
        let mut allies = Vec::new();
        let mut units = Vec::new();
        let mut resources = Vec::new();
        let view = self.view();
        for v in self.dobjets.values() {
            if !self.is_in(v, place) {
                continue;
            }
            if zoom.is_some_and(|z| !z.contains(v.model.x, v.model.y)) {
                continue;
            }
            let t = self.short_title(cx, v);
            if t.is_empty() {
                continue;
            }
            if self.is_an_enemy(&v.model) {
                enemies.push(t.clone());
            }
            if let Some(p) = v.player() {
                if view.is_ally(p) && p != self.me {
                    allies.push(t.clone());
                }
                if p == self.me {
                    units.push(t.clone());
                }
            }
            if self.resource_type(&v.model).is_some() {
                resources.push(t);
            }
        }
        let mut result = Vec::new();
        if !enemies.is_empty() {
            result.extend(comma());
            result.extend(summary(&enemies, brief));
            result.extend(msg(Phrase::Enemy));
        }
        if me && !allies.is_empty() {
            result.extend(comma());
            result.extend(summary(&allies, false));
            result.extend(msg(Phrase::Ally));
        }
        if me && !units.is_empty() {
            result.extend(comma());
            result.extend(summary(&units, false));
        }
        if !resources.is_empty() && (enemies.is_empty() || !brief) {
            result.extend(comma());
            result.extend(summary(&resources, false));
        }
        result
    }

    /// Says a square: name, terrain and fog, then what is on it.
    pub fn say_square(&self, cx: &mut Client, place: Option<SquareId>, prefix: &Msg) {
        let Some(p) = place else { return };
        let m = [
            prefix.clone(),
            self.view().square(p).title().clone(),
            self.square_postfix(cx, p),
            self.place_summary(cx, Some(p), true, None, false),
        ]
        .concat();
        cx.voice.item(&m);
    }

    /// Says the zoom: the subsquare, the square's terrain, and what is in
    /// the subsquare.
    pub fn say_zoom(&self, cx: &mut Client, prefix: &Msg) {
        let (Some(z), Some(p)) = (&self.zoom, self.place) else {
            return;
        };
        let m = [
            prefix.clone(),
            z.title(self.view().square(p)),
            self.square_postfix(cx, p),
            self.place_summary(cx, Some(p), true, Some(z), false),
        ]
        .concat();
        cx.voice.item(&m);
    }

    pub(crate) fn select_and_say_square(
        &mut self,
        cx: &mut Client,
        square: SquareId,
        prefix: &Msg,
    ) {
        self.move_to_square(cx, square);
        self.target = None;
        self.say_square(cx, Some(square), prefix);
        self.follow_mode = false;
    }

    /// The group flies, or the selected order reaches far: squares are
    /// crossed freely.
    fn shouldnt_collide(&self) -> bool {
        self.group.iter().any(|x| {
            self.dobjets
                .get(x)
                .is_some_and(|v| v.model.airground == AirGround::Air)
        }) || self
            .order
            .as_ref()
            .is_some_and(|o| o.nb_args != 0 && o.target_shouldnt_collide(self.view()))
    }

    /// Whether the player may see the object now: in the zoom, on the
    /// square, with a title; in first person mode, in front; an exit not
    /// blocked.
    pub fn is_visible(&self, cx: &Client, v: &EntityView) -> bool {
        if let Some(z) = &self.zoom
            && !z.contains(v.model.x, v.model.y)
        {
            return false;
        }
        if !self.is_in(v, self.place) || self.title(cx, v).is_empty() {
            return false;
        }
        if self.immersion {
            if self.group.contains(&v.id()) {
                return false;
            }
            let a = angle(self.x, self.y, v.x(), v.y(), self.o);
            return a.cos() > 95f64.to_radians().cos();
        }
        !(self.is_an_exit(cx, v) && self.view().exit_is_blocked(v.id(), false))
    }

    /// The sound of moving from `place` to `new_square`, and whether the
    /// way is closed: it is to no square, or to the same one.
    fn get_prefix_and_collision(
        &self,
        cx: &Client,
        place: SquareId,
        new_square: Option<SquareId>,
        dxc: i64,
        dyc: i64,
    ) -> (Msg, bool) {
        let style = cx.res.style();
        let no_path = mine(&style.get("parameters").no_path_in_this_direction);
        let Some(new_square) = new_square.filter(|&s| s != place) else {
            return (no_path, true);
        };
        let (p, n) = (self.view().square(place), self.view().square(new_square));
        if !self.scouted_before_squares.contains(&place)
            || (p.is_water() || n.is_water()) && p.high_ground() == n.high_ground()
            || self.shouldnt_collide()
        {
            return (Vec::new(), false);
        }
        let (xc, yc) = self.coords_in_map(Some(place));
        let sw = self.square_width();
        let (x, y) = ((xc as f64 + 0.5) * sw, (yc as f64 + 0.5) * sw);
        for v in self.dobjets.values() {
            if !(self.is_in(v, Some(place))
                && self.is_visible(cx, v)
                && self.is_an_exit(cx, v)
                && !self.view().exit_is_blocked(v.id(), true))
            {
                continue;
            }
            if dxc == 1 && v.x() > x
                || dxc == -1 && v.x() < x
                || dyc == 1 && v.y() > y
                || dyc == -1 && v.y() < y
            {
                let prefix = if cx.config.parameters.play_movement_sound {
                    mine(&style.get(v.type_name()).when_moving_through)
                } else {
                    Vec::new()
                };
                return (prefix, false);
            }
        }
        (no_path, true)
    }

    /// The arrows: through the paths (or across, without collision), five
    /// squares at a time with Shift, or in the zoom, or turning in first
    /// person mode.
    pub(crate) fn cmd_select_square(
        &mut self,
        cx: &mut Client,
        dxc: i64,
        dyc: i64,
        no_collision: bool,
    ) {
        if self.immersion {
            if (dxc, dyc) == (-1, 0) {
                self.cmd_rotate_left(cx);
            } else if (dxc, dyc) == (1, 0) {
                self.cmd_rotate_right(cx);
            }
        } else if self.zoom.is_some() {
            self.zoom_move(cx, dxc, dyc);
            self.target = None;
            self.follow_mode = false;
            self.say_zoom(cx, &Vec::new());
        } else if let Some(mut place) = self.place {
            let n = (dxc + dyc).abs();
            let prefix = if n > 1 {
                let step = if dxc != 0 {
                    (dxc.signum(), 0)
                } else {
                    (0, dyc.signum())
                };
                let mut prefixes = Vec::new();
                for _ in 0..n {
                    let to = self.compute_move(place, step.0, step.1);
                    let (prefix, collision) =
                        self.get_prefix_and_collision(cx, place, to, step.0, step.1);
                    if !no_collision {
                        prefixes.extend(prefix);
                    }
                    match to {
                        Some(s) if !collision || no_collision => {
                            self.move_to_square(cx, s);
                            place = s;
                        }
                        _ => break,
                    }
                }
                prefixes
            } else if no_collision {
                if let Some(s) = self.compute_move(place, dxc, dyc) {
                    self.move_to_square(cx, s);
                    place = s;
                }
                Vec::new()
            } else {
                let to = self.compute_move(place, dxc, dyc);
                let (prefix, collision) = self.get_prefix_and_collision(cx, place, to, dxc, dyc);
                if let Some(s) = to.filter(|_| !collision) {
                    self.move_to_square(cx, s);
                    place = s;
                }
                prefix
            };
            self.select_and_say_square(cx, place, &prefix);
        }
    }

    /// Moves the zoom; past an edge, to the next square.
    fn zoom_move(&mut self, cx: &mut Client, dx: i64, dy: i64) {
        self.follow_mode = false;
        let Some(mut z) = self.zoom.take() else {
            return;
        };
        if let Some((sx, sy)) = z.step(dx, dy)
            && let Some(s) = self.place.and_then(|p| self.compute_move(p, sx, sy))
        {
            self.place = Some(s);
        }
        if let Some(p) = self.place {
            z.update_coords(self.view().square(p));
        }
        self.zoom = Some(z);
        self.set_obs_pos(cx);
    }

    /// Leaves first person mode and the zoom, before jumping to a square.
    pub(crate) fn leave_zoom_and_immersion(&mut self, cx: &mut Client) {
        if self.immersion {
            self.toggle_immersion(cx);
        }
        if self.zoom.is_some() {
            self.cmd_toggle_zoom(cx);
        }
    }

    fn select_square_from_list(&mut self, cx: &mut Client, increment: i64, squares: Vec<SquareId>) {
        if squares.is_empty() {
            cx.voice.item(&msg(Phrase::Nothing));
            return;
        }
        self.leave_zoom_and_immersion(cx);
        let mut squares = squares;
        if let Some(p) = self.place
            && !squares.contains(&p)
        {
            squares.push(p);
        }
        let view = self.view();
        if let Some(u) = view.own_units().next() {
            let (ux, uy) = (u.x as f64, u.y as f64);
            squares.sort_by(|a, b| {
                let (a, b) = (view.square(*a).center(), view.square(*b).center());
                distance(a.0 as f64, a.1 as f64, ux, uy)
                    .total_cmp(&distance(b.0 as f64, b.1 as f64, ux, uy))
            });
        }
        // `increment` squares from the selected one, round the list.
        let index = match self
            .place
            .and_then(|p| squares.iter().position(|&s| s == p))
        {
            Some(i) => (i as i64 + increment).rem_euclid(squares.len() as i64) as usize,
            None => 0,
        };
        self.select_and_say_square(cx, squares[index], &Vec::new());
    }

    pub(crate) fn cmd_select_scouted_square(&mut self, cx: &mut Client, increment: i64) {
        let squares = self.scouted_squares.iter().copied().collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_conflict_square(&mut self, cx: &mut Client, increment: i64) {
        let mut squares: Vec<Option<Place>> = Vec::new();
        for v in self.dobjets.values() {
            let enemy = v.player().is_some_and(|p| !self.view().allied(p, self.me));
            if enemy && !squares.contains(&v.place()) {
                squares.push(v.place());
            }
        }
        let squares = squares.into_iter().filter_map(|p| p?.square()).collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_unknown_square(&mut self, cx: &mut Client, increment: i64) {
        let squares = (0..self.view().square_count() as u32)
            .map(SquareId)
            .filter(|s| !self.scouted_before_squares.contains(s))
            .collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_resource_square(&mut self, cx: &mut Client, increment: i64) {
        let mut squares: Vec<Option<Place>> = Vec::new();
        for v in self.dobjets.values() {
            if self.resource_type(&v.model).is_some() && !squares.contains(&v.place()) {
                squares.push(v.place());
            }
        }
        let squares = squares.into_iter().filter_map(|p| p?.square()).collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_toggle_zoom(&mut self, cx: &mut Client) {
        let Some(p) = self.place else { return };
        if self.zoom.is_none() {
            self.zoom = Some(Zoom::new(self.view().square(p)));
            self.target = None;
            let prefix = [msg(Phrase::Zoom), msg(Phrase::IsNowOn), comma()].concat();
            self.say_zoom(cx, &prefix);
        } else {
            self.zoom = None;
            let prefix = [msg(Phrase::Zoom), msg(Phrase::IsNowOff), comma()].concat();
            self.select_and_say_square(cx, p, &prefix);
        }
    }

    /// The unit of the group nearest to its goal.
    fn group_head(&self) -> Option<&EntityView> {
        let mut best: Option<&EntityView> = None;
        for id in &self.group {
            let v = self.dobjets.get(id)?;
            let distance =
                |v: &EntityView| v.model.own.as_ref().map_or(INF, |o| o.distance_to_goal);
            if best.is_none_or(|b| distance(v) < distance(b)) {
                best = Some(v);
            }
        }
        best
    }

    /// In follow mode, the selection goes where the group goes.
    pub(crate) fn follow_if_needed(&mut self, cx: &mut Client) {
        self.update_group();
        if !(self.follow_mode && !self.an_order_requiring_a_target_is_selected()) {
            return;
        }
        // The group keeps only units the interface knows: one that is not
        // empty has a head.
        let Some(head) = self.group_head() else {
            return;
        };
        let (hx, hy, hplace) = (head.model.x, head.model.y, head.place());
        let head_is_here = self.is_in(head, self.place);
        if let Some(z) = &self.zoom {
            // A head on no square, as one that has left the map, is not
            // followed.
            if !z.contains(hx, hy)
                && let Some(s) = self.outer_square(hplace)
            {
                self.place = Some(s);
                let mut z = z.clone();
                let found = z.move_to(self.view().square(s), hx, hy);
                self.zoom = Some(z);
                if found {
                    self.set_obs_pos(cx);
                } else {
                    cx.log.warning("zoom: couldn't move to object");
                }
                if !cx.voice.is_busy() {
                    self.say_zoom(cx, &Vec::new());
                }
            }
        } else if !head_is_here {
            if let Some(s) = self.outer_square(hplace) {
                self.move_to_square(cx, s);
            }
            if !cx.voice.is_busy()
                && let Some(p) = self.place
            {
                let t = self.view().square(p).title().clone();
                cx.voice.item(&t);
            }
            if self.immersion {
                self.target = None;
            }
        }
    }

    /// Places the listener: on the first unit's square at first, then
    /// following the group, the zoom or the selected square.
    pub(crate) fn set_obs_pos(&mut self, cx: &mut Client) {
        if self.place.is_none() {
            let units = self.units(cx, true, true);
            if let Some(&first) = units.first()
                && let Some(s) = self.outer_square(self.dobjets[&first].place())
            {
                self.select_and_say_square(cx, s, &Vec::new());
            }
        }
        self.follow_if_needed(cx);
        let head = self.group.first().and_then(|g| self.dobjets.get(g));
        if self.immersion
            && let Some(v) = head
        {
            self.x = v.x();
            self.y = v.y();
            self.o = v.o();
        } else if let Some(z) = &self.zoom {
            (self.x, self.y) = z.obs_pos();
        } else {
            let (xc, yc) = self.coords_in_map(self.place);
            let sw = self.square_width();
            let x = sw * (xc as f64 + 0.5);
            let y = sw * (yc as f64 + 1.0 / 8.0);
            if (x, y) != (self.x, self.y) {
                let k = ((cx.clock.now() - self.previous_animation)
                    * cx.config.parameters.observer_reactivity)
                    .min(1.0);
                self.x += (x - self.x) * k;
                self.y += (y - self.y) * k;
            }
        }
        if !self.immersion {
            self.o = 90.0;
        }
        self.update_listener(cx);
    }

    /// The noises of the selected square's terrain (and of the squares
    /// around), for the squares known.
    pub(crate) fn animate_terrain(&mut self, cx: &mut Client) {
        let Some(p) = self.place else { return };
        let mut squares = vec![p];
        if cx.config.parameters.render_nearby_land {
            squares.extend(self.view().square(p).neighbors().iter().copied());
        }
        squares.retain(|s| {
            self.scouted_squares.contains(s) || self.scouted_before_squares.contains(s)
        });
        let player = NoisePlayer {
            audio: &cx.audio,
            sounds: &cx.sounds,
            rng: &cx.rng,
            now: cx.clock.now(),
            fog_of_war_factor: cx.config.parameters.fog_of_war_factor,
        };
        let mut kept = Vec::new();
        for (s, mut n) in std::mem::take(&mut self.terrain_noises) {
            if !squares.contains(&s) {
                n.stop(&cx.audio);
            } else {
                let (x, y) = self.view().square(s).center();
                n.update(
                    &player,
                    NoiseSite {
                        x: x as f64 / 1000.0,
                        y: y as f64 / 1000.0,
                        fow: !self.scouted_squares.contains(&s),
                    },
                );
                kept.push((s, n));
            }
        }
        for s in squares {
            if kept.iter().any(|(k, _)| *k == s) {
                continue;
            }
            let t = self.view().terrain(s);
            if t.is_empty() {
                continue;
            }
            let st = cx.res.style().get(t).noise.clone();
            let st = st.unwrap_or(crate::style::Noise::Silent);
            if let Some(n) = NoiseSource::new("noise", &st) {
                kept.push((s, n));
            }
        }
        self.terrain_noises = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::super::GameInterface;
    use super::super::testing::{cheat, enemy, go, run_until, said, training, training_on, unit};
    use super::*;
    use soundrts_world::EntityId;

    #[test]
    fn a_square_in_the_fog_has_the_terrain_last_observed() {
        let (mut h, mut g) = training();
        let townhall = unit(&g, g.game.me, "townhall");
        let game = &mut g.game;
        let home = game.host().world().entity(townhall).unwrap().square();
        let home = home.unwrap();
        // Observed: the terrain noted at the end of each turn.
        assert!(game.scouted_squares.contains(&home));
        assert_eq!(game.view().terrain(home), "_meadows");
        let m = game.square_postfix(&h.client, home);
        assert_eq!(said(&mut h, &m), ", meadows");
        // Glimpsed from the next square: its terrain too, in the fog.
        let a1 = game.view().square_at(0, 0).unwrap();
        assert!(!game.scouted_squares.contains(&a1));
        assert!(game.view().partly_observed_squares().contains(&a1));
        let m = game.square_postfix(&h.client, a1);
        assert_eq!(said(&mut h, &m), ", meadows , in the fog of war");
        // Never observed nor glimpsed: unknown, with no terrain.
        let b3 = game.view().square_at(1, 2).unwrap();
        assert_eq!(game.host().world().square(b3).type_name, "_meadows");
        assert_eq!(game.view().terrain(b3), "");
        let m = game.square_postfix(&h.client, b3);
        assert_eq!(said(&mut h, &m), ", unknown");
    }

    #[test]
    fn a_glimpsed_square_says_its_terrain_and_plays_its_noise() {
        let (mut h, g) = training_on("jl5");
        // On jl5, the sea of b 1 is glimpsed from the start square.
        let b1 = g.game.view().square_at(1, 0).unwrap();
        assert!(!g.game.scouted_squares.contains(&b1));
        assert!(g.game.view().partly_observed_squares().contains(&b1));
        let m = g.game.square_postfix(&h.client, b1);
        assert_eq!(said(&mut h, &m), ", sea , in the fog of war");
        // The game's animation plays the terrain noise of the selected
        // square and of those around it: the sea of b 1 among them.
        assert!(g.game.terrain_noises.iter().any(|(s, _)| *s == b1));
        assert!(h.audio.played().iter().any(|(_, s)| s == "ambience/lake"));
    }

    #[test]
    fn a_forest_cut_out_of_sight_stays_a_forest_until_seen_again() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let peasant = unit(&g, me, "peasant");
        let (home, a3, a4) = {
            let v = g.game.view();
            let home = v.square_at(1, 0).unwrap();
            (home, v.square_at(0, 2).unwrap(), v.square_at(0, 3).unwrap())
        };
        let glimpsed = |g: &GameInterface, s| {
            g.scouted_squares.contains(&s) || g.view().partly_observed_squares().contains(&s)
        };
        // The player's peasant glimpses the forest of a 4 from a 3, then
        // goes home, out of sight of it.
        go(&mut g, me, peasant, a3);
        run_until(&mut h, &mut g, |g| glimpsed(g, a4));
        let turn = g.game.view().turn() + 1;
        h.run_until_turn(&mut g, turn);
        assert_eq!(g.game.view().terrain(a4), "_forest");
        go(&mut g, me, peasant, home);
        run_until(&mut h, &mut g, |g| !glimpsed(g, a4));
        // The computer cuts the only wood of a 4, out of the player's sight.
        cheat(&mut g, them, "add_units a4 10 peasant");
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        let w = g.game.host().world();
        let on_a4 = |t: &str| -> Vec<EntityId> {
            w.square(a4)
                .objects
                .iter()
                .copied()
                .filter(|&o| &*w.entity(o).unwrap().type_name == t)
                .collect()
        };
        // The computer's own peasant may be there too.
        let (wood, cutters) = (on_a4("wood"), on_a4("peasant"));
        assert_eq!(wood.len(), 1);
        assert!(cutters.len() >= 10);
        let ids: Vec<String> = cutters.iter().map(|u| u.0.to_string()).collect();
        let host = g.game.host_mut();
        host.queue_line(them, &format!("control {}", ids.join(" ")));
        host.queue_line(them, &format!("order 0 0 gather {}", wood[0].0));
        for _ in 0..3000 {
            if g.game.host().world().square(a4).type_name != "_forest" {
                break;
            }
            assert!(!glimpsed(&g.game, a4));
            let next = g.game.view().turn() + 1;
            h.run_until_turn(&mut g, next);
        }
        assert_eq!(g.game.host().world().square(a4).type_name, "_meadows");
        assert_eq!(g.game.view().terrain(a4), "_forest");
        assert!(!glimpsed(&g.game, a4));
        let m = g.game.square_postfix(&h.client, a4);
        assert_eq!(said(&mut h, &m), ", forest , in the fog of war");
        // Seen again: meadows.
        go(&mut g, me, peasant, a3);
        run_until(&mut h, &mut g, |g| glimpsed(g, a4));
        let turn = g.game.view().turn() + 1;
        h.run_until_turn(&mut g, turn);
        let m = g.game.square_postfix(&h.client, a4);
        assert_eq!(said(&mut h, &m), ", meadows , in the fog of war");
    }

    #[test]
    fn moves_wrap_around_the_edges_of_the_map() {
        let max = (4, 2);
        assert_eq!(wrapped_coords((0, 0), (1, 0), max), (1, 0));
        assert_eq!(wrapped_coords((0, 0), (-1, 0), max), (4, 0));
        assert_eq!(wrapped_coords((4, 2), (1, 1), max), (0, 0));
        assert_eq!(wrapped_coords((0, 1), (0, -1), max), (0, 0));
        assert_eq!(wrapped_coords((0, 0), (0, -1), max), (0, 2));
    }
}
