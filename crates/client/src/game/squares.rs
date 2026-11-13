//! Moving through the squares of the map, and what a square says: its
//! name, terrain, fog, and the summary of what is on it.

use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::rules::AirGround;
use soundrts_world::{Place, SquareId};

use crate::audio::{angle, distance};
use crate::client::Client;

use super::animation::{NoisePlayer, NoiseSite, NoiseSource};
use super::entity::EntityView;
use super::focus::Zoom;
use super::{GameInterface, Oops, R, style_msg};

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

/// "3 footmen, 2 archers and 1 peasant": each kind with its count, in the
/// order they come. With `brief` and more than two kinds, only the count.
pub fn summary(group: &[Msg], brief: bool) -> Msg {
    let types = remove_duplicates(group);
    if brief && types.len() > 2 {
        return nb2msg(group.len() as i64);
    }
    let mut result = Vec::new();
    for t in &types {
        if Some(t) == types.last() && types.len() > 1 {
            result.extend(msg(mp::AND));
        } else if Some(t) != types.first() {
            result.extend(msg(mp::COMMA));
        }
        result.extend(nb2msg(group.iter().filter(|x| *x == t).count() as i64));
        result.extend(t.iter().cloned());
    }
    result
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
        let xcmax = self.world.squares.iter().map(|s| s.col).max().unwrap_or(0);
        let ycmax = self.world.squares.iter().map(|s| s.row).max().unwrap_or(0);
        (xcmax, ycmax)
    }

    /// The column and row of a square, or (-1, -1).
    pub fn coords_in_map(&self, square: Option<SquareId>) -> (i64, i64) {
        match square {
            Some(s) => {
                let s = self.world.square(s);
                (s.col, s.row)
            }
            None => (-1, -1),
        }
    }

    pub(crate) fn compute_move(&self, dxc: i64, dyc: i64) -> R<SquareId> {
        let (xc, yc) = wrapped_coords(self.coords_in_map(self.place), (dxc, dyc), self.grid_max());
        self.world
            .square_at(xc, yc)
            .ok_or_else(|| Oops::missing(format!("({xc}, {yc})")))
    }

    /// Selects a square; the noises of the previous one stop.
    pub(crate) fn move_to_square(&mut self, cx: &Client, square: SquareId) {
        if self.place != Some(square) {
            self.place = Some(square);
            if cx.params.flag_or("silence_previous_square", true) {
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

    fn square_terrain(&self, cx: &Client, place: SquareId) -> Msg {
        let mut result = Vec::new();
        let sq = self.world.square(place);
        if !sq.type_name.is_empty() {
            let title = style_msg(cx.res.style(), &sq.type_name, "title");
            if !title.is_empty() {
                result.extend(msg(mp::COMMA));
                result.extend(title);
            }
        }
        if sq.high_ground {
            result.extend(msg(mp::COMMA));
            result.extend(msg(mp::PLATEAU));
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
            postfix.extend(msg(mp::COMMA));
            postfix.extend(msg(mp::IN_THE_FOG));
        } else {
            postfix.extend(msg(mp::COMMA));
            postfix.extend(msg(mp::UNKNOWN));
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
        let allied = &self.world.player(self.me).allied;
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
                if allied.contains(&p) && p != self.me {
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
            result.extend(msg(mp::COMMA));
            result.extend(summary(&enemies, brief));
            result.extend(msg(mp::ENEMY));
        }
        if me && !allies.is_empty() {
            result.extend(msg(mp::COMMA));
            result.extend(summary(&allies, false));
            result.extend(msg(mp::ALLY));
        }
        if me && !units.is_empty() {
            result.extend(msg(mp::COMMA));
            result.extend(summary(&units, false));
        }
        if !resources.is_empty() && (enemies.is_empty() || !brief) {
            result.extend(msg(mp::COMMA));
            result.extend(summary(&resources, false));
        }
        result
    }

    /// Says a square: name, terrain and fog, then what is on it.
    pub fn say_square(&self, cx: &mut Client, place: Option<SquareId>, prefix: &Msg) {
        let Some(p) = place else { return };
        let m = [
            prefix.clone(),
            self.world.square(p).title.clone(),
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
            z.title(self.world.square(p)),
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
                .is_some_and(|v| v.model.airground_type == AirGround::Air)
        }) || self
            .order
            .as_ref()
            .is_some_and(|o| o.nb_args != 0 && o.target_shouldnt_collide(&self.world))
    }

    /// Whether the player may see the object now: in the zoom, on the
    /// square, with a title; in first person mode, in front; an exit not
    /// blocked.
    pub fn is_visible(&self, cx: &Client, v: &EntityView) -> R<bool> {
        if let Some(z) = &self.zoom
            && !z.contains(v.model.x, v.model.y)
        {
            return Ok(false);
        }
        if !self.is_in(v, self.place) || self.title(cx, v)?.is_empty() {
            return Ok(false);
        }
        if self.immersion {
            if self.group.contains(&v.id()) {
                return Ok(false);
            }
            let a = angle(self.x, self.y, v.x(), v.y(), self.o);
            return Ok(a.cos() > 95f64.to_radians().cos());
        }
        if self.is_an_exit(cx, v) && self.exit_is_blocked(v.id(), None) {
            return Ok(false);
        }
        Ok(true)
    }

    /// The sound of moving to a square, and whether the way is closed.
    fn get_prefix_and_collision(
        &self,
        cx: &Client,
        new_square: SquareId,
        dxc: i64,
        dyc: i64,
    ) -> R<(Msg, bool)> {
        let style = cx.res.style();
        let no_path = style_msg(style, "parameters", "no_path_in_this_direction");
        let Some(place) = self.place else {
            return Err(Oops::new("'NoneType' object has no attribute 'is_water'"));
        };
        if new_square == place {
            return Ok((no_path, true));
        }
        let (p, n) = (self.world.square(place), self.world.square(new_square));
        if !self.scouted_before_squares.contains(&place)
            || (p.is_water || n.is_water) && p.height() == n.height()
            || self.shouldnt_collide()
        {
            return Ok((Vec::new(), false));
        }
        let (xc, yc) = self.coords_in_map(Some(place));
        let sw = self.square_width();
        let (x, y) = ((xc as f64 + 0.5) * sw, (yc as f64 + 0.5) * sw);
        for v in self.dobjets.values() {
            if !(self.is_in(v, Some(place))
                && self.is_visible(cx, v)?
                && self.is_an_exit(cx, v)
                && !self.exit_is_blocked(v.id(), Some(self.me)))
            {
                continue;
            }
            if dxc == 1 && v.x() > x
                || dxc == -1 && v.x() < x
                || dyc == 1 && v.y() > y
                || dyc == -1 && v.y() < y
            {
                let prefix = if cx.params.flag_or("play_movement_sound", true) {
                    style_msg(style, v.type_name(), "when_moving_through")
                } else {
                    Vec::new()
                };
                return Ok((prefix, false));
            }
        }
        Ok((no_path, true))
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
    ) -> R {
        if self.immersion {
            if (dxc, dyc) == (-1, 0) {
                self.cmd_rotate_left(cx)?;
            } else if (dxc, dyc) == (1, 0) {
                self.cmd_rotate_right(cx)?;
            }
        } else if self.zoom.is_some() {
            self.zoom_move(cx, dxc, dyc)?;
            self.target = None;
            self.follow_mode = false;
            self.say_zoom(cx, &Vec::new());
        } else if self.place.is_some() {
            let n = (dxc + dyc).abs();
            if n > 1 {
                let step = if dxc != 0 {
                    (dxc.signum(), 0)
                } else {
                    (0, dyc.signum())
                };
                let mut prefixes = Vec::new();
                for _ in 0..n {
                    let new_square = self.compute_move(step.0, step.1)?;
                    let (prefix, collision) =
                        self.get_prefix_and_collision(cx, new_square, step.0, step.1)?;
                    if !no_collision {
                        prefixes.extend(prefix);
                    }
                    if !collision || no_collision {
                        self.move_to_square(cx, new_square);
                    } else {
                        break;
                    }
                }
                let place = self.place.expect("checked");
                self.select_and_say_square(cx, place, &prefixes);
            } else if no_collision {
                let new_square = self.compute_move(dxc, dyc)?;
                self.move_to_square(cx, new_square);
                let place = self.place.expect("checked");
                self.select_and_say_square(cx, place, &Vec::new());
            } else {
                let new_square = self.compute_move(dxc, dyc)?;
                let (prefix, collision) =
                    self.get_prefix_and_collision(cx, new_square, dxc, dyc)?;
                if !collision {
                    self.move_to_square(cx, new_square);
                }
                let place = self.place.expect("checked");
                self.select_and_say_square(cx, place, &prefix);
            }
        }
        Ok(())
    }

    /// Moves the zoom; past an edge, to the next square.
    fn zoom_move(&mut self, cx: &mut Client, dx: i64, dy: i64) -> R {
        self.follow_mode = false;
        let Some(mut z) = self.zoom.take() else {
            return Ok(());
        };
        let crossing = z.step(dx, dy);
        let r = match crossing {
            Some((sx, sy)) => self.compute_move(sx, sy).map(|s| self.place = Some(s)),
            None => Ok(()),
        };
        if let Some(p) = self.place {
            z.update_coords(self.world.square(p));
        }
        self.zoom = Some(z);
        r?;
        self.set_obs_pos(cx)
    }

    fn select_square_from_list(
        &mut self,
        cx: &mut Client,
        increment: i64,
        squares: Vec<SquareId>,
    ) -> R {
        if squares.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            return Ok(());
        }
        if self.immersion {
            self.toggle_immersion(cx)?;
        }
        if self.zoom.is_some() {
            self.cmd_toggle_zoom(cx);
        }
        let mut squares = squares;
        if let Some(p) = self.place
            && !squares.contains(&p)
        {
            squares.push(p);
        }
        if let Some(&u) = self.world.player(self.me).units.first()
            && let Some(u) = self.world.entity(u)
        {
            let (ux, uy) = (u.x as f64, u.y as f64);
            squares.sort_by(|a, b| {
                let (a, b) = (self.world.square(*a), self.world.square(*b));
                distance(a.x as f64, a.y as f64, ux, uy)
                    .total_cmp(&distance(b.x as f64, b.y as f64, ux, uy))
            });
        }
        let len = squares.len() as i64;
        let mut index = match self
            .place
            .and_then(|p| squares.iter().position(|&s| s == p))
        {
            Some(i) => i as i64 + increment,
            None => 0,
        };
        if index < 0 {
            index = len - 1;
        } else if index == len {
            index = 0;
        }
        let square = *squares
            .get(index as usize)
            .ok_or_else(|| Oops::new("list index out of range"))?;
        self.select_and_say_square(cx, square, &Vec::new());
        Ok(())
    }

    pub(crate) fn cmd_select_scouted_square(&mut self, cx: &mut Client, increment: i64) -> R {
        let squares = self.scouted_squares.iter().copied().collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_conflict_square(&mut self, cx: &mut Client, increment: i64) -> R {
        let mut squares: Vec<Option<Place>> = Vec::new();
        for v in self.dobjets.values() {
            let enemy = v
                .player()
                .is_some_and(|p| !self.world.player(p).allied.contains(&self.me));
            if enemy && !squares.contains(&v.place()) {
                squares.push(v.place());
            }
        }
        let squares = squares.into_iter().filter_map(|p| p?.square()).collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_unknown_square(&mut self, cx: &mut Client, increment: i64) -> R {
        let squares = (0..self.world.squares.len() as u32)
            .map(SquareId)
            .filter(|s| !self.scouted_before_squares.contains(s))
            .collect();
        self.select_square_from_list(cx, increment, squares)
    }

    pub(crate) fn cmd_select_resource_square(&mut self, cx: &mut Client, increment: i64) -> R {
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
            self.zoom = Some(Zoom::new(self.world.square(p)));
            self.target = None;
            let prefix = [msg(mp::ZOOM), msg(mp::IS_NOW_ON), msg(mp::COMMA)].concat();
            self.say_zoom(cx, &prefix);
        } else {
            self.zoom = None;
            let prefix = [msg(mp::ZOOM), msg(mp::IS_NOW_OFF), msg(mp::COMMA)].concat();
            self.select_and_say_square(cx, p, &prefix);
        }
    }

    /// The unit of the group nearest to its goal.
    fn group_head(&self) -> Option<&EntityView> {
        let mut best: Option<&EntityView> = None;
        for id in &self.group {
            let v = self.dobjets.get(id)?;
            if best.is_none_or(|b| v.model.distance_to_goal < b.model.distance_to_goal) {
                best = Some(v);
            }
        }
        best
    }

    /// In follow mode, the selection goes where the group goes.
    pub(crate) fn follow_if_needed(&mut self, cx: &mut Client) -> R {
        self.update_group();
        if !(self.follow_mode
            && !self.group.is_empty()
            && !self.an_order_requiring_a_target_is_selected())
        {
            return Ok(());
        }
        let head = self
            .group_head()
            .ok_or_else(|| Oops::missing("group head"))?;
        let (hx, hy, hplace) = (head.model.x, head.model.y, head.place());
        let head_is_here = self.is_in(head, self.place);
        if let Some(z) = &self.zoom {
            if !z.contains(hx, hy) {
                let s = self
                    .outer_square(hplace)
                    .ok_or_else(|| Oops::new("'NoneType' object has no attribute 'xmin'"))?;
                self.place = Some(s);
                let mut z = z.clone();
                let found = z.move_to(self.world.square(s), hx, hy);
                self.zoom = Some(z);
                if found {
                    self.set_obs_pos(cx)?;
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
                let t = self.world.square(p).title.clone();
                cx.voice.item(&t);
            }
            if self.immersion {
                self.target = None;
            }
        }
        Ok(())
    }

    /// Places the listener: on the first unit's square at first, then
    /// following the group, the zoom or the selected square.
    pub(crate) fn set_obs_pos(&mut self, cx: &mut Client) -> R {
        if self.place.is_none() {
            let units = self.units(cx, true, true);
            if let Some(&first) = units.first()
                && let Some(s) = self.outer_square(self.dobjets[&first].place())
            {
                self.select_and_say_square(cx, s, &Vec::new());
            }
        }
        self.follow_if_needed(cx)?;
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
                    * cx.params.number("observer_reactivity", 1000.0))
                .min(1.0);
                self.x += (x - self.x) * k;
                self.y += (y - self.y) * k;
            }
        }
        if !self.immersion {
            self.o = 90.0;
        }
        self.update_listener(cx);
        Ok(())
    }

    /// The noises of the selected square's terrain (and of the squares
    /// around), for the squares known.
    pub(crate) fn animate_terrain(&mut self, cx: &mut Client) {
        let Some(p) = self.place else { return };
        let mut squares = vec![p];
        if cx.params.flag_or("render_nearby_land", false) {
            squares.extend(self.world.square(p).neighbors.iter().copied());
        }
        squares.retain(|s| {
            self.scouted_squares.contains(s) || self.scouted_before_squares.contains(s)
        });
        let player = NoisePlayer {
            audio: &cx.audio,
            sounds: &cx.sounds,
            rng: &cx.rng,
            now: cx.clock.now(),
            fog_of_war_factor: cx.params.number("fog_of_war_factor", 0.5),
        };
        let mut kept = Vec::new();
        for (s, mut n) in std::mem::take(&mut self.terrain_noises) {
            if !squares.contains(&s) {
                n.stop(&cx.audio);
            } else {
                let sq = self.world.square(s);
                n.update(
                    &player,
                    NoiseSite {
                        x: sq.x as f64 / 1000.0,
                        y: sq.y as f64 / 1000.0,
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
            let t = &self.world.square(s).type_name;
            if t.is_empty() {
                continue;
            }
            let st = cx.res.style().noise(t, "noise");
            if let Some(n) = NoiseSource::new("noise", &st) {
                kept.push((s, n));
            }
        }
        self.terrain_noises = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundrts_world::msg::MsgPart;

    fn t(n: u64) -> Msg {
        vec![MsgPart::Num(n)]
    }

    #[test]
    fn a_summary_counts_each_kind_in_order() {
        let n = |k| nb2msg(k);
        assert_eq!(summary(&[], false), Vec::new());
        assert_eq!(summary(&[t(1), t(1)], false), [n(2), t(1)].concat());
        assert_eq!(
            summary(&[t(1), t(2), t(1), t(3)], false),
            [
                n(2),
                t(1),
                msg(mp::COMMA),
                n(1),
                t(2),
                msg(mp::AND),
                n(1),
                t(3)
            ]
            .concat()
        );
        assert_eq!(
            summary(&[t(1), t(2)], true),
            [n(1), t(1), msg(mp::AND), n(1), t(2)].concat()
        );
        assert_eq!(summary(&[t(1), t(2), t(3), t(3)], true), n(4));
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
