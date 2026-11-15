//! What changed in a seat's [`TurnMessage`] since one it was sent before:
//! the form a host sends each turn once a seat has a whole message.
//!
//! [`TurnChange::between`] makes the change from the message a seat last
//! had and the current one, and [`TurnChange::apply`] gives back exactly
//! the current one from the last. The first message of a game, and the
//! message to a seat that has just joined or returned, is sent whole.
//!
//! - Entities are by id: a new one whole, a gone one by its id, and a
//!   changed one as its id and the fields that changed ([`EntityFields`]).
//!   The perceived and remembered entities are one list here, since an
//!   entity is in one or the other and its `last_seen` says which.
//! - A remembered entity seen again, on a square the seat still partly
//!   sees, changes only its `last_seen`, every turn. It is named, not sent:
//!   [`SeenAgain`] gives the time once for all of them.
//! - The seat's state and squares are sent as the fields that changed, a
//!   set of squares as the squares that came in or went out.
//! - Events are the turn's, whole. A seat that fell behind gets one change
//!   from the message it last had, made by [`TurnChange::catching_up`].
//!
//! A change from a stranger can name what is not there; applying it then
//! fails with [`Malformed`], and never panics.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::entity::{AiMode, EntityId, Place, PlayerId, SquareId};
use crate::events::Event;
use crate::known::{Activity, KnownEntity, KnownExit, KnownOrder, OwnPart};
use crate::message::{ControlledPlayer, KnownSquares, SeatState, TurnMessage};
use crate::player::Objective;
use crate::rules::{AirGround, EntityClass, TypeId};

/// Why what was received cannot be what the host sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Malformed(pub String);

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Malformed {}

fn malformed<T>(why: impl Into<String>) -> Result<T, Malformed> {
    Err(Malformed(why.into()))
}

/// What changed in a seat's message: see the [module](self).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TurnChange {
    /// The game time at the end of the turn.
    pub time: i64,
    /// The events to tell, in order: the turn's, and for a seat catching
    /// up those it missed that are not about an entity, first.
    pub events: Vec<Event>,
    pub state: StateChange,
    pub squares: SquaresChange,
    /// The perceived and remembered entities.
    pub known: EntitiesChange,
    /// The remembered entities whose only change is when they were last
    /// seen, by that time.
    pub seen_again: Vec<SeenAgain>,
    pub unperceived_units: EntitiesChange,
}

/// Remembered entities last seen at `time`, and otherwise unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeenAgain {
    pub time: i64,
    pub ids: Ids,
}

/// What changed in a list of entities by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntitiesChange {
    /// The entities that were not there, whole, by id.
    pub new: Vec<KnownEntity>,
    /// The entities no longer there.
    pub gone: Ids,
    /// The entities still there whose fields changed: those fields.
    pub changed: ById<EntityFields>,
}

/// Entity ids, each once, in increasing order. Sent as the first id and
/// then the gap to each next one, which is small in a list of nearby ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ids(Vec<EntityId>);

