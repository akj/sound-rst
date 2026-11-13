//! What the player knows of an entity (`clientgameentity.py`): its titles
//! and description, its noise and footsteps, and the reactions to what
//! happens to it.

use std::sync::Arc;

use soundrts_world::events::{EntityEvent, Reason};
use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::{Keyword, Order};
use soundrts_world::rules::AirGround;
use soundrts_world::{Entity, EntityId, Place, PlayerId, SquareId, Target};

use crate::audio::{angle, distance};
use crate::client::Client;
use crate::rng::ClientRng;
use crate::sound::{Audio, Placement};
use crate::style::{Noise, StyleValue};

use super::animation::{NoisePlayer, NoiseSite, NoiseSource};
use super::news::must_be_said;
use super::order::{PRECISION, substitute_args};
use super::{GameInterface, Oops, R, compute_title, style_msg};

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
}

/// An entity as the player knows it: a copy from the last turn, or a
/// remembered copy.
pub struct EntityView {
    pub model: Arc<Entity>,
    /// When it was last seen, for a remembered entity.
    pub time_stamp: Option<i64>,
    /// A facing given by the rotation keys, which hides the model's.
    pub o_override: Option<f64>,
    /// So that several walking units do not step together.
    pub footstep_random: f64,
    pub anim: AnimState,
}

impl EntityView {
    pub fn new(model: Arc<Entity>, time_stamp: Option<i64>, rng: &ClientRng) -> EntityView {
        EntityView {
            model,
            time_stamp,
            o_override: None,
            footstep_random: Self::footstep_offset(rng),
            anim: AnimState::default(),
        }
    }

    /// The offset added to a view's footstep interval, drawn when the view
    /// is made.
    pub fn footstep_offset(rng: &ClientRng) -> f64 {
        rng.random() * 0.2
    }

    pub fn id(&self) -> EntityId {
        self.model.id
    }

    pub fn is_memory(&self) -> bool {
        self.time_stamp.is_some()
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
        self.model.stats.hp_max / PRECISION
    }

    pub fn mana(&self) -> i64 {
        self.model.mana / PRECISION
    }

    pub fn mana_max(&self) -> i64 {
        self.model.stats.mana_max / PRECISION
    }

    pub fn qty(&self) -> i64 {
        self.model.stats.qty / PRECISION
    }

    pub fn stop(&self, audio: &Audio) {
        if let Some(n) = &self.anim.noise {
            n.stop(audio);
        }
    }
}

impl GameInterface {
    /// The resource a deposit holds.
    pub fn resource_type(&self, e: &Entity) -> Option<i64> {
        if !e.is_deposit() {
            return None;
        }
        self.world
            .rules
            .unit_type(&e.type_name)
            .and_then(|t| t.resource_type)
    }

