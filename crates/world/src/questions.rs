//! The rules behind the questions an interface asks about orders and what
//! its player sees, written once over facts.
//!
//! The simulation asks them of its world, which knows everything. An
//! interface asks them of what its player was sent
//! ([`SeatView`](crate::SeatView)): one turn's message, the data sent once
//! at the start, and the rules. Both read their inputs through the same
//! traits ([`Knowledge`], [`EntityFacts`], [`UnitFacts`]), so there is one
//! order menu, one cost, one requirements check, one default order, one
//! blocked exit and one count of comrades, and the two cannot drift.

use std::collections::BTreeSet;

use crate::entity::{AiMode, Entity, EntityId, Place, PlayerId, Target};
use crate::known::{Activity, KnownEntity, KnownOrder, OwnPart};
use crate::orders::{
    BuildStep, GatherStep, Keyword, MenuEntry, Order, OrderInfo, OrderKind, TargetRef,
};
use crate::player::Upgrades;
use crate::rules::{EntityClass, MAX_NB_OF_RESOURCE_TYPES, Rules, TypeId, UnitType};
use crate::seat_view::Comrades;
use crate::world::{Abort, Res, World};

/// The effects of an ability that its `use` order may always be given for.
const EFFECTS: &[&str] = &[
    "teleportation",
    "recall",
    "conversion",
    "summon",
    "raise_dead",
    "resurrection",
];

/// An order as the rules read it.
pub(crate) trait OrderFacts {
    fn keyword(&self) -> Keyword;
    /// The type of a complex order.
    fn type_id(&self) -> Option<TypeId>;
    fn is_imperative(&self) -> bool;
}

/// An entity as the rules read it: the world's, or one as a player knows
/// it, perceived or remembered.
pub(crate) trait EntityFacts {
    fn id(&self) -> EntityId;
    fn class(&self) -> EntityClass;
    fn owner(&self) -> Option<PlayerId>;
    fn place(&self) -> Option<Place>;
    fn type_name(&self) -> &str;
    /// Its type in the rules. A building site has none.
    fn type_id(&self, rules: &Rules) -> Option<TypeId>;
    fn unit_type<'r>(&self, rules: &'r Rules) -> Option<&'r UnitType> {
        self.type_id(rules).map(|t| rules.ty(t))
    }
    /// For a building site, the type of the building it becomes.
    fn site_type(&self) -> Option<TypeId>;
    fn hp(&self) -> i64;
    fn hp_max(&self) -> i64;
    fn is_repairable(&self) -> bool;
    fn is_a_gate(&self) -> bool;
    /// Whether it counts as a soldier when a fight is lost or won.
    fn is_a_threat(&self) -> bool;
    /// For an exit: whether dense forests close it, and its blockers.
    fn exit(&self) -> Option<(bool, &[EntityId])>;
    /// The room it takes in a transport, if known: only the player who
    /// controls an entity is told it.
    fn transport_volume(&self) -> Option<i64>;
}

/// A unit of a player whose units the asker controls, as the rules of
/// orders read it.
pub(crate) trait UnitFacts: EntityFacts {
    type Order: OrderFacts;
    fn orders(&self) -> &[Self::Order];
    fn is_idle(&self) -> bool;
    fn auto_gather(&self) -> Option<bool>;
    fn auto_repair(&self) -> Option<bool>;
    fn stance(&self) -> Option<AiMode>;
    fn inside(&self) -> Option<&[EntityId]>;
    fn transport_capacity(&self) -> i64;
    fn food_cost(&self) -> i64;
}

/// What the rules read of a player whose units the asker controls.
#[derive(Clone, Copy)]
pub(crate) struct PlayerFacts<'a> {
    pub upgrades: &'a Upgrades,
    pub forbidden_techs: &'a BTreeSet<TypeId>,
    /// The player has everything any requirement asks for.
    pub has_everything: bool,
    pub is_human: bool,
    pub units: &'a [EntityId],
    /// The player's allies, the player among them.
    pub allied: &'a [PlayerId],
}

/// A target as resolved for the rules: what it means, and the entity it
/// shows, if any.
pub(crate) struct Resolved<T, E> {
    pub target: T,
    pub entity: Option<E>,
}