impl Ids {
    pub fn as_slice(&self) -> &[EntityId] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<EntityId> for Ids {
    /// The ids, sorted, each once.
    fn from_iter<I: IntoIterator<Item = EntityId>>(ids: I) -> Ids {
        Ids(ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

/// A value for each of some entities, each entity once, in increasing order
/// of id. Sent as [`Ids`] are, each gap followed by its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ById<T>(Vec<(EntityId, T)>);

impl<T> Default for ById<T> {
    fn default() -> Self {
        ById(Vec::new())
    }
}

impl<T> ById<T> {
    pub fn iter(&self) -> impl Iterator<Item = &(EntityId, T)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> FromIterator<(EntityId, T)> for ById<T> {
    /// The values, by id; of two values for one id, the last.
    fn from_iter<I: IntoIterator<Item = (EntityId, T)>>(values: I) -> ById<T> {
        ById(
            values
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        )
    }
}

/// A struct of fields that may each have changed, every one optional. It
/// is sent as one number whose bits say which fields are there, the first
/// field the lowest bit, followed by those fields in order: the fields
/// that change most often come first, so that their bits fit in the
/// number's first byte.
macro_rules! changed_fields {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty, )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field: Option<$ty>, )*
        }

        impl $name {
            /// The number of fields.
            const COUNT: usize = [$(stringify!($field)),*].len();

            /// Whether no field changed.
            pub fn is_empty(&self) -> bool {
                self.count() == 0
            }

            /// The number of fields that changed.
            fn count(&self) -> usize {
                [$( self.$field.is_some() ),*].iter().filter(|&&there| there).count()
            }

            #[cfg(feature = "serde")]
            fn mask(&self) -> u64 {
                [$( self.$field.is_some() ),*]
                    .iter()
                    .rev()
                    .fold(0, |m: u64, &there| m.wrapping_shl(1) | u64::from(there))
            }
        }

        // One bit a field, in a u64.
        const _: () = assert!($name::COUNT <= 64);

        #[cfg(feature = "serde")]
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeTuple;
                let mut t = s.serialize_tuple(self.count().saturating_add(1))?;
                t.serialize_element(&self.mask())?;
                $( if let Some(v) = &self.$field { t.serialize_element(v)?; } )*
                t.end()
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = $name;

                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        f.write_str(concat!("the changed fields of ", stringify!($name)))
                    }

                    fn visit_seq<A: serde::de::SeqAccess<'de>>(
                        self,
                        mut seq: A,
                    ) -> Result<$name, A::Error> {
                        use serde::de::Error;
                        let missing = || A::Error::custom(concat!(
                            "a field of ", stringify!($name), " is missing"
                        ));
                        let mut mask: u64 = seq.next_element()?.ok_or_else(missing)?;
                        let mut value = $name::default();
                        $(
                            if mask & 1 == 1 {
                                value.$field = Some(seq.next_element()?.ok_or_else(missing)?);
                            }
                            mask = mask.wrapping_shr(1);
                        )*
                        if mask != 0 {
                            return Err(A::Error::custom(concat!(
                                "a field ", stringify!($name), " does not have"
                            )));
                        }
                        Ok(value)
                    }
                }
                d.deserialize_tuple($name::COUNT.saturating_add(1), V)
            }
        }
    };
}

/// Sets `$to.$field` for each field given that is in `$from`.
macro_rules! set_fields {
    ($from:expr, $to:expr; $($field:ident),* $(,)?) => {
        $( if let Some(v) = $from.$field { $to.$field = v; } )*
    };
}

/// The fields of `$b` that differ from `$a`, into `$out`. Every field of
/// `$ty` is named, compared or skipped, so that one added to it later
/// fails to compile here rather than go unsent.
macro_rules! diff_fields {
    ($ty:ident { $($skip:ident),* } $a:expr, $b:expr, $out:expr; $($field:ident),* $(,)?) => {
        let $ty { $($skip: _,)* $($field: _,)* } = $b;
        $( if $a.$field != $b.$field { $out.$field = Some($b.$field.clone()); } )*
    };
}

changed_fields! {
    /// The fields of a [`KnownEntity`] that changed: each is the new value.
    /// Its own part is told field by field while the entity keeps one.
    pub struct EntityFields {
        pub x: i64,
        pub y: i64,
        pub o: i64,
        pub is_moving: bool,
        pub own_fields: OwnFields,
        pub hp: i64,
        pub place: Option<Place>,
        pub actual_speed: i64,
        pub activity: Option<Activity>,
        pub last_seen: Option<i64>,
        pub qty: i64,
        pub mana: i64,
        pub is_cloaked: bool,
        pub upgrades: Vec<String>,
        pub exit: Option<KnownExit>,
        pub blocked_exit: Option<EntityId>,
        /// The own part, when the entity gains or loses one.
        pub own: Option<OwnPart>,
        pub player: Option<PlayerId>,
        pub class: EntityClass,
        pub type_name: Arc<str>,
        pub hp_max: i64,
        pub mana_max: i64,
        pub site_type: Option<TypeId>,
        pub number: Option<i64>,
        pub airground: AirGround,
        pub is_mobile: bool,
        pub is_invisible: bool,
        pub is_a_detector: bool,
        pub is_a_cloaker: bool,
        pub is_a_building_land: bool,
        pub is_repairable: bool,
        pub is_healable: bool,
        pub is_a_gate: bool,
        pub is_a_threat: bool,
    }
}

