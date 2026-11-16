//! The computer player's one door to the game: everything it reads and
//! everything it does goes through a [`Door`], and a door offers only what
//! a human in the computer's seat could know or do.
//!
//! The world is a private field of the door, so the computer's code in
//! the parent module cannot name it. A read of the true world can only be
//! added here, where a reviewer sees it. What a door offers, by source:
//!
//! - **What every player knows:** the rules, the clock, the map (squares,
//!   exits and where they lead, starting squares), who plays and who is
//!   allied with whom.
//! - **The computer's own state:** its resources, food, upgrades, units
//!   and their orders ([`Own`]), and the state of its plan.
//! - **What it perceives now, or remembers** as its memory holds it, with
//!   only what a player who perceives a thing is told of it ([`Seen`]),
//!   and what the world derives from these alone: the enemy menace and
//!   presence by square, the places with corpses or friends.
//!
//! A target, of an order or of the computer's plan, is a [`Mark`]: it names
//! a square or a thing, and the computer reads the thing through the door,
//! as [`Seen`]. A remembered thing's copy stays behind the mark.
//! - **Paths**, as its units' path finder finds them.
//! - **Orders**, given to its own units through the checks a human's
//!   order goes through: the unit's menu, its requirements, its targets
//!   as the player knows them.
//!
//! The random draws are the world's, so that a game stays the same for a
//! seed.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::abort::Res;
use crate::entity::{
    Entity, EntityId, MemoryRef, Names, Place, PlayerId, SquareId, Target, TargetIdentity,
};
use crate::orders::{Keyword, Likeness, MenuEntry, Order, OrderInfo, OrderKind, Request};
use crate::pathfinding::{Forests, PathResult};
use crate::player::PlayerKind;
use crate::questions;
use crate::rules::{AirGround, ClassId, EntityClass, FactionId, Rules, TypeId};
use crate::world::{INF, TPlace, World};

use super::Computer;
use super::script::Script;

/// What a computer player may read and do: see the [module](self).
pub(super) struct Door<'w> {
    world: &'w mut World,
    me: PlayerId,
}

/// Something the player perceives now, or its copy as the player last
/// perceived it: only what a player who perceives it is told.
#[derive(Clone, Copy)]
pub(super) struct Seen<'a> {
    e: &'a Entity,
}

impl<'a> Seen<'a> {
    pub(super) fn id(self) -> EntityId {
        self.e.id
    }

    pub(super) fn class(self) -> EntityClass {
        self.e.class()
    }

    /// Its type in the rules. A building site has none.
    pub(super) fn type_id(self) -> Option<TypeId> {
        self.e.type_id()
    }

    pub(super) fn owner(self) -> Option<PlayerId> {
        self.e.player()
    }

    /// Where it is, or was when last perceived.
    pub(super) fn place(self) -> Option<Place> {
        self.e.place
    }

    pub(super) fn is_meadow(self) -> bool {
        self.e.is_meadow()
    }

    pub(super) fn is_deposit(self) -> bool {
        self.e.is_deposit()
    }

    pub(super) fn is_exit(self) -> bool {
        self.e.is_exit()
    }

    /// For a deposit, its type.
    pub(super) fn deposit_type(self) -> Option<TypeId> {
        self.e.deposit().map(|d| d.type_id)
    }
}

/// A target as the computer holds it: a square, or a thing it perceived
/// or remembers. A mark only names; what the computer reads of the thing
/// goes through the door, as [`Seen`].
#[derive(Clone, Debug, Hash)]
pub(super) struct Mark(Target);

impl Mark {
    /// A mark of any target, for the tests.
    #[cfg(test)]
    pub(super) fn of(t: Target) -> Mark {
        Mark(t)
    }

    pub(super) fn square(s: SquareId) -> Mark {
        Mark(Target::Square(s))
    }

    /// The same square or thing, and for a remembered thing the same
    /// memory of it.
    pub(super) fn same(&self, other: &Mark) -> bool {
        self.0.same(&other.0)
    }

    /// The id of the thing it names.
    pub(super) fn entity_id(&self) -> Option<EntityId> {
        self.0.entity_id()
    }

    /// Whether it names this thing as perceived, not as remembered.
    pub(super) fn is_entity(&self, id: EntityId) -> bool {
        matches!(self.0, Target::Entity(e) if e == id)
    }

