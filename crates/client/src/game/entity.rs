//! What the player knows of an entity (`clientgameentity.py`): its titles
//! and description, its noise and footsteps, and the reactions to what
//! happens to it.

use soundrts_world::events::{EntityEvent, Reason};
use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;
use soundrts_world::rules::AirGround;
use soundrts_world::{EntityId, KnownEntity, KnownOrder, KnownTarget, Place, PlayerId, SquareId};

use crate::audio::{angle, distance};
use crate::client::Client;
use crate::message::{Category, News, Subject};
use crate::sound::{Audio, Placement};
use crate::style::{Noise, StyleValue};

use super::animation::{NoisePlayer, NoiseSite, NoiseSource};
use super::news::must_be_said;
use super::order::{PRECISION, substitute_args};
use super::{GameInterface, Timing, compute_title, style_msg};

/// The same footstep sound does not start again within this many seconds.
pub const FOOTSTEP_LIMIT: f64 = 0.1;

/// The compass point nearest to `o` degrees (0 is east, 90 north).
pub fn direction_to_msgpart(o: f64) -> &'static [u64] {
    let mut o = (o / 45.0).round_ties_even() * 45.0;
    while o >= 360.0 {
        o -= 360.0;
    }
    while o < 0.0 {
        o += 360.0;
    }
    match o as i64 {
        0 => mp::EAST,
        45 => mp::NORTHEAST,
        90 => mp::NORTH,
        135 => mp::NORTHWEST,
        180 => mp::WEST,
        225 => mp::SOUTHWEST,
        270 => mp::SOUTH,
        _ => mp::SOUTHEAST,
    }
}

/// Where a point is from the center of a square: "at the center", or "to
/// the" and a compass point. Coordinates in meters.
pub fn direction_msg(center: (f64, f64), square_width: f64, x: f64, y: f64) -> Msg {
    let (cx, cy) = center;
    if distance(cx, cy, x, y) < square_width / 3.0 / 2.0 {
        return msg(mp::AT_THE_CENTER);
    }
    let d = direction_to_msgpart(angle(cx, cy, x, y, 0.0).to_degrees());
    if d == mp::EAST {
        msg(mp::TO_THE_EAST)
    } else if d == mp::WEST {
        msg(mp::TO_THE_WEST)
    } else {
        [msg(mp::TO_THE), msg(d)].concat()
    }
}

/// The view's own state for sounds: its noise, footsteps and the hit
/// points last heard.
#[derive(Default)]
pub struct AnimState {
    pub noise: Option<NoiseSource>,
    pub previous_hp: Option<i64>,
    pub next_step: Option<f64>,
    pub step_side: usize,
    /// Added to the footstep interval, so that several walking units do
    /// not step together: drawn when the entity first walks near the
    /// listener.
    pub footstep_offset: Option<f64>,
}

/// An entity as the player knows it: as the last turn's end told it,
/// perceived or remembered.
pub struct EntityView {
    pub model: KnownEntity,
    /// A facing given by the rotation keys, which hides the model's.
    pub o_override: Option<f64>,
    pub anim: AnimState,
}

impl EntityView {
    pub fn new(model: KnownEntity) -> EntityView {
        EntityView {
            model,
            o_override: None,
            anim: AnimState::default(),
        }
    }

    pub fn id(&self) -> EntityId {
        self.model.id
    }

    pub fn is_memory(&self) -> bool {
        self.model.is_memory()
    }

    pub fn type_name(&self) -> &str {
        &self.model.type_name
    }

    /// In meters.
    pub fn x(&self) -> f64 {
        self.model.x as f64 / 1000.0
    }

    pub fn y(&self) -> f64 {
        self.model.y as f64 / 1000.0
    }

    pub fn o(&self) -> f64 {
        self.o_override.unwrap_or(self.model.o as f64)
    }

    pub fn player(&self) -> Option<PlayerId> {
        self.model.player
    }

    pub fn place(&self) -> Option<Place> {
        self.model.place
    }

    /// Creatures have hit points, mana and upgrades.
    pub fn is_creature(&self) -> bool {
        self.model.is_creature()
    }

    pub fn hp(&self) -> i64 {
        self.model.hp / PRECISION
    }

    pub fn hp_max(&self) -> i64 {
        self.model.hp_max / PRECISION
    }

    pub fn mana(&self) -> i64 {
        self.model.mana / PRECISION
    }

    pub fn mana_max(&self) -> i64 {
        self.model.mana_max / PRECISION
    }

    pub fn qty(&self) -> i64 {
        self.model.qty / PRECISION
    }

    /// The orders of a unit the player controls; none for another.
    pub fn orders(&self) -> &[KnownOrder] {
        self.model.own.as_ref().map_or(&[], |o| &o.orders)
    }

    pub fn stop(&self, audio: &Audio) {
        if let Some(n) = &self.anim.noise {
            n.stop(audio);
        }
    }
}

impl GameInterface {
    /// The resource a deposit holds.
    pub fn resource_type(&self, e: &KnownEntity) -> Option<i64> {
        if !e.is_deposit() {
            return None;
        }
        self.view()
            .unit_type(&e.type_name)
            .and_then(|t| t.resource_type)
    }

    /// The type a building site becomes.
    fn site_type_name(&self, e: &KnownEntity) -> Option<String> {
        e.site_type
            .map(|t| self.view().unit_type_by_id(t).name.clone())
    }

