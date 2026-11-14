//! What one player knows of a game, as the interface reads it.
//!
//! A [`View`] answers the interface's questions for one player. It is got
//! from the [`Host`](crate::Host) with [`Host::view`](crate::Host::view)
//! and borrows the host's world for as long as it is used: it never hands
//! out the world, a [`Player`](crate::Player), or anything that changes
//! the game. Its answers are facts: small owned values, ids, or borrowed
//! data that a copy sent over a network could carry as well.
//!
//! The methods come in five groups. The groups are the contract for a view
//! that is sent rather than borrowed: each says where its answers would
//! come from.
//!
//! 1. **The game, known to everyone**: the map (its squares, their names,
//!    neighbours, exits and terrain, the map's objective and introduction),
//!    the rules (unit types and costs), the clock and whether cheats are
//!    allowed. A host sends the map and the rules once, at the start.
//! 2. **The player's own state**: resources, food, units, upgrades,
//!    recallable groups, objectives, alliances, cheat mode, victory and
//!    statistics. Sent to that player only.
//! 3. **What everyone may know of the other players**: names, who is
//!    allied with whom, who is still playing.
//! 4. **Entities the player perceives or remembers**, looked up through
//!    that knowledge, and what follows from them alone: whether an exit
//!    is closed, the units beside one that died, what a remembered unit
//!    was busy with. Nothing else is returned or read.
//! 5. **Questions computed on demand** about units the player perceives or
//!    controls: order menus, order costs, what a unit is busy with, its
//!    upgrades, the default order for a target. A host in the same process
//!    answers them from the world; a host over a network has to send the
//!    answers or the inputs they are computed from.
//!
//! The methods of groups 1, 3 and 5 do not check their arguments against
//! what the player knows: they answer for the square, player or unit they
//! are given. Their documentation says for which arguments the answer is
//! the player's knowledge, and the interface asks them only for those.

use std::collections::BTreeSet;

use crate::command::OrderTarget;
use crate::entity::{Entity, EntityId, Place, PlayerId, SquareId, Target};
use crate::msg::Msg;
use crate::orders::{Keyword, Mode, OrderInfo};
use crate::player::{Objective, PlayerStats, Remembrance};
use crate::rules::{TypeId, UnitType};
use crate::square::Square;
use crate::world::{Abort, World};

/// What one player knows of a game: see the [module](self) for its groups.
#[derive(Clone, Copy)]
pub struct View<'a> {
    world: &'a World,
    player: PlayerId,
}

/// A square of the map, as everyone knows it: its place, name and shape.
/// What is on the square is not part of it. Coordinates are in
/// millimeters.
#[derive(Clone, Copy)]
pub struct MapSquare<'a> {
    square: &'a Square,
}

impl<'a> MapSquare<'a> {
    /// The square's id in the text protocol.
    pub fn protocol_id(&self) -> u32 {
        self.square.id
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
        (self.square.x, self.square.y)
    }

    /// The smallest x and y inside the square.
    pub fn min(&self) -> (i64, i64) {
        (self.square.xmin, self.square.ymin)
    }

