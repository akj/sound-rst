//! What the player learns without asking: the fog of war kept up to date,
//! and the news about resources, food, menus, units and attacks.

use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{EntityId, KnownEntity, Place, SquareId};

use crate::audio::stereo;
use crate::client::Client;
use crate::message::{Category, News, Subject};
use crate::sound::StereoVolume;

use super::entity::EntityView;
use super::news::must_be_said;
use super::order::{PRECISION, nb2msg_f};
use super::squares::summary;
use super::{ALERT_LIMIT, GameInterface, style_msg};

/// An attack on a place is news again once the place has been quiet for
/// this many seconds.
pub const ATTACK_QUIET: f64 = 10.0;

/// What happened to the units a piece of news counts.
#[derive(Clone, Copy, Debug)]
enum UnitsNews {
    Neutralized,
    Lost,
    /// Enemies seen for the first time.
    Sighted,
}

impl UnitsNews {
    fn end(self) -> &'static [u64] {
        match self {
            UnitsNews::Neutralized => mp::NEUTRALIZED,
            UnitsNews::Lost => mp::LOST,
            UnitsNews::Sighted => mp::ENEMY,
        }
    }

    /// Whether more than two types are said as one count.
    fn brief(self) -> bool {
        !matches!(self, UnitsNews::Neutralized)
    }

    fn category(self) -> Category {
        match self {
            UnitsNews::Neutralized | UnitsNews::Lost => Category::Alert,
            UnitsNews::Sighted => Category::Scouting,
        }
    }

    fn subject(self, place: Place) -> Subject {
        match self {
            UnitsNews::Neutralized => Subject::Neutralized(place),
            UnitsNews::Lost => Subject::Lost(place),
            UnitsNews::Sighted => Subject::EnemiesSighted(place),
        }
    }
}

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

impl GameInterface {
    /// The resources of the player, in units.
    pub fn resources(&self) -> Vec<i64> {
        self.view()
            .resources()
            .iter()
            .map(|r| r / PRECISION)
            .collect()
    }

    pub fn available_food(&self) -> i64 {
        self.view().available_food()
    }

    pub fn used_food(&self) -> i64 {
        self.view().used_food()
    }