    pub(super) fn as_square(&self) -> Option<SquareId> {
        match self.0 {
            Target::Square(s) => Some(s),
            _ => None,
        }
    }
}

/// One of the computer's own units, as it is now. The targets of its
/// orders are [`Mark`]s.
#[derive(Clone, Copy)]
pub(super) struct Own<'a> {
    e: &'a Entity,
}

impl<'a> Own<'a> {
    pub(super) fn place(self) -> Option<Place> {
        self.e.place
    }

    pub(super) fn xy(self) -> (i64, i64) {
        (self.e.x, self.e.y)
    }

    pub(super) fn speed(self) -> i64 {
        self.e.stats.speed
    }

    pub(super) fn is_a_building(self) -> bool {
        self.e.stats.is_a_building
    }

    pub(super) fn class(self) -> EntityClass {
        self.e.class()
    }

    /// Its type in the rules. A building site has none.
    pub(super) fn type_id(self) -> Option<TypeId> {
        self.e.type_id()
    }

    pub(super) fn hp(self) -> i64 {
        self.e.creature().map_or(0, |c| c.hp)
    }

    pub(super) fn airground_type(self) -> AirGround {
        self.e.airground_type
    }

    pub(super) fn menace(self) -> i64 {
        self.e.menace()
    }

    pub(super) fn time_limit(self) -> Option<i64> {
        self.e.time_limit()
    }

    /// For a building site, the building it will become.
    pub(super) fn site_type(self) -> Option<TypeId> {
        self.e.site().map(|s| s.building_type)
    }

    /// Its orders, first to last.
    pub(super) fn orders(self) -> impl ExactSizeIterator<Item = OwnOrder<'a>> {
        self.e.orders().iter().map(|o| OwnOrder { o })
    }

    pub(super) fn first_order(self) -> Option<OwnOrder<'a>> {
        self.orders().next()
    }

    pub(super) fn has_orders(self) -> bool {
        !self.e.orders().is_empty()
    }
}

/// An order of one of the computer's units.
#[derive(Clone, Copy)]
pub(super) struct OwnOrder<'a> {
    o: &'a Order,
}

impl OwnOrder<'_> {
    pub(super) fn keyword(self) -> Keyword {
        self.o.keyword()
    }

    pub(super) fn is_impossible(self) -> bool {
        self.o.is_impossible()
    }

    /// What it aims at now.
    pub(super) fn target(self) -> Option<Mark> {
        self.o.target().cloned().map(Mark)
    }

    /// The type it trains, researches, upgrades to or builds.
    pub(super) fn made(self) -> Option<TypeId> {
        match &self.o.kind {
            OrderKind::Production(p) => Some(p.product.type_id()),
            OrderKind::Build { building, .. } => Some(*building),
            OrderKind::Go { .. }
            | OrderKind::Attack { .. }
            | OrderKind::Patrol { .. }
            | OrderKind::Gather { .. }
            | OrderKind::AutoAttack
            | OrderKind::AutoExplore
            | OrderKind::Wait { .. }
            | OrderKind::Block { .. }
            | OrderKind::Repair { .. }
            | OrderKind::BuildPhaseTwo { .. }
            | OrderKind::Use { .. }
            | OrderKind::Load { .. }
            | OrderKind::LoadAll { .. }
            | OrderKind::UnloadAll { .. } => None,
        }
    }
}

/// An order the computer gave one of its units, followed until it is
/// done: the order's uid, and what it needs of the order as last seen.
#[derive(Clone, Debug, Hash)]
pub(super) struct Followed {
    unit: EntityId,
    uid: u64,
    /// The order was complete when last seen.
    complete: bool,
    /// An order of the unit alike to it, as last seen, is taken for it.
    likeness: Option<Likeness>,
}

impl Followed {
    fn new(unit: EntityId, o: &Order) -> Followed {
        Followed {
            unit,
            uid: o.uid,
            complete: o.is_complete(),
            likeness: o.likeness(),
        }
    }
}

impl Names for Mark {
    fn names(&self, f: &mut impl FnMut(EntityId)) {
        self.0.names(f);
    }
}

/// Its unit, and the thing its order aimed at as last seen, which an
/// order alike to it aims at too.
impl Names for Followed {
    fn names(&self, f: &mut impl FnMut(EntityId)) {
        f(self.unit);
        if let Some(Likeness::Go { target, .. } | Likeness::Build { target, .. }) = self.likeness
            && let Some(TargetIdentity::Entity(e)) = target
        {
            f(e);
        }
    }
}

