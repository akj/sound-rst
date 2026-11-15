//! What one player knows of a game, made only of what its seat was sent.
//!
//! A [`SeatView`] answers every question the interface asks during a game
//! from the [`GameStart`] sent once, the latest [`TurnMessage`], the
//! [`GameEnd`] once the game is over, and what the interface keeps of
//! earlier turns: the terrain of squares it no longer sees, and the
//! upgrades of a unit as it last perceived the unit. It never asks the
//! host anything. The order menus, costs, requirements, default orders,
//! blocked exits and comrades are computed by the same functions the
//! simulation uses.
//!
//! The questions come in five groups, each answered from one source:
//!
//! 1. **The game, known to everyone**: the map, the rules, the clock and
//!    whether cheats are allowed. From the [`GameStart`], and the time
//!    from the turn's message.
//! 2. **The player's own state**: resources, food, units, recallable
//!    groups, objectives, alliances, cheat mode, victory, and at the end
//!    the statistics.
//! 3. **What everyone may know of the other players**: names, who is
//!    allied with whom, who is still playing.
//! 4. **Entities the player perceives or remembers**, and what follows
//!    from them alone: whether an exit is closed, the units beside one that
//!    died, what a remembered unit was busy with.
//! 5. **Questions computed from what the player was sent** about the
//!    units it perceives or controls: order menus, order costs, what a
//!    unit is busy with, its upgrades, the default order for a target.
//!
//! The interface asks the questions of groups 1, 3 and 5 for the squares
//! the player observes or partly observes and the units it controls. For
//! other arguments a seat view answers from what the seat knows: nothing,
//! an empty list, or `None`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::command::OrderTarget;
use crate::entity::{EntityId, Place, PlayerId, SquareId};
use crate::known::{KnownEntity, KnownOrder, KnownTarget};
use crate::message::SquareInfo;
use crate::message::{ControlledPlayer, GameEnd, GameStart, TurnMessage};
use crate::msg::Msg;
use crate::orders::{Keyword, MenuEntry, OrderInfo, TargetRef};
use crate::player::{Objective, PlayerStats};
use crate::questions::{self, Controlled, Knowledge, PlayerFacts, Resolved};
use crate::rules::{Rules, TypeId, UnitType, VIRTUAL_TIME_INTERVAL};
use crate::world::{Abort, Res};

/// What one seat knows of a game, from what it was sent: see the
/// [module](self).
#[derive(Clone, Debug)]
pub struct SeatView {
    start: Arc<GameStart>,
    turn: Arc<TurnMessage>,
    end: Option<GameEnd>,
    /// The terrain of every square as the seat last took note of it.
    terrain: BTreeMap<SquareId, String>,
    /// The upgrades of every known creature with an owner, as the seat
    /// last took note of them while it perceived the creature.
    upgrades: BTreeMap<EntityId, Vec<String>>,
}

impl SeatView {
    /// The view of a seat that has been sent the start of the game and a
    /// first message: the state when it joins. It remembers nothing yet:
    /// see [`SeatView::remember`].
    pub fn new(start: Arc<GameStart>, first: impl Into<Arc<TurnMessage>>) -> SeatView {
        SeatView {
            start,
            turn: first.into(),
            end: None,
            terrain: BTreeMap::new(),
            upgrades: BTreeMap::new(),
        }
    }

    /// Takes a message: a turn's, or one the host sends between turns
    /// after a change made at once. Every answer is from it from now on,
    /// except what the seat remembers, which waits for
    /// [`SeatView::remember`].
    pub fn install(&mut self, message: impl Into<Arc<TurnMessage>>) {
        self.turn = message.into();
    }

    /// Takes the end of the game: the statistics.
    pub fn install_end(&mut self, end: GameEnd) {
        self.end = Some(end);
    }

    /// The latest message.
    pub fn message(&self) -> &TurnMessage {
        &self.turn
    }

    pub fn game_start(&self) -> &GameStart {
        &self.start
    }