changed_fields! {
    /// The fields of an [`OwnPart`] that changed: each is the new value.
    pub struct OwnFields {
        pub distance_to_goal: i64,
        /// The `time_left` of each order, when nothing else of the orders
        /// changed: production counts down every turn.
        pub times_left: Vec<Option<i64>>,
        pub orders: Vec<KnownOrder>,
        pub is_idle: bool,
        pub cargo: Option<(i64, i64)>,
        pub inside: Option<Vec<EntityId>>,
        pub has_basic_abilities: bool,
        pub auto_gather: Option<bool>,
        pub auto_repair: Option<bool>,
        pub stance: Option<AiMode>,

        pub group: Option<String>,
        pub transport_capacity: i64,
        pub transport_volume: i64,
        pub food_cost: i64,
    }
}

changed_fields! {
    /// The fields of the [`SeatState`] that changed: each is the new value.
    pub struct StateChange {
        pub resources: Vec<i64>,
        pub used_food: i64,
        pub available_food: i64,
        pub controlled: Vec<ControlledPlayer>,
        pub groups: Vec<(String, Vec<EntityId>)>,
        pub players: Vec<PlayerId>,
        pub defeated: Vec<PlayerId>,
        pub allies: Vec<Vec<PlayerId>>,
        pub objectives: Vec<Objective>,
        pub cheat_mode: bool,
        pub has_victory: bool,
    }
}

changed_fields! {
    /// What changed in the [`KnownSquares`]: for a set, the squares that
    /// came in or went out; for the terrain, each square's new terrain,
    /// `None` for a square no longer seen.
    pub struct SquaresChange {
        pub observed: BTreeSet<SquareId>,
        pub partly_observed: BTreeSet<SquareId>,
        pub observed_before: BTreeSet<SquareId>,
        pub terrain: Vec<(SquareId, Option<String>)>,
    }
}

impl EntityFields {
    /// What changed from `a` to `b`, the same entity.
    fn between(a: &KnownEntity, b: &KnownEntity) -> EntityFields {
        let mut f = EntityFields::default();
        diff_fields!(KnownEntity { id, own } a, b, f;
            x, y, o, is_moving, hp, place, actual_speed, activity, last_seen, qty, mana,
            is_cloaked, upgrades, exit, blocked_exit, player, class, type_name, hp_max,
            mana_max, site_type, number, airground, is_mobile, is_invisible, is_a_detector,
            is_a_cloaker, is_a_building_land, is_repairable, is_healable, is_a_gate,
            is_a_threat,
        );
        match (&a.own, &b.own) {
            (Some(a), Some(b)) => {
                let own = OwnFields::between(a, b);
                if !own.is_empty() {
                    f.own_fields = Some(own);
                }
            }
            (a, b) if a != b => f.own = Some(b.clone()),
            _ => {}
        }
        f
    }

    fn apply(self, e: &mut KnownEntity) -> Result<(), Malformed> {
        if self.own.is_some() && self.own_fields.is_some() {
            return malformed(format!("entity {} gets an own part and changes it", e.id));
        }
        if let Some(own) = self.own {
            e.own = own;
        }
        if let Some(fields) = self.own_fields {
            let Some(own) = e.own.as_mut() else {
                return malformed(format!("entity {} has no own part to change", e.id));
            };
            fields.apply(own)?;
        }
        set_fields!(self, e;
            x, y, o, is_moving, hp, place, actual_speed, activity, last_seen, qty, mana,
            is_cloaked, upgrades, exit, blocked_exit, player, class, type_name, hp_max,
            mana_max, site_type, number, airground, is_mobile, is_invisible, is_a_detector,
            is_a_cloaker, is_a_building_land, is_repairable, is_healable, is_a_gate,
            is_a_threat,
        );
        Ok(())
    }
}