/// What a player knows of a unit that attacked one of its own: its owner
/// and whether it can be hurt if the player perceives it. Of an attacker it
/// does not perceive, it knows nothing: not every hit says who struck.
pub(super) struct Attacker {
    pub owner: Option<PlayerId>,
    pub is_vulnerable: bool,
}

impl World {
    /// The door of a computer player.
    pub(super) fn door(&mut self, me: PlayerId) -> Door<'_> {
        Door { world: self, me }
    }

    /// The state of a new scripted computer, with its AI type set.
    pub(crate) fn new_computer(&mut self, p: PlayerId) {
        self.plm(p).ai = Some(Box::default());
        let t = self.pl(p).ai_type().unwrap_or("timers").to_owned();
        self.computer_set_ai(p, &t);
    }

    /// `set_ai`, after the AI type of the player changed: loads the script
    /// and resets the script's settings.
    pub(crate) fn computer_set_ai(&mut self, p: PlayerId, ai_type: &str) {
        self.door(p).set_ai(ai_type);
    }

    /// `play`: what a computer player does during its update.
    pub(crate) fn ai_play(&mut self, p: PlayerId) -> Res {
        self.door(p).play()
    }

    /// `Computer.on_unit_attacked`: remembers where to retaliate.
    pub(crate) fn computer_on_unit_attacked(
        &mut self,
        p: PlayerId,
        unit: EntityId,
        attacker: EntityId,
    ) {
        self.door(p).on_unit_attacked(unit, attacker);
    }

    /// `Computer.time_has_come` without the cataclysm check: whether the
    /// units waiting on a square are strong enough.
    pub(crate) fn computer_time_has_come(&self, p: PlayerId, s: SquareId) -> bool {
        self.pl(p).ai.as_deref().is_some_and(|c| c.time_has_come(s))
    }

    #[cfg(test)]
    pub(crate) fn update_effect_users_and_workers(&mut self, p: PlayerId) {
        self.door(p).update_effect_users_and_workers();
    }

    /// `best_explorers`: the mobile units not upgrading, best first.
    #[cfg(test)]
    pub(crate) fn best_explorers(&mut self, p: PlayerId) -> Vec<EntityId> {
        self.door(p).best_explorers()
    }

    /// `is_ok_for_warehouse`: as far as the player knows, no warehouse for
    /// the resource on the square, and a deposit of it.
    #[cfg(test)]
    pub(crate) fn is_ok_for_warehouse(&mut self, p: PlayerId, z: SquareId, rt: i64) -> bool {
        self.door(p).is_ok_for_warehouse(z, rt)
    }

    /// `get`: works toward having `nb` units of the type; true if they are
    /// there.
    #[cfg(test)]
    pub(crate) fn ai_get(&mut self, p: PlayerId, nb: i64, type_name: &str) -> Result<bool, String> {
        let t = self.rules.type_id(type_name);
        self.door(p).get(nb, t).map_err(|e| format!("{e:?}"))
    }

    /// `nb`: the number of units of the type.
    #[cfg(test)]
    pub(crate) fn computer_nb(&mut self, p: PlayerId, type_name: &str) -> i64 {
        let t = self.rules.type_id(type_name).expect("a type of the rules");
        self.door(p).nb(&super::Sel::One(t))
    }

    /// The state of a scripted computer, for the tests.
    #[cfg(test)]
    pub(super) fn computer_state(&mut self, p: PlayerId) -> &mut Computer {
        self.csm(p)
    }

    fn cs(&self, p: PlayerId) -> &Computer {
        self.pl(p)
            .ai
            .as_deref()
            .expect("a scripted computer has its state")
    }

    fn csm(&mut self, p: PlayerId) -> &mut Computer {
        self.plm(p)
            .ai
            .as_deref_mut()
            .expect("a scripted computer has its state")
    }
}

