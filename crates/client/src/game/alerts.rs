//! What the player learns without asking: the fog of war kept up to date,
//! and the alerts about resources, food, menus, units and attacks.

use std::sync::Arc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{EntityId, Place, SquareId};

use crate::audio::stereo;
use crate::client::Client;
use crate::message::InfoOptions;
use crate::sound::StereoVolume;

use super::entity::EntityView;
use super::news::must_be_said;
use super::order::{PRECISION, nb2msg_f};
use super::squares::summary;
use super::{ALERT_LIMIT, GameInterface, R, style_msg};

/// Orders that do not make a menu richer.
const IRRELEVANT_ORDERS: &[&str] = &[
    "stop",
    "cancel_training",
    "cancel_upgrading",
    "cancel_building",
    "mode_offensive",
    "mode_defensive",
    "load",
    "load_all",
    "unload",
    "unload_all",
];

fn relevant_menu(menu: &[String]) -> Vec<String> {
    let mut m = menu.to_vec();
    for x in IRRELEVANT_ORDERS {
        if let Some(i) = m.iter().position(|o| o == x) {
            m.remove(i);
        }
    }
    m
}

/// A key that sorts titles as Python sorts lists of numbers.
fn title_key(t: &Msg) -> Vec<(u64, String)> {
    t.iter()
        .map(|p| match p {
            MsgPart::Num(n) => (*n, String::new()),
            MsgPart::Text(s) | MsgPart::Literal(s) => (u64::MAX, s.clone()),
        })
        .collect()
}

impl GameInterface {
    /// The resources of the player, in units.
    pub fn resources(&self) -> Vec<i64> {
        self.world
            .player(self.me)
            .resources
            .iter()
            .map(|r| r / PRECISION)
            .collect()
    }

    pub fn available_food(&self) -> i64 {
        self.world
            .player(self.me)
            .available_food(self.world.food_limit())
    }

    pub fn used_food(&self) -> i64 {
        self.world.player(self.me).used_food
    }

    /// Says a message once the game has started, dropped after 1.5 s.
    fn send_msg_if_playing(&self, cx: &mut Client, m: &Msg, update_type: &str) {
        if self.last_virtual_time != 0.0 {
            cx.voice.info_with(
                m,
                InfoOptions {
                    expiration_delay: 1.5,
                    update_type: Some(update_type.to_owned()),
                    ..InfoOptions::default()
                },
            );
        }
    }

    pub(crate) fn food_msg(&self, cx: &Client) -> Msg {
        [
            nb2msg_f(self.available_food()),
            style_msg(cx.res.style(), "parameters", "food_title"),
            msg(mp::TO),
            nb2msg(self.used_food()),
        ]
        .concat()
    }

    /// Says the resources that changed to a round count, and the food when
    /// it is almost all used.
    pub(crate) fn send_resource_alerts_if_needed(&mut self, cx: &mut Client) {
        let resources = self.resources();
        let previous = self
            .previous_resources
            .get_or_insert_with(|| resources.clone())
            .clone();
        let mut previous = previous;
        for (i, &r) in resources.iter().enumerate() {
            if previous.get(i) != Some(&r) {
                if i < previous.len() {
                    previous[i] = r;
                }
                if cx.config.verbosity.contains("resources") && must_be_said(r) {
                    let m = [
                        nb2msg(r),
                        style_msg(cx.res.style(), "parameters", &format!("resource_{i}_title")),
                    ]
                    .concat();
                    self.send_msg_if_playing(cx, &m, &format!("resource_{i}"));
                }
            }
        }
        self.previous_resources = Some(previous);
        let (available, used) = (self.available_food(), self.used_food());
        if available != self.previous_available_food
            || used > self.previous_used_food
            || used < self.previous_used_food && self.previous_used_food == available
        {
            if cx.config.verbosity.contains("food")
                && 0 <= available - used
                && (available - used) as f64 <= available as f64 * 0.20
            {
                let m = self.food_msg(cx);
                self.send_msg_if_playing(cx, &m, "food");
            }
            self.previous_available_food = available;
            self.previous_used_food = used;
        }
    }

    /// Says when a kind of unit can do more than before.
    pub(crate) fn send_menu_alerts_if_needed(&mut self, cx: &mut Client) {
        if !cx.config.verbosity.contains("menu_changed") {
            return;
        }
        let mut done: Vec<String> = Vec::new();
        for u in self.world.player(self.me).units.clone() {
            let Some(e) = self.world.entity(u) else {
                continue;
            };
            // A view of each unit, as the original makes one: it draws its
            // footstep offset even when its type is done.
            let view = EntityView::new(Arc::new(e.clone()), None, &cx.rng);
            let type_name = e.type_name.to_string();
            if done.contains(&type_name) {
                continue;
            }
            let menu = relevant_menu(&self.view_menu(u, true));
            let title = self.short_title(cx, &view);
            match self.previous_menus.get(&type_name) {
                None => {
                    self.previous_menus.insert(type_name.clone(), Vec::new());
                }
                Some(previous) => {
                    if menu.iter().any(|i| !previous.contains(i)) {
                        let m = [msg(mp::MENU_OF), title, msg(mp::CHANGED)].concat();
                        cx.voice.info(&m);
                    }
                }
            }
            let previous = self.previous_menus.get_mut(&type_name).expect("inserted");
            for i in menu {
                if !previous.contains(&i) {
                    previous.push(i);
                }
            }
            done.push(type_name);
        }
    }