/// Where the rules find their facts: the world, or what one seat was sent.
/// Implemented on a reference, so that the facts it gives borrow from it.
pub(crate) trait Knowledge<'a>: Copy {
    /// An entity as the asker knows it.
    type Entity: EntityFacts + Copy;
    /// A unit the asker controls.
    type Unit: UnitFacts<Order = Self::Order> + Copy;
    type Order: OrderFacts + 'a;
    /// What an order aims at, once resolved.
    type Target;
    fn rules(self) -> &'a Rules;
    /// A unit of a player whose units the asker controls, as it is now.
    fn unit(self, id: EntityId) -> Option<Self::Unit>;
    /// A player whose units the asker controls.
    fn player_facts(self, p: PlayerId) -> Option<PlayerFacts<'a>>;
    /// What a target means to `owner`, and the entity it shows. The error
    /// is a point outside its square.
    fn resolve(
        self,
        owner: PlayerId,
        target: &TargetRef,
    ) -> Res<Option<Resolved<Self::Target, Self::Entity>>>;
    /// Whether an order aims at a resolved target.
    fn aims_at(self, order: &Self::Order, target: &Self::Target) -> bool;
}

// the facts of the world

impl OrderFacts for Order {
    fn keyword(&self) -> Keyword {
        self.keyword()
    }

    fn type_id(&self) -> Option<TypeId> {
        self.kind.type_id()
    }

    fn is_imperative(&self) -> bool {
        self.is_imperative
    }
}

impl EntityFacts for &Entity {
    fn id(&self) -> EntityId {
        self.id
    }

    fn class(&self) -> EntityClass {
        self.class
    }

    fn owner(&self) -> Option<PlayerId> {
        self.player
    }

    fn place(&self) -> Option<Place> {
        self.place
    }

    fn type_name(&self) -> &str {
        &self.type_name
    }

    fn type_id(&self, _: &Rules) -> Option<TypeId> {
        self.type_id
    }

    fn site_type(&self) -> Option<TypeId> {
        self.site.as_ref().map(|s| s.building_type)
    }

    fn hp(&self) -> i64 {
        self.hp
    }

    fn hp_max(&self) -> i64 {
        self.stats.hp_max
    }

    fn is_repairable(&self) -> bool {
        self.stats.is_repairable != 0
    }

    fn is_a_gate(&self) -> bool {
        self.stats.is_a_gate != 0
    }

    fn is_a_threat(&self) -> bool {
        self.menace() != 0
    }

    fn exit(&self) -> Option<(bool, &[EntityId])> {
        let x = self.exit.as_ref()?;
        Some((x.is_blocked_by_forests, &x.blockers))
    }

    fn transport_volume(&self) -> Option<i64> {
        Some(self.stats.transport_volume)
    }
}

impl UnitFacts for &Entity {
    type Order = Order;

    fn orders(&self) -> &[Order] {
        &self.orders
    }

    fn is_idle(&self) -> bool {
        Entity::is_idle(self)
    }

    fn auto_gather(&self) -> Option<bool> {
        self.auto_gather
    }

    fn auto_repair(&self) -> Option<bool> {
        self.auto_repair
    }

    fn stance(&self) -> Option<AiMode> {
        self.ai_mode
    }

    fn inside(&self) -> Option<&[EntityId]> {
        self.inside.as_deref()
    }

    fn transport_capacity(&self) -> i64 {
        self.stats.transport_capacity
    }

    fn food_cost(&self) -> i64 {
        self.stats.food_cost
    }
}

impl World {
    /// What the rules read of a player.
    pub(crate) fn facts(&self, p: PlayerId) -> PlayerFacts<'_> {
        let pl = self.player(p);
        PlayerFacts {
            upgrades: &pl.upgrades,
            forbidden_techs: &pl.forbidden_techs,
            has_everything: pl.cheat_has_all,
            is_human: pl.is_human(),
            units: &pl.units,
            allied: &pl.allied,
        }
    }
}