    /// Takes note of what the latest message shows: the terrain of the
    /// squares the seat observes or partly observes, and the upgrades of
    /// the creatures it perceives. Forgets the upgrades of the entities it
    /// no longer perceives or remembers. The interface calls it when it
    /// handles the end of a turn, which may come a few passes of its loop
    /// after the turn's message was installed: until then it describes the
    /// squares and remembered units as it knew them before.
    pub fn remember(&mut self) {
        for (s, t) in &self.turn.squares.terrain {
            if self.terrain.get(s) != Some(t) {
                self.terrain.insert(*s, t.clone());
            }
        }
        for e in &self.turn.perceived {
            if e.is_creature() && e.player.is_some() {
                self.upgrades.insert(e.id, e.upgrades.clone());
            }
        }
        let turn = &self.turn;
        self.upgrades.retain(|&id, _| {
            find(&turn.perceived, id).is_some() || find(&turn.remembered, id).is_some()
        });
    }

    /// The player whose knowledge this is.
    pub fn player(&self) -> PlayerId {
        self.turn.seat
    }

    // 1. The game, known to everyone.

    /// Game time in milliseconds.
    pub fn time(&self) -> i64 {
        self.turn.time
    }

    /// Game time in turns.
    pub fn turn(&self) -> i64 {
        self.turn.time / VIRTUAL_TIME_INTERVAL
    }

    /// Whether the game allows cheats.
    pub fn cheats_allowed(&self) -> bool {
        self.start.cheats_allowed
    }

    /// The map's objective, as sound numbers. Empty if it has none.
    pub fn map_objective(&self) -> &[i64] {
        &self.start.map.objective
    }

    /// The map's introduction, as sound numbers. Empty if it has none.
    pub fn map_intro(&self) -> &[i64] {
        &self.start.map.intro
    }

    /// The width of a square, in millimeters.
    pub fn square_width(&self) -> i64 {
        self.start.map.square_width
    }

    /// The number of squares. Square ids go from 0 to this, excluded.
    pub fn square_count(&self) -> usize {
        self.start.map.squares.len()
    }

    /// The last column and the last row of the map.
    pub fn grid_max(&self) -> (i64, i64) {
        self.start.map.grid_max()
    }

    /// The square at a column and a row, from 0.
    pub fn square_at(&self, col: i64, row: i64) -> Option<SquareId> {
        self.start.map.square_at(col, row)
    }