impl OwnFields {
    fn between(a: &OwnPart, b: &OwnPart) -> OwnFields {
        let mut f = OwnFields::default();
        diff_fields!(OwnPart { orders } a, b, f;
            distance_to_goal, is_idle, cargo, inside, has_basic_abilities,
            auto_gather, auto_repair, stance, group, transport_capacity,
            transport_volume, food_cost,
        );
        if a.orders != b.orders {
            let only_times = a.orders.len() == b.orders.len()
                && a.orders.iter().zip(&b.orders).all(|(a, b)| {
                    *a == KnownOrder {
                        time_left: a.time_left,
                        ..b.clone()
                    }
                });
            if only_times {
                f.times_left = Some(b.orders.iter().map(|o| o.time_left).collect());
            } else {
                f.orders = Some(b.orders.clone());
            }
        }
        f
    }

    fn apply(self, o: &mut OwnPart) -> Result<(), Malformed> {
        if let Some(times) = self.times_left {
            if self.orders.is_some() || times.len() != o.orders.len() {
                return malformed("times left that are not one for each order");
            }
            for (order, time) in o.orders.iter_mut().zip(times) {
                order.time_left = time;
            }
        }
        set_fields!(self, o;
            distance_to_goal, orders, is_idle, cargo, inside, has_basic_abilities,
            auto_gather, auto_repair, stance, group, transport_capacity,
            transport_volume, food_cost,
        );
        Ok(())
    }
}

impl StateChange {
    fn between(a: &SeatState, b: &SeatState) -> StateChange {
        let mut f = StateChange::default();
        diff_fields!(SeatState {} a, b, f;
            resources, used_food, available_food, controlled, groups, players, defeated, allies,
            objectives, cheat_mode, has_victory,
        );
        f
    }

    fn apply(self, s: &mut SeatState) {
        set_fields!(self, s;
            resources, used_food, available_food, controlled, groups, players, defeated, allies,
            objectives, cheat_mode, has_victory,
        );
    }
}

/// The members of one set and not of the other.
fn toggled(a: &BTreeSet<SquareId>, b: &BTreeSet<SquareId>) -> Option<BTreeSet<SquareId>> {
    let t: BTreeSet<SquareId> = a.symmetric_difference(b).copied().collect();
    (!t.is_empty()).then_some(t)
}

fn toggle(set: &mut BTreeSet<SquareId>, squares: BTreeSet<SquareId>) {
    for s in squares {
        if !set.remove(&s) {
            set.insert(s);
        }
    }
}

impl SquaresChange {
    fn between(a: &KnownSquares, b: &KnownSquares) -> SquaresChange {
        let KnownSquares {
            observed: _,
            observed_before: _,
            partly_observed: _,
            terrain: _,
        } = b;
        let mut terrain: Vec<(SquareId, Option<String>)> = Vec::new();
        for (s, t) in &b.terrain {
            if a.terrain.get(s) != Some(t) {
                terrain.push((*s, Some(t.clone())));
            }
        }
        for s in a.terrain.keys() {
            if !b.terrain.contains_key(s) {
                terrain.push((*s, None));
            }
        }
        terrain.sort_by_key(|(s, _)| *s);
        SquaresChange {
            observed: toggled(&a.observed, &b.observed),
            partly_observed: toggled(&a.partly_observed, &b.partly_observed),
            observed_before: toggled(&a.observed_before, &b.observed_before),
            terrain: (!terrain.is_empty()).then_some(terrain),
        }
    }

    fn apply(self, squares: &mut KnownSquares) -> Result<(), Malformed> {
        if let Some(t) = self.observed {
            toggle(&mut squares.observed, t);
        }
        if let Some(t) = self.partly_observed {
            toggle(&mut squares.partly_observed, t);
        }
        if let Some(t) = self.observed_before {
            toggle(&mut squares.observed_before, t);
        }
        let mut last = None;
        for (s, t) in self.terrain.unwrap_or_default() {
            if last.is_some_and(|l| l >= s) {
                return malformed("terrain changes out of order");
            }
            last = Some(s);
            match t {
                Some(t) => {
                    squares.terrain.insert(s, t);
                }
                None => {
                    if squares.terrain.remove(&s).is_none() {
                        return malformed(format!("no terrain to forget on square {}", s.0));
                    }
                }
            }
        }
        Ok(())
    }
}