    /// The x and y just past the square.
    pub fn max(&self) -> (i64, i64) {
        (self.square.xmax, self.square.ymax)
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

impl<'a> View<'a> {
    pub(crate) fn new(world: &'a World, player: PlayerId) -> View<'a> {
        View { world, player }
    }

    /// The player whose knowledge this is.
    pub fn player(&self) -> PlayerId {
        self.player
    }

    // 1. The game, known to everyone.

    /// Game time in milliseconds.
    pub fn time(&self) -> i64 {
        self.world.time
    }

    /// Game time in turns.
    pub fn turn(&self) -> i64 {
        self.world.turn()
    }

    /// Whether the game allows cheats.
    pub fn cheats_allowed(&self) -> bool {
        self.world.allow_cheatmode
    }

    /// The map's objective, as sound numbers. Empty if it has none.
    pub fn map_objective(&self) -> &'a [i64] {
        &self.world.map.objective
    }

    /// The map's introduction, as sound numbers. Empty if it has none.
    pub fn map_intro(&self) -> &'a [i64] {
        &self.world.map.intro
    }

    /// The width of a square, in millimeters.
    pub fn square_width(&self) -> i64 {
        self.world.square_width()
    }

    /// The number of squares. Square ids go from 0 to this, excluded.
    pub fn square_count(&self) -> usize {
        self.world.squares.len()
    }

    /// The last column and the last row of the map.
    pub fn grid_max(&self) -> (i64, i64) {
        let squares = &self.world.squares;
        (
            squares.iter().map(|s| s.col).max().unwrap_or(0),
            squares.iter().map(|s| s.row).max().unwrap_or(0),
        )
    }

    /// The square at a column and a row, from 0.
    pub fn square_at(&self, col: i64, row: i64) -> Option<SquareId> {
        self.world.square_at(col, row)
    }

    pub fn square(&self, id: SquareId) -> MapSquare<'a> {
        MapSquare {
            square: self.world.square(id),
        }
    }

