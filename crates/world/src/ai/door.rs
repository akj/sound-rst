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
//!   and their orders, and the state of its plan.
//! - **What it perceives now, or remembers** as its memory holds it, with
//!   only what a player who perceives a thing is told of it ([`Seen`]),
//!   and what the world derives from these alone: the enemy menace and
//!   presence by square, the places with corpses or friends.
//! - **Paths**, as its units' path finder finds them.
//! - **Orders**, given to its own units through the checks a human's
//!   order goes through: the unit's menu, its requirements, its targets
//!   as the player knows them.
//!
//! The random draws are the world's, so that a game stays the same for a
//! seed.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::entity::{Entity, EntityId, MemoryRef, Place, PlayerId, SquareId, Target};
use crate::orders::{Keyword, MenuEntry, Order, OrderInfo, Request};
use crate::pathfinding::Viewer;
use crate::player::PlayerKind;
use crate::rules::{AirGround, EntityClass, FactionId, Rules, TypeId};
use crate::world::{Abort, Res, TPlace, World};

use super::Computer;

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

    pub(super) fn type_name(self) -> &'a str {
        &self.e.type_name
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

/// What a player is told of a unit that attacked one of its own: its
/// owner if the player perceives it, and whether it can be hurt, which
/// its type says. A wound tells the attacker's type.
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
    pub(crate) fn new_computer(&mut self, p: PlayerId) -> Res {
        self.plm(p).ai = Some(Box::default());
        let t = self.pl(p).ai_type().unwrap_or("timers").to_owned();
        self.computer_set_ai(p, &t)
    }

    /// `set_ai`, after the AI type of the player changed: loads the script
    /// and resets the script's settings.
    pub(crate) fn computer_set_ai(&mut self, p: PlayerId, ai_type: &str) -> Res {
        self.door(p).set_ai(ai_type)
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
    pub(crate) fn update_effect_users_and_workers(&mut self, p: PlayerId) -> Res {
        self.door(p).update_effect_users_and_workers()
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
    pub(crate) fn ai_get(&mut self, p: PlayerId, nb: i64, type_name: &str) -> Res<bool> {
        self.door(p).get(nb, type_name)
    }

    /// `nb`: the number of units of the type.
    #[cfg(test)]
    pub(crate) fn computer_nb(&mut self, p: PlayerId, type_name: &str) -> i64 {
        self.door(p).nb(&super::Sel::One(type_name.to_owned()))
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

    /// The place on the other side of an exit, and whether it has one.
    pub(super) fn beyond_exit(&self, e: EntityId) -> Option<Option<Place>> {
        let os = self.world.exit_other_side(e)?;
        Some(self.world.e(os).place)
    }

    /// The script of an AI type, from `ai.txt`.
    pub(super) fn script(&self, name: &str) -> Option<&[String]> {
        self.world.ai_scripts.get(name)
    }

    /// The faction's own version of an AI type.
    pub(super) fn faction_ai_type(&self, ai_type: &str) -> String {
        self.world.faction_ai_type(self.me, ai_type)
    }

    /// Whether `unit_type` can harm `other_type`, by the rules.
    pub(super) fn can_harm(&mut self, unit_type: &str, other_type: &str) -> bool {
        self.world.can_harm(unit_type, other_type)
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
    pub(super) fn unit(&self, u: EntityId) -> Option<&Entity> {
        self.world.entity(u).filter(|e| e.player() == Some(self.me))
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

    /// Whether it has an upgrade, or a unit of the type or derived from it.
    pub(super) fn has(&self, type_name: &str) -> bool {
        self.world.has(self.me, type_name)
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
    pub(super) fn is_order_allowed(
        &self,
        u: EntityId,
        kw: Keyword,
        type_name: Option<&str>,
    ) -> Res<bool> {
        if self.unit(u).is_none() {
            return Ok(false);
        }
        self.world.is_order_allowed(u, kw, type_name)
    }

    /// Whether two orders of its units are the same order.
    pub(super) fn order_eq(&self, a: &Order, b: &Order) -> bool {
        self.world.order_eq(a, b)
    }

    /// An order as the words it is written with.
    pub(super) fn words(&self, r: &Request) -> Vec<String> {
        r.words(self.world)
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
    /// each with a target that names the copy.
    pub(super) fn remembered(&self) -> impl Iterator<Item = (Seen<'_>, Target)> {
        let me = self.me;
        self.world.pl(me).memory.values().map(move |r| {
            let target = Target::Memory(MemoryRef {
                owner: me,
                snapshot: Arc::clone(&r.snapshot),
            });
            (Seen { e: &r.snapshot }, target)
        })
    }

    /// An entity as it knows it: perceived now, or remembered.
    pub(super) fn known(&self, o: EntityId) -> Option<Seen<'_>> {
        if self.perceives(o) {
            return self.world.entity(o).map(|e| Seen { e });
        }
        let r = self.world.pl(self.me).memory.get(&o)?;
        Some(Seen { e: &r.snapshot })
    }

    /// The type of what a target names: a thing it perceived when it
    /// named it, whose type never changes.
    pub(super) fn target_type(&self, t: &Target) -> Option<TypeId> {
        match t {
            Target::Entity(id) => self.world.entity(*id).and_then(Entity::type_id),
            Target::Memory(m) => m.snapshot.type_id(),
            Target::Square(_) | Target::Zoom(_) | Target::Xy(..) => None,
        }
    }

    /// Where a target is, as it knows it.
    pub(super) fn target_place(&self, t: &Target) -> TPlace {
        match t {
            Target::Entity(id) if !self.perceives(*id) => match self.known(*id).map(Seen::place) {
                Some(Some(p)) => TPlace::At(p),
                _ => TPlace::Nowhere,
            },
            _ => self.world.t_place(t),
        }
    }

    pub(super) fn target_place_of(&self, t: &Target) -> Option<Place> {
        match self.target_place(t) {
            TPlace::At(p) => Some(p),
            TPlace::World | TPlace::Nowhere => None,
        }
    }

    /// Whether it still knows the target: perceives it, or remembers it
    /// as the copy shows it.
    pub(super) fn is_known(&self, t: &Target) -> bool {
        match t {
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
        let is_vulnerable = self
            .world
            .entity(a)
            .and_then(Entity::type_id)
            .is_none_or(|t| self.world.rules.ty(t).stats.is_vulnerable != 0);
        Attacker {
            owner: None,
            is_vulnerable,
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

    /// A leak: the true objects of a square, in the order they came.
    pub(super) fn objects_on(&self, s: SquareId) -> &[EntityId] {
        &self.world.square(s).objects
    }

    /// A leak: whether an exit is closed by forests or by a blocker of its
    /// two sides, gates included, as they truly are.
    pub(super) fn exit_is_blocked(&self, e: EntityId) -> bool {
        self.world.exit_is_blocked(e, Viewer::Nobody, false, false)
    }

    // paths

    /// The ground distance by the shortest path, through the exits not
    /// closed by forests or by its own and its allies' walls; with
    /// `avoid`, around the squares it knows to be very dangerous.
    pub(super) fn distance(&mut self, start: SquareId, dest: SquareId, avoid: bool) -> Res<i64> {
        let me = Some(self.me);
        self.world
            .shortest_path_distance_to(start, dest, me, AirGround::Ground, avoid)
    }

    /// The ground distance by the shortest path through the exits not
    /// closed by forests, whatever blocks them.
    pub(super) fn distance_through_walls(&mut self, start: SquareId, dest: SquareId) -> Res<i64> {
        self.world
            .shortest_path_distance_to(start, dest, None, AirGround::Ground, false)
    }

    /// The squares of the exits on the shortest ground path through the
    /// exits not closed by forests, whatever blocks them.
    pub(super) fn path_through_walls(
        &mut self,
        start: SquareId,
        dest: SquareId,
    ) -> Res<Vec<SquareId>> {
        self.world
            .shortest_path_places(start, dest, None, AirGround::Ground, false)
    }

    /// Whether a ground path that avoids the squares it knows to be very
    /// dangerous leaves `start` toward `dest`.
    pub(super) fn has_a_safe_path(&mut self, start: SquareId, dest: SquareId) -> Res<bool> {
        let me = Some(self.me);
        Ok(self
            .world
            .shortest_path_to(start, dest, me, AirGround::Ground, true)?
            .is_some())
    }

    /// Its or an ally's unit storing the resource with the shortest path
    /// from `place`.
    pub(super) fn nearest_warehouse(
        &mut self,
        place: Place,
        resource_type: i64,
        include_building_sites: bool,
    ) -> Res<Option<EntityId>> {
        self.world
            .nearest_warehouse(self.me, place, resource_type, include_building_sites)
    }

    // acting

    /// Gives an order to one of its units, as a human's order is given:
    /// refused, with the sound of an impossible order, if the unit's menu
    /// does not have it.
    pub(super) fn give(&mut self, u: EntityId, r: Request, forget_previous: bool) -> Res {
        if self.unit(u).is_none() {
            return Err(Abort::new("not one of the player's units"));
        }
        self.world.take_order(u, r, forget_previous, false, None)
    }

    /// A change no order makes: cancels the orders of one of its units.
    pub(super) fn cancel_orders(&mut self, u: EntityId) {
        if self.unit(u).is_some() {
            self.world.cancel_all_orders(u, true);
        }
    }

    /// The handicap of the random computer: 1000 of each resource each
    /// time it plays, and every requirement met.
    pub(super) fn grant_handicap(&mut self) {
        let pl = self.world.plm(self.me);
        let n = pl.resources.len();
        pl.cheat_has_all = true;
        pl.resources = vec![1000 * crate::nofloat::PRECISION; n];
    }

    /// An index drawn at random below `len`, from the game's draws.
    pub(super) fn choice_index(&mut self, len: usize) -> Option<usize> {
        self.world.random.choice_index(len)
    }

    pub(super) fn warn(&mut self, s: impl Into<String>) {
        self.world.warn(s);
    }
}