impl Door<'_> {
    // what every player knows

    pub(super) fn rules(&self) -> &Rules {
        &self.world.rules
    }

    /// The game time in milliseconds.
    pub(super) fn time(&self) -> i64 {
        self.world.time
    }

    pub(super) fn turn(&self) -> i64 {
        self.world.turn()
    }

    /// The map's limit of food for every player.
    pub(super) fn food_limit(&self) -> i64 {
        self.world.food_limit()
    }

    pub(super) fn square_width(&self) -> i64 {
        self.world.square_width()
    }

    /// The computer players in the game whose units are clever, which
    /// share the planning between the turns.
    pub(super) fn planning_computers(&self) -> Vec<PlayerId> {
        self.world.cpu_intensive_players()
    }

    /// The number of a square on the map.
    pub(super) fn square_number(&self, s: SquareId) -> u32 {
        self.world.square(s).id
    }

    /// The exits of a square.
    pub(super) fn exits(&self, s: SquareId) -> &[EntityId] {
        &self.world.square(s).exits
    }

    /// Where an exit stands.
    pub(super) fn exit_xy(&self, e: EntityId) -> (i64, i64) {
        let x = self.world.e(e);
        (x.x, x.y)
    }

    /// The squares the exits of a square lead to, in the order of its
    /// exits. The map makes each exit with its other side on a square.
    pub(super) fn squares_beyond(&self, s: SquareId) -> Vec<SquareId> {
        self.exits(s)
            .iter()
            .filter_map(|&e| self.world.exit_other_side(e))
            .filter_map(|os| match self.world.e(os).place {
                Some(Place::Square(beyond)) => Some(beyond),
                _ => None,
            })
            .collect()
    }

    /// The script of an AI type, from `ai.txt`.
    pub(super) fn script(&self, name: &str) -> Option<&Script> {
        self.world.ai_scripts.get(name)
    }

    /// The faction's own version of an AI type.
    pub(super) fn faction_ai_type(&self, ai_type: &str) -> String {
        self.world.faction_ai_type(self.me, ai_type)
    }

    /// Whether units of `unit_type` can harm units of `other`, by the
    /// rules.
    pub(super) fn can_harm(&mut self, unit_type: TypeId, other: TypeId) -> bool {
        self.world.can_harm(unit_type, other)
    }

    // its own state

    pub(super) fn me(&self) -> PlayerId {
        self.me
    }

    pub(super) fn kind(&self) -> &PlayerKind {
        &self.world.pl(self.me).kind
    }

    pub(super) fn faction(&self) -> Option<FactionId> {
        self.world.pl(self.me).faction
    }

    /// Its units, in the order they came.
    pub(super) fn units(&self) -> &[EntityId] {
        &self.world.pl(self.me).units
    }

    /// One of its units, as it is now. `None` for any other entity.
    pub(super) fn unit(&self, u: EntityId) -> Option<Own<'_>> {
        self.world
            .entity(u)
            .filter(|e| e.player() == Some(self.me))
            .map(|e| Own { e })
    }

    pub(super) fn resources(&self) -> &[i64] {
        &self.world.pl(self.me).resources
    }

    pub(super) fn used_food(&self) -> i64 {
        self.world.pl(self.me).used_food
    }

    /// Its food, up to the map's limit.
    pub(super) fn available_food(&self) -> i64 {
        let limit = self.world.food_limit();
        self.world.pl(self.me).available_food(limit)
    }

    /// Whether it has an upgrade, or a unit of the class or derived from
    /// it.
    pub(super) fn has(&self, class: ClassId) -> bool {
        self.world.has(self.me, class)
    }

    /// Whether it has the upgrade.
    pub(super) fn has_upgrade(&self, t: TypeId) -> bool {
        self.world.pl(self.me).upgrades.has(t)
    }

    pub(super) fn has_state(&self) -> bool {
        self.world.pl(self.me).ai.is_some()
    }

    /// The state of its plan.
    pub(super) fn state(&self) -> &Computer {
        self.world.cs(self.me)
    }

    pub(super) fn state_mut(&mut self) -> &mut Computer {
        self.world.csm(self.me)
    }

    /// The orders of one kind its unit may receive now: `go`, or `train
    /// footman`, `train archer`... With `strict`, the requirements must be
    /// met too.
    pub(super) fn order_menu(&self, u: EntityId, kw: Keyword, strict: bool) -> Vec<MenuEntry> {
        if self.unit(u).is_none() {
            return Vec::new();
        }
        self.world.order_menu(u, kw, strict)
    }

    /// The costs and number of targets of an order its unit could receive.
    pub(super) fn order_info(
        &self,
        u: EntityId,
        kw: Keyword,
        type_id: Option<TypeId>,
    ) -> Option<OrderInfo> {
        self.unit(u)?;
        self.world.order_info(u, kw, type_id)
    }

    /// Whether its unit may receive the order now, by its menu.
    pub(super) fn is_order_allowed(&self, u: EntityId, kw: Keyword, t: Option<TypeId>) -> bool {
        let name = t.map(|t| self.world.rules.ty(t).name.as_str());
        self.unit(u).is_some() && self.world.is_order_allowed(u, kw, name)
    }

    /// The first order of its unit, to follow, if it has this keyword.
    pub(super) fn follow_first(&self, u: EntityId, kw: Keyword) -> Option<Followed> {
        let o = self.unit(u)?.e.orders().first()?;
        (o.keyword() == kw).then(|| Followed::new(u, o))
    }

    /// Whether an order it follows is still in progress: in its unit's
    /// queue and not complete, the unit on the map. An order that left the
    /// queue counts while the unit has an order alike to it (see
    /// [`Likeness`]) and it was not complete when last seen.
    pub(super) fn still_followed(&self, f: &mut Followed) -> bool {
        let Some(unit) = self.unit(f.unit).map(|u| u.e) else {
            return false;
        };
        if let Some(live) = unit.orders().iter().find(|o| o.uid == f.uid) {
            *f = Followed::new(f.unit, live);
            return !live.is_complete() && unit.place.is_some();
        }
        !f.complete
            && unit.place.is_some()
            && f.likeness
                .is_some_and(|l| unit.orders().iter().any(|o| o.likeness() == Some(l)))
    }

    // what it perceives and remembers

    pub(super) fn allied(&self) -> &[PlayerId] {
        &self.world.pl(self.me).allied
    }

    /// The entities it perceives now, in id order.
    pub(super) fn perceived(&self) -> impl Iterator<Item = Seen<'_>> {
        let w = &*self.world;
        w.pl(self.me)
            .perception
            .iter()
            .filter_map(|&o| w.entity(o))
            .map(|e| Seen { e })
    }

    pub(super) fn perceives(&self, o: EntityId) -> bool {
        self.world.pl(self.me).perception.contains(&o)
    }

    /// The entities it remembers, as it last perceived them, in id order,
    /// each with a mark that names the memory.
    pub(super) fn remembered(&self) -> impl Iterator<Item = (Seen<'_>, Mark)> {
        let me = self.me;
        self.world.pl(me).memory.values().map(move |r| {
            let target = Target::Memory(MemoryRef {
                owner: me,
                snapshot: Arc::clone(&r.snapshot),
            });
            (Seen { e: &r.snapshot }, Mark(target))
        })
    }

    /// A mark of a thing it perceives now.
    pub(super) fn mark(s: Seen) -> Mark {
        Mark(Target::Entity(s.e.id))
    }

    /// An entity as it knows it: perceived now, or remembered.
    pub(super) fn known(&self, o: EntityId) -> Option<Seen<'_>> {
        if self.perceives(o) {
            return self.world.entity(o).map(|e| Seen { e });
        }
        let r = self.world.pl(self.me).memory.get(&o)?;
        Some(Seen { e: &r.snapshot })
    }

    /// The type of what a mark names: a thing it perceived when it
    /// named it, whose type never changes.
    pub(super) fn target_type(&self, t: &Mark) -> Option<TypeId> {
        match &t.0 {
            Target::Entity(id) => self.world.entity(*id).and_then(Entity::type_id),
            Target::Memory(m) => m.snapshot.type_id(),
            Target::Square(_) | Target::Zoom(_) | Target::Xy(..) => None,
        }
    }

    /// Where a mark is, as it knows it.
    pub(super) fn target_place(&self, t: &Mark) -> TPlace {
        match &t.0 {
            Target::Entity(id) if !self.perceives(*id) => match self.known(*id).map(Seen::place) {
                Some(Some(p)) => TPlace::At(p),
                _ => TPlace::Nowhere,
            },
            t => self.world.t_place(t),
        }
    }

    pub(super) fn target_place_of(&self, t: &Mark) -> Option<Place> {
        match self.target_place(t) {
            TPlace::At(p) => Some(p),
            TPlace::World | TPlace::Nowhere => None,
        }
    }

    /// Whether it still knows the target: perceives it, or remembers it
    /// as the copy shows it.
    pub(super) fn is_known(&self, t: &Mark) -> bool {
        match &t.0 {
            Target::Entity(id) => self.perceives(*id),
            Target::Memory(m) => self.world.memory_is_current(m),
            Target::Square(_) | Target::Zoom(_) | Target::Xy(..) => false,
        }
    }

    /// What it is told of a unit that attacked one of its own.
    pub(super) fn attacker(&self, a: EntityId) -> Attacker {
        if let Some(e) = self.world.entity(a).filter(|_| self.perceives(a)) {
            return Attacker {
                owner: e.player(),
                is_vulnerable: e.is_vulnerable(),
            };
        }
        Attacker {
            owner: None,
            is_vulnerable: true,
        }
    }

    pub(super) fn is_an_enemy(&self, s: Seen) -> bool {
        s.owner().is_some_and(|o| !self.allied().contains(&o))
    }

    /// The squares where it knows of an enemy, in the order found.
    pub(super) fn enemy_presence(&self) -> &[SquareId] {
        &self.world.pl(self.me).enemy_presence
    }

    /// The menace of the enemies it knows on a square, and around it for
    /// ranged ones.
    pub(super) fn enemy_menace(&self, s: SquareId) -> i64 {
        self.world.enemy_menace(self.me, s)
    }

    pub(super) fn square_is_dangerous(&self, s: SquareId) -> bool {
        self.world.square_is_dangerous(self.me, s)
    }

    pub(super) fn places_with_friends(&self) -> &BTreeSet<SquareId> {
        &self.world.pl(self.me).places_with_friends
    }

    pub(super) fn places_with_corpses(&self) -> &BTreeSet<SquareId> {
        &self.world.pl(self.me).places_with_corpses
    }

    /// Whether an exit is closed, gates included, by what it perceives or
    /// remembers of the exit, its other side and their blockers: the rule
    /// a human's seat follows.
    pub(super) fn exit_is_blocked(&self, e: EntityId) -> bool {
        questions::exit_is_blocked(
            |id| self.known(id).map(|s| s.e),
            |p| self.allied().contains(&p),
            e,
            self.world.exit_other_side(e),
            false,
        )
    }

    // paths
    //
    // A path is planned through the exits as it perceives or remembers
    // their forests: an exit it knows nothing of is open. Its units walk
    // the true paths when they move.

    fn path(
        &mut self,
        start: SquareId,
        dest: SquareId,
        walls_of: Option<PlayerId>,
        places: bool,
        avoid: bool,
    ) -> Res<PathResult> {
        let forests = Forests::AsKnownBy(self.me);
        self.world.shortest_path(
            start,
            dest,
            AirGround::Ground,
            walls_of,
            places,
            avoid,
            forests,
        )
    }

    /// The ground distance by the shortest path, through the exits not
    /// closed by forests or by its own and its allies' walls; with
    /// `avoid`, around the squares it knows to be very dangerous.
    pub(super) fn distance(&mut self, start: SquareId, dest: SquareId, avoid: bool) -> Res<i64> {
        match self.path(start, dest, Some(self.me), false, avoid)? {
            PathResult::Next(_, d) => Ok(d),
            PathResult::Places(_) => Ok(INF),
        }
    }

    /// The ground distance by the shortest path through the exits not
    /// closed by forests, whatever blocks them.
    pub(super) fn distance_through_walls(&mut self, start: SquareId, dest: SquareId) -> Res<i64> {
        match self.path(start, dest, None, false, false)? {
            PathResult::Next(_, d) => Ok(d),
            PathResult::Places(_) => Ok(INF),
        }
    }

    /// The squares of the exits on the shortest ground path through the
    /// exits not closed by forests, whatever blocks them.
    pub(super) fn path_through_walls(
        &mut self,
        start: SquareId,
        dest: SquareId,
    ) -> Res<Vec<SquareId>> {
        match self.path(start, dest, None, true, false)? {
            PathResult::Places(p) => Ok(p),
            PathResult::Next(..) => Ok(Vec::new()),
        }
    }

    /// Whether a ground path that avoids the squares it knows to be very
    /// dangerous leaves `start` toward `dest`.
    pub(super) fn has_a_safe_path(&mut self, start: SquareId, dest: SquareId) -> Res<bool> {
        match self.path(start, dest, Some(self.me), false, true)? {
            PathResult::Next(n, _) => Ok(n.is_some()),
            PathResult::Places(_) => Ok(false),
        }
    }

    /// Its or an ally's unit storing the resource with the shortest path
    /// from `place`; one without a path comes last. Its allies' units are
    /// all perceived.
    pub(super) fn nearest_warehouse(
        &mut self,
        start: SquareId,
        resource_type: i64,
        include_building_sites: bool,
    ) -> Res<Option<EntityId>> {
        let rules = &self.world.rules;
        let stores = |t: TypeId| rules.ty(t).storable_resource_types.contains(&resource_type);
        let mut found: Vec<(EntityId, Option<Place>)> = Vec::new();
        for &a in self.allied() {
            for &u in &self.world.pl(a).units {
                let Some(e) = self.world.entity(u) else {
                    continue;
                };
                let site =
                    include_building_sites && e.site().is_some_and(|s| stores(s.building_type));
                if e.type_id().is_some_and(stores) || site {
                    found.push((u, e.place));
                }
            }
        }
        let mut warehouses: Vec<(i64, EntityId)> = Vec::new();
        for (u, p) in found {
            // A warehouse inside a unit cannot be reached.
            let Some(Place::Square(s)) = p else {
                continue;
            };
            let d = self.distance(start, s, false)?;
            if d == 0 {
                return Ok(Some(u));
            }
            warehouses.push((d, u));
        }
        warehouses.sort();
        Ok(warehouses.first().map(|w| w.1))
    }

    // acting

    /// Gives an order to one of its units, as a human's order is given:
    /// refused, with the sound of an impossible order, if the unit's menu
    /// does not have it. A unit no longer its own, which died or changed
    /// sides during its turn, is given nothing.
    pub(super) fn give(&mut self, u: EntityId, r: Request, forget_previous: bool) -> Res {
        if self.unit(u).is_none() {
            return Ok(());
        }
        self.world.take_order(u, r, forget_previous, false, None)
    }

    /// Grants the privileges of its handicap, which its name announces:
    /// nothing for a fair computer.
    pub(super) fn grant_handicap(&mut self) {
        let pl = self.world.plm(self.me);
        let h = pl.handicap;
        if h.ignores_requirements {
            pl.cheat_has_all = true;
        }
        if let Some(r) = h.resources_with_precision() {
            pl.resources = vec![r; pl.resources.len()];
        }
    }

    /// An index drawn at random below `len`, from the game's draws.
    pub(super) fn choice_index(&mut self, len: usize) -> Option<usize> {
        self.world.random.choice_index(len)
    }

    /// One of `first` and `rest`, drawn at random from the game's draws as
    /// the original's `choice` draws from them all.
    pub(super) fn pick<T: Copy>(&mut self, first: T, rest: &[T]) -> T {
        let n = u64::try_from(rest.len()).map_or(u64::MAX, |n| n.saturating_add(1));
        let i = self.world.random.randbelow(n);
        i.checked_sub(1)
            .and_then(|i| rest.get(usize::try_from(i).ok()?))
            .copied()
            .unwrap_or(first)
    }

    pub(super) fn warn(&mut self, s: impl Into<String>) {
        self.world.warn(s);
    }
}