impl EntitiesChange {
    fn between(a: &BTreeMap<EntityId, &KnownEntity>, b: &BTreeMap<EntityId, &KnownEntity>) -> Self {
        let mut change = EntitiesChange::default();
        let mut changed = Vec::new();
        for (id, e) in b {
            match a.get(id) {
                None => change.new.push((*e).clone()),
                Some(old) => {
                    let f = EntityFields::between(old, e);
                    if !f.is_empty() {
                        changed.push((*id, f));
                    }
                }
            }
        }
        change.gone = a.keys().filter(|id| !b.contains_key(id)).copied().collect();
        change.changed = ById(changed);
        change
    }

    /// Applies the change to the entities by id.
    fn apply(self, entities: &mut BTreeMap<EntityId, KnownEntity>) -> Result<(), Malformed> {
        for id in self.gone.0 {
            if entities.remove(&id).is_none() {
                return malformed(format!("entity {id} is gone but was not there"));
            }
        }
        for (id, fields) in self.changed.0 {
            if fields.is_empty() {
                return malformed(format!("entity {id} changed nothing"));
            }
            let Some(e) = entities.get_mut(&id) else {
                return malformed(format!("entity {id} changed but is not there"));
            };
            fields.apply(e)?;
        }
        for e in self.new {
            if entities.insert(e.id, e).is_some() {
                return malformed("an entity is new but was there");
            }
        }
        Ok(())
    }
}

fn by_id<'a>(lists: &[&'a [KnownEntity]]) -> BTreeMap<EntityId, &'a KnownEntity> {
    lists
        .iter()
        .flat_map(|l| l.iter())
        .map(|e| (e.id, e))
        .collect()
}

impl TurnChange {
    /// What changed from `last`, the message a seat last had, to `now`, its
    /// message for the turn just played. The events are `now`'s.
    ///
    /// Both must be for the same seat, and keep the promises of a
    /// [`TurnMessage`]: lists by id, and an entity perceived or remembered,
    /// not both. Messages made by the world keep them.
    pub fn between(last: &TurnMessage, now: &TurnMessage) -> TurnChange {
        let TurnMessage {
            seat: _,
            time: _,
            events: _,
            state: _,
            squares: _,
            perceived: _,
            remembered: _,
            unperceived_units: _,
        } = now;
        let old = by_id(&[&last.perceived, &last.remembered]);
        let new = by_id(&[&now.perceived, &now.remembered]);
        let mut known = EntitiesChange::between(&old, &new);
        // Of the remembered entities, those whose only change is
        // `last_seen`.
        let mut seen: BTreeMap<i64, Vec<EntityId>> = BTreeMap::new();
        known.changed.0.retain(|(id, f)| {
            let remembered = old.get(id).is_some_and(|e| e.is_memory());
            match f.last_seen {
                Some(Some(time)) if remembered && f.count() == 1 => {
                    seen.entry(time).or_default().push(*id);
                    false
                }
                _ => true,
            }
        });
        TurnChange {
            time: now.time,
            events: now.events.clone(),
            state: StateChange::between(&last.state, &now.state),
            squares: SquaresChange::between(&last.squares, &now.squares),
            known,
            seen_again: seen
                .into_iter()
                .map(|(time, ids)| SeenAgain {
                    time,
                    ids: Ids(ids),
                })
                .collect(),
            unperceived_units: EntitiesChange::between(
                &by_id(&[&last.unperceived_units]),
                &by_id(&[&now.unperceived_units]),
            ),
        }
    }