/// The world knows everything. A target is resolved through what the
/// unit's owner perceives or remembers.
impl<'a> Knowledge<'a> for &'a World {
    type Entity = &'a Entity;
    type Unit = &'a Entity;
    type Order = Order;
    type Target = Target;

    fn rules(self) -> &'a Rules {
        &self.rules
    }

    fn unit(self, id: EntityId) -> Option<&'a Entity> {
        self.entity(id)
    }

    fn player_facts(self, p: PlayerId) -> Option<PlayerFacts<'a>> {
        Some(self.facts(p))
    }

    fn resolve(
        self,
        owner: PlayerId,
        target: &TargetRef,
    ) -> Res<Option<Resolved<Target, &'a Entity>>> {
        let Some(t) = self.resolve_target(owner, target)? else {
            return Ok(None);
        };
        // The same entity as `World::t_entity`, borrowed from the world
        // rather than from the target: a remembered one is the owner's copy.
        let e = match &t {
            Target::Entity(id) => self.entity(*id),
            Target::Memory(m) => self
                .player(m.owner)
                .memory
                .get(&m.snapshot.id)
                .map(|r| &*r.snapshot),
            _ => None,
        };
        Ok(Some(Resolved {
            target: t,
            entity: e,
        }))
    }

    fn aims_at(self, order: &Order, target: &Target) -> bool {
        order.target().is_some_and(|t| t.same(target))
    }
}

// the facts a seat was sent

impl OrderFacts for KnownOrder {
    fn keyword(&self) -> Keyword {
        self.keyword
    }

    fn type_id(&self) -> Option<TypeId> {
        self.type_id
    }

    fn is_imperative(&self) -> bool {
        self.is_imperative
    }
}

impl EntityFacts for &KnownEntity {
    fn id(&self) -> EntityId {
        self.id
    }

    fn class(&self) -> EntityClass {
        self.class
    }

    fn owner(&self) -> Option<PlayerId> {
        self.player
    }

    fn place(&self) -> Option<Place> {
        self.place
    }

    fn type_name(&self) -> &str {
        &self.type_name
    }

    fn type_id(&self, rules: &Rules) -> Option<TypeId> {
        if self.is_building_site() {
            return None;
        }
        rules.type_id(&self.type_name)
    }

    fn site_type(&self) -> Option<TypeId> {
        self.site_type
    }

    fn hp(&self) -> i64 {
        self.hp
    }

    fn hp_max(&self) -> i64 {
        self.hp_max
    }

    fn is_repairable(&self) -> bool {
        self.is_repairable
    }

    fn is_a_gate(&self) -> bool {
        self.is_a_gate
    }

    fn is_a_threat(&self) -> bool {
        self.is_a_threat
    }

    fn exit(&self) -> Option<(bool, &[EntityId])> {
        let x = self.exit.as_ref()?;
        Some((x.is_blocked_by_forests, &x.blockers))
    }

    fn transport_volume(&self) -> Option<i64> {
        self.own.as_ref().map(|o| o.transport_volume)
    }
}

/// A unit the seat controls, with what only its controller is told.
#[derive(Clone, Copy)]
pub(crate) struct Controlled<'a> {
    pub known: &'a KnownEntity,
    pub own: &'a OwnPart,
}

impl<'a> Controlled<'a> {
    pub(crate) fn of(known: &'a KnownEntity) -> Option<Controlled<'a>> {
        Some(Controlled {
            known,
            own: known.own.as_ref()?,
        })
    }
}

impl EntityFacts for Controlled<'_> {
    fn id(&self) -> EntityId {
        self.known.id
    }

    fn class(&self) -> EntityClass {
        self.known.class
    }

    fn owner(&self) -> Option<PlayerId> {
        self.known.player
    }

    fn place(&self) -> Option<Place> {
        self.known.place
    }

    fn type_name(&self) -> &str {
        &self.known.type_name
    }

    fn type_id(&self, rules: &Rules) -> Option<TypeId> {
        self.known.type_id(rules)
    }

    fn site_type(&self) -> Option<TypeId> {
        self.known.site_type
    }

    fn hp(&self) -> i64 {
        self.known.hp
    }

    fn hp_max(&self) -> i64 {
        self.known.hp_max
    }