/// The computer's bookkeeping names the units it tracks, its workers, its
/// sites, the users of its abilities and the targets it chose. The targets
/// of the orders it builds are given as targets, resolved through what it
/// perceives or remembers, so they are not names.
impl Names for Computer {
    fn names(&self, f: &mut impl FnMut(EntityId)) {
        let Computer {
            plan: _,
            line_nb: _,
            previous_linechange: _,
            watchdog: _,
            constant_attacks: _,
            research: _,
            attacked_places,
            orders,
            previous_choose,
            sensible_building,
            workers,
            gathered_deposits,
            building_sites,
            raise_dead_users,
            teleportation_users,
            cataclysm_users,
            detector_users,
            summon_users,
            time_has_come,
            safe_cnt: _,
            lacking: _,
        } = self;
        attacked_places.names(f);
        for followed in orders.values().flatten() {
            followed.names(f);
        }
        for ((_, _, place), target) in previous_choose {
            place.names(f);
            target.names(f);
        }
        sensible_building.names(f);
        workers.names(f);
        for (target, _) in gathered_deposits {
            target.names(f);
        }
        for (target, _) in time_has_come {
            target.names(f);
        }
        building_sites.names(f);
        for users in [
            raise_dead_users,
            teleportation_users,
            cataclysm_users,
            detector_users,
            summon_users,
        ] {
            for (u, _) in users {
                f(*u);
            }
        }
    }
}