    pub fn square(&self, id: SquareId) -> MapSquare<'_> {
        MapSquare::of(&self.start, id)
    }

    /// The terrain style of a square as the seat knows it: as it is now
    /// for a square it observes or partly observes, as it last saw it for
    /// a square in the fog, and nothing for a square it never saw.
    pub fn terrain(&self, square: SquareId) -> &str {
        self.terrain.get(&square).map_or("", String::as_str)
    }

    /// The place of the exit on the other side of `exit`, if `exit` is an
    /// exit with another side.
    pub fn other_side_of_exit(&self, exit: EntityId) -> Option<Place> {
        self.start.map.other_side_of_exit(exit)
    }

    /// The unit type of this name, from the rules.
    pub fn unit_type(&self, name: &str) -> Option<&UnitType> {
        self.start.rules.unit_type(name)
    }

    /// The unit type of this id, from the rules.
    pub fn unit_type_by_id(&self, id: TypeId) -> &UnitType {
        self.start.rules.ty(id)
    }

    // 2. The player's own state.

    /// The amount of each resource, in thousandths.
    pub fn resources(&self) -> &[i64] {
        &self.turn.state.resources
    }

    /// The food the player has, up to the map's limit.
    pub fn available_food(&self) -> i64 {
        self.turn.state.available_food
    }

    /// The food the player's units use.
    pub fn used_food(&self) -> i64 {
        self.turn.state.used_food
    }

    /// The player's units, in the order the player got them.
    pub fn own_units(&self) -> impl Iterator<Item = KnownEntity> + '_ {
        self.controlled_player(self.player())
            .into_iter()
            .flat_map(|c| self.units_now(c))
    }

    /// Whether the player may give orders to the units of `p`: its own,
    /// and an ally's that shares control.
    pub fn controls(&self, p: PlayerId) -> bool {
        self.controlled_player(p).is_some()
    }

    /// The units the player may give orders to: each controlled player's
    /// units in turn, the player's own among them.
    pub fn controlled_units(&self) -> impl Iterator<Item = KnownEntity> + '_ {
        self.turn
            .state
            .controlled
            .iter()
            .flat_map(|c| self.units_now(c))
    }

    /// The units of a recallable group that the player may give orders
    /// to, in the group's order.
    pub fn recallable_group(&self, name: &str) -> Vec<EntityId> {
        self.turn
            .state
            .groups
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, units)| units.clone())
            .unwrap_or_default()
    }

    /// The squares the player sees only partly, from a unit on the next
    /// square.
    pub fn partly_observed_squares(&self) -> &BTreeSet<SquareId> {
        &self.turn.squares.partly_observed
    }

    /// Whether `p` is the player or one of its allies.
    pub fn is_ally(&self, p: PlayerId) -> bool {
        self.allied(self.player(), p)
    }

    pub fn objectives(&self) -> &[Objective] {
        &self.turn.state.objectives
    }

    /// Whether the player's cheat mode is on.
    pub fn cheat_mode(&self) -> bool {
        self.turn.state.cheat_mode
    }

    pub fn has_victory(&self) -> bool {
        self.turn.state.has_victory
    }

    /// What the player produced, lost, killed and gathered, for the score:
    /// once the game has ended for it.
    pub fn stats(&self) -> Option<&PlayerStats> {
        self.end.as_ref().map(|e| &e.stats)
    }

    // 3. What everyone may know of the other players.

    /// The players still in the game, in seat order.
    pub fn players(&self) -> &[PlayerId] {
        &self.turn.state.players
    }

    /// The players defeated, in id order: those who still watch the game
    /// and those who left it.
    pub fn defeated(&self) -> &[PlayerId] {
        &self.turn.state.defeated
    }

    /// The player's name as it is said.
    pub fn player_name(&self, p: PlayerId) -> Msg {
        self.start
            .player_names
            .get(p.0 as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether `p` counts `q` among its allies. Every player counts itself.
    pub fn allied(&self, p: PlayerId, q: PlayerId) -> bool {
        self.allies(p).contains(&q)
    }

    // 4. Entities the player perceives or remembers.

    /// The entity as it is now, if the player perceives it.
    pub fn perceived(&self, id: EntityId) -> Option<KnownEntity> {
        find(&self.turn.perceived, id).cloned()
    }

    /// The player's memory of an entity it does not perceive now: as it
    /// was last seen, with when.
    pub fn remembered(&self, id: EntityId) -> Option<KnownEntity> {
        find(&self.turn.remembered, id).cloned()
    }

    /// The entity as the player knows it: as it is now if the player
    /// perceives it, else as it was last seen, with when.
    pub fn known(&self, id: EntityId) -> Option<KnownEntity> {
        self.known_ref(id).cloned()
    }

    fn known_ref(&self, id: EntityId) -> Option<&KnownEntity> {
        find(&self.turn.perceived, id).or_else(|| find(&self.turn.remembered, id))
    }

    /// The units of `owner` that the player perceives on `place`, `except`
    /// one that died there.
    pub fn comrades(&self, owner: PlayerId, place: Option<Place>, except: EntityId) -> Comrades {
        questions::comrades(&self.turn.perceived, owner, place, except)
    }

    /// What a remembered unit was busy with when the player last saw it.
    pub fn remembered_activity(&self, unit: EntityId) -> Option<String> {
        let e = find(&self.turn.remembered, unit)?;
        e.activity.as_ref().map(|a| a.name())
    }

    /// The upgrades a remembered creature had when the player last
    /// perceived it.
    pub fn remembered_upgrades(&self, unit: EntityId) -> &[String] {
        self.upgrades.get(&unit).map_or(&[], Vec::as_slice)
    }

    /// Whether an exit is closed as far as the player knows. With
    /// `friendly_gates_open`, a gate of the player or an ally does not
    /// close it.
    pub fn exit_is_blocked(&self, exit: EntityId, friendly_gates_open: bool) -> bool {
        questions::exit_is_blocked(
            |id| self.known_ref(id),
            |p| self.is_ally(p),
            exit,
            self.start.map.exit(exit).and_then(|x| x.other_side),
            friendly_gates_open,
        )
    }

    // 5. Questions computed from what the player was sent.

    /// The entries of a unit's order menu for one keyword. With `strict`,
    /// only the orders the unit may receive now. Nothing for a unit the
    /// player does not control.
    pub fn order_menu(&self, unit: EntityId, keyword: Keyword, strict: bool) -> Vec<MenuEntry> {
        match self.unit(unit) {
            Some(u) => questions::order_menu(self, u, keyword, strict),
            None => Vec::new(),
        }
    }

    /// The costs of an order and the number of targets it takes, for a
    /// unit the player controls. `None` if the order makes no sense.
    pub fn order_info(
        &self,
        unit: EntityId,
        keyword: Keyword,
        type_id: Option<TypeId>,
    ) -> Option<OrderInfo> {
        questions::order_info(&self.start.rules, &self.unit(unit)?, keyword, type_id)
    }

    /// Whether the owner of a unit has a type, built or researched. `None`
    /// unless the player controls the owner's units.
    pub fn owner_has(&self, unit: EntityId, t: TypeId) -> Option<bool> {
        let owner = self.unit(unit)?.known.player?;
        let name = &self.start.rules.ty(t).name;
        Some(questions::has(self, self.player_facts(owner)?, name))
    }

    /// What a unit the player perceives is busy with, for its noise.
    pub fn activity(&self, unit: EntityId) -> Option<String> {
        let e = find(&self.turn.perceived, unit)?;
        e.activity.as_ref().map(|a| a.name())
    }

    /// The upgrades of its owner that apply to a unit the player
    /// perceives. None for a unit it does not perceive: see
    /// [`SeatView::remembered_upgrades`].
    pub fn unit_upgrades(&self, unit: EntityId) -> Vec<String> {
        find(&self.turn.perceived, unit).map_or_else(Vec::new, |e| e.upgrades.clone())
    }

    /// The keyword of the order a unit the player controls would choose
    /// for `target` if it received the default order now. The target is
    /// resolved through what the player perceives or remembers. Nothing
    /// for a unit the player does not control.
    pub fn default_order(
        &self,
        unit: EntityId,
        target: OrderTarget,
    ) -> Result<Option<Keyword>, Abort> {
        match self.unit(unit) {
            Some(u) => questions::default_keyword(self, u, &TargetRef::from(target)),
            None => Ok(None),
        }
    }

    // lookups

    fn controlled_player(&self, p: PlayerId) -> Option<&ControlledPlayer> {
        self.turn.state.controlled.iter().find(|c| c.player == p)
    }

    fn allies(&self, p: PlayerId) -> &[PlayerId] {
        self.turn
            .state
            .allies
            .get(p.0 as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// A unit the player controls, as it is now.
    fn unit_now(&self, id: EntityId) -> Option<&KnownEntity> {
        find(&self.turn.perceived, id)
            .filter(|e| e.own.is_some())
            .or_else(|| find(&self.turn.unperceived_units, id))
    }

    fn units_now<'s>(&'s self, c: &'s ControlledPlayer) -> impl Iterator<Item = KnownEntity> + 's {
        c.units.iter().filter_map(|&u| self.unit_now(u)).cloned()
    }
}

/// A square of the map, as everyone knows it: its place, name and shape.
/// What is on the square is not part of it. Coordinates are in
/// millimeters.
#[derive(Clone, Copy)]
pub struct MapSquare<'a> {
    square: &'a SquareInfo,
}

impl<'a> MapSquare<'a> {
    /// The square's id in the text protocol.
    pub fn protocol_id(&self) -> u32 {
        self.square.protocol_id
    }

    /// The column, from 0.
    pub fn col(&self) -> i64 {
        self.square.col
    }

    /// The row, from 0.
    pub fn row(&self) -> i64 {
        self.square.row
    }

    /// `a1`, `b3`...
    pub fn name(&self) -> &'a str {
        &self.square.name
    }

    /// The column sound and the row number.
    pub fn title(&self) -> &'a Msg {
        &self.square.title
    }

    /// The center.
    pub fn center(&self) -> (i64, i64) {
        self.square.center
    }

    /// The smallest x and y inside the square.
    pub fn min(&self) -> (i64, i64) {
        self.square.min
    }

    /// The x and y just past the square.
    pub fn max(&self) -> (i64, i64) {
        self.square.max
    }

    pub fn high_ground(&self) -> bool {
        self.square.high_ground
    }

    pub fn is_water(&self) -> bool {
        self.square.is_water
    }

    /// The eight squares around, fewer at an edge.
    pub fn neighbors(&self) -> &'a [SquareId] {
        &self.square.neighbors
    }
}