    fn is_repairable(&self) -> bool {
        self.known.is_repairable
    }

    fn is_a_gate(&self) -> bool {
        self.known.is_a_gate
    }

    fn is_a_threat(&self) -> bool {
        self.known.is_a_threat
    }

    fn exit(&self) -> Option<(bool, &[EntityId])> {
        let x = self.known.exit.as_ref()?;
        Some((x.is_blocked_by_forests, &x.blockers))
    }

    fn transport_volume(&self) -> Option<i64> {
        Some(self.own.transport_volume)
    }
}

impl UnitFacts for Controlled<'_> {
    type Order = KnownOrder;

    fn orders(&self) -> &[KnownOrder] {
        &self.own.orders
    }

    fn is_idle(&self) -> bool {
        self.own.is_idle
    }

    fn auto_gather(&self) -> Option<bool> {
        self.own.auto_gather
    }

    fn auto_repair(&self) -> Option<bool> {
        self.own.auto_repair
    }

    fn stance(&self) -> Option<AiMode> {
        self.own.stance
    }

    fn inside(&self) -> Option<&[EntityId]> {
        self.own.inside.as_deref()
    }

    fn transport_capacity(&self) -> i64 {
        self.own.transport_capacity
    }

    fn food_cost(&self) -> i64 {
        self.own.food_cost
    }
}

// abilities

/// The orders a unit of this class can always receive, by keyword, unless
/// it is being upgraded.
pub(crate) fn basic_abilities(class: EntityClass, upgrading: bool) -> &'static [&'static str] {
    match class {
        EntityClass::Worker if !upgrading => {
            &["go", "attack", "gather", "repair", "block", "join_group"]
        }
        EntityClass::Soldier if !upgrading => &["go", "attack", "patrol", "block", "join_group"],
        EntityClass::BuildingSite => &["cancel_building"],
        _ => &[],
    }
}

fn unit_basic_abilities(u: &impl UnitFacts) -> &'static [&'static str] {
    let upgrading = u.orders().iter().any(|o| o.keyword() == Keyword::UpgradeTo);
    basic_abilities(u.class(), upgrading)
}

/// The type list of a unit type called `attr`: `can_train`...
pub(crate) fn type_list<'t>(t: Option<&'t UnitType>, attr: &str) -> &'t [String] {
    let Some(t) = t else { return &[] };
    match attr {
        "can_build" => &t.can_build,
        "can_train" => &t.can_train,
        "can_use" => &t.can_use,
        "can_research" => &t.can_research,
        "can_upgrade_to" => &t.can_upgrade_to,
        _ => &[],
    }
}

// requirements

/// `has`: an upgrade, or a unit of the type or derived from it. A
/// requirement is a class name of the rules, which may name no type.
pub(crate) fn has<'a>(k: impl Knowledge<'a>, p: PlayerFacts<'a>, type_name: &str) -> bool {
    if p.has_everything
        || k.rules()
            .type_id(type_name)
            .is_some_and(|t| p.upgrades.has(t))
    {
        return true;
    }
    p.units.iter().filter_map(|&u| k.unit(u)).any(|e| {
        e.type_name() == type_name
            || e.unit_type(k.rules())
                .is_some_and(|t| t.expanded_is_a.iter().any(|a| a == type_name))
    })
}

pub(crate) fn has_all<'a>(
    k: impl Knowledge<'a>,
    p: PlayerFacts<'a>,
    type_names: &[String],
) -> bool {
    type_names.iter().all(|t| has(k, p, t))
}

/// `future_count`: units of a type, building sites for it, and units
/// being trained or upgraded to it.
pub(crate) fn future_count<'a>(k: impl Knowledge<'a>, p: PlayerFacts<'a>, t: TypeId) -> i64 {
    let rules = k.rules();
    let mut result = 0;
    for e in p.units.iter().filter_map(|&u| k.unit(u)) {
        if e.type_id(rules) == Some(t) || e.site_type() == Some(t) {
            result += 1;
        }
        for o in e.orders() {
            if matches!(o.keyword(), Keyword::Train | Keyword::UpgradeTo) && o.type_id() == Some(t)
            {
                result += 1;
            }
        }
    }
    result
}