    /// The type a building site becomes.
    fn site_type_name(&self, e: &Entity) -> Option<String> {
        e.site
            .as_ref()
            .map(|s| self.world.rules.ty(s.building_type).name.clone())
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

    pub fn title(&self, cx: &Client, v: &EntityView) -> R<Msg> {
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
                let n = v
                    .model
                    .number
                    .ok_or_else(|| Oops::new("int() argument must be a number, not 'NoneType'"))?;
                title.extend(nb2msg(n));
            } else {
                if self.world.player(self.me).allied.contains(&p) {
                    title.extend(msg(mp::ALLY));
                } else {
                    title.extend(msg(mp::ENEMY));
                }
                title.extend(msg(mp::COMMA));
                title.extend(self.world.player(p).name());
                title.extend(msg(mp::COMMA));
            }
        }
        if let Some(ts) = v.time_stamp {
            title.extend(msg(mp::IN_THE_FOG));
            title.extend(msg(mp::COMMA));
            if v.model.stats.speed != 0 {
                let s = (self.world.time - ts).div_euclid(1000);
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
        Ok(title)
    }

    /// The title and where it is: "footman 1 at a 2".
    pub fn ext_title(&self, cx: &Client, v: &EntityView) -> R<Msg> {
        let place = v
            .place()
            .ok_or_else(|| Oops::new(format!("problem with {}.ext_title", v.type_name())))?;
        Ok([self.title(cx, v)?, msg(mp::AT), self.place_title(cx, place)].concat())
    }

    /// The upgrades of a creature's player that apply to it.
    fn upgrades_status(&self, cx: &Client, v: &EntityView) -> Msg {
        let style = cx.res.style();
        self.world
            .unit_upgrades(v.id())
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
        let s = &v.model.stats;
        if s.is_invisible != 0 || v.model.is_cloaked {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::INVISIBLE));
        }
        if s.is_a_detector != 0 {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::DETECTOR));
        }
        if s.is_a_cloaker != 0 {
            d.extend(msg(mp::COMMA));
            d.extend(msg(mp::CLOAKER));
        }
        d
    }

    /// The center of the selected square, in meters.
    pub fn place_xy(&self) -> R<(f64, f64)> {
        let p = self
            .place
            .ok_or_else(|| Oops::new("'NoneType' object has no attribute 'x'"))?;
        let s = self.world.square(p);
        Ok((s.x as f64 / 1000.0, s.y as f64 / 1000.0))
    }

    /// The distance from the listener, in meters.
    pub fn distance_to(&self, v: &EntityView) -> f64 {
        distance(self.x, self.y, v.x(), v.y())
    }

    /// The title, the distance in first person mode, the direction from
    /// the square's center and the description.
    pub fn positional_description(&self, cx: &Client, v: &EntityView) -> R<Msg> {
        let mut d = self.title(cx, v)?;
        if self.immersion {
            d.extend(msg(mp::AT2));
            d.extend(nb2msg(self.distance_to(v) as i64));
            d.extend(msg(mp::METERS));
        }
        d.extend(direction_msg(
            self.place_xy()?,
            self.square_width(),
            v.x(),
            v.y(),
        ));
        d.extend(self.description(cx, v));
        Ok(d)
    }

    /// The title of an order in a unit's list of orders.
    fn order_title_msg(&self, cx: &Client, order: &Order, nb: i64) -> R<Msg> {
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
        if let Some(targets) = &order.patrol_targets {
            for place in targets {
                result.extend(self.place_view_title(cx, *place)?);
                result.extend(msg(mp::COMMA));
            }
        } else if let Some(t) = &order.target {
            if order.keyword == Keyword::BuildPhaseTwo {
                let e = self
                    .target_entity(t)
                    .ok_or_else(|| Oops::new("'Square' object has no attribute 'type'"))?;
                let tn = self.site_type_name(&e).unwrap_or_default();
                result.extend(style_msg(style, &tn, "title"));
            } else {
                result.extend(self.target_title(cx, t)?);
            }
        }
        Ok([msg(mp::COMMA), result].concat())
    }

    /// The entity an order's target shows: the live one or the remembered
    /// copy.
    fn target_entity(&self, t: &Target) -> Option<Arc<Entity>> {
        match t {
            Target::Entity(id) => self.world.entity(*id).map(|e| Arc::new(e.clone())),
            Target::Memory(m) => Some(m.snapshot.clone()),
            _ => None,
        }
    }

    /// The title of a place as the original gets it from a view of the
    /// place: the view's own title needs a player, which a place has not,
    /// so Python falls back to the place's `title` (the container's for an
    /// inside). The view draws its footstep offset first.
    fn place_view_title(&self, cx: &Client, place: Option<Place>) -> R<Msg> {
        EntityView::footstep_offset(&cx.rng);
        match place {
            Some(p) => Ok(self.place_title(cx, p)),
            None => Err(Oops::new("'NoneType' object has no attribute 'title'")),
        }
    }

    /// The title of an order's target, made as the original makes a view
    /// of it (the view draws its footstep offset first). A square or a
    /// subsquare gives the square's title (see
    /// [`GameInterface::place_view_title`]); a point has no title.
    fn target_title(&self, cx: &Client, t: &Target) -> R<Msg> {
        match t {
            Target::Entity(id) => {
                let e = self
                    .world
                    .entity(*id)
                    .ok_or_else(|| Oops::new("unknown target"))?;
                let v = EntityView::new(Arc::new(e.clone()), None, &cx.rng);
                self.title(cx, &v)
            }
            Target::Memory(m) => {
                let ts = self
                    .world
                    .player(m.owner)
                    .memory
                    .get(&m.snapshot.id)
                    .map_or(self.world.time, |r| r.time_stamp);
                let v = EntityView::new(m.snapshot.clone(), Some(ts), &cx.rng);
                self.title(cx, &v)
            }
            Target::Square(s) => self.place_view_title(cx, Some(Place::Square(*s))),
            Target::Zoom(z) => self.place_view_title(cx, Some(Place::Square(z.place))),
            Target::Xy(..) => {
                EntityView::footstep_offset(&cx.rng);
                Err(Oops::new("'tuple' object has no attribute 'title'"))
            }
        }
    }

    /// The orders of a unit, trainings of the same type counted together,
    /// up to the first patrol.
    pub fn orders_txt(&self, cx: &Client, v: &EntityView) -> R<Msg> {
        let mut t = Vec::new();
        let mut prev: Option<&Order> = None;
        let mut nb = 0;
        for o in &v.model.orders {
            if let Some(p) = prev {
                if o.keyword == Keyword::Train && p.type_name == o.type_name {
                    nb += 1;
                } else {
                    t.extend(self.order_title_msg(cx, p, nb)?);
                    if o.keyword == Keyword::Train {
                        prev = Some(o);
                        nb = 1;
                    } else {
                        t.extend(self.order_title_msg(cx, o, 1)?);
                        prev = None;
                    }
                }
            } else if o.keyword == Keyword::Train {
                prev = Some(o);
                nb = 1;
            } else {
                t.extend(self.order_title_msg(cx, o, 1)?);
            }
            if o.keyword == Keyword::Patrol {
                break;
            }
        }
        if let Some(p) = prev {
            t.extend(self.order_title_msg(cx, p, nb)?);
        }
        t.extend(msg(mp::COMMA));
        Ok(t)
    }

    /// The order menu of a unit: the menu kinds it may receive.
    pub fn view_menu(&self, id: EntityId, strict: bool) -> Vec<String> {
        if self.world.entity(id).is_none() {
            return Vec::new();
        }
        self.orders_list
            .iter()
            .flat_map(|&k| self.world.order_menu(id, k, strict))
            .collect()
    }

    /// Whether the entity is on `place`. A unit in a transport is on the
    /// transport's square; a blocker is also on the other side of its exit.
    pub fn is_in(&self, v: &EntityView, place: Option<SquareId>) -> bool {
        self.model_is_in(&v.model, place)
    }

    pub fn model_is_in(&self, e: &Entity, place: Option<SquareId>) -> bool {
        match e.place {
            Some(Place::Inside(c)) => {
                return self
                    .world
                    .entity(c)
                    .and_then(|c| c.place)
                    .map(|p| p.square())
                    == Some(place);
            }
            Some(Place::Square(s)) if Some(s) == place => return true,
            None if place.is_none() => return true,
            _ => {}
        }
        e.blocked_exit
            .and_then(|x| self.world.entity(x))
            .and_then(|x| x.exit.as_ref())
            .and_then(|x| x.other_side)
            .and_then(|o| self.world.entity(o))
            .is_some_and(|o| o.place.map(|p| p.square()) == Some(place))
    }

    /// An exit, for the interface: a type with a sound for moving through.
    pub fn is_an_exit(&self, cx: &Client, v: &EntityView) -> bool {
        cx.res.style().has(v.type_name(), "when_moving_through")
    }

    /// Whether an exit is closed by a forest or a blocker; with `player`,
    /// a friendly gate lets the player through.
    pub fn exit_is_blocked(&self, id: EntityId, player: Option<PlayerId>) -> bool {
        let Some(e) = self.world.entity(id) else {
            return false;
        };
        let Some(x) = &e.exit else {
            return false;
        };
        if x.is_blocked_by_forests {
            return true;
        }
        let other = x
            .other_side
            .and_then(|o| self.world.entity(o))
            .and_then(|o| o.exit.as_ref())
            .map(|o| o.blockers.clone())
            .unwrap_or_default();
        x.blockers.iter().chain(&other).any(|&b| {
            let Some(b) = self.world.entity(b) else {
                return false;
            };
            let enemy = match player {
                None => true,
                Some(p) => b
                    .player
                    .is_some_and(|bp| !self.world.player(p).allied.contains(&bp)),
            };
            b.stats.is_a_gate == 0 || enemy
        })
    }

    /// A deposit, a building land, a damaged repairable or an exit: what a
    /// worker may work on.
    pub fn is_a_useful_target(&self, cx: &Client, v: &EntityView) -> bool {
        v.qty() > 0
            || v.model.is_a_building_land()
            || v.model.stats.is_repairable != 0 && v.hp() < v.hp_max()
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
                    && self.world.square(ip).neighbors.contains(&s)
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
            Some(Place::Square(s)) if self.world.square(ip).neighbors.contains(&s) => priority -= 1,
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
            Some(Place::Inside(c)) => self.world.entity(c).and_then(|c| c.place),
            p => p,
        };
        self.launch_alert(cx, self.outer_square(place), sound);
    }

    // reactions to events

    /// Reacts to an event of the entity.
    pub fn notify(&mut self, cx: &mut Client, v: &EntityView, event: &EntityEvent) -> R {
        match event {
            EntityEvent::Wounded {
                attacker_type,
                attacker,
                attacker_damage_level,
            } => self.on_wounded(cx, v, attacker_type, *attacker, *attacker_damage_level),
            EntityEvent::DeathBy(a) => self.on_death_by(cx, v, *a)?,
            EntityEvent::OrderOk => {
                if v.player() == Some(self.me) {
                    self.launch_event_style(cx, v, "order_ok", true, 0);
                }
            }
            EntityEvent::OrderImpossible(reason) => {
                if v.player() == Some(self.me) {
                    self.launch_event_style(cx, v, "order_impossible", true, 0);
                    if let Some(r) = reason {
                        self.say_reason(cx, r);
                    }
                }
            }
            EntityEvent::Attack => self.launch_event_style(cx, v, "attack", true, 0),
            EntityEvent::Collision => self.launch_event_style(cx, v, "blocked", false, 0),
            EntityEvent::Flee => self.launch_event_style(cx, v, "flee", true, 0),
            EntityEvent::Complete => self.on_complete(cx, v)?,
            EntityEvent::Completeness(s) => {
                self.launch_event_style(cx, v, "production", false, 0);
                self.launch_event_style(cx, v, &format!("proportion_{s}"), false, 0);
            }
            EntityEvent::ProductionDeferred => {
                let m = style_msg(cx.res.style(), "messages", "production_deferred");
                cx.voice.info(&m);
            }
            EntityEvent::ResearchComplete => {
                let m = self.style_text(cx, v, "research_complete_msg");
                cx.voice.info(&m);
                self.send_menu_alerts_if_needed(cx);
            }
            EntityEvent::UseComplete(ability) => {
                let st = cx.res.style().words(ability, "alert");
                if !st.is_empty() {
                    let s = st[cx.rng.below(st.len())].clone();
                    self.launch_alert_of(cx, v, &s);
                }
            }
            EntityEvent::Store(r) => {
                self.launch_event_style(cx, v, &format!("store_resource_{r}"), false, 0)
            }
            EntityEvent::Exhausted => {
                self.launch_event_style(cx, v, "exhausted", false, 0);
                if cx.config.verbosity.contains("resource_exhausted") {
                    let m = [self.title(cx, v)?, msg(mp::EXHAUSTED)].concat();
                    cx.voice.info(&m);
                }
            }
            EntityEvent::Added => {
                self.launch_event_style(cx, v, "added", true, 0);
                if cx.config.verbosity.contains("unit_added") && self.number_must_be_said(v)? {
                    let t = self.style_text(cx, v, "added_msg");
                    let m = substitute_args(t, &self.ext_title(cx, v)?);
                    cx.voice.info(&m);
                }
            }
            other => {
                let name = other.text();
                self.launch_event_style(cx, v, &name, false, 0);
            }
        }
        Ok(())
    }

    fn say_reason(&self, cx: &mut Client, r: &Reason) {
        let m = style_msg(cx.res.style(), "messages", &r.name());
        cx.voice.info(&m);
    }

    fn number_must_be_said(&self, v: &EntityView) -> R<bool> {
        v.model
            .number
            .map(must_be_said)
            .ok_or_else(|| Oops::new("'<=' not supported between 'NoneType' and 'int'"))
    }

    fn on_complete(&mut self, cx: &mut Client, v: &EntityView) -> R {
        if v.player() != Some(self.me) {
            return Ok(());
        }
        self.launch_event_style(cx, v, "complete", true, 0);
        if cx.config.verbosity.contains("unit_complete") && self.number_must_be_said(v)? {
            let t = self.style_text(cx, v, "complete_msg");
            let m = substitute_args(t, &self.title(cx, v)?);
            cx.voice.info(&m);
        }
        self.send_menu_alerts_if_needed(cx);
        Ok(())
    }

    fn on_wounded(
        &mut self,
        cx: &mut Client,
        v: &EntityView,
        attacker_type: &str,
        _attacker: EntityId,
        level: i64,
    ) {
        if v.player() == Some(self.me) {
            self.unit_attacked_alert(cx, v);
        }
        let s = cx
            .res
            .style()
            .words(attacker_type, &format!("attack_hit_level_{level}"));
        if !s.is_empty() {
            let sound = s[cx.rng.below(s.len())].clone();
            self.launch_event(cx, v, &sound, 1.0, 0, 0.0, false);
        }
    }

    fn unit_attacked_alert(&mut self, cx: &mut Client, v: &EntityView) {
        let now = cx.clock.now();
        if let Some(p) = v.place() {
            self.alert_squares.insert(p, now);
        }
        self.squares_alert_if_needed(cx);
        if self
            .previous_unit_attacked_alert
            .is_none_or(|t| now > t + 10.0)
        {
            self.launch_event_style(cx, v, "alert", true, 0);
            self.previous_unit_attacked_alert = Some(now);
        }
    }

    fn on_lose_fight(&mut self, cx: &mut Client, v: &EntityView) -> R {
        self.launch_event_style(cx, v, "lose_fight", true, 0);
        self.units_alert_if_needed(cx, v.place())
    }

    fn on_win_fight(&mut self, cx: &mut Client, v: &EntityView) -> R {
        self.launch_event_style(cx, v, "win_fight", true, 0);
        self.units_alert_if_needed(cx, None)
    }

    fn on_death_by(&mut self, cx: &mut Client, v: &EntityView, attacker_id: EntityId) -> R {
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
        let owner = v
            .player()
            .ok_or_else(|| Oops::new("'NoneType' object has no attribute 'units'"))?;
        let friends: Vec<&Entity> = self
            .world
            .player(owner)
            .units
            .iter()
            .filter_map(|&u| self.world.entity(u))
            .filter(|u| u.place == v.place() && u.id != v.id())
            .collect();
        let friend_soldiers = friends.iter().any(|u| u.menace() != 0);
        if !friend_soldiers && v.model.menace() != 0 || friends.is_empty() {
            if v.player() == Some(self.me) {
                self.on_lose_fight(cx, v)?;
            }
            if let Some(a) = attacker
                && attacker_player == Some(self.me)
            {
                let av = EntityView {
                    model: a,
                    time_stamp: None,
                    o_override: None,
                    footstep_random: 0.0,
                    anim: AnimState::default(),
                };
                self.on_win_fight(cx, &av)?;
            }
        }
        Ok(())
    }

    // animation

    /// Plays the noise, footsteps and health of an object near the
    /// listener.
    pub(crate) fn animate(&mut self, cx: &mut Client, id: EntityId) -> R {
        let Some(v) = self.dobjets.get_mut(&id) else {
            return Ok(());
        };
        let mut anim = std::mem::take(&mut v.anim);
        let v = &self.dobjets[&id];
        let r = if self.is_local(cx, v) {
            self.footstep(cx, v, &mut anim);
            self.update_noise(cx, v, &mut anim);
            self.render_hp(cx, v, &mut anim)
        } else {
            Ok(())
        };
        if let Some(v) = self.dobjets.get_mut(&id) {
            v.anim = anim;
        }
        r
    }

    fn footstep_interval(&self, v: &EntityView) -> f64 {
        let s = v.model.actual_speed;
        if s == 0 {
            return 1000.0 * 999.0;
        }
        1000.0 / s as f64 / 2.0 + v.footstep_random
    }

    fn terrain_footstep(&self, cx: &Client, v: &EntityView) -> Option<Vec<String>> {
        let style = cx.res.style();
        let s = v.place()?.square()?;
        let t = &self.world.square(s).type_name;
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
    /// nearest still object with a ground, else its own.
    fn footstep_noise(&self, cx: &Client, v: &EntityView) -> Vec<String> {
        let style = cx.res.style();
        let mut result = style.words(v.type_name(), "move");
        if v.model.airground_type != AirGround::Ground {
            return result;
        }
        if let Some(t) = self.terrain_footstep(cx, v) {
            return t;
        }
        let Some(s) = v.place().and_then(|p| p.square()) else {
            return result;
        };
        let objects = &self.world.square(s).objects;
        if objects.len() >= 30 {
            return result;
        }
        let mut d_min = 9_999_999.0;
        for &mid in objects {
            let Some(m) = self.world.entity(mid) else {
                continue;
            };
            if m.stats.speed != 0 {
                continue;
            }
            let g = style.words(&m.type_name, "ground");
            let Some(g0) = g.first() else { continue };
            let attr = format!("move_on_{g0}");
            if !style.has(v.type_name(), &attr) {
                continue;
            }
            let k = g
                .get(1)
                .and_then(|k| soundrts_world::nofloat::py_float(k).ok())
                .unwrap_or(1.0);
            let Some(o) = self.dobjets.get(&mid) else {
                continue;
            };
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
        let now = cx.clock.now();
        let real_speed = self.real_speed();
        match anim.next_step {
            None => {
                anim.step_side = 1;
                anim.next_step =
                    Some(now + cx.rng.random() * self.footstep_interval(v) / real_speed);
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
                anim.next_step = Some(now + self.footstep_interval(v) / real_speed);
                anim.step_side = 1 - anim.step_side;
            }
            Some(_) => {}
        }
    }

    fn noise_style(&self, cx: &Client, v: &EntityView) -> (String, Noise) {
        if let Some(a) = self.world.activity(v.id()) {
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

    fn render_hp(&self, cx: &Client, v: &EntityView, anim: &mut AnimState) -> R {
        if !v.is_creature() || v.hp() < 0 {
            return Ok(());
        }
        let hp = v.hp();
        if Some(hp) == anim.previous_hp {
            return Ok(());
        }
        if let Some(previous) = anim.previous_hp {
            let hp_max = v.hp_max();
            if hp_max == 0 {
                anim.previous_hp = Some(hp);
                return Err(Oops::new("division by zero"));
            }
            let noise = |h: i64| (h as f64 * 10.0 / hp_max as f64).trunc() as i64;
            if hp < previous || noise(hp) != noise(previous) {
                self.launch_event_style(cx, v, &format!("proportion_{}", noise(hp)), false, 0);
            }
            if hp > previous && v.model.stats.is_healable != 0 {
                self.launch_event_style(cx, v, "healed", false, 0);
            }
        }
        anim.previous_hp = Some(hp);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
