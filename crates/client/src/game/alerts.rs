//! What the player learns without asking: the fog of war kept up to date,
//! and the news about resources, food, menus, units and attacks.

use soundrts_world::msg::{Cue, Msg, MsgPart, Phrase, msg, nb2msg};
use soundrts_world::orders::{Keyword, MenuEntry};
use soundrts_world::{EntityId, KnownEntity, Place, PlayerId, SquareId};

use crate::audio::{Sound, stereo};
use crate::client::Client;
use crate::message::{Category, Count, News, Rank, Subject, summary};
use crate::sound::StereoVolume;

use super::entity::EntityView;
use super::news::must_be_said;
use super::order::{PRECISION, nb2msg_f};
use super::{ALERT_LIMIT, GameInterface};
use crate::style::mine;

/// An attack on a place is news again once the place has been quiet for
/// this many seconds.
pub const ATTACK_QUIET: f64 = 10.0;

/// What happened to the units a piece of news counts.
#[derive(Clone, Copy, Debug)]
pub(crate) enum UnitsNews {
    Neutralized,
    Lost,
    /// Enemies seen for the first time.
    Sighted,
}

impl UnitsNews {
    fn end(self) -> Phrase {
        match self {
            UnitsNews::Neutralized => Phrase::Neutralized,
            UnitsNews::Lost => Phrase::Lost,
            UnitsNews::Sighted => Phrase::Enemy,
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

    /// The counts of a moment are said by kind in this order.
    fn rank(self) -> u32 {
        match self {
            UnitsNews::Neutralized => 0,
            UnitsNews::Lost => 1,
            UnitsNews::Sighted => 2,
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
const IRRELEVANT_ORDERS: &[Keyword] = &[
    Keyword::Stop,
    Keyword::CancelTraining,
    Keyword::CancelUpgrading,
    Keyword::CancelBuilding,
    Keyword::ModeOffensive,
    Keyword::ModeDefensive,
    Keyword::Load,
    Keyword::LoadAll,
    Keyword::UnloadAll,
];

fn relevant_menu(menu: &[MenuEntry]) -> Vec<MenuEntry> {
    let mut m = menu.to_vec();
    for &x in IRRELEVANT_ORDERS {
        if let Some(i) = m.iter().position(|o| o.keyword == x) {
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
    fn economy_news(&self, cx: &mut Client, news: News) {
        if self.known.time() != 0.0 {
            cx.news(news);
        }
    }

    pub(crate) fn food_msg(&self, cx: &Client) -> Msg {
        [
            nb2msg_f(self.available_food()),
            mine(&cx.res.style().get("parameters").food_title),
            msg(Phrase::To),
            nb2msg(self.used_food()),
        ]
        .concat()
    }

    /// Says the resources that changed to a round count, and the food when
    /// it is almost all used. A count the player has not heard yet is
    /// brought up to date, whatever the new one is.
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
                let m = [
                    nb2msg(r),
                    cx.res.style().get("parameters").resource_title_msg(i),
                ]
                .concat();
                let news = News::new(Category::Economy, m);
                let news = if must_be_said(r) {
                    news.about(subject)
                } else {
                    news.updating(subject)
                };
                self.economy_news(cx, news);
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
        if changed {
            let news = News::new(Category::Economy, self.food_msg(cx));
            let news = if worth && short {
                news.about(Subject::Food)
            } else {
                news.updating(Subject::Food)
            };
            self.economy_news(cx, news);
        }
    }

    /// Says when a kind of unit can do more than before.
    pub(crate) fn send_menu_alerts_if_needed(&mut self, cx: &mut Client) {
        let mut done: Vec<String> = Vec::new();
        for e in self.known.seat().own_units() {
            let u = e.id;
            let type_name = e.type_name.to_string();
            if done.contains(&type_name) {
                continue;
            }
            let view = EntityView::new(e);
            let menu = relevant_menu(&self.view_menu(u, true));
            let title = self.short_title(cx, &view);
            match self.previous_menus.get(&type_name) {
                None => {
                    self.previous_menus.insert(type_name.clone(), Vec::new());
                }
                Some(previous) => {
                    if menu.iter().any(|i| !previous.contains(i)) {
                        let m = [msg(Phrase::MenuOf), title, msg(Phrase::Changed)].concat();
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
            .known
            .views()
            .filter(|v| self.is_in(v, square) && self.is_an_enemy(&v.model))
            .map(|v| self.short_title(cx, v))
            .collect();
        if !enemies.is_empty() {
            let m = [
                summary(&enemies, true),
                msg(Phrase::Enemy),
                msg(Phrase::At),
                self.alert_place_title(cx, place),
            ]
            .concat();
            let news = News::new(Category::Alert, m).about(Subject::EnemiesAt(place));
            cx.news(news.at(square));
        }
    }

    /// Gives the voice the units by place, one count for each place,
    /// ranked by kind then by square id. The voice holds the counts until
    /// [`GameInterface::units_alert_if_needed`], and adds them to the news
    /// about the same place the player has not heard yet.
    pub(crate) fn units_news(
        &self,
        cx: &mut Client,
        units: Vec<(Msg, Option<Place>)>,
        kind: UnitsNews,
    ) {
        let mut places: Vec<Place> = Vec::new();
        for (_, p) in &units {
            if let Some(p) = p
                && !places.contains(p)
            {
                places.push(*p);
            }
        }
        for place in places {
            let square = self.outer_square(Some(place));
            let rank = Rank(
                kind.rank(),
                square.map_or(u32::MAX, |s| self.view().square(s).protocol_id()),
            );
            let items = units
                .iter()
                .filter(|(_, p)| *p == Some(place))
                .map(|(t, _)| t.clone())
                .collect();
            let then = [
                msg(kind.end()),
                msg(Phrase::At),
                self.alert_place_title(cx, place),
            ]
            .concat();
            let count = Count::new(items, kind.brief(), then, rank);
            let news = News::counting(kind.category(), kind.subject(place), count);
            cx.news(news.at(square));
        }
    }

    /// The news of a turn or of a fight is complete: the voice says the
    /// units lost, neutralized and the new enemies it holds; after a lost
    /// fight at `place`, the enemies there.
    pub(crate) fn units_alert_if_needed(&mut self, cx: &mut Client, place: Option<Place>) {
        cx.voice.news_complete();
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
                msg(Phrase::Alert),
                msg(Phrase::At),
                self.alert_place_title(cx, place),
            ]
            .concat();
            let news = News::new(Category::Alert, m).about(Subject::Attack(place));
            cx.news(news.at(self.outer_square(Some(place))));
        }
        new
    }

    /// `from` points at `square`, as the server says: an ally's flare is
    /// an alert sound from the square and alert news tied to it, which the
    /// jump to the last alert goes to; the player's own is only confirmed.
    /// A watcher, who sent nothing, hears the alert of the player it
    /// follows as an ally's.
    /// A square the map does not have, or a sender who is not an ally still
    /// in the game, cannot be right: it is noted in the log, not said.
    pub(crate) fn flare_alert(&mut self, cx: &mut Client, from: PlayerId, square: SquareId) {
        let view = self.view();
        if view.game_start().map.square(square).is_none() {
            cx.log
                .warning(format!("a flare at square {}, not on the map", square.0));
            return;
        }
        if !view.players().contains(&from) || !view.is_ally(from) {
            cx.log.warning(format!(
                "a flare from player {}, no ally in the game",
                from.0
            ));
            return;
        }
        if from == self.me && !self.is_watching() {
            cx.voice.item(&msg(Phrase::FlareSent));
            return;
        }
        self.launch_alert(cx, Some(square), cx.sounds.cue(Cue::Flare));
        let m = [
            view.player_name(from),
            vec![MsgPart::literal(":")],
            msg(Phrase::FlareAt),
            self.alert_place_title(cx, Place::Square(square)),
        ]
        .concat();
        cx.news(News::new(Category::Alert, m).at(Some(square)));
    }

    /// Says what was found on newly seen squares with resources.
    pub(crate) fn scout_info_if_needed(&mut self, cx: &mut Client) {
        let mut places: Vec<Place> = std::mem::take(&mut self.scout_info).into_iter().collect();
        self.sort_by_square_id(&mut places);
        for place in places {
            let square = self.outer_square(Some(place));
            let s = self.place_summary(cx, square, false, None, true);
            if !s.is_empty() {
                let m = [s, msg(Phrase::At), self.alert_place_title(cx, place)].concat();
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
        if let Some(v) = self.known.end_of_turn_copy_mut().shift_remove(&id) {
            v.stop(&cx.audio);
        }
        self.group.retain(|&g| g != id);
    }

    /// Makes the views match what the player remembered and perceived at
    /// the end of the last turn, and gives the voice the enemies perceived
    /// for the first time.
    pub(crate) fn update_fog_of_war(
        &mut self,
        cx: &mut Client,
        memory: Vec<KnownEntity>,
        perception: Vec<KnownEntity>,
    ) {
        let mut sighted = Vec::new();
        let known: std::collections::HashSet<EntityId> =
            memory.iter().chain(&perception).map(|m| m.id).collect();
        for m in memory {
            if self
                .known
                .end_of_turn_copy()
                .get(&m.id)
                .is_some_and(|v| !v.is_memory())
            {
                self.delete_object(cx, m.id);
            }
            match self.known.end_of_turn_copy_mut().get_mut(&m.id) {
                Some(v) => v.model = m,
                None => {
                    let is_resource = self.resource_type(&m).is_some();
                    let place = m.place;
                    self.known
                        .end_of_turn_copy_mut()
                        .insert(m.id, EntityView::new(m));
                    if self.must_report_resource(place, is_resource)
                        && let Some(place) = place
                    {
                        self.scout_info.insert(place);
                    }
                }
            }
        }
        for m in perception {
            if !self.known.end_of_turn_copy().contains_key(&m.id) {
                if self.is_an_enemy(&m) {
                    let v = EntityView::new(m.clone());
                    let t = self.short_title(cx, &v);
                    sighted.push((t, m.place));
                }
                let is_resource = self.resource_type(&m).is_some();
                if self.must_report_resource(m.place, is_resource)
                    && let Some(place) = m.place
                {
                    self.scout_info.insert(place);
                }
            } else if self
                .known
                .end_of_turn_copy()
                .get(&m.id)
                .is_some_and(EntityView::is_memory)
            {
                self.delete_object(cx, m.id);
            }
            match self.known.end_of_turn_copy_mut().get_mut(&m.id) {
                Some(v) => v.model = m,
                None => {
                    self.known
                        .end_of_turn_copy_mut()
                        .insert(m.id, EntityView::new(m));
                }
            }
        }
        let gone: Vec<EntityId> = self
            .known
            .end_of_turn_copy()
            .keys()
            .copied()
            .filter(|i| !known.contains(i))
            .collect();
        for i in gone {
            self.delete_object(cx, i);
        }
        // The target stays only on an entity the interface shows.
        if self.target.is_some_and(|t| self.known.unit(t).is_none()) {
            self.target = None;
        }
        self.units_news(cx, sighted, UnitsNews::Sighted);
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
    pub fn launch_alert(&self, cx: &Client, place: Option<SquareId>, sound: Option<Sound>) {
        let (l, r) = self.minimap_stereo(place);
        cx.audio
            .play_stereo(sound, StereoVolume::Split(l, r), ALERT_LIMIT);
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

    /// What the game says until the voice has nothing left to say.
    fn said_until_idle(h: &mut Harness, g: &mut TestGame) -> Vec<String> {
        let from = h.utterances().len();
        h.run_until_voice_idle(g);
        h.said_since(from)
    }

    /// The player's farm completes, as an event handled `late` seconds
    /// after its turn. Returns what was said and how many sounds played.
    fn farm_completes(h: &mut Harness, g: &mut TestGame, late: f64) -> (Vec<String>, usize) {
        h.run_until_voice_idle(g);
        let farm = unit(g, g.game.me, "farm");
        let entity = Box::new(g.game.view().perceived(farm).unwrap());
        let (from, sounds) = (h.utterances().len(), h.audio.played().len());
        g.game.set_turn_due(h.clock.now() - late);
        let event = SrvEvent::Entity {
            entity,
            event: EntityEvent::Complete,
        };
        g.game.process_srv_event(&mut h.client, event);
        let sounds = h.audio.played().len() - sounds;
        h.run_until_voice_idle(g);
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
            [msg(Phrase::Alert), msg(Phrase::At), t].concat()
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

    /// The units of `p` of this type, in the whole world.
    fn units(g: &TestGame, p: PlayerId, type_name: &str) -> Vec<EntityId> {
        let w = g.game.host().world();
        w.player(p)
            .units
            .iter()
            .copied()
            .filter(|&u| w.entity(u).is_some_and(|e| &*e.type_name == type_name))
            .collect()
    }

    /// The name of the square of the player's town hall.
    fn home(g: &TestGame) -> String {
        let w = g.game.host().world();
        let townhall = unit(g, g.game.me, "townhall");
        let square = w.entity(townhall).unwrap().square().unwrap();
        g.game.view().square(square).name().to_owned()
    }

    /// Runs `n` turns of the game.
    fn run_turns(h: &mut Harness, g: &mut TestGame, n: i64) {
        let turn = g.game.view().turn() + n;
        h.run_until_turn(g, turn);
    }

    /// The server tells the interface that `id` died, killed by `by`.
    fn dies(h: &mut Harness, g: &mut TestGame, id: EntityId, by: EntityId) {
        let entity = Box::new(g.game.view().perceived(id).expect("a unit perceived"));
        let event = SrvEvent::Entity {
            entity,
            event: EntityEvent::DeathBy(by),
        };
        g.game.process_srv_event(&mut h.client, event);
    }

    /// A training game where the player has three more footmen at home,
    /// with nothing left to say.
    fn three_footmen() -> (Harness, TestGame, Vec<EntityId>) {
        let (mut h, mut g) = training();
        let home = home(&g);
        let me = g.game.me;
        cheat(&mut g, me, &format!("add_units {home} 3 footman"));
        run_turns(&mut h, &mut g, 2);
        h.run_until_voice_idle(&mut g);
        let footmen = units(&g, g.game.me, "footman");
        (h, g, footmen)
    }

    #[test]
    fn units_lost_while_their_news_waits_are_counted_in_it() {
        let (mut h, mut g, footmen) = three_footmen();
        let by = unit(&g, enemy(&g), "townhall");
        let peasant = unit(&g, g.game.me, "peasant");
        let from = h.utterances().len();
        cx_news(&mut h, "a long message of news");
        dies(&mut h, &mut g, footmen[0], by);
        // Told at the end of the turn, the loss waits behind the news.
        run_turns(&mut h, &mut g, 1);
        dies(&mut h, &mut g, peasant, by);
        dies(&mut h, &mut g, footmen[1], by);
        h.run_until_voice_idle(&mut g);
        assert_eq!(
            h.said_since(from),
            [
                "a long message of news",
                "2 footman and 1 peasant lost at b 1"
            ]
        );
        // Heard, the next loss is news of its own, at the end of the turn.
        let from = h.utterances().len();
        dies(&mut h, &mut g, footmen[2], by);
        run_turns(&mut h, &mut g, 1);
        h.run_until_voice_idle(&mut g);
        assert_eq!(h.said_since(from), ["1 footman lost at b 1"]);
    }

    /// The enemy's buildings on the player's square, after `cheat` added
    /// them and the end of the turn after the one that saw them.
    fn enemy_buildings_at_home(
        h: &mut Harness,
        g: &mut TestGame,
        cheat_line: &str,
    ) -> Vec<EntityId> {
        let (them, home) = (enemy(g), home(g));
        cheat(g, them, &format!("add_units {home} {cheat_line}"));
        run_turns(h, g, 2);
        let square = g.game.place;
        let w = g.game.host().world();
        w.player(them)
            .units
            .iter()
            .copied()
            .filter(|&u| w.entity(u).is_some_and(|e| e.square() == square))
            .collect()
    }

    #[test]
    fn units_neutralized_while_their_news_waits_are_counted_in_it() {
        let (mut h, mut g, footmen) = three_footmen();
        let buildings = enemy_buildings_at_home(&mut h, &mut g, "2 farm 1 barracks");
        h.run_until_voice_idle(&mut g);
        let from = h.utterances().len();
        cx_news(&mut h, "a long message of news");
        let (first, others) = buildings.split_first().expect("buildings");
        dies(&mut h, &mut g, *first, footmen[0]);
        run_turns(&mut h, &mut g, 1);
        for &id in others {
            dies(&mut h, &mut g, id, footmen[0]);
        }
        h.run_until_voice_idle(&mut g);
        assert_eq!(
            h.said_since(from),
            [
                "a long message of news",
                "2 farm and 1 barracks neutralized at b 1"
            ]
        );
    }

    #[test]
    fn enemies_seen_while_their_news_waits_are_counted_in_it() {
        let (mut h, mut g) = training();
        h.run_until_voice_idle(&mut g);
        let from = h.utterances().len();
        cx_news(&mut h, "a long message of news");
        enemy_buildings_at_home(&mut h, &mut g, "2 farm");
        run_turns(&mut h, &mut g, 1);
        enemy_buildings_at_home(&mut h, &mut g, "1 barracks");
        h.run_until_voice_idle(&mut g);
        assert_eq!(
            h.said_since(from),
            [
                "a long message of news",
                "2 farm and 1 barracks enemy at b 1"
            ]
        );
    }

    #[test]
    fn a_loss_cut_off_is_said_again_with_the_losses_since() {
        let (mut h, mut g, footmen) = three_footmen();
        let by = unit(&g, enemy(&g), "townhall");
        let from = h.utterances().len();
        dies(&mut h, &mut g, footmen[0], by);
        h.run_for(&mut g, 0.5);
        // The food key cuts the news, and another footman is lost.
        h.press(Key::C, Mods::NONE);
        h.step(&mut g);
        dies(&mut h, &mut g, footmen[1], by);
        h.run_until_voice_idle(&mut g);
        assert_eq!(
            h.said_since(from),
            [
                "1 footman lost at b 1",
                "9 food to 4",
                "2 footman lost at b 1"
            ]
        );
    }

    /// The server's next turn reaches the watcher, and the interface
    /// handles it.
    fn watched_turn(
        h: &mut Harness,
        g: &mut TestGame,
        server: &crate::testkit::FakeServer,
        stream: &mut crate::testkit::FakeStream,
    ) {
        let next = g.game.view().turn() + 1;
        for frame in stream.turn(Vec::new()) {
            server.frame_at(h.clock.now(), frame);
        }
        h.run_until_turn(g, next);
    }

    #[test]
    fn a_watcher_moved_to_another_player_hears_only_that_players_losses() {
        use crate::testkit::{FakeStream, slot};
        use soundrts_protocol::{Computer, Skill, SlotPlayer, Speed};
        let mut h = Harness::new("en", crate::testkit::Script::new().keep_open());
        let server = h.join_fake_server("carol", &[]);
        let quiet = || SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL));
        let slots = [slot(quiet(), 1), slot(quiet(), 2)];
        let mut stream = FakeStream::new(&mut h, "jl1", &slots, 0, 1).watching();
        let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
        for _ in 0..3 {
            watched_turn(&mut h, &mut g, &server, &mut stream);
        }
        h.run_until_voice_idle(&mut g);
        let w = stream.host().world();
        let seats = stream.host().seats();
        let (a, b) = (seats[0], seats[1]);
        let of = |p: PlayerId, t: &str| {
            w.player(p)
                .units
                .iter()
                .copied()
                .find(|&u| w.entity(u).is_some_and(|e| &*e.type_name == t))
                .unwrap()
        };
        let (a_peasant, a_hall, b_peasant, b_hall) = (
            of(a, "peasant"),
            of(a, "townhall"),
            of(b, "peasant"),
            of(b, "townhall"),
        );
        assert_eq!(g.game.me, a);
        let from = h.utterances().len();
        cx_news(&mut h, "a long message of news");
        // A's peasant is lost; its news waits behind the long one.
        let a_place = g.game.view().perceived(a_peasant).unwrap().place;
        dies(&mut h, &mut g, a_peasant, b_hall);
        watched_turn(&mut h, &mut g, &server, &mut stream);
        // The watcher is moved to B, whose peasant is lost where A's was.
        server.frame_at(h.clock.now(), stream.follow(1));
        while g.game.me != b {
            h.step(&mut g);
        }
        let mut entity = g.game.view().perceived(b_peasant).unwrap();
        entity.place = a_place;
        let event = SrvEvent::Entity {
            entity: Box::new(entity),
            event: EntityEvent::DeathBy(a_hall),
        };
        g.game.process_srv_event(&mut h.client, event);
        watched_turn(&mut h, &mut g, &server, &mut stream);
        h.run_until_voice_idle(&mut g);
        let said = h.said_since(from);
        // B's loss replaces A's unheard one, and does not count it. The
        // scouting news of B's side follows.
        assert_eq!(
            said[..3],
            [
                "a long message of news",
                "watching quiet computer , level 3",
                "1 peasant lost at b 1"
            ]
        );
        assert_eq!(said.iter().filter(|s| s.contains("lost")).count(), 1);
    }

    #[test]
    fn a_watcher_moved_to_another_player_hears_none_of_the_news_that_waited() {
        use crate::testkit::{FakeStream, slot};
        use soundrts_protocol::{Computer, Skill, SlotPlayer, Speed};
        let mut h = Harness::new("en", crate::testkit::Script::new().keep_open());
        let server = h.join_fake_server("carol", &[]);
        let quiet = || SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL));
        let slots = [slot(quiet(), 1), slot(quiet(), 2)];
        let mut stream = FakeStream::new(&mut h, "jl1", &slots, 0, 1).watching();
        let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
        for _ in 0..3 {
            watched_turn(&mut h, &mut g, &server, &mut stream);
        }
        h.run_until_voice_idle(&mut g);
        let b = stream.host().seats()[1];
        let from = h.utterances().len();
        cx_news(&mut h, "a long message of news");
        cx_news(&mut h, "news of the first player");
        server.frame_at(h.clock.now(), stream.follow(1));
        while g.game.me != b {
            h.step(&mut g);
        }
        h.run_until_voice_idle(&mut g);
        // The message being said goes on; the one waiting stays unheard.
        let said = h.said_since(from);
        assert_eq!(
            said[..2],
            [
                "a long message of news",
                "watching quiet computer , level 3"
            ]
        );
        assert!(!said.iter().any(|s| s.contains("first player")), "{said:?}");
        let waited = h
            .client
            .voice
            .log()
            .find(|e| e.parts() == &[MsgPart::literal("news of the first player")])
            .map(|e| e.heard());
        assert_eq!(waited, Some(Heard::No));
    }

    /// Queues news that keeps the voice busy for a while.
    fn cx_news(h: &mut Harness, s: &str) {
        let m = vec![soundrts_world::msg::MsgPart::literal(s)];
        h.client.news(News::new(Category::Production, m));
    }

    #[test]
    fn unheard_food_news_follows_the_food() {
        let (mut h, mut g) = training();
        h.run_until_voice_idle(&mut g);
        g.game.known.set_time(1.0);
        cx_news(&mut h, "a long message of news");
        // Food news waits to be said...
        let old = News::new(Category::Economy, msg(Phrase::Nothing)).about(Subject::Food);
        h.client.news(old);
        // ...when the food changes, even to no shortage.
        g.game.previous_used_food += 1;
        g.game.send_resource_alerts_if_needed(&mut h.client);
        assert_eq!(said_until_idle(&mut h, &mut g), ["9 food to 1"]);
    }

    #[test]
    fn food_news_waiting_follows_usage_that_goes_down_then_up() {
        let (mut h, mut g) = training();
        h.run_until_voice_idle(&mut g);
        g.game.known.set_time(1.0);
        cx_news(&mut h, "a long message of news");
        let old = News::new(Category::Economy, msg(Phrase::Nothing)).about(Subject::Food);
        h.client.news(old);
        // The usage goes down: the news follows, and so does the usage
        // seen, so that going back up is seen too.
        g.game.previous_used_food += 1;
        g.game.send_resource_alerts_if_needed(&mut h.client);
        assert_eq!(g.game.previous_used_food, g.game.used_food());
        assert_eq!(said_until_idle(&mut h, &mut g), ["9 food to 1"]);
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
        let objective = [msg(Phrase::ObjectiveComplete), msg(Phrase::Ok)].concat();
        assert_eq!(
            super::super::important_category(&objective),
            Category::Story
        );
        let defeated = [msg(Phrase::YouHaveBeenDefeated)].concat();
        assert_eq!(
            super::super::important_category(&defeated),
            Category::System
        );
    }

    #[test]
    fn an_attack_is_one_alert_tied_to_its_square_while_it_lasts() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let w = g.game.host().world();
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
                    e.category() == Some(Category::Alert) && e.parts()[0] == msg(Phrase::Alert)[0]
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
        let plain = |k| MenuEntry::plain(k).unwrap();
        let train_footman = MenuEntry {
            keyword: Keyword::Train,
            type_id: Some(soundrts_world::rules::TypeId(3)),
            requirements: Vec::new(),
        };
        let menu = [
            plain(Keyword::Go),
            plain(Keyword::Stop),
            train_footman.clone(),
            plain(Keyword::Stop),
            plain(Keyword::ModeOffensive),
            plain(Keyword::Load),
        ];
        assert_eq!(
            relevant_menu(&menu),
            [plain(Keyword::Go), train_footman, plain(Keyword::Stop)]
        );
    }
}
