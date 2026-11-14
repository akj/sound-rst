//! What a player asks of the game, as typed values.
//!
//! A [`Command`] is given to a [`crate::Host`] for a player. The world
//! still runs commands from text: [`Command::line`] writes the line the
//! world reads, and a game on a server sends that line to the other
//! players. Temporary: the text goes once the world takes commands as
//! values.

use crate::World;
use crate::entity::{EntityId, PlayerId, SquareId};
use crate::orders::Keyword;

/// Something a player asks of the game.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Chooses the units the next orders go to: the player's units, or an
    /// ally's that the player may control, among these. An empty list
    /// chooses none.
    Control(Vec<EntityId>),
    /// Gives an order to the units chosen by the last [`Command::Control`].
    Order {
        order: Order,
        queue: Queue,
        /// Given in the imperative: a `go` towards a unit becomes an
        /// attack, and some orders are carried out at once.
        imperative: bool,
    },
    /// Sets the speed of the game for every player: 1 is normal, 2 twice
    /// as fast. Only the first player may.
    Speed(f64),
    /// Leaves the game, defeated.
    Quit,
    /// A cheat. The game refuses it unless it allows cheats.
    Cheat(Cheat),
}

/// Whether an order replaces the orders a unit has or comes after them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queue {
    Replace,
    Append,
}

/// An order to the units chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Order {
    /// An order named by its keyword, as a unit's menu offers it.
    Keyword(KeywordOrder),
    /// The order each unit chooses for the target: gather from a mine,
    /// attack an enemy, block an exit, go to a square...
    Default(OrderTarget),
    /// Empties the recallable group of this name.
    ResetGroup(String),
    /// Adds the units to the recallable group of this name.
    JoinGroup(String),
}

/// An order named by its keyword: `go b2`, `stop`, `train footman`,
/// `build farm a1`. A keyword that needs a type (see
/// [`Keyword::is_complex`]) always has one, and no other keyword has one.
///
/// Whether the order needs a target depends on the keyword and, for `use`,
/// on the ability (see [`World::order_info`]); a missing or extra target is
/// refused by the world when the order runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeywordOrder {
    keyword: Keyword,
    type_name: Option<String>,
    target: Option<OrderTarget>,
}

impl KeywordOrder {
    /// An order of a keyword without a type: `go`, `stop`... `None` if the
    /// keyword needs a type.
    pub fn new(keyword: Keyword) -> Option<KeywordOrder> {
        (!keyword.is_complex()).then_some(KeywordOrder {
            keyword,
            type_name: None,
            target: None,
        })
    }

    /// An order of a keyword with a type: `train footman`, `use
    /// a_holy_vision`. `None` if the keyword takes no type, or if the type
    /// name is not one word.
    pub fn typed(keyword: Keyword, type_name: impl Into<String>) -> Option<KeywordOrder> {
        let type_name = type_name.into();
        let one_word = !type_name.is_empty() && !type_name.contains(char::is_whitespace);
        (keyword.is_complex() && one_word).then_some(KeywordOrder {
            keyword,
            type_name: Some(type_name),
            target: None,
        })
    }

    /// The order, aimed at a target.
    pub fn at(self, target: OrderTarget) -> KeywordOrder {
        KeywordOrder {
            target: Some(target),
            ..self
        }
    }

    pub fn keyword(&self) -> Keyword {
        self.keyword
    }

    pub fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }

    pub fn target(&self) -> Option<&OrderTarget> {
        self.target.as_ref()
    }
}

/// What an order aims at, as the player names it. The world resolves an
/// entity through what the player perceives or remembers, so an order
/// cannot aim at something the player has not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderTarget {
    /// A square: its center.
    Square(SquareId),
    /// A point of a square, in millimeters, such as the center of one of
    /// its nine subsquares.
    Point { square: SquareId, x: i64, y: i64 },
    /// An entity.
    Entity(EntityId),
}

/// A cheat. Each is refused unless the game allows cheats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cheat {
    /// Turns the player's cheat mode on or off. In cheat mode a player has
    /// observed every square.
    ToggleMode,
    /// Wins the game.
    Victory,
    /// Adds units on a square, or on no square: `units` are the words of
    /// the map language after the square, such as `3 footman`.
    AddUnits {
        square: Option<SquareId>,
        units: String,
    },
    /// An action of the map language, typed by the player in the console:
    /// `add_units a1 footman`, `victory`... The console is a way to type
    /// that language, so this stays text.
    Trigger(String),
    /// Gives the player 1000 of each resource.
    Resources,
    /// Gives the player every technology.
    AllTechs,
    /// Multiplies the mana regeneration of the player's units by 1000.
    InfiniteMana,
    /// Plays as another player: that player becomes the local human,
    /// whatever controlled it, and receives the events from now on; the
    /// player who asked stops receiving them. Each keeps its login.
    PlayAs(PlayerId),
}

impl OrderTarget {
    /// The id the world's `order` command reads: a square's protocol id, a
    /// point as `zoom-<square>-<x>-<y>`, or an entity's id.
    pub(crate) fn text(self, world: &World) -> String {
        match self {
            OrderTarget::Square(s) => world.square(s).id.to_string(),
            OrderTarget::Point { square, x, y } => {
                format!("zoom-{}-{x}-{y}", world.square(square).id)
            }
            OrderTarget::Entity(e) => e.0.to_string(),
        }
    }
}

impl Command {
    /// The text line of the command, as the world reads it and a game on a
    /// server sends it. `None` for the cheats the host carries out itself
    /// ([`Cheat::Resources`], [`Cheat::AllTechs`], [`Cheat::InfiniteMana`],
    /// [`Cheat::PlayAs`]), which have no text; a game on a server allows
    /// no cheat. `world` names the squares.
    ///
    /// Temporary: the text goes when the world takes commands as values.
    pub fn line(&self, world: &World) -> Option<String> {
        Some(match self {
            Command::Control(units) => {
                let ids: Vec<i64> = units.iter().map(|u| i64::from(u.0)).collect();
                format!("control {}", crate::group::encode(&ids))
            }
            Command::Order {
                order,
                queue,
                imperative,
            } => {
                let words = match order {
                    Order::Keyword(o) => {
                        let mut w = o.keyword.as_str().to_owned();
                        for part in o
                            .type_name
                            .iter()
                            .cloned()
                            .chain(o.target.map(|t| t.text(world)))
                        {
                            w.push(' ');
                            w.push_str(&part);
                        }
                        w
                    }
                    Order::Default(t) => format!("default {}", t.text(world)),
                    Order::ResetGroup(name) => format!("reset_group {name}"),
                    Order::JoinGroup(name) => format!("join_group {name}"),
                };
                format!(
                    "order {} {} {words}",
                    i32::from(*queue == Queue::Append),
                    i32::from(*imperative)
                )
            }
            Command::Speed(s) => format!("speed {s:?}"),
            Command::Quit => "quit".to_owned(),
            Command::Cheat(c) => match c {
                Cheat::ToggleMode => "toggle_cheatmode".to_owned(),
                Cheat::Victory => "cmd victory".to_owned(),
                Cheat::AddUnits { square, units } => {
                    let name = square.map(|s| world.square(s).name.as_str()).unwrap_or("");
                    format!("cmd add_units {name} {units}")
                }
                Cheat::Trigger(text) => format!("cmd {text}"),
                Cheat::Resources | Cheat::AllTechs | Cheat::InfiniteMana | Cheat::PlayAs(_) => {
                    return None;
                }
            },
        })
    }
}

#[cfg(test)]
mod tests;
