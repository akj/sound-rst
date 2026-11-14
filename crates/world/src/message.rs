//! What a seat is sent: once when the game starts ([`GameStart`]), with
//! every turn ([`TurnMessage`]), and once when the game ends
//! ([`GameEnd`]).
//!
//! These are plain owned data with no reference into the world, made to be
//! sent. An interface needs nothing else to answer its player's questions:
//! [`SeatView`](crate::SeatView) answers them from these and the rules,
//! and asks the host nothing during play.
//!
//! A turn's message is everything as of the end of that turn. It is whole,
//! not a change since the last one: sending only what changed is a matter
//! of encoding. Lists said to be "by id" are sorted by id, with each id
//! once. Lengths are in millimeters, hit points, mana, quantities and
//! resources in thousandths, and times in milliseconds of game time, as in
//! the world.

use std::collections::{BTreeMap, BTreeSet};

use crate::entity::{EntityId, Place, PlayerId, SquareId};
use crate::events::Event;
use crate::known::KnownEntity;
use crate::msg::Msg;
use crate::player::{Objective, PlayerStats};
use crate::rules::Rules;
use crate::world::World;

/// What every seat knows from the start of the game: what does not change
/// during a game. The host sends the [`StartMessage`]; the seat's client
/// supplies the rules itself ([`GameStart::from_message`]).
#[derive(Clone, Debug)]
pub struct GameStart {
    /// The rules of the game, every layer applied: the game's, a mod's, a
    /// campaign's and the map's. The interface reads unit types, costs and
    /// requirements from them, and computes order menus with them.
    ///
    /// Never sent: every client of a game has loaded the same layers from
    /// its own files, which the room's content hash checks.
    pub rules: Rules,
    pub map: GameMap,
    /// Every player's name as it is said, by player id: the name of
    /// `PlayerId(i)` is at `i`. A neutral player's is empty.
    pub player_names: Vec<Msg>,
    /// Whether the game allows cheats.
    pub cheats_allowed: bool,
}

/// What the host sends every seat once, when the game starts: the
/// [`GameStart`] without its rules, which the seat's client supplies.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StartMessage {
    pub map: GameMap,
    /// As [`GameStart::player_names`].
    pub player_names: Vec<Msg>,
    pub cheats_allowed: bool,
}

impl GameStart {
    /// What the host sends of it.
    pub fn message(&self) -> StartMessage {
        StartMessage {
            map: self.map.clone(),
            player_names: self.player_names.clone(),
            cheats_allowed: self.cheats_allowed,
        }
    }

    /// The start of the game from what the host sent and the rules the
    /// client loaded.
    pub fn from_message(message: StartMessage, rules: Rules) -> GameStart {
        GameStart {
            rules,
            map: message.map,
            player_names: message.player_names,
            cheats_allowed: message.cheats_allowed,
        }
    }
}

/// The map as every player knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GameMap {
    /// The width of a square.
    pub square_width: i64,
    /// Every square, by id: square `SquareId(i)` is at `i`.
    pub squares: Vec<SquareInfo>,
    /// Every exit, by id.
    pub exits: Vec<MapExit>,
    /// The map's objective, as sound numbers. Empty if it has none.
    pub objective: Vec<i64>,
    /// The map's introduction, as sound numbers. Empty if it has none.
    pub intro: Vec<i64>,
}

/// A square of the map: its place, name and shape. What is on it is not
/// part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SquareInfo {
    /// Its id in the text protocol. An order may name the square by it.
    pub protocol_id: u32,
    /// The column, from 0.
    pub col: i64,
    /// The row, from 0.
    pub row: i64,
    /// `a1`, `b3`...
    pub name: String,
    /// The column sound and the row number.
    pub title: Msg,
    /// The x and y of its center.
    pub center: (i64, i64),
    /// The smallest x and y inside it.
    pub min: (i64, i64),
    /// The x and y just past it.
    pub max: (i64, i64),
    pub high_ground: bool,
    pub is_water: bool,
    /// The eight squares around, fewer at an edge.
    pub neighbors: Vec<SquareId>,
}