    pub fn short_title(&self, cx: &Client, v: &EntityView) -> Msg {
        let style = cx.res.style();
        if v.type_name() == "buildingsite" {
            let t = self.site_type_name(&v.model).unwrap_or_default();
            [
                compute_title(style, &t),
                compute_title(style, v.type_name()),
            ]
            .concat()
        } else {
            compute_title(style, v.type_name())
        }
    }

    /// The title: the type, a unit of the player's with its number, another
    /// player's with whose it is, and for a memory, how long ago it was
    /// seen.
    pub fn title(&self, cx: &Client, v: &EntityView) -> Msg {
        let style = cx.res.style();
        let mut title = if v.model.is_building_site() {
            let t = self.site_type_name(&v.model).unwrap_or_default();
            [
                compute_title(style, &t),
                compute_title(style, "buildingsite"),
            ]
            .concat()
        } else {
            self.short_title(cx, v)
        };
        if let Some(p) = v.player() {
            if p == self.me {
                // The world numbers every unit a player has.
                if let Some(n) = v.model.number {
                    title.extend(nb2msg(n));
                }
            } else {
                if self.view().is_ally(p) {
                    title.extend(msg(mp::ALLY));
                } else {
                    title.extend(msg(mp::ENEMY));
                }
                title.extend(msg(mp::COMMA));
                title.extend(self.view().player_name(p));
                title.extend(msg(mp::COMMA));
            }
        }
        if let Some(ts) = v.model.last_seen {
            title.extend(msg(mp::IN_THE_FOG));
            title.extend(msg(mp::COMMA));
            if v.model.is_mobile {
                let s = (self.view().time() - ts).div_euclid(1000);
                let m = s.div_euclid(60);
                if m != 0 {
                    title.extend(nb2msg(m));
                    title.extend(msg(mp::MINUTES));
                } else if s != 0 {
                    title.extend(nb2msg(s));
                    title.extend(msg(mp::SECONDS));
                }
                title.extend(msg(mp::COMMA));
            }
        }
        title
    }

    /// The title and where it is: "footman 1 at a 2". Only the title for
    /// an entity that has left the map.
    pub fn ext_title(&self, cx: &Client, v: &EntityView) -> Msg {
        let mut t = self.title(cx, v);
        if let Some(place) = v.place() {
            t.extend(msg(mp::AT));
            t.extend(self.place_title(cx, place));
        }
        t
    }

    /// The upgrades of a creature's player that apply to it: for a
    /// remembered creature, those it had when the player last perceived
    /// it.
    fn upgrades_status(&self, cx: &Client, v: &EntityView) -> Msg {
        let style = cx.res.style();
        let upgrades = if v.is_memory() {
            self.view().remembered_upgrades(v.id()).to_vec()
        } else {
            self.view().unit_upgrades(v.id())
        };
        upgrades
            .iter()
            .flat_map(|u| style_msg(style, u, "title"))
            .collect()
    }