impl<'a> MapSquare<'a> {
    /// The square `id` of a map. A square id comes from the same map.
    pub(crate) fn of(start: &'a GameStart, id: SquareId) -> MapSquare<'a> {
        let square = start
            .map
            .square(id)
            .expect("a square id names a square of its map");
        MapSquare { square }
    }
}

/// The units of a player that the player perceives on the place where one
/// of theirs died: see [`SeatView::comrades`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comrades {
    /// How many there are.
    pub units: usize,
    /// How many of them can fight.
    pub soldiers: usize,
}

/// The entity of this id in a list by id.
fn find(list: &[KnownEntity], id: EntityId) -> Option<&KnownEntity> {
    let i = list.binary_search_by_key(&id, |e| e.id).ok()?;
    list.get(i)
}

/// What a target means to a seat, for the rules: only whether it names an
/// entity matters to them.
#[derive(Clone, Copy)]
pub(crate) enum SeatTarget {
    Place,
    Perceived(EntityId),
    Remembered(EntityId),
}

/// A seat knows the units it controls as they are now. It resolves a
/// target through what it perceives or remembers, as the unit's owner
/// would. For an ally's unit it also reads the ally's own units as they are
/// now, since a player perceives all its own units out of cheat mode.
/// Allies see alike, so an ally's knowledge of anything else differs from
/// the seat's only for a unit inside something, or when one of them is in
/// cheat mode: there the seat answers from its own knowledge, since it is
/// not sent the ally's.
impl<'a> Knowledge<'a> for &'a SeatView {
    type Entity = &'a KnownEntity;
    type Unit = Controlled<'a>;
    type Order = KnownOrder;
    type Target = SeatTarget;

    fn rules(self) -> &'a Rules {
        &self.start.rules
    }

    fn unit(self, id: EntityId) -> Option<Controlled<'a>> {
        Controlled::of(self.unit_now(id)?)
    }

    fn player_facts(self, p: PlayerId) -> Option<PlayerFacts<'a>> {
        let c = self.controlled_player(p)?;
        Some(PlayerFacts {
            upgrades: &c.upgrades,
            forbidden_techs: &c.forbidden_techs,
            has_everything: c.has_everything,
            is_human: c.is_human,
            units: &c.units,
            allied: self.allies(p),
        })
    }

    fn resolve(
        self,
        owner: PlayerId,
        target: &TargetRef,
    ) -> Res<Option<Resolved<SeatTarget, &'a KnownEntity>>> {
        let map = &self.start.map;
        let place = Resolved {
            target: SeatTarget::Place,
            entity: None,
        };
        match *target {
            TargetRef::Square(_) | TargetRef::SquareNamed(_) => Ok(Some(place)),
            TargetRef::Point { square, x, y } => {
                if !map.square(square).is_some_and(|s| s.contains(x, y)) {
                    return Err(Abort::new("the zoom is outside its square"));
                }
                Ok(Some(place))
            }
            TargetRef::Entity(id) => {
                if map.square_by_protocol_id(id.0).is_some() {
                    return Ok(Some(place));
                }
                let owners = owner != self.player()
                    && self
                        .controlled_player(owner)
                        .is_some_and(|c| c.units.contains(&id));
                let now = owners.then(|| self.unit_now(id)).flatten();
                if let Some(e) = now
                    .or_else(|| find(&self.turn.perceived, id))
                    .filter(|e| e.place.is_some())
                {
                    return Ok(Some(Resolved {
                        target: SeatTarget::Perceived(id),
                        entity: Some(e),
                    }));
                }
                Ok(find(&self.turn.remembered, id).map(|e| Resolved {
                    target: SeatTarget::Remembered(id),
                    entity: Some(e),
                }))
            }
            TargetRef::Nothing => Ok(None),
        }
    }

    /// The seat does not know whether a remembered target is the same copy
    /// as the one an order aims at, as the world does: the same entity is
    /// enough.
    fn aims_at(self, order: &KnownOrder, target: &SeatTarget) -> bool {
        match (&order.target, target) {
            (Some(KnownTarget::Entity(a)), SeatTarget::Perceived(b)) => a == b,
            (Some(KnownTarget::Remembered(r)), SeatTarget::Remembered(b)) => r.id == *b,
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