/// `check_count_limit`: whether the player may have one more of the type.
pub(crate) fn check_count_limit<'a>(k: impl Knowledge<'a>, p: PlayerFacts<'a>, t: TypeId) -> bool {
    let limit = k.rules().ty(t).stats.count_limit;
    limit == 0 || future_count(k, p, t) < limit
}

// permissions and menus

fn additional_condition<'a, K: Knowledge<'a>>(
    k: K,
    unit: K::Unit,
    p: PlayerFacts<'a>,
    kw: Keyword,
    t: TypeId,
) -> bool {
    match kw {
        Keyword::Research => {
            if p.upgrades.has(t) {
                return false;
            }
            let researching = p.units.iter().filter_map(|&u| k.unit(u)).any(|e| {
                e.orders()
                    .iter()
                    .any(|o| o.keyword() == Keyword::Research && o.type_id() == Some(t))
            });
            !researching
        }
        Keyword::UpgradeTo => unit.orders().is_empty(),
        Keyword::Use => k
            .rules()
            .ty(t)
            .effect
            .as_ref()
            .and_then(|e| e.first())
            .is_some_and(|e| EFFECTS.contains(&e.as_str())),
        _ => true,
    }
}

fn is_almost_allowed<'a, K: Knowledge<'a>>(k: K, unit: K::Unit, kw: Keyword, t: TypeId) -> bool {
    let Some(attr) = kw.menu_attribute() else {
        return false;
    };
    let rules = k.rules();
    if !type_list(unit.unit_type(rules), attr)
        .iter()
        .any(|n| *n == rules.ty(t).name)
    {
        return false;
    }
    let Some(p) = unit.owner().and_then(|p| k.player_facts(p)) else {
        return false;
    };
    !p.forbidden_techs.contains(&t)
        && unit
            .orders()
            .last()
            .is_none_or(|o| o.keyword().can_be_followed())
        && additional_condition(k, unit, p, kw, t)
        && check_count_limit(k, p, t)
}

fn complex_is_allowed<'a, K: Knowledge<'a>>(k: K, unit: K::Unit, kw: Keyword, t: TypeId) -> bool {
    if !is_almost_allowed(k, unit, kw, t) {
        return false;
    }
    let Some(p) = unit.owner().and_then(|p| k.player_facts(p)) else {
        return false;
    };
    has_all(k, p, &k.rules().ty(t).requirements)
}

/// Whether a unit may receive an order now (`cls.is_allowed(unit,
/// *args)`), with its type for `train`, `build`... A complex order
/// without its type raises in the original.
pub(crate) fn is_order_allowed<'a, K: Knowledge<'a>>(
    k: K,
    unit: K::Unit,
    kw: Keyword,
    t: Option<TypeId>,
) -> Res<bool> {
    if kw.is_complex() {
        let t = t.ok_or_else(|| {
            Abort::new("is_allowed() missing 1 required positional argument: 'type_name'")
        })?;
        return Ok(complex_is_allowed(k, unit, kw, t));
    }
    Ok(order_is_allowed(k, unit, kw))
}

/// Whether a unit may receive an order without a type.
pub(crate) fn order_is_allowed<'a, K: Knowledge<'a>>(k: K, unit: K::Unit, kw: Keyword) -> bool {
    let switchable = matches!(unit.class(), EntityClass::Worker | EntityClass::Soldier);
    match kw {
        Keyword::Stop => !unit.is_idle() || !unit.orders().is_empty(),
        Keyword::CancelTraining | Keyword::CancelUpgrading => unit
            .orders()
            .last()
            .is_some_and(|o| o.keyword().cancel_order() == kw.as_str()),
        Keyword::EnableAutoGather => unit.auto_gather() == Some(false),
        Keyword::DisableAutoGather => unit.auto_gather() == Some(true),
        Keyword::EnableAutoRepair => unit.auto_repair() == Some(false),
        Keyword::DisableAutoRepair => unit.auto_repair() == Some(true),
        Keyword::ModeOffensive => switchable && unit.stance() == Some(AiMode::Defensive),
        Keyword::ModeDefensive => switchable && unit.stance() == Some(AiMode::Offensive),
        Keyword::RallyingPoint => !order_menu(k, unit, Keyword::Train, false).is_empty(),
        Keyword::AutoAttack | Keyword::AutoExplore | Keyword::Wait => unit
            .owner()
            .and_then(|p| k.player_facts(p))
            .is_some_and(|p| !p.is_human),
        Keyword::Load | Keyword::LoadAll | Keyword::UnloadAll => unit.transport_capacity() > 0,
        Keyword::Enter => true,
        k if k.is_complex() => false,
        k => unit_basic_abilities(&unit).contains(&k.as_str()),
    }
}