    /// Tells economy news once the game has started.
    fn economy_news(&self, cx: &mut Client, m: Msg, subject: Subject) {
        if self.last_virtual_time != 0.0 {
            cx.news(News::new(Category::Economy, m).about(subject));
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
    /// it is almost all used. A count the player has not heard yet is
    /// replaced by the new one, whatever it is.
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
                let subject = Subject::Resource(i);
                if must_be_said(r) || cx.voice.is_unheard(&subject) {
                    let m = [
                        nb2msg(r),
                        style_msg(cx.res.style(), "parameters", &format!("resource_{i}_title")),
                    ]
                    .concat();
                    self.economy_news(cx, m, subject);
                }
            }
        }
        self.previous_resources = Some(previous);
        let (available, used) = (self.available_food(), self.used_food());
        let (previous_available, previous_used) =
            (self.previous_available_food, self.previous_used_food);
        let changed = available != previous_available || used != previous_used;
        let worth = available != previous_available
            || used > previous_used
            || used < previous_used && previous_used == available;
        let short = 0 <= available - used && (available - used) as f64 <= available as f64 * 0.20;
        self.previous_available_food = available;
        self.previous_used_food = used;
        if worth && short || changed && cx.voice.is_unheard(&Subject::Food) {
            let m = self.food_msg(cx);
            self.economy_news(cx, m, Subject::Food);
        }
    }

    /// Says when a kind of unit can do more than before.
    pub(crate) fn send_menu_alerts_if_needed(&mut self, cx: &mut Client) {
        let mut done: Vec<String> = Vec::new();
        for e in self.host.view(self.me).own_units() {
            let u = e.id;
            let type_name = e.type_name.to_string();
            // A view of each unit, as the original makes one: it draws its
            // footstep offset even when its type is done.
            let view = EntityView::new(e, &cx.rng);
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
                        let subject = Subject::Menu(type_name.clone());
                        cx.news(News::new(Category::Production, m).about(subject));
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
            let news = News::new(Category::Alert, m).about(Subject::EnemiesAt(place));
            cx.news(news.at(square));
        }
    }

    /// Tells the units by place. The news about a place counts the units
    /// of its news the player has not heard yet too, and replaces it.
    fn units_alert(&mut self, cx: &mut Client, units: Vec<(Msg, Option<Place>)>, kind: UnitsNews) {
        let mut places: Vec<Place> = Vec::new();
        for (_, p) in &units {
            if let Some(p) = p
                && !places.contains(p)
            {
                places.push(*p);
            }
        }
        self.sort_by_square_id(&mut places);
        for place in places {
            let subject = kind.subject(place);
            let unheard = cx.voice.is_unheard(&subject);
            let told = self.told_units.entry(subject.clone()).or_default();
            if !unheard {
                told.clear();
            }
            told.extend(
                units
                    .iter()
                    .filter(|(_, p)| *p == Some(place))
                    .map(|(t, _)| t.clone()),
            );
            let s = summary(told, kind.brief());
            if !s.is_empty() {
                let m = [
                    s,
                    msg(kind.end()),
                    msg(mp::AT),
                    self.alert_place_title(cx, place),
                ]
                .concat();
                let news = News::new(kind.category(), m).about(subject);
                cx.news(news.at(self.outer_square(Some(place))));
            }
        }
    }

    /// Says the units lost, neutralized and the new enemies; after a lost
    /// fight at `place`, the enemies there.
    pub(crate) fn units_alert_if_needed(&mut self, cx: &mut Client, place: Option<Place>) {
        let n = std::mem::take(&mut self.neutralized_units);
        self.units_alert(cx, n, UnitsNews::Neutralized);
        let l = std::mem::take(&mut self.lost_units);
        self.units_alert(cx, l, UnitsNews::Lost);
        let e = std::mem::take(&mut self.new_enemy_units);
        self.units_alert(cx, e, UnitsNews::Sighted);
        if let Some(p) = place {
            self.tell_enemies_in_square(cx, p);
        }
    }

    /// Tells of an attack on the player's units at `place`, unless one is
    /// going on there: an attack lasts until its place has been quiet for
    /// [`ATTACK_QUIET`] seconds. Whether it was news.
    pub(crate) fn attack_alert(&mut self, cx: &mut Client, place: Place) -> bool {
        let now = cx.clock.now();
        let new = self
            .attacks
            .get(&place)
            .is_none_or(|&t| now > t + ATTACK_QUIET);
        self.attacks.insert(place, now);
        if new {
            let m = [
                msg(mp::ALERT),
                msg(mp::AT),
                self.alert_place_title(cx, place),
            ]
            .concat();
            let news = News::new(Category::Alert, m).about(Subject::Attack(place));
            cx.news(news.at(self.outer_square(Some(place))));
        }
        new
    }

    /// Says what was found on newly seen squares with resources.
    pub(crate) fn scout_info_if_needed(&mut self, cx: &mut Client) {
        let mut places: Vec<Place> = std::mem::take(&mut self.scout_info).into_iter().collect();
        self.sort_by_square_id(&mut places);
        for place in places {
            let square = self.outer_square(Some(place));
            let s = self.place_summary(cx, square, false, None, true);
            if !s.is_empty() {
                let m = [s, msg(mp::AT), self.alert_place_title(cx, place)].concat();
                cx.news(News::new(Category::Scouting, m).at(square));
            }
        }
    }

    /// Orders places as the reference orders the original's sets of
    /// squares: by square id.
    fn sort_by_square_id(&self, places: &mut [Place]) {
        places.sort_by_key(|p| {
            self.outer_square(Some(*p))
                .map_or(u32::MAX, |s| self.view().square(s).protocol_id())
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
        memory: Vec<KnownEntity>,
        perception: Vec<KnownEntity>,
    ) {
        let known: std::collections::HashSet<EntityId> =
            memory.iter().chain(&perception).map(|m| m.id).collect();
        for m in memory {
            if self.dobjets.get(&m.id).is_some_and(|v| !v.is_memory()) {
                self.delete_object(cx, m.id);
            }
            match self.dobjets.get_mut(&m.id) {
                Some(v) => v.model = m,
                None => {
                    let is_resource = self.resource_type(&m).is_some();
                    let place = m.place;
                    self.dobjets.insert(m.id, EntityView::new(m, &cx.rng));
                    if self.must_report_resource(place, is_resource)
                        && let Some(place) = place
                    {
                        self.scout_info.insert(place);
                    }
                }
            }
        }
        for m in perception {
            if !self.dobjets.contains_key(&m.id) {
                if self.is_an_enemy(&m) {
                    let v = EntityView::new(m.clone(), &cx.rng);
                    let t = self.short_title(cx, &v);
                    self.new_enemy_units.push((t, m.place));
                }
                let is_resource = self.resource_type(&m).is_some();
                if self.must_report_resource(m.place, is_resource)
                    && let Some(place) = m.place
                {
                    self.scout_info.insert(place);
                }
            } else if self.dobjets[&m.id].is_memory() {
                self.delete_object(cx, m.id);
            }
            match self.dobjets.get_mut(&m.id) {
                Some(v) => v.model = m,
                None => {
                    self.dobjets.insert(m.id, EntityView::new(m, &cx.rng));
                }
            }
        }
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
    use super::super::testing::{cheat, enemy, training, unit};
    use super::super::{EVENT_LIMIT, SrvEvent};
    use super::*;
    use crate::clock::Clock;
    use crate::input::{Key, Mods};
    use crate::message::Policy;
    use crate::testkit::{Harness, TestGame};
    use crate::voice::Heard;
    use soundrts_world::events::EntityEvent;

    /// Lets the voice say everything waiting, without running the game.
    fn drain(h: &mut Harness) {
        for _ in 0..600 {
            if h.client.voice.is_idle() {
                return;
            }
            h.clock.advance(0.1);
            h.client.voice.update();
        }
        panic!("the voice is still busy");
    }

    /// The player's farm completes, as an event handled `late` seconds
    /// after its turn. Returns what was said and how many sounds played.
    fn farm_completes(h: &mut Harness, g: &mut TestGame, late: f64) -> (Vec<String>, usize) {
        drain(h);
        let farm = unit(g, g.game.me, "farm");
        let entity = Box::new(g.game.view().perceived(farm).unwrap());
        let (from, sounds) = (h.utterances().len(), h.audio.played().len());
        g.game.next_update = h.clock.now() - late;
        let event = SrvEvent::Entity {
            entity,
            event: EntityEvent::Complete,
        };
        assert!(g.game.process_srv_event(&mut h.client, event).is_ok());
        let sounds = h.audio.played().len() - sounds;
        drain(h);
        (h.said_since(from), sounds)
    }

    #[test]
    fn news_handled_late_is_said_without_its_sound() {
        let (mut h, mut g) = training();
        let (said, sounds) = farm_completes(&mut h, &mut g, 0.0);
        assert_eq!(said, ["farm 1 complete"]);
        assert!(sounds > 0);
        let (said, sounds) = farm_completes(&mut h, &mut g, EVENT_LIMIT + 1.0);
        assert_eq!(said, ["farm 1 complete"]);
        assert_eq!(sounds, 0);
    }

    #[test]
    fn news_logged_silently_plays_no_sound_and_waits_unheard() {
        let (mut h, mut g) = training();
        h.client.config.news.set(Category::Production, Policy::Log);
        let (said, sounds) = farm_completes(&mut h, &mut g, 0.0);
        assert_eq!(said, Vec::<String>::new());
        assert_eq!(sounds, 0);
        let last = h.client.voice.log().last().unwrap();
        assert_eq!(last.category(), Some(Category::Production));
        assert_eq!(last.heard(), Heard::No);
    }

    #[test]
    fn under_the_sound_policy_news_plays_its_sound_without_its_words() {
        let (mut h, mut g) = training();
        h.client
            .config
            .news
            .set(Category::Production, Policy::Sound);
        let (said, sounds) = farm_completes(&mut h, &mut g, 0.0);
        assert_eq!(said, Vec::<String>::new());
        assert!(sounds > 0);
        assert_eq!(h.client.voice.log().last().unwrap().heard(), Heard::No);
    }

    #[test]
    fn the_jump_goes_to_the_square_of_the_last_alert_then_the_one_before() {
        let (mut h, mut g) = training();
        let start = g.game.place.unwrap();
        let other = SquareId((start.0 + 1) % g.game.view().square_count() as u32);
        g.game.attack_alert(&mut h.client, Place::Square(start));
        g.game.attack_alert(&mut h.client, Place::Square(other));
        h.run_until_voice_idle(&mut g);
        let title = |g: &TestGame, s: SquareId| {
            let t = g.game.view().square(s).title().clone();
            [msg(mp::ALERT), msg(mp::AT), t].concat()
        };
        let alerts: Vec<Msg> = h
            .client
            .voice
            .log()
            .filter(|e| e.category() == Some(Category::Alert))
            .map(|e| e.parts().clone())
            .collect();
        assert_eq!(alerts, [title(&g, start), title(&g, other)]);
        let said = h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(g.game.place, Some(other));
        assert_eq!(said[0], "b 2 , unknown");
        h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(g.game.place, Some(start));
        // The oldest alert stays.
        let said = h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(g.game.place, Some(start));
        assert_eq!(
            said[0],
            "b 1 , meadows , 1 town hall , 1 farm and 1 peasant"
        );
        // Moved away, the jump starts again from the newest.
        h.key(&mut g, Key::Right, Mods::NONE);
        h.key(&mut g, Key::Right, Mods::NONE);
        h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(g.game.place, Some(other));
    }

    /// What the voice says until it is idle, without running the game.
    fn said_until_idle(h: &mut Harness) -> Vec<String> {
        let from = h.utterances().len();
        drain(h);
        h.said_since(from)
    }

    #[test]
    fn units_lost_while_their_news_waits_are_counted_in_it() {
        let (mut h, mut g) = training();
        drain(&mut h);
        let square = g.game.place.unwrap();
        let place = Place::Square(square);
        let peasant = || (msg(mp::NOTHING), Some(place));
        cx_news(&mut h, "a long message of news");
        g.game.lost_units.push(peasant());
        g.game.units_alert_if_needed(&mut h.client, None);
        g.game.lost_units.extend([peasant(), peasant()]);
        g.game.units_alert_if_needed(&mut h.client, None);
        assert_eq!(said_until_idle(&mut h), ["3 nothing! lost at b 1"]);
        // Heard, the next loss is news of its own.
        let from = h.utterances().len();
        g.game.lost_units.push(peasant());
        g.game.units_alert_if_needed(&mut h.client, None);
        drain(&mut h);
        assert_eq!(h.said_since(from), ["1 nothing! lost at b 1"]);
    }

    /// Queues news that keeps the voice busy for a while.
    fn cx_news(h: &mut Harness, s: &str) {
        let m = vec![soundrts_world::msg::MsgPart::from(s)];
        h.client.news(News::new(Category::Production, m));
    }

    #[test]
    fn unheard_food_news_follows_the_food() {
        let (mut h, mut g) = training();
        drain(&mut h);
        g.game.last_virtual_time = 1.0;
        cx_news(&mut h, "a long message of news");
        // Food news waits to be said...
        let old = News::new(Category::Economy, msg(mp::NOTHING)).about(Subject::Food);
        h.client.news(old);
        // ...when the food changes, even to no shortage.
        g.game.previous_used_food += 1;
        g.game.send_resource_alerts_if_needed(&mut h.client);
        assert_eq!(said_until_idle(&mut h), ["9 food to 1"]);
    }

    #[test]
    fn food_news_waiting_follows_usage_that_goes_down_then_up() {
        let (mut h, mut g) = training();
        drain(&mut h);
        g.game.last_virtual_time = 1.0;
        cx_news(&mut h, "a long message of news");
        let old = News::new(Category::Economy, msg(mp::NOTHING)).about(Subject::Food);
        h.client.news(old);
        // The usage goes down: the news follows, and so does the usage
        // seen, so that going back up is seen too.
        g.game.previous_used_food += 1;
        g.game.send_resource_alerts_if_needed(&mut h.client);
        assert_eq!(g.game.previous_used_food, g.game.used_food());
        assert_eq!(said_until_idle(&mut h), ["9 food to 1"]);
    }

    #[test]
    fn the_jump_ignores_the_alerts_of_an_earlier_game() {
        let (mut h, mut g) = training();
        let start = g.game.place.unwrap();
        g.game.attack_alert(&mut h.client, Place::Square(start));
        g.game.srv_quit(&mut h.client);
        let mut g = h.start_training("jl1", &["quiet"], 1);
        h.run_until_turn(&mut g, 3);
        let place = g.game.place;
        let said = h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(said[0], "nothing!");
        assert_eq!(g.game.place, place);
    }

    #[test]
    fn the_worlds_messages_said_now_are_story_or_system_news() {
        let objective = [msg(mp::OBJECTIVE_COMPLETE), msg(mp::OK)].concat();
        assert_eq!(
            super::super::important_category(&objective),
            Category::Story
        );
        let defeated = [msg(mp::YOU_HAVE_BEEN_DEFEATED)].concat();
        assert_eq!(
            super::super::important_category(&defeated),
            Category::System
        );
    }

    #[test]
    fn an_attack_is_one_alert_tied_to_its_square_while_it_lasts() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let w = g.game.host.world();
        let square = w
            .entity(unit(&g, me, "townhall"))
            .unwrap()
            .square()
            .unwrap();
        let name = g.game.view().square(square).name().to_owned();
        cheat(&mut g, them, &format!("add_units {name} 2 footman"));
        let alerts = |h: &Harness| {
            h.client
                .voice
                .log()
                .filter(|e| {
                    e.category() == Some(Category::Alert) && e.parts()[0] == msg(mp::ALERT)[0]
                })
                .count()
        };
        let turn = g.game.view().turn() + 60;
        h.run_until_turn(&mut g, turn);
        assert_eq!(alerts(&h), 1);
        assert_eq!(
            h.client.voice.alert_before(None).map(|(_, s)| s),
            Some(square)
        );
        g.game.place = None;
        h.key(&mut g, Key::Delete, Mods::NONE);
        assert_eq!(g.game.place, Some(square));
    }

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
}