impl SquareInfo {
    /// Whether the point is inside the square.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        self.min.0 <= x && x < self.max.0 && self.min.1 <= y && y < self.max.1
    }
}

/// An exit: one side of a path, a bridge or a portal. Exits are made with
/// the map and do not move. What closes one changes, and is told with the
/// exit when the player knows it ([`KnownExit`](crate::KnownExit)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MapExit {
    pub id: EntityId,
    /// The square it is on.
    pub square: SquareId,
    /// The exit on the other side, on the next square.
    pub other_side: Option<EntityId>,
}

impl GameMap {
    pub fn square(&self, id: SquareId) -> Option<&SquareInfo> {
        self.squares.get(id.0 as usize)
    }

    /// The square at a column and a row, from 0.
    pub fn square_at(&self, col: i64, row: i64) -> Option<SquareId> {
        let i = self
            .squares
            .iter()
            .position(|s| s.col == col && s.row == row)?;
        Some(SquareId(i as u32))
    }

    /// The square with this id in the text protocol.
    pub fn square_by_protocol_id(&self, id: u32) -> Option<SquareId> {
        let i = self.squares.iter().position(|s| s.protocol_id == id)?;
        Some(SquareId(i as u32))
    }

    /// The last column and the last row.
    pub fn grid_max(&self) -> (i64, i64) {
        (
            self.squares.iter().map(|s| s.col).max().unwrap_or(0),
            self.squares.iter().map(|s| s.row).max().unwrap_or(0),
        )
    }

    pub fn exit(&self, id: EntityId) -> Option<&MapExit> {
        let i = self.exits.binary_search_by_key(&id, |x| x.id).ok()?;
        self.exits.get(i)
    }

    /// The place of the exit on the other side of `exit`, if `exit` is an
    /// exit with another side.
    pub fn other_side_of_exit(&self, exit: EntityId) -> Option<Place> {
        let other = self.exit(exit)?.other_side?;
        Some(Place::Square(self.exit(other)?.square))
    }
}

/// What one seat is told at the end of a turn: everything its interface
/// reads until the next one. The interface installs it when it takes the
/// turn.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TurnMessage {
    /// The player the message is for.
    pub seat: PlayerId,
    /// The game time at the end of the turn. The turn is this divided by
    /// [`VIRTUAL_TIME_INTERVAL`](crate::VIRTUAL_TIME_INTERVAL).
    pub time: i64,
    /// What the seat perceived during the turn, in order. An event about
    /// an entity carries the entity as the seat knew it at that moment.
    /// The end of the turn, which [`Event::Voila`] tells, is the rest of
    /// this message and is not among them. Only a local human receives
    /// events: for any other seat this is empty.
    pub events: Vec<Event>,
    pub state: SeatState,
    pub squares: KnownSquares,
    /// Every entity the seat perceives, as it is now, by id.
    pub perceived: Vec<KnownEntity>,
    /// Every entity the seat remembers and does not perceive, as it was
    /// when last seen, by id. An entity can be both remembered and among
    /// [`TurnMessage::unperceived_units`].
    pub remembered: Vec<KnownEntity>,
    /// The units the seat controls and does not perceive, as they are now,
    /// by id: an ally's unit inside an ally's transport or shelter, and in
    /// cheat mode any unit inside one. Each has its own part.
    pub unperceived_units: Vec<KnownEntity>,
}