/// The orders of one kind a unit may receive, without their target: `go`,
/// or `train footman`, `train archer`... With `strict`, the requirements
/// must be met too. A type list's name of no type is never allowed.
pub(crate) fn order_menu<'a, K: Knowledge<'a>>(
    k: K,
    unit: K::Unit,
    kw: Keyword,
    strict: bool,
) -> Vec<MenuEntry> {
    let Some(attr) = kw.menu_attribute() else {
        return MenuEntry::plain(kw)
            .filter(|_| order_is_allowed(k, unit, kw))
            .into_iter()
            .collect();
    };
    let rules = k.rules();
    type_list(unit.unit_type(rules), attr)
        .iter()
        .filter_map(|n| rules.type_id(n))
        .filter(|&t| {
            if strict {
                complex_is_allowed(k, unit, kw, t)
            } else {
                is_almost_allowed(k, unit, kw, t)
            }
        })
        .filter_map(|t| MenuEntry::typed(rules, kw, t))
        .collect()
}

// costs

/// The type of a complex order: `footman` in `train footman`.
pub(crate) fn complex_type(rules: &Rules, t: Option<TypeId>) -> Res<&UnitType> {
    t.map(|t| rules.ty(t))
        .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'cost'"))
}

/// The cost of a unit's type: zeros for building sites and others.
pub(crate) fn entity_cost(rules: &Rules, unit: &impl EntityFacts) -> Vec<i64> {
    match unit.unit_type(rules) {
        Some(t) if !t.cost.is_empty() => t.cost.clone(),
        _ => vec![0; MAX_NB_OF_RESOURCE_TYPES],
    }
}

/// What an order of a unit costs in each resource. Upgrading to a type
/// costs the difference with the unit's own type.
pub(crate) fn order_cost(
    rules: &Rules,
    unit: &impl UnitFacts,
    kw: Keyword,
    type_id: Option<TypeId>,
) -> Res<Vec<i64>> {
    if !kw.is_complex() {
        return Ok(priced_cost(rules, unit, None));
    }
    let t = complex_type(rules, type_id)?;
    Ok(priced_cost(
        rules,
        unit,
        Some((t, kw == Keyword::UpgradeTo)),
    ))
}

/// What an order costs in each resource, from the type it pays for and
/// whether it pays only the difference with the unit's own type; nothing
/// for an order without a type.
pub(crate) fn priced_cost(
    rules: &Rules,
    unit: &impl UnitFacts,
    priced: Option<(&UnitType, bool)>,
) -> Vec<i64> {
    match priced {
        None => vec![0; MAX_NB_OF_RESOURCE_TYPES],
        Some((t, true)) => {
            let uc = entity_cost(rules, unit);
            t.cost
                .iter()
                .enumerate()
                .map(|(i, &c)| c - uc.get(i).copied().unwrap_or(0))
                .collect()
        }
        Some((t, false)) => t.cost.clone(),
    }
}

/// The food an order of a unit uses.
pub(crate) fn order_food_cost(
    rules: &Rules,
    unit: &impl UnitFacts,
    kw: Keyword,
    type_id: Option<TypeId>,
) -> Res<i64> {
    if !kw.is_complex() {
        return Ok(priced_food_cost(unit, None));
    }
    let t = complex_type(rules, type_id)?;
    Ok(priced_food_cost(unit, Some((t, kw == Keyword::UpgradeTo))))
}