    /// The title of a place an alert names.
    fn alert_place_title(&self, cx: &Client, place: Place) -> Msg {
        self.place_title(cx, place)
    }

    fn tell_enemies_in_square(&self, cx: &mut Client, place: Place) {
        let square = self.outer_square(Some(place));
        let enemies: Vec<Msg> = self
            .dobjets
            .values()
            .filter(|v| self.is_in(v, square) && self.is_an_enemy(&v.model))
            .map(|v| self.short_title(cx, v))
            .collect();
        if !enemies.is_empty() {
            let m = [
                summary(&enemies, true),
                msg(mp::ENEMY),
                msg(mp::AT),
                self.alert_place_title(cx, place),
            ]
            .concat();
            cx.voice.info(&m);
        }
    }

    fn units_alert(
        &self,
        cx: &mut Client,
        units: &mut Vec<(Msg, Option<Place>)>,
        end: &[u64],
        brief: bool,
    ) {
        let mut places: Vec<Place> = Vec::new();
        for (_, p) in units.iter() {
            if let Some(p) = p
                && !places.contains(p)
            {
                places.push(*p);
            }
        }
        self.sort_by_square_id(&mut places);
        for place in places {
            let in_place: Vec<Msg> = units
                .iter()
                .filter(|(_, p)| *p == Some(place))
                .map(|(t, _)| t.clone())
                .collect();
            let s = summary(&in_place, brief);
            if !s.is_empty() {
                let m = [s, msg(end), msg(mp::AT), self.alert_place_title(cx, place)].concat();
                cx.voice.info(&m);
            }
        }
        units.clear();
    }

    /// Says the units lost, neutralized and the new enemies, at most every
    /// ten seconds.
    pub(crate) fn units_alert_if_needed(&mut self, cx: &mut Client, place: Option<Place>) -> R {
        let now = cx.clock.now();
        if (!self.neutralized_units.is_empty()
            || !self.lost_units.is_empty()
            || !self.new_enemy_units.is_empty())
            && self.previous_units_alert.is_none_or(|t| now > t + 10.0)
        {
            let mut n = std::mem::take(&mut self.neutralized_units);
            self.units_alert(cx, &mut n, mp::NEUTRALIZED, false);
            let mut l = std::mem::take(&mut self.lost_units);
            self.units_alert(cx, &mut l, mp::LOST, true);
            let mut e = std::mem::take(&mut self.new_enemy_units);
            self.units_alert(cx, &mut e, mp::ENEMY, true);
            if let Some(p) = place {
                self.tell_enemies_in_square(cx, p);
            }
            self.previous_units_alert = Some(now);
        }
        Ok(())
    }

    /// Says the squares recently attacked, at most every ten seconds.
    pub(crate) fn squares_alert_if_needed(&mut self, cx: &mut Client) {
        let now = cx.clock.now();
        if self.alert_squares.is_empty()
            || self.previous_squares_alert.is_some_and(|t| now <= t + 10.0)
        {
            return;
        }
        let mut titles: Vec<Msg> = self
            .alert_squares
            .iter()
            .filter(|(_, t)| now < **t + 5.0)
            .map(|(p, _)| self.alert_place_title(cx, *p))
            .collect();
        titles.sort_by_key(title_key);
        if titles.len() > 1 {
            let i = titles.len() - 1;
            titles.insert(i, msg(mp::AND));
        }
        if !titles.is_empty() {
            let mut m = [msg(mp::ALERT), msg(mp::AT)].concat();
            for t in titles {
                m.extend(t);
            }
            cx.voice.info(&m);
            self.previous_squares_alert = Some(now);
        }
        self.alert_squares.clear();
    }

    /// Says what was found on newly seen squares with resources, at most
    /// every ten seconds.
    pub(crate) fn scout_info_if_needed(&mut self, cx: &mut Client) -> R {
        if !cx.config.verbosity.contains("scout_info") {
            return Ok(());
        }
        let now = cx.clock.now();
        if !self.scout_info.is_empty() && self.previous_scout_info.is_none_or(|t| now > t + 10.0) {
            let mut places: Vec<Place> = std::mem::take(&mut self.scout_info).into_iter().collect();
            self.sort_by_square_id(&mut places);
            for place in places {
                let s = self.place_summary(cx, self.outer_square(Some(place)), false, None, true);
                if !s.is_empty() {
                    let m = [s, msg(mp::AT), self.alert_place_title(cx, place)].concat();
                    cx.voice.info(&m);
                }
            }
            self.previous_scout_info = Some(now);
        }
        Ok(())
    }