/// The seat's own state, and what everyone may know of the players.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeatState {
    /// The amount of each resource.
    pub resources: Vec<i64>,
    /// The food the seat has, up to the map's limit.
    pub available_food: i64,
    /// The food the seat's units use.
    pub used_food: i64,
    /// Whether the seat's cheat mode is on: it then observes every square.
    pub cheat_mode: bool,
    pub has_victory: bool,
    pub objectives: Vec<Objective>,
    /// The players still in the game, in seat order.
    pub players: Vec<PlayerId>,
    /// The players defeated, in id order: those who still watch the game
    /// and those who left it.
    pub defeated: Vec<PlayerId>,
    /// Each player's allies, the player among them, by player id: the
    /// allies of `PlayerId(i)` are at `i`. Every player of the game is
    /// here, those who left too.
    pub allies: Vec<Vec<PlayerId>>,
    /// The players whose units the seat may give orders to, the seat among
    /// them, with what the order menus of their units read.
    pub controlled: Vec<ControlledPlayer>,
    /// The recallable groups, each cut down to the units the seat
    /// controls, in the order the groups were made. A group left with no
    /// unit is left out.
    pub groups: Vec<(String, Vec<EntityId>)>,
}

/// A player whose units the seat may give orders to: the seat, or an ally
/// that shares control. What its units' order menus and requirements read
/// of it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ControlledPlayer {
    pub player: PlayerId,
    /// Its units, in the order it got them. Each is among the perceived
    /// entities or the unperceived units.
    pub units: Vec<EntityId>,
    /// The upgrades it researched; one researched twice is here twice.
    pub upgrades: Vec<String>,
    /// The types it may not build, train, research or use.
    pub forbidden_techs: Vec<String>,
    /// It has everything any requirement asks for: the random computer
    /// player, or the all-techs cheat.
    pub has_everything: bool,
    /// A person rather than a computer. Only a computer's units may be
    /// told to attack, explore or wait by themselves.
    pub is_human: bool,
}

/// What the seat knows of the squares at the end of the turn.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KnownSquares {
    /// The squares the seat observes. Every square in cheat mode.
    pub observed: BTreeSet<SquareId>,
    /// The squares the seat has observed, at least partly, since the game
    /// started. Every square in cheat mode.
    pub observed_before: BTreeSet<SquareId>,
    /// The squares the seat sees only partly, from a unit on the next
    /// square: not observed, but seen enough to know their terrain.
    pub partly_observed: BTreeSet<SquareId>,
    /// The terrain style of each square observed or partly observed: the
    /// map's `terrain`, or `_meadows`, `_forest`, `_dense_forest` or
    /// nothing, from what grows on the square now. The interface keeps the
    /// terrain of a square in the fog as it last saw it.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::pairs"))]
    pub terrain: BTreeMap<SquareId, String>,
}

/// What a seat is sent once, when the game ends for it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GameEnd {
    /// What the seat produced, lost, killed and gathered, for the score.
    /// The result is the last turn's [`SeatState::has_victory`].
    pub stats: PlayerStats,
}

impl GameStart {
    /// What the world's game is made of, for every seat.
    pub(crate) fn of(w: &World) -> GameStart {
        let squares = w
            .squares
            .iter()
            .map(|s| SquareInfo {
                protocol_id: s.id,
                col: s.col,
                row: s.row,
                name: s.name.clone(),
                title: s.title.clone(),
                center: (s.x, s.y),
                min: (s.xmin, s.ymin),
                max: (s.xmax, s.ymax),
                high_ground: s.high_ground,
                is_water: s.is_water,
                neighbors: s.neighbors.clone(),
            })
            .collect();
        let exits = w
            .entities()
            .filter_map(|e| {
                Some(MapExit {
                    id: e.id,
                    square: e.square()?,
                    other_side: e.exit.as_ref()?.other_side,
                })
            })
            .collect();
        GameStart {
            rules: w.rules.clone(),
            map: GameMap {
                square_width: w.square_width(),
                squares,
                exits,
                objective: w.map.objective.clone(),
                intro: w.map.intro.clone(),
            },
            player_names: w.all_players().map(|(_, p)| p.name()).collect(),
            cheats_allowed: w.allow_cheatmode,
        }
    }
}