/// The food an order uses, as [`priced_cost`] reads its type.
pub(crate) fn priced_food_cost(unit: &impl UnitFacts, priced: Option<(&UnitType, bool)>) -> i64 {
    match priced {
        None => 0,
        Some((t, true)) => t.stats.food_cost - unit.food_cost(),
        Some((t, false)) => t.stats.food_cost,
    }
}

/// The costs and the number of targets of an order a unit could receive
/// (`keyword` or `keyword type`), as the menu shows them. `None` if the
/// order makes no sense.
pub(crate) fn order_info(
    rules: &Rules,
    unit: &impl UnitFacts,
    kw: Keyword,
    type_id: Option<TypeId>,
) -> Option<OrderInfo> {
    let nb_args = if kw == Keyword::Use {
        let t = complex_type(rules, type_id).ok()?;
        usize::from(t.effect_target == ["ask"])
    } else {
        kw.nb_args()
    };
    Some(OrderInfo {
        cost: order_cost(rules, unit, kw, type_id).ok()?,
        food_cost: order_food_cost(rules, unit, kw, type_id).ok()?,
        nb_args,
    })
}

// the default order

/// `have_enough_space`: whether the transport can take `target`.
pub(crate) fn have_enough_space<'a, K: Knowledge<'a>>(
    k: K,
    transport: K::Unit,
    target: &impl EntityFacts,
) -> bool {
    let Some(inside) = transport.inside() else {
        return false;
    };
    let mut capacity = transport.transport_capacity();
    for &o in inside {
        let Some(volume) = k.unit(o).and_then(|o| o.transport_volume()) else {
            return false;
        };
        capacity -= volume;
    }
    target.transport_volume().is_some_and(|v| capacity >= v)
}

/// `Creature.is_an_enemy`: a creature of an enemy player, or the target
/// of the unit's imperative `go` order.
pub(crate) fn is_an_enemy<'a, K: Knowledge<'a>>(
    k: K,
    unit: K::Unit,
    target: &K::Target,
    entity: Option<K::Entity>,
) -> bool {
    let Some(ce) = entity else {
        return false;
    };
    if !ce.class().is_creature() {
        return false;
    }
    let first = unit.orders().first();
    if first
        .is_some_and(|o| o.is_imperative() && o.keyword() == Keyword::Go && k.aims_at(o, target))
    {
        return true;
    }
    match unit.owner().and_then(|p| k.player_facts(p)) {
        Some(p) => !ce.owner().is_some_and(|q| p.allied.contains(&q)),
        None => false,
    }
}

/// The keyword of the order a unit would choose for a target if it
/// received the default order now: the order a click on the target means.
/// The target is resolved through what the unit's owner knows.
pub(crate) fn default_keyword<'a, K: Knowledge<'a>>(
    k: K,
    unit: K::Unit,
    target: &TargetRef,
) -> Res<Option<Keyword>> {
    let Some(p) = unit.owner() else {
        return Err(Abort::new(
            "'NoneType' object has no attribute 'get_object_by_id'",
        ));
    };
    let Some(Resolved { target, entity: te }) = k.resolve(p, target)? else {
        return Ok(None);
    };
    if te.is_some_and(|t| t.class() == EntityClass::Exit) {
        return Ok(Some(Keyword::Block));
    }
    if let Some(t) = te.filter(|t| t.owner() == Some(p)) {
        if have_enough_space(k, unit, &t) {
            return Ok(Some(Keyword::Load));
        }
        if let Some(transport) = k.unit(t.id())
            && transport.inside().is_some()
            && have_enough_space(k, transport, &unit)
        {
            return Ok(Some(Keyword::Enter));
        }
    }
    if unit_basic_abilities(&unit).contains(&"gather")
        && te.is_some_and(|t| t.class() == EntityClass::Deposit)
    {
        return Ok(Some(Keyword::Gather));
    }
    if let Some(t) = te {
        let rules = k.rules();
        let can_build = type_list(unit.unit_type(rules), "can_build");
        let site_ok = t
            .site_type()
            .is_some_and(|s| can_build.contains(&rules.ty(s).name));
        let repair_ok = t.is_repairable() && t.hp() < t.hp_max() && !can_build.is_empty();
        if (site_ok || repair_ok) && !is_an_enemy(k, unit, &target, te) {
            return Ok(Some(Keyword::Repair));
        }
    }
    if order_is_allowed(k, unit, Keyword::RallyingPoint) {
        return Ok(Some(Keyword::RallyingPoint));
    }
    if order_is_allowed(k, unit, Keyword::Go) {
        return Ok(Some(Keyword::Go));
    }
    Ok(None)
}