    /// The terrain style of a square: the map's `terrain`, or for a square
    /// without one, `_meadows`, `_forest`, `_dense_forest` or nothing,
    /// from the woods and meadows on it now. The player knows it for a
    /// square it observes now: the interface keeps the terrain of a square
    /// in the fog as it last observed it.
    pub fn terrain(&self, square: SquareId) -> &'a str {
        &self.world.square(square).type_name
    }

    /// The place of the exit on the other side of `exit`, if `exit` is an
    /// exit with another side. Exits are part of the map and do not move.
    pub fn other_side_of_exit(&self, exit: EntityId) -> Option<Place> {
        let w = self.world;
        let other = w.entity(exit)?.exit.as_ref()?.other_side?;
        w.entity(other)?.place
    }

    /// The unit type of this name, from the rules.
    pub fn unit_type(&self, name: &str) -> Option<&'a UnitType> {
        self.world.rules.unit_type(name)
    }

    /// The unit type of this id, from the rules: the type a building site
    /// becomes, for example.
    pub fn unit_type_by_id(&self, id: TypeId) -> &'a UnitType {
        self.world.rules.ty(id)
    }

    // 2. The player's own state.

    /// The amount of each resource, in thousandths.
    pub fn resources(&self) -> &'a [i64] {
        &self.world.player(self.player).resources
    }

    /// The food the player has, up to the map's limit.
    pub fn available_food(&self) -> i64 {
        self.world
            .player(self.player)
            .available_food(self.world.food_limit())
    }

    /// The food the player's units use.
    pub fn used_food(&self) -> i64 {
        self.world.player(self.player).used_food
    }

    /// The player's units, in the order the player got them.
    pub fn own_units(&self) -> impl Iterator<Item = &'a Entity> + use<'a> {
        let w = self.world;
        w.player(self.player)
            .units
            .iter()
            .filter_map(move |&u| w.entity(u))
    }

    /// Whether the player may give orders to the units of `p`: its own,
    /// and an ally's that shares control.
    pub fn controls(&self, p: PlayerId) -> bool {
        self.world.player(self.player).allied_control.contains(&p)
    }

    /// The units the player may give orders to: each controlled player's
    /// units in turn, the player's own among them.
    pub fn controlled_units(&self) -> impl Iterator<Item = &'a Entity> + use<'a> {
        let w = self.world;
        w.player(self.player)
            .allied_control
            .iter()
            .flat_map(move |&p| w.player(p).units.iter())
            .filter_map(move |&u| w.entity(u))
    }

    /// The units of a recallable group that the player may give orders
    /// to, in the group's order. The groups of a game are shared by every
    /// player, as in the original: a group of this name may also hold
    /// other players' units, which are left out.
    pub fn recallable_group(&self, name: &str) -> Vec<EntityId> {
        let w = self.world;
        let units = w.group_list(name).map_or(&[][..], Vec::as_slice);
        units
            .iter()
            .copied()
            .filter(|&u| {
                w.entity(u)
                    .and_then(|e| e.player)
                    .is_some_and(|p| self.controls(p))
            })
            .collect()
    }

    /// The squares the player sees only partly at the end of the turn,
    /// from a unit on the next square: not observed, but seen enough to
    /// know their terrain. The squares it observes come with each turn's
    /// report.
    pub fn partly_observed_squares(&self) -> &'a BTreeSet<SquareId> {
        &self.world.player(self.player).partly_observed_squares
    }

    /// Whether `p` is the player or one of its allies.
    pub fn is_ally(&self, p: PlayerId) -> bool {
        self.allied(self.player, p)
    }

    pub fn objectives(&self) -> &'a [Objective] {
        &self.world.player(self.player).objectives
    }

    /// Whether the player's cheat mode is on.
    pub fn cheat_mode(&self) -> bool {
        self.world.player(self.player).cheatmode
    }

    pub fn has_victory(&self) -> bool {
        self.world.player(self.player).has_victory
    }

    /// What the player produced, lost, killed and gathered, for the score.
    pub fn stats(&self) -> &'a PlayerStats {
        &self.world.player(self.player).stats
    }

    // 3. What everyone may know of the other players.

    /// The players still in the game, in seat order.
    pub fn players(&self) -> &'a [PlayerId] {
        &self.world.players
    }

    /// The player's name as it is said.
    pub fn player_name(&self, p: PlayerId) -> Msg {
        self.world.player(p).name()
    }

    /// Whether `p` counts `q` among its allies. Every player counts itself.
    pub fn allied(&self, p: PlayerId, q: PlayerId) -> bool {
        self.world.player(p).allied.contains(&q)
    }

    // 4. Entities the player perceives or remembers.

    /// The entity as it is now, if the player perceives it.
    pub fn perceived(&self, id: EntityId) -> Option<&'a Entity> {
        self.world
            .player(self.player)
            .perception
            .contains(&id)
            .then(|| self.world.entity(id))
            .flatten()
    }

    /// The player's memory of an entity it does not perceive now: a copy
    /// as it was last seen, and when.
    pub fn remembered(&self, id: EntityId) -> Option<&'a Remembrance> {
        self.world.player(self.player).memory.get(&id)
    }

    /// The entity as the player knows it: as it is now if the player
    /// perceives it, else as it was last seen, with when.
    pub fn known(&self, id: EntityId) -> Option<(&'a Entity, Option<i64>)> {
        match self.perceived(id) {
            Some(e) => Some((e, None)),
            None => self
                .remembered(id)
                .map(|r| (&*r.snapshot, Some(r.time_stamp))),
        }
    }

    /// The units of `owner` that the player perceives on `place`, `except`
    /// one that died there: what the player sees left of its side of the
    /// fight.
    pub fn comrades(&self, owner: PlayerId, place: Option<Place>, except: EntityId) -> Comrades {
        let w = self.world;
        let mut c = Comrades {
            units: 0,
            soldiers: 0,
        };
        let perceived = w.player(self.player).perception.iter();
        for u in perceived.filter_map(|&u| w.entity(u)) {
            if u.player == Some(owner) && u.place == place && u.id != except {
                c.units += 1;
                if u.menace() != 0 {
                    c.soldiers += 1;
                }
            }
        }
        c
    }

    /// What a remembered unit was busy with when the player last saw it,
    /// as [`View::activity`] says it of a perceived one, from the
    /// remembered copy's first order. What it gathers from is named as the
    /// player knows it: `None` if the player knows nothing of it.
    pub fn remembered_activity(&self, unit: EntityId) -> Option<String> {
        let o = self.remembered(unit)?.snapshot.orders.first()?;
        match o.mode {
            Mode::Build => Some("building".into()),
            Mode::Gather => {
                let name = match o.target.as_ref()? {
                    Target::Entity(id) => &self.known(*id)?.0.type_name,
                    Target::Memory(m) => &m.snapshot.type_name,
                    _ => return None,
                };
                Some(format!("exploiting_{name}"))
            }
            _ => None,
        }
    }

    /// Whether an exit is closed as far as the player knows: by dense
    /// forests, or by a blocker on either side, as the player perceives or
    /// remembers the exits and the blockers. With `friendly_gates_open`, a
    /// gate of the player or an ally does not close it. An exit the player
    /// knows nothing of is open.
    pub fn exit_is_blocked(&self, exit: EntityId, friendly_gates_open: bool) -> bool {
        let Some(x) = self.known(exit).and_then(|(e, _)| e.exit.as_ref()) else {
            return false;
        };
        if x.is_blocked_by_forests {
            return true;
        }
        let other = x
            .other_side
            .and_then(|o| self.known(o))
            .and_then(|(o, _)| o.exit.as_ref())
            .map_or(&[][..], |o| &o.blockers);
        x.blockers.iter().chain(other).any(|&b| {
            let Some((b, _)) = self.known(b) else {
                return false;
            };
            let enemy = !friendly_gates_open || b.player.is_some_and(|bp| !self.is_ally(bp));
            b.stats.is_a_gate == 0 || enemy
        })
    }

    // 5. Questions computed on demand.

    /// The entries of a unit's order menu for one keyword: `go`, or `train
    /// footman`, `train archer`... With `strict`, only the orders the unit
    /// may receive now; without, also those waiting for a requirement or
    /// resources. Nothing for an unknown unit. For a unit the player
    /// controls.
    pub fn order_menu(&self, unit: EntityId, keyword: Keyword, strict: bool) -> Vec<String> {
        if self.world.entity(unit).is_none() {
            return Vec::new();
        }
        self.world.order_menu(unit, keyword, strict)
    }

    /// The costs of an order and the number of targets it takes, for a
    /// unit the player controls. `None` if the order makes no sense.
    pub fn order_info(
        &self,
        unit: EntityId,
        keyword: Keyword,
        type_name: Option<&str>,
    ) -> Option<OrderInfo> {
        self.world.order_info(unit, keyword, type_name)
    }

    /// Whether the owner of a unit has a type, built or researched: what
    /// an order of that unit requires. `None` unless the player controls
    /// the owner's units: another player's state is not the player's to
    /// know.
    pub fn owner_has(&self, unit: EntityId, type_name: &str) -> Option<bool> {
        let owner = self.world.entity(unit)?.player?;
        self.controls(owner)
            .then(|| self.world.has(owner, type_name))
    }

    /// What a unit the player perceives is busy with, for its noise:
    /// `building`, or `exploiting_` and what it gathers from. None for a
    /// unit the player does not perceive; for a remembered one, see
    /// [`View::remembered_activity`].
    pub fn activity(&self, unit: EntityId) -> Option<String> {
        self.perceived(unit)?;
        self.world.activity(unit)
    }

    /// The upgrades of its owner that apply to a unit the player
    /// perceives, for its description. None for a unit the player does not
    /// perceive: the interface describes a remembered unit with those it
    /// was told when it last perceived the unit.
    pub fn unit_upgrades(&self, unit: EntityId) -> Vec<String> {
        if self.perceived(unit).is_none() {
            return Vec::new();
        }
        self.world.unit_upgrades(unit)
    }

    /// The keyword of the order a unit the player controls would choose
    /// for `target` if it received the default order now. The target is
    /// resolved through what the unit's owner perceives or remembers. The
    /// error is the world's.
    pub fn default_order(
        &self,
        unit: EntityId,
        target: OrderTarget,
    ) -> Result<Option<Keyword>, Abort> {
        let id = target.text(self.world);
        Ok(self
            .world
            .get_default_order(unit, &id)?
            .and_then(Keyword::parse))
    }
}

/// The units of a player that the player perceives on the place where one
/// of theirs died: see [`View::comrades`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comrades {
    /// How many there are.
    pub units: usize,
    /// How many of them can fight.
    pub soldiers: usize,
}

#[cfg(test)]
mod tests;