impl World {
    /// What `seat` is told at the end of a turn, with the turn's events.
    /// It reads the world and changes nothing.
    pub(crate) fn turn_message(&self, seat: PlayerId, events: Vec<Event>) -> TurnMessage {
        let pl = self.player(seat);
        let now = |e| KnownEntity::of(self, seat, e, None);
        let perceived = pl
            .perception
            .iter()
            .filter_map(|&id| self.entity(id))
            .map(now)
            .collect();
        let remembered = pl
            .memory
            .values()
            .map(|r| KnownEntity::of(self, seat, &r.snapshot, Some(r.time_stamp)))
            .collect();
        let controlled_units = pl
            .allied_control
            .iter()
            .flat_map(|&p| &self.player(p).units);
        let mut unperceived_units: Vec<KnownEntity> = controlled_units
            .filter(|u| !pl.perception.contains(u))
            .filter_map(|&u| self.entity(u))
            .map(now)
            .collect();
        unperceived_units.sort_by_key(|e| e.id);
        let terrain = pl
            .observed_squares
            .iter()
            .chain(&pl.partly_observed_squares)
            .map(|&s| (s, self.square(s).type_name.clone()))
            .collect();
        TurnMessage {
            seat,
            time: self.time,
            events,
            state: self.seat_state(seat),
            squares: KnownSquares {
                observed: pl.observed_squares.clone(),
                observed_before: self.observed_before_squares(seat),
                partly_observed: pl.partly_observed_squares.clone(),
                terrain,
            },
            perceived,
            remembered,
            unperceived_units,
        }
    }

    fn seat_state(&self, seat: PlayerId) -> SeatState {
        let pl = self.player(seat);
        let controls = |p: PlayerId| pl.allied_control.contains(&p);
        let controlled = pl
            .allied_control
            .iter()
            .map(|&p| {
                let c = self.player(p);
                ControlledPlayer {
                    player: p,
                    units: c.units.clone(),
                    upgrades: c.upgrades.clone(),
                    forbidden_techs: c.forbidden_techs.clone(),
                    has_everything: c.cheat_has_all,
                    is_human: c.is_human(),
                }
            })
            .collect();
        // A group is found by its name, the first of that name.
        let mut seen: Vec<&str> = Vec::new();
        let mut groups = Vec::new();
        for (name, units) in &self.groups {
            if seen.contains(&name.as_str()) {
                continue;
            }
            seen.push(name);
            let units: Vec<EntityId> = units
                .iter()
                .copied()
                .filter(|&u| self.entity(u).and_then(|e| e.player).is_some_and(controls))
                .collect();
            if !units.is_empty() {
                groups.push((name.clone(), units));
            }
        }
        SeatState {
            resources: pl.resources.clone(),
            available_food: pl.available_food(self.food_limit()),
            used_food: pl.used_food,
            cheat_mode: pl.cheatmode,
            has_victory: pl.has_victory,
            objectives: pl.objectives.clone(),
            players: self.players.clone(),
            defeated: self
                .all_players()
                .filter(|(_, p)| p.has_been_defeated)
                .map(|(id, _)| id)
                .collect(),
            allies: self.all_players().map(|(_, p)| p.allied.clone()).collect(),
            controlled,
            groups,
        }
    }

    /// The squares a player has observed before: every square in cheat
    /// mode.
    pub(crate) fn observed_before_squares(&self, p: PlayerId) -> BTreeSet<SquareId> {
        let pl = self.player(p);
        if pl.cheatmode {
            (0..self.squares.len() as u32).map(SquareId).collect()
        } else {
            pl.observed_before_squares.clone()
        }
    }

    /// What `seat` is told when the game ends for it.
    pub(crate) fn game_end(&self, seat: PlayerId) -> GameEnd {
        GameEnd {
            stats: self.player(seat).stats.clone(),
        }
    }
}