// what a player sees

/// Whether an exit is closed as far as a player knows: by dense forests,
/// or by a blocker on either side, as the player perceives or remembers
/// the exits and the blockers. With `friendly_gates_open`, a gate of the
/// player or an ally does not close it. An exit the player knows nothing
/// of is open.
pub(crate) fn exit_is_blocked<E: EntityFacts + Copy>(
    known: impl Fn(EntityId) -> Option<E>,
    is_ally: impl Fn(PlayerId) -> bool,
    exit: EntityId,
    other_side: Option<EntityId>,
    friendly_gates_open: bool,
) -> bool {
    let Some(x) = known(exit) else {
        return false;
    };
    let Some((by_forests, blockers)) = x.exit() else {
        return false;
    };
    if by_forests {
        return true;
    }
    let other = other_side.and_then(&known);
    let other_blockers = other
        .as_ref()
        .and_then(|o| o.exit())
        .map_or(&[][..], |(_, b)| b);
    blockers.iter().chain(other_blockers).any(|&b| {
        let Some(b) = known(b) else {
            return false;
        };
        let enemy = !friendly_gates_open || b.owner().is_some_and(|bp| !is_ally(bp));
        !b.is_a_gate() || enemy
    })
}

/// The units of `owner` among the perceived entities on `place`, `except`
/// one that died there: what a player sees left of that side of the fight.
pub(crate) fn comrades<E: EntityFacts>(
    perceived: impl IntoIterator<Item = E>,
    owner: PlayerId,
    place: Option<Place>,
    except: EntityId,
) -> Comrades {
    let mut c = Comrades {
        units: 0,
        soldiers: 0,
    };
    for u in perceived {
        if u.owner() == Some(owner) && u.place() == place && u.id() != except {
            c.units += 1;
            if u.is_a_threat() {
                c.soldiers += 1;
            }
        }
    }
    c
}

/// What a unit is busy with, from its first order, for its noise. What it
/// gathers from is named by `name_of`: as the world has it, or as a player
/// knows it.
pub(crate) fn activity<'n>(
    first: Option<&'n Order>,
    name_of: impl FnOnce(&'n Target) -> Option<&'n str>,
) -> Option<Activity> {
    let o = first?;
    match &o.kind {
        OrderKind::Repair { step, .. } | OrderKind::BuildPhaseTwo { step, .. } => match step {
            BuildStep::Working => Some(Activity::Building),
            BuildStep::Waiting | BuildStep::Going => None,
        },
        OrderKind::Gather { aim, step } => match step {
            GatherStep::Extract { .. } => Some(Activity::Exploiting(
                name_of(aim.target.as_ref()?)?.to_owned(),
            )),
            GatherStep::Start
            | GatherStep::BringBack { .. }
            | GatherStep::Store { .. }
            | GatherStep::GoGather => None,
        },
        OrderKind::Production(_)
        | OrderKind::Go { .. }
        | OrderKind::Attack { .. }
        | OrderKind::Patrol { .. }
        | OrderKind::AutoAttack
        | OrderKind::AutoExplore
        | OrderKind::Wait { .. }
        | OrderKind::Block { .. }
        | OrderKind::Build { .. }
        | OrderKind::Use { .. }
        | OrderKind::Load { .. }
        | OrderKind::LoadAll { .. }
        | OrderKind::UnloadAll { .. } => None,
    }
}

/// The upgrades of its owner that apply to a unit of type `t`.
pub(crate) fn upgrades_applying(
    rules: &Rules,
    t: Option<&UnitType>,
    owner_upgrades: &Upgrades,
) -> Vec<String> {
    t.map_or(&[][..], |t| &t.can_use)
        .iter()
        .filter(|u| rules.type_id(u).is_some_and(|u| owner_upgrades.has(u)))
        .cloned()
        .collect()
}