    pub fn description(&self, cx: &Client, v: &EntityView) -> Msg {
        let style = cx.res.style();
        let mut d = Vec::new();
        if v.qty() != 0 {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::CONTAINS));
            d.extend(nb2msg(v.qty()));
            let rt = self
                .resource_type(&v.model)
                .map_or("None".to_owned(), |r| r.to_string());
            d.extend(style_msg(
                style,
                "parameters",
                &format!("resource_{rt}_title"),
            ));
        }
        if v.is_creature() {
            d.extend(msg(mp::COMMA));
            d.extend(nb2msg(v.hp()));
            d.extend(msg(mp::HITPOINTS_ON));
            d.extend(nb2msg(v.hp_max()));
            d.extend(msg(mp::COMMA));
            if v.mana_max() > 0 {
                d.extend(nb2msg(v.mana()));
                d.extend(msg(mp::MANA_POINTS_ON));
                d.extend(nb2msg(v.mana_max()));
            }
            if v.player().is_some() {
                d.extend(msg(mp::COMMA));
                d.extend(self.upgrades_status(cx, v));
            }
        }
        let e = &v.model;
        if e.is_invisible || e.is_cloaked {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::INVISIBLE));
        }
        if e.is_a_detector {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::DETECTOR));
        }
        if e.is_a_cloaker {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::CLOAKER));
        }
        d
    }

    /// The center of the selected square, in meters; none before a square
    /// is selected.
    pub fn place_xy(&self) -> Option<(f64, f64)> {
        let (x, y) = self.view().square(self.place?).center();
        Some((x as f64 / 1000.0, y as f64 / 1000.0))
    }

    /// The distance from the listener, in meters.
    pub fn distance_to(&self, v: &EntityView) -> f64 {
        distance(self.x, self.y, v.x(), v.y())
    }

    /// The title, the distance in first person mode, the direction from
    /// the selected square's center and the description.
    pub fn positional_description(&self, cx: &Client, v: &EntityView) -> Msg {
        let mut d = self.title(cx, v);
        if self.immersion {
            d.extend(msg(mp::AT2));
            d.extend(nb2msg(self.distance_to(v) as i64));
            d.extend(msg(mp::METERS));
        }
        if let Some(center) = self.place_xy() {
            d.extend(direction_msg(center, self.square_width(), v.x(), v.y()));
        }
        d.extend(self.description(cx, v));
        d
    }

    /// The title of an order in a unit's list of orders.
    fn order_title_msg(&self, cx: &Client, order: &KnownOrder, nb: i64) -> Msg {
        let style = cx.res.style();
        let mut result = if order.is_deferred {
            style_msg(style, "messages", "production_deferred")
        } else {
            Vec::new()
        };
        result.extend(style_msg(style, order.keyword.as_str(), "title"));
        if let Some(t) = &order.type_name {
            let mut tt = style_msg(style, t, "title");
            if nb != 1 {
                tt = [nb2msg(nb), tt].concat();
            }
            result = substitute_args(result, &tt);
        }
        if let Some(targets) = &order.patrol {
            for place in targets {
                result.extend(self.place_view_title(cx, *place));
                result.extend(msg(mp::COMMA));
            }
        } else if let Some(t) = &order.target {
            if order.keyword == Keyword::BuildPhaseTwo {
                // A site the player does not know: no building said.
                if let Some(e) = self.target_entity(t) {
                    let tn = self.site_type_name(&e).unwrap_or_default();
                    result.extend(style_msg(style, &tn, "title"));
                }
            } else {
                result.extend(self.target_title(cx, t));
            }
        }
        [msg(mp::COMMA), result].concat()
    }

    /// The entity an order's target shows: as the player perceives or
    /// remembers it, or the copy the player's own order remembers.
    fn target_entity(&self, t: &KnownTarget) -> Option<KnownEntity> {
        match t {
            KnownTarget::Entity(id) => self.view().known(*id),
            KnownTarget::Remembered(e) => Some((**e).clone()),
            KnownTarget::Square(_) | KnownTarget::Point => None,
        }
    }

    /// The title of a place an order names: the square's, or the
    /// container's type title; none for a place off the map.
    fn place_view_title(&self, cx: &Client, place: Option<Place>) -> Msg {
        place.map_or_else(Vec::new, |p| self.place_title(cx, p))
    }

    /// The title of an order's target. An entity is described as the
    /// player perceives or remembers it, and has no title if the player
    /// knows nothing of it. The remembered target of the player's own order
    /// is its own memory, said as old as the player's memory of the entity
    /// is now. A square gives the square's title; a point has no title.
    fn target_title(&self, cx: &Client, t: &KnownTarget) -> Msg {
        match t {
            KnownTarget::Entity(id) => match self.view().known(*id) {
                Some(e) => self.title(cx, &EntityView::new(e)),
                None => Vec::new(),
            },
            KnownTarget::Remembered(e) => {
                let view = self.view();
                let remembered = view.remembered(e.id).and_then(|r| r.last_seen);
                let model = KnownEntity {
                    last_seen: Some(remembered.unwrap_or(view.time())),
                    ..(**e).clone()
                };
                self.title(cx, &EntityView::new(model))
            }
            KnownTarget::Square(s) => self.place_view_title(cx, Some(Place::Square(*s))),
            KnownTarget::Point => Vec::new(),
        }
    }

    /// The orders of a unit, trainings of the same type counted together,
    /// up to the first patrol.
    pub fn orders_txt(&self, cx: &Client, v: &EntityView) -> Msg {
        let mut t = Vec::new();
        let mut prev: Option<&KnownOrder> = None;
        let mut nb = 0;
        for o in v.orders() {
            if let Some(p) = prev {
                if o.keyword == Keyword::Train && p.type_name == o.type_name {
                    nb += 1;
                } else {
                    t.extend(self.order_title_msg(cx, p, nb));
                    if o.keyword == Keyword::Train {
                        prev = Some(o);
                        nb = 1;
                    } else {
                        t.extend(self.order_title_msg(cx, o, 1));
                        prev = None;
                    }
                }
            } else if o.keyword == Keyword::Train {
                prev = Some(o);
                nb = 1;
            } else {
                t.extend(self.order_title_msg(cx, o, 1));
            }
            if o.keyword == Keyword::Patrol {
                break;
            }
        }
        if let Some(p) = prev {
            t.extend(self.order_title_msg(cx, p, nb));
        }
        t.extend(msg(mp::COMMA));
        t
    }

    /// The order menu of a unit: the menu kinds it may receive.
    pub fn view_menu(&self, id: EntityId, strict: bool) -> Vec<String> {
        let view = self.view();
        self.orders_list
            .iter()
            .flat_map(|&k| view.order_menu(id, k, strict))
            .collect()
    }

    /// Whether the entity is on `place`. A unit in a transport is on the
    /// transport's square, as the player knows the transport, and on no
    /// square if the player knows nothing of it; a blocker is also on the
    /// other side of its exit.
    pub fn is_in(&self, v: &EntityView, place: Option<SquareId>) -> bool {
        self.model_is_in(&v.model, place)
    }

    pub fn model_is_in(&self, e: &KnownEntity, place: Option<SquareId>) -> bool {
        match e.place {
            Some(Place::Inside(c)) => {
                return self.container_place(c).map(|p| p.square()) == Some(place);
            }
            Some(Place::Square(s)) if Some(s) == place => return true,
            None if place.is_none() => return true,
            _ => {}
        }
        e.blocked_exit
            .and_then(|x| self.view().other_side_of_exit(x))
            .map(|p| p.square())
            == Some(place)
    }

    /// An exit, for the interface: a type with a sound for moving through.
    pub fn is_an_exit(&self, cx: &Client, v: &EntityView) -> bool {
        cx.res.style().has(v.type_name(), "when_moving_through")
    }

    /// A deposit, a building land, a damaged repairable or an exit: what a
    /// worker may work on.
    pub fn is_a_useful_target(&self, cx: &Client, v: &EntityView) -> bool {
        v.qty() > 0
            || v.model.is_a_building_land
            || v.model.is_repairable && v.hp() < v.hp_max()
            || self.is_an_exit(cx, v)
    }

    /// The style value of the entity's type, the player's side of an
    /// `if_me` value chosen.
    fn get_style_words(&self, cx: &Client, v: &EntityView, attr: &str) -> Vec<String> {
        match cx.res.style().get(v.type_name(), attr) {
            Some(StyleValue::Words(w)) => w.clone(),
            Some(StyleValue::IfMe(a, b)) => {
                if self.is_allied(v.player()) {
                    a.clone()
                } else {
                    b.clone()
                }
            }
            _ => Vec::new(),
        }
    }

    fn get_style_noise(&self, cx: &Client, v: &EntityView, attr: &str) -> Noise {
        match cx.res.style().get(v.type_name(), attr) {
            Some(StyleValue::Noise(Noise::IfMe(a, b))) => {
                if self.is_allied(v.player()) {
                    (**a).clone()
                } else {
                    (**b).clone()
                }
            }
            Some(StyleValue::Noise(n)) => n.clone(),
            _ => Noise::Silent,
        }
    }

    fn style_text(&self, cx: &Client, v: &EntityView, attr: &str) -> Msg {
        self.get_style_words(cx, v, attr)
            .into_iter()
            .map(MsgPart::Text)
            .collect()
    }

    /// Whether the entity is on the selected square, or next to it when
    /// nearby objects are heard.
    pub fn is_local(&self, cx: &Client, v: &EntityView) -> bool {
        let Some(ip) = self.place else {
            return false;
        };
        match v.place() {
            Some(Place::Square(s)) if s == ip => true,
            Some(Place::Square(s)) => {
                cx.params.flag_or("render_nearby_objects", false)
                    && self.view().square(ip).neighbors().contains(&s)
            }
            _ => false,
        }
    }

    /// Plays one of the sounds of a style attribute: near the entity, or
    /// as an alert from far away if `alert` and the entity is not local.
    pub fn launch_event_style(
        &self,
        cx: &Client,
        v: &EntityView,
        attr: &str,
        alert: bool,
        priority: i32,
    ) {
        let st = self.get_style_words(cx, v, attr);
        if st.is_empty() {
            return;
        }
        let s = st[cx.rng.below(st.len())].clone();
        if alert && !self.is_local(cx, v) {
            self.launch_alert_of(cx, v, &s);
        } else {
            self.launch_event(cx, v, &s, 1.0, priority, 0.0, false);
        }
    }

    /// Plays a sound at the entity if it is on or next to the selected
    /// square.
    #[allow(clippy::too_many_arguments)]
    pub fn launch_event(
        &self,
        cx: &Client,
        v: &EntityView,
        sound: &str,
        mut volume: f64,
        mut priority: i32,
        limit: f64,
        ambient: bool,
    ) {
        let Some(ip) = self.place else { return };
        match v.place() {
            Some(Place::Square(s)) if s == ip => {}
            Some(Place::Square(s)) if self.view().square(ip).neighbors().contains(&s) => {
                priority -= 1
            }
            _ => return,
        }
        if v.is_memory() {
            volume *= cx.params.number("fog_of_war_factor", 0.5);
        }
        cx.audio.play(
            cx.sounds.get_sound(sound),
            Placement {
                limit,
                ambient,
                ..Placement::new(volume, v.x(), v.y(), priority)
            },
        );
    }

    /// Plays an alert sound from the entity's square, as on a minimap.
    fn launch_alert_of(&self, cx: &Client, v: &EntityView, sound: &str) {
        let place = match v.place() {
            Some(Place::Inside(c)) => self.container_place(c),
            p => p,
        };
        self.launch_alert(cx, self.outer_square(place), sound);
    }

    // reactions to events

    /// Plays a sound of an event, unless the event came too late for its
    /// sounds.
    fn event_sound(&self, cx: &Client, v: &EntityView, attr: &str, alert: bool, timing: Timing) {
        if timing == Timing::OnTime {
            self.launch_event_style(cx, v, attr, alert, 0);
        }
    }

    /// Reacts to an event of the entity. A late event plays no sound, but
    /// its news is told.
    pub fn notify(&mut self, cx: &mut Client, v: &EntityView, event: &EntityEvent, timing: Timing) {
        match event {
            EntityEvent::Wounded {
                attacker_type,
                attacker,
                attacker_damage_level,
            } => self.on_wounded(
                cx,
                v,
                attacker_type,
                *attacker,
                *attacker_damage_level,
                timing,
            ),
            EntityEvent::DeathBy(a) => self.on_death_by(cx, v, *a, timing),
            EntityEvent::OrderOk => {
                if v.player() == Some(self.me) {
                    self.event_sound(cx, v, "order_ok", true, timing);
                }
            }
            EntityEvent::OrderImpossible(reason) => {
                if v.player() == Some(self.me) {
                    self.event_sound(cx, v, "order_impossible", true, timing);
                    if let Some(r) = reason {
                        self.say_reason(cx, r);
                    }
                }
            }
            EntityEvent::Attack => self.event_sound(cx, v, "attack", true, timing),
            EntityEvent::Collision => self.event_sound(cx, v, "blocked", false, timing),
            EntityEvent::Flee => self.event_sound(cx, v, "flee", true, timing),
            EntityEvent::Complete => self.on_complete(cx, v, timing),
            EntityEvent::Completeness(s) => {
                self.event_sound(cx, v, "production", false, timing);
                self.event_sound(cx, v, &format!("proportion_{s}"), false, timing);
            }
            EntityEvent::ProductionDeferred => {
                let m = style_msg(cx.res.style(), "messages", "production_deferred");
                let news = News::new(Category::Production, m);
                cx.news(news.about(Subject::ProductionDeferred));
            }
            EntityEvent::ResearchComplete => {
                let m = self.style_text(cx, v, "research_complete_msg");
                cx.news(News::new(Category::Production, m));
                self.send_menu_alerts_if_needed(cx);
            }
            EntityEvent::UseComplete(ability) => {
                let st = cx.res.style().words(ability, "alert");
                if !st.is_empty() && timing == Timing::OnTime {
                    let s = st[cx.rng.below(st.len())].clone();
                    self.launch_alert_of(cx, v, &s);
                }
            }
            EntityEvent::Store(r) => {
                self.event_sound(cx, v, &format!("store_resource_{r}"), false, timing)
            }
            EntityEvent::Exhausted => {
                if cx.cue(Category::Economy) {
                    self.event_sound(cx, v, "exhausted", false, timing);
                }
                let m = [self.title(cx, v), msg(mp::EXHAUSTED)].concat();
                cx.news(News::new(Category::Economy, m));
            }
            EntityEvent::Added => {
                if cx.cue(Category::Production) {
                    self.event_sound(cx, v, "added", true, timing);
                }
                if self.number_must_be_said(v) {
                    let t = self.style_text(cx, v, "added_msg");
                    let m = substitute_args(t, &self.ext_title(cx, v));
                    cx.news(News::new(Category::Production, m));
                }
            }
            other => {
                let name = other.text();
                self.event_sound(cx, v, &name, false, timing);
            }
        }
    }

    fn say_reason(&self, cx: &mut Client, r: &Reason) {
        let m = style_msg(cx.res.style(), "messages", &r.name());
        let news = News::new(Category::Production, m);
        cx.news(news.about(Subject::Refusal(r.clone())));
    }

    /// Whether the arrival of a unit with this number is worth telling.
    /// One of no player's has no number, and is not told.
    fn number_must_be_said(&self, v: &EntityView) -> bool {
        v.model.number.is_some_and(must_be_said)
    }

    fn on_complete(&mut self, cx: &mut Client, v: &EntityView, timing: Timing) {
        if v.player() != Some(self.me) {
            return;
        }
        if cx.cue(Category::Production) {
            self.event_sound(cx, v, "complete", true, timing);
        }
        if self.number_must_be_said(v) {
            let t = self.style_text(cx, v, "complete_msg");
            let m = substitute_args(t, &self.title(cx, v));
            cx.news(News::new(Category::Production, m));
        }
        self.send_menu_alerts_if_needed(cx);
    }

    fn on_wounded(
        &mut self,
        cx: &mut Client,
        v: &EntityView,
        attacker_type: &str,
        _attacker: EntityId,
        level: i64,
        timing: Timing,
    ) {
        if v.player() == Some(self.me) {
            self.unit_attacked_alert(cx, v, timing);
        }
        let s = cx
            .res
            .style()
            .words(attacker_type, &format!("attack_hit_level_{level}"));
        if !s.is_empty() && timing == Timing::OnTime {
            let sound = s[cx.rng.below(s.len())].clone();
            self.launch_event(cx, v, &sound, 1.0, 0, 0.0, false);
        }
    }

    /// Tells of an attack on a unit of the player, with the alert sound,
    /// when its place was not under attack already.
    fn unit_attacked_alert(&mut self, cx: &mut Client, v: &EntityView, timing: Timing) {
        if let Some(place) = v.place()
            && self.attack_alert(cx, place)
            && cx.cue(Category::Alert)
        {
            self.event_sound(cx, v, "alert", true, timing);
        }
    }

    fn on_lose_fight(&mut self, cx: &mut Client, v: &EntityView, timing: Timing) {
        if cx.cue(Category::Alert) {
            self.event_sound(cx, v, "lose_fight", true, timing);
        }
        self.units_alert_if_needed(cx, v.place());
    }

    fn on_win_fight(&mut self, cx: &mut Client, v: &EntityView, timing: Timing) {
        if cx.cue(Category::Alert) {
            self.event_sound(cx, v, "win_fight", true, timing);
        }
        self.units_alert_if_needed(cx, None);
    }

    fn on_death_by(
        &mut self,
        cx: &mut Client,
        v: &EntityView,
        attacker_id: EntityId,
        timing: Timing,
    ) {
        let attacker = self.dobjets.get(&attacker_id).map(|a| a.model.clone());
        let attacker_player = attacker.as_ref().and_then(|a| a.player);
        if v.player() == Some(self.me) {
            let t = self.short_title(cx, v);
            self.lost_units.push((t, v.place()));
        }
        if attacker_player == Some(self.me) {
            let t = self.short_title(cx, v);
            self.neutralized_units.push((t, v.place()));
        }
        // A fight is lost or won only by players.
        let Some(owner) = v.player() else {
            return;
        };
        let friends = self.view().comrades(owner, v.place(), v.id());
        let friend_soldiers = friends.soldiers > 0;
        if !friend_soldiers && v.model.is_a_threat || friends.units == 0 {
            if v.player() == Some(self.me) {
                self.on_lose_fight(cx, v, timing);
            }
            if let Some(a) = attacker
                && attacker_player == Some(self.me)
            {
                self.on_win_fight(cx, &EntityView::new(a), timing);
            }
        }
    }

    // animation

    /// Plays the noise, footsteps and health of an object near the
    /// listener.
    pub(crate) fn animate(&mut self, cx: &mut Client, id: EntityId) {
        let Some(v) = self.dobjets.get_mut(&id) else {
            return;
        };
        let mut anim = std::mem::take(&mut v.anim);
        let v = &self.dobjets[&id];
        if self.is_local(cx, v) {
            self.footstep(cx, v, &mut anim);
            self.update_noise(cx, v, &mut anim);
            self.render_hp(cx, v, &mut anim);
        }
        if let Some(v) = self.dobjets.get_mut(&id) {
            v.anim = anim;
        }
    }

    fn footstep_interval(&self, v: &EntityView, offset: f64) -> f64 {
        let s = v.model.actual_speed;
        if s == 0 {
            return 1000.0 * 999.0;
        }
        1000.0 / s as f64 / 2.0 + offset
    }

    fn terrain_footstep(&self, cx: &Client, v: &EntityView) -> Option<Vec<String>> {
        let style = cx.res.style();
        let s = v.place()?.square()?;
        let t = self.view().terrain(s);
        if t.is_empty() {
            return None;
        }
        let g = style.words(t, "ground");
        let attr = format!("move_on_{}", g.first()?);
        style
            .has(v.type_name(), &attr)
            .then(|| style.words(v.type_name(), &attr))
    }

    /// The footstep sounds of the entity: on the terrain, else on the
    /// nearest still object with a ground that the player knows on its
    /// square, else its own.
    fn footstep_noise(&self, cx: &Client, v: &EntityView) -> Vec<String> {
        let style = cx.res.style();
        let mut result = style.words(v.type_name(), "move");
        if v.model.airground != AirGround::Ground {
            return result;
        }
        if let Some(t) = self.terrain_footstep(cx, v) {
            return t;
        }
        let Some(s) = v.place().and_then(|p| p.square()) else {
            return result;
        };
        let objects: Vec<&EntityView> = self
            .dobjets
            .values()
            .filter(|o| o.place() == Some(Place::Square(s)))
            .collect();
        if objects.len() >= 30 {
            return result;
        }
        let mut d_min = 9_999_999.0;
        for o in objects {
            if o.model.is_mobile {
                continue;
            }
            let g = style.words(o.type_name(), "ground");
            let Some(g0) = g.first() else { continue };
            let attr = format!("move_on_{g0}");
            if !style.has(v.type_name(), &attr) {
                continue;
            }
            let k = g
                .get(1)
                .and_then(|k| soundrts_world::nofloat::py_float(k).ok())
                .unwrap_or(1.0);
            if k == 0.0 {
                continue;
            }
            let d = distance(o.x(), o.y(), v.x(), v.y()) / k;
            if d < d_min {
                result = style.words(v.type_name(), &attr);
                d_min = d;
            }
        }
        result
    }

    fn footstep(&self, cx: &Client, v: &EntityView, anim: &mut AnimState) {
        if !(v.model.is_moving && !v.is_memory()) {
            anim.next_step = None;
            return;
        }
        let offset = *anim
            .footstep_offset
            .get_or_insert_with(|| cx.rng.random() * 0.2);
        let now = cx.clock.now();
        let real_speed = self.real_speed();
        match anim.next_step {
            None => {
                anim.step_side = 1;
                anim.next_step =
                    Some(now + cx.rng.random() * self.footstep_interval(v, offset) / real_speed);
            }
            Some(next) if now > next => {
                let volume = if self.immersion && (v.x(), v.y()) == (self.x, self.y) {
                    0.5
                } else {
                    1.0
                };
                if let Some(s) = self.footstep_noise(cx, v).get(anim.step_side) {
                    self.launch_event(cx, v, s, volume, -10, FOOTSTEP_LIMIT, false);
                }
                anim.next_step = Some(now + self.footstep_interval(v, offset) / real_speed);
                anim.step_side = 1 - anim.step_side;
            }
            Some(_) => {}
        }
    }

    fn noise_style(&self, cx: &Client, v: &EntityView) -> (String, Noise) {
        let view = self.view();
        let activity = if v.is_memory() {
            view.remembered_activity(v.id())
        } else {
            view.activity(v.id())
        };
        if let Some(a) = activity {
            let attr = format!("noise_when_{a}");
            let st = self.get_style_noise(cx, v, &attr);
            if st != Noise::Silent {
                return (attr, st);
            }
        }
        if v.is_creature() {
            let (hp, hp_max) = (v.hp() as f64, v.hp_max() as f64);
            if hp < hp_max / 3.0 {
                let st = self.get_style_noise(cx, v, "noise_if_very_damaged");
                if st != Noise::Silent {
                    return ("noise_if_very_damaged".into(), st);
                }
            }
            if hp < hp_max * 2.0 / 3.0 {
                let st = self.get_style_noise(cx, v, "noise_if_damaged");
                if st != Noise::Silent {
                    return ("noise_if_damaged".into(), st);
                }
            }
        }
        ("noise".into(), self.get_style_noise(cx, v, "noise"))
    }

    fn update_noise(&self, cx: &Client, v: &EntityView, anim: &mut AnimState) {
        let (attr, st) = self.noise_style(cx, v);
        match &mut anim.noise {
            Some(n) if n.is(&attr, &st) => {
                let player = NoisePlayer {
                    audio: &cx.audio,
                    sounds: &cx.sounds,
                    rng: &cx.rng,
                    now: cx.clock.now(),
                    fog_of_war_factor: cx.params.number("fog_of_war_factor", 0.5),
                };
                n.update(
                    &player,
                    NoiseSite {
                        x: v.x(),
                        y: v.y(),
                        fow: v.is_memory(),
                    },
                );
            }
            current => {
                if let Some(n) = current {
                    n.stop(&cx.audio);
                }
                *current = NoiseSource::new(&attr, &st);
            }
        }
    }

    /// Plays the proportion of hit points left when it changes, and the
    /// sound of healing. A creature without hit points to lose has no
    /// proportion to tell.
    fn render_hp(&self, cx: &Client, v: &EntityView, anim: &mut AnimState) {
        if !v.is_creature() || v.hp() < 0 {
            return;
        }
        let hp = v.hp();
        if Some(hp) == anim.previous_hp {
            return;
        }
        let hp_max = v.hp_max();
        if let Some(previous) = anim.previous_hp
            && hp_max != 0
        {
            let noise = |h: i64| (h as f64 * 10.0 / hp_max as f64).trunc() as i64;
            if hp < previous || noise(hp) != noise(previous) {
                self.launch_event_style(cx, v, &format!("proportion_{}", noise(hp)), false, 0);
            }
            if hp > previous && v.model.is_healable {
                self.launch_event_style(cx, v, "healed", false, 0);
            }
        }
        anim.previous_hp = Some(hp);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{cheat, enemy, go, run_until, said, step, training, unit};
    use super::*;

    /// The player's peasant as the player knows it, with its first order, a
    /// real one, aiming at `target`.
    fn peasant_aiming_at(g: &mut crate::testkit::TestGame, target: KnownTarget) -> EntityView {
        let me = g.game.me;
        let peasant = unit(g, me, "peasant");
        let farm = unit(g, me, "farm");
        g.game
            .host_mut()
            .queue_line(me, &format!("control {}", peasant.0));
        g.game
            .host_mut()
            .queue_line(me, &format!("order 0 0 go {}", farm.0));
        step(g);
        let mut model = g.game.view().perceived(peasant).unwrap();
        model.own.as_mut().unwrap().orders[0].target = Some(target);
        EntityView::new(model)
    }

    #[test]
    fn footsteps_sound_on_the_objects_the_player_knows() {
        let (h, mut g) = training();
        let me = g.game.me;
        let peasant = unit(&g, me, "peasant");
        let v = &g.game.dobjets[&peasant];
        // On its meadows the peasant steps on the grass of the nearest
        // building or meadow.
        assert_eq!(g.game.footstep_noise(&h.client, v), ["1053", "1054"]);
        // The player remembers a wood under its feet, which is not there:
        // the steps are on leaves.
        let wood = g
            .game
            .dobjets
            .values()
            .find(|o| o.type_name() == "wood")
            .unwrap();
        let mut model = wood.model.clone();
        (model.id, model.place, model.x, model.y, model.last_seen) =
            (EntityId(999_999), v.place(), v.model.x, v.model.y, Some(0));
        let remembered = EntityView::new(model);
        g.game.dobjets.insert(EntityId(999_999), remembered);
        let v = &g.game.dobjets[&peasant];
        assert_eq!(g.game.footstep_noise(&h.client, v), ["1069", "1070"]);
    }

    #[test]
    fn a_unit_inside_a_container_the_player_does_not_know_is_nowhere() {
        let (h, mut g) = training();
        let them = enemy(&g);
        let townhall = unit(&g, them, "townhall");
        let there = g.game.host().world().entity(townhall).unwrap().square();
        assert!(there.is_some());
        assert!(g.game.view().known(townhall).is_none());
        // The player remembers a unit going into the computer's town hall,
        // which it has never seen.
        let me = g.game.me;
        let peasant = unit(&g, me, "peasant");
        let mut model = g.game.dobjets[&peasant].model.clone();
        (model.id, model.player, model.own) = (EntityId(999_999), Some(them), None);
        (model.place, model.last_seen) = (Some(Place::Inside(townhall)), Some(0));
        let v = EntityView::new(model);
        assert!(!g.game.is_in(&v, there));
        assert!(!g.game.is_in(&v, None));
        assert_eq!(g.game.outer_square(v.place()), None);
        assert_eq!(g.game.place_title(&h.client, Place::Inside(townhall)), []);
        g.game.dobjets.insert(v.id(), v);
        let m = g.game.place_summary(&h.client, there, true, None, false);
        assert_eq!(m, []);
    }

    #[test]
    fn an_order_names_its_target_as_the_player_knows_it() {
        let (mut h, mut g) = training();
        let them = enemy(&g);
        let townhall = unit(&g, them, "townhall");
        let v = peasant_aiming_at(&mut g, KnownTarget::Entity(townhall));
        // The computer's town hall is there, unseen: the order has no
        // target title.
        assert!(g.game.view().known(townhall).is_none());
        let m = g.game.orders_txt(&h.client, &v);
        assert_eq!(said(&mut h, &m), ", go to ,");
    }

    #[test]
    fn a_remembered_target_is_named_as_the_order_remembers_it() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let peasant = unit(&g, me, "peasant");
        let meadow = g.game.dobjets.values().find(|o| o.type_name() == "meadow");
        let meadow = meadow.unwrap().id();
        g.game
            .host_mut()
            .queue_line(me, &format!("control {}", peasant.0));
        g.game
            .host_mut()
            .queue_line(me, &format!("order 0 0 build farm {}", meadow.0));
        let has_site = |g: &crate::testkit::TestGame| {
            let w = g.game.host().world();
            let units = &w.player(me).units;
            units
                .iter()
                .any(|&u| &*w.entity(u).unwrap().type_name == "buildingsite")
        };
        for _ in 0..300 {
            if has_site(&g) {
                break;
            }
            step(&mut g);
        }
        // The player's memory of a farm's site that is no longer there.
        let site = unit(&g, me, "buildingsite");
        let mut remembered = g.game.view().perceived(site).unwrap();
        remembered.id = EntityId(999_999);
        let remembered = KnownTarget::Remembered(Box::new(remembered));
        let mut v = peasant_aiming_at(&mut g, remembered);
        let m = g.game.orders_txt(&h.client, &v);
        assert_eq!(
            said(&mut h, &m),
            ", go to farm at building 1 in the fog of war , ,"
        );
        // The building a second build phase makes: as the order remembers
        // the site, or nothing for a site the player knows nothing of.
        let order = &mut v.model.own.as_mut().unwrap().orders[0];
        order.keyword = Keyword::BuildPhaseTwo;
        let m = g.game.orders_txt(&h.client, &v);
        assert_eq!(said(&mut h, &m), ", build farm ,");
        let order = &mut v.model.own.as_mut().unwrap().orders[0];
        order.target = Some(KnownTarget::Entity(EntityId(999_999)));
        let m = g.game.orders_txt(&h.client, &v);
        assert_eq!(said(&mut h, &m), ", build ,");
    }

    #[test]
    fn directions_round_to_the_nearest_compass_point_as_python_does() {
        assert_eq!(direction_to_msgpart(0.0), mp::EAST);
        assert_eq!(direction_to_msgpart(22.5), mp::EAST); // round(0.5) == 0
        assert_eq!(direction_to_msgpart(67.5), mp::NORTH); // round(1.5) == 2
        assert_eq!(direction_to_msgpart(90.0), mp::NORTH);
        assert_eq!(direction_to_msgpart(-90.0), mp::SOUTH);
        assert_eq!(direction_to_msgpart(-157.5), mp::WEST); // round(-3.5) == -4
        assert_eq!(direction_to_msgpart(-112.5), mp::SOUTH); // round(-2.5) == -2
        assert_eq!(direction_to_msgpart(-130.0), mp::SOUTHWEST);
        assert_eq!(direction_to_msgpart(180.0), mp::WEST);
        assert_eq!(direction_to_msgpart(405.0), mp::NORTHEAST);
        assert_eq!(direction_to_msgpart(315.0), mp::SOUTHEAST);
    }

    #[test]
    fn positions_are_said_from_the_center_of_the_square() {
        let c = (6.0, 6.0);
        assert_eq!(direction_msg(c, 12.0, 7.9, 6.0), msg(mp::AT_THE_CENTER));
        assert_eq!(direction_msg(c, 12.0, 10.0, 6.0), msg(mp::TO_THE_EAST));
        assert_eq!(direction_msg(c, 12.0, 2.0, 6.0), msg(mp::TO_THE_WEST));
        assert_eq!(
            direction_msg(c, 12.0, 6.0, 10.0),
            [msg(mp::TO_THE), msg(mp::NORTH)].concat()
        );
        assert_eq!(
            direction_msg(c, 12.0, 2.0, 2.0),
            [msg(mp::TO_THE), msg(mp::SOUTHWEST)].concat()
        );
    }

    #[test]
    fn a_remembered_unit_has_the_upgrades_it_had_when_last_seen() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let w = g.game.host().world();
        let mine = w
            .entity(unit(&g, me, "townhall"))
            .unwrap()
            .square()
            .unwrap();
        let theirs = w
            .entity(unit(&g, them, "townhall"))
            .unwrap()
            .square()
            .unwrap();
        let name = g.game.view().square(mine).name().to_owned();
        // An enemy footman with one upgrade, on the player's square.
        cheat(
            &mut g,
            them,
            &format!("add_units {name} footman melee_weapon"),
        );
        let footman = {
            run_until(&mut h, &mut g, |g| {
                g.dobjets.values().any(|v| v.type_name() == "footman")
            });
            unit(&g, them, "footman")
        };
        let upgrades = |h: &mut crate::testkit::Harness, g: &crate::testkit::TestGame| {
            let m = g.game.upgrades_status(&h.client, &g.game.dobjets[&footman]);
            said(h, &m)
        };
        assert!(!g.game.dobjets[&footman].is_memory());
        assert_eq!(upgrades(&mut h, &g), "swords");

        // Out of sight, its owner gains a second upgrade.
        go(&mut g, them, footman, theirs);
        run_until(&mut h, &mut g, |g| g.dobjets[&footman].is_memory());
        cheat(&mut g, them, "add_units melee_armor");
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        assert!(g.game.dobjets[&footman].is_memory());
        let w = g.game.host().world();
        assert_eq!(w.unit_upgrades(footman), ["melee_weapon", "melee_armor"]);
        assert_eq!(upgrades(&mut h, &g), "swords");

        // Seen again.
        go(&mut g, them, footman, mine);
        run_until(&mut h, &mut g, |g| !g.dobjets[&footman].is_memory());
        assert_eq!(upgrades(&mut h, &g), "swords footmen armors");
    }
}