    /// Orders places as the reference orders the original's sets of
    /// squares: by square id.
    fn sort_by_square_id(&self, places: &mut [Place]) {
        places.sort_by_key(|p| {
            self.outer_square(Some(*p))
                .map_or(u32::MAX, |s| self.world.square(s).id)
        });
    }

    fn must_report_resource(&mut self, place: Option<Place>, is_resource: bool) -> bool {
        is_resource && self.known_resource_places.insert(place)
    }

    pub(crate) fn delete_object(&mut self, cx: &Client, id: EntityId) {
        if let Some(v) = self.dobjets.shift_remove(&id) {
            v.stop(&cx.audio);
        }
        self.group.retain(|&g| g != id);
    }

    /// Makes the views match what the player remembered and perceived at
    /// the end of the last turn.
    pub(crate) fn update_fog_of_war(
        &mut self,
        cx: &Client,
        memory: Vec<(Arc<soundrts_world::Entity>, i64)>,
        perception: Vec<Arc<soundrts_world::Entity>>,
    ) {
        for (m, ts) in &memory {
            if self.dobjets.get(&m.id).is_some_and(|v| !v.is_memory()) {
                self.delete_object(cx, m.id);
            }
            match self.dobjets.get_mut(&m.id) {
                Some(v) => {
                    v.model = m.clone();
                    v.time_stamp = Some(*ts);
                }
                None => {
                    let v = EntityView::new(m.clone(), Some(*ts), &cx.rng);
                    self.dobjets.insert(m.id, v);
                    let is_resource = self.resource_type(m).is_some();
                    if self.must_report_resource(m.place, is_resource)
                        && let Some(place) = m.place
                    {
                        self.scout_info.insert(place);
                    }
                }
            }
        }
        for m in &perception {
            if !self.dobjets.contains_key(&m.id) {
                if self.is_an_enemy(m) {
                    let v = EntityView::new(m.clone(), None, &cx.rng);
                    let t = self.short_title(cx, &v);
                    self.new_enemy_units.push((t, m.place));
                }
                let is_resource = self.resource_type(m).is_some();
                if self.must_report_resource(m.place, is_resource)
                    && let Some(place) = m.place
                {
                    self.scout_info.insert(place);
                }
            } else if self.dobjets[&m.id].is_memory() {
                self.delete_object(cx, m.id);
            }
            match self.dobjets.get_mut(&m.id) {
                Some(v) => v.model = m.clone(),
                None => {
                    let v = EntityView::new(m.clone(), None, &cx.rng);
                    self.dobjets.insert(m.id, v);
                }
            }
        }
        let known: std::collections::HashSet<EntityId> = memory
            .iter()
            .map(|(m, _)| m.id)
            .chain(perception.iter().map(|m| m.id))
            .collect();
        let gone: Vec<EntityId> = self
            .dobjets
            .keys()
            .copied()
            .filter(|i| !known.contains(i))
            .collect();
        for i in gone {
            self.delete_object(cx, i);
            if self.target == Some(i) {
                self.target = None;
            }
        }
    }

    /// The volumes of a sound from `place`, as on a minimap centered on
    /// the selected square.
    pub fn minimap_stereo(&self, place: Option<SquareId>) -> (f64, f64) {
        let (x, y) = self.coords_in_map(place);
        let (xc, yc) = self.coords_in_map(self.place);
        let (xcmax, ycmax) = self.grid_max();
        let flattening_factor = 2.0;
        let dx = (x - xc) as f64 * 6.0 / (xcmax + 1) as f64;
        let dy = (y - yc) as f64 * 6.0 / (ycmax + 1) as f64 / flattening_factor;
        stereo(0.0, 0.0, dx, dy, 90.0, 1.0, false)
    }

    /// Plays an alert sound from a square, at most every half second.
    pub fn launch_alert(&self, cx: &Client, place: Option<SquareId>, sound: &str) {
        let (l, r) = self.minimap_stereo(place);
        cx.audio.play_stereo(
            cx.sounds.get_sound(sound),
            StereoVolume::Split(l, r),
            ALERT_LIMIT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_cancel_modes_and_transport_orders_do_not_change_a_menu() {
        let menu: Vec<String> = [
            "go",
            "stop",
            "train footman",
            "stop",
            "mode_offensive",
            "load",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(relevant_menu(&menu), ["go", "train footman", "stop"]);
    }

    #[test]
    fn square_titles_sort_as_python_lists() {
        let a1 = vec![MsgPart::Num(5000), MsgPart::Num(1_000_001)];
        let a2 = vec![MsgPart::Num(5000), MsgPart::Num(1_000_002)];
        let b1 = vec![MsgPart::Num(5001), MsgPart::Num(1_000_001)];
        let mut t = vec![b1.clone(), a2.clone(), a1.clone()];
        t.sort_by_key(title_key);
        assert_eq!(t, [a1, a2, b1]);
    }
}