    /// The change for a seat that fell behind: from `last`, the message it
    /// last had, to `now`. Of the events of the turns between, `missed`,
    /// only those not about an entity are told, before `now`'s: the
    /// interface would discard an entity's event that old.
    pub fn catching_up<'a>(
        last: &TurnMessage,
        missed: impl IntoIterator<Item = &'a Event>,
        now: &TurnMessage,
    ) -> TurnChange {
        let mut change = TurnChange::between(last, now);
        change.events = missed
            .into_iter()
            .filter(|e| !matches!(e, Event::Entity { .. }))
            .cloned()
            .chain(now.events.iter().cloned())
            .collect();
        change
    }

    /// The message the change makes of `last`, the message the seat last
    /// had. Fails if the change names what `last` does not have.
    pub fn apply(self, last: &TurnMessage) -> Result<TurnMessage, Malformed> {
        let mut known: BTreeMap<EntityId, KnownEntity> = BTreeMap::new();
        for e in last.perceived.iter().chain(&last.remembered) {
            if known.insert(e.id, e.clone()).is_some() {
                return malformed(format!("entity {} is both perceived and remembered", e.id));
            }
        }
        self.known.apply(&mut known)?;
        for SeenAgain { time, ids } in self.seen_again {
            for id in ids.0 {
                match known.get_mut(&id) {
                    Some(e) if e.last_seen.is_some() => e.last_seen = Some(time),
                    _ => return malformed(format!("entity {id} seen again is not remembered")),
                }
            }
        }
        let mut unperceived: BTreeMap<EntityId, KnownEntity> = last
            .unperceived_units
            .iter()
            .map(|e| (e.id, e.clone()))
            .collect();
        self.unperceived_units.apply(&mut unperceived)?;
        let mut state = last.state.clone();
        self.state.apply(&mut state);
        let mut squares = last.squares.clone();
        self.squares.apply(&mut squares)?;
        let (remembered, perceived): (Vec<KnownEntity>, Vec<KnownEntity>) =
            known.into_values().partition(KnownEntity::is_memory);
        Ok(TurnMessage {
            seat: last.seat,
            time: self.time,
            events: self.events,
            state,
            squares,
            perceived,
            remembered,
            unperceived_units: unperceived.into_values().collect(),
        })
    }
}

/// [`Ids`] and [`ById`] as gaps: the first id, then each id less the one
/// before. A gap of 0 after the first would repeat an id, and is refused.
#[cfg(feature = "serde")]
mod gaps {
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::*;

    /// The gap of each id from the one before, the first from 0.
    fn gaps(ids: impl Iterator<Item = EntityId>) -> impl Iterator<Item = u32> {
        let mut last: Option<u32> = None;
        ids.map(move |id| {
            let gap = id.0.wrapping_sub(last.unwrap_or(0));
            last = Some(id.0);
            gap
        })
    }

    /// The id after `last` at `gap`, if it is one.
    fn next<E: Error>(last: Option<EntityId>, gap: u32) -> Result<EntityId, E> {
        let id = match last {
            None => Some(gap),
            Some(_) if gap == 0 => None,
            Some(l) => l.0.checked_add(gap),
        };
        id.map(EntityId)
            .ok_or_else(|| E::custom("ids out of order or beyond the largest"))
    }

    impl Serialize for Ids {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(gaps(self.0.iter().copied()))
        }
    }

    impl<'de> Deserialize<'de> for Ids {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Ids, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Ids;

                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a list of ids")
                }

                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Ids, A::Error> {
                    let mut ids: Vec<EntityId> = Vec::new();
                    while let Some(gap) = seq.next_element()? {
                        ids.push(next(ids.last().copied(), gap)?);
                    }
                    Ok(Ids(ids))
                }
            }
            d.deserialize_seq(V)
        }
    }

    impl<T: Serialize> Serialize for ById<T> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut seq = s.serialize_seq(Some(self.0.len()))?;
            for (gap, (_, value)) in gaps(self.0.iter().map(|(id, _)| *id)).zip(&self.0) {
                seq.serialize_element(&(gap, value))?;
            }
            seq.end()
        }
    }

    impl<'de, T: Deserialize<'de>> Deserialize<'de> for ById<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<ById<T>, D::Error> {
            struct V<T>(std::marker::PhantomData<T>);
            impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
                type Value = ById<T>;

                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a list of values by id")
                }

                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ById<T>, A::Error> {
                    let mut values: Vec<(EntityId, T)> = Vec::new();
                    while let Some((gap, value)) = seq.next_element::<(u32, T)>()? {
                        values.push((next(values.last().map(|(id, _)| *id), gap)?, value));
                    }
                    Ok(ById(values))
                }
            }
            d.deserialize_seq(V(std::marker::PhantomData))
        }
    }
}

// The tests may panic: that is how they fail.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests;
