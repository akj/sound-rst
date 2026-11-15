//! What a player asks of the game, as typed values.
//!
//! A [`Command`] is given to a [`crate::Host`] for a player, and the world
//! carries it out as it is. The tests' scenarios also write commands as
//! the original's lines of text, which only the `truth` feature reads.

use crate::entity::{EntityId, PlayerId, SquareId};
use crate::orders::Keyword;
#[cfg(feature = "truth")]
use crate::{World, nofloat::py_float, orders::TargetRef};

/// Something a player asks of the game.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Queue {
    Replace,
    Append,
}

/// An order to the units chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "KeywordOrderParts"))]
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

/// A [`KeywordOrder`] as it is sent, read through [`KeywordOrder::new`]
/// and [`KeywordOrder::typed`] so that a keyword and a type that do not go
/// together are refused.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct KeywordOrderParts {
    keyword: Keyword,
    type_name: Option<String>,
    target: Option<OrderTarget>,
}

#[cfg(feature = "serde")]
impl TryFrom<KeywordOrderParts> for KeywordOrder {
    type Error = String;

    fn try_from(p: KeywordOrderParts) -> Result<KeywordOrder, String> {
        let order = match p.type_name {
            None => KeywordOrder::new(p.keyword),
            Some(t) => KeywordOrder::typed(p.keyword, t),
        }
        .ok_or_else(|| format!("{:?} with a type it cannot have", p.keyword))?;
        Ok(match p.target {
            Some(t) => order.at(t),
            None => order,
        })
    }
}

/// What an order aims at, as the player names it. The world resolves an
/// entity through what the player perceives or remembers, so an order
/// cannot aim at something the player has not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

#[cfg(feature = "truth")]
impl OrderTarget {
    /// Reads a target written in a line of text. A square may also be
    /// written by its name. The error is a word that names no target.
    fn parse(world: &World, word: &str) -> Result<OrderTarget, String> {
        Ok(match TargetRef::parse(world, word).map_err(|a| a.0)? {
            TargetRef::Square(s) | TargetRef::SquareNamed(s) => OrderTarget::Square(s),
            TargetRef::Point { square, x, y } => OrderTarget::Point { square, x, y },
            TargetRef::Entity(e) => OrderTarget::Entity(e),
            TargetRef::Nothing => return Err(format!("not a target: {word:?}")),
        })
    }
}

#[cfg(feature = "truth")]
impl Command {
    /// Reads a line of text as the original wrote a command. The error
    /// says why the line is not a command. The warning is about a line read
    /// in part: a `control` whose list of ids cannot be read chooses no
    /// unit.
    pub(crate) fn parse(line: &str, world: &World) -> Result<(Command, Option<String>), String> {
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else {
            return Err("list index out of range".to_owned());
        };
        let rest: Vec<&str> = words.collect();
        let command = match first.to_lowercase().as_str() {
            "control" => {
                return Ok(match crate::group::decode(&rest.join(" ")) {
                    Ok(ids) => {
                        // An id beyond those of entities chooses nothing.
                        let ids = ids.into_iter().filter_map(|i| u32::try_from(i).ok());
                        (Command::Control(ids.map(EntityId).collect()), None)
                    }
                    Err(m) => (Command::Control(Vec::new()), Some(m)),
                });
            }
            "order" => Command::parse_order(&rest, world)?,
            "speed" => {
                let s = rest.first().ok_or("list index out of range")?;
                let f =
                    py_float(s).map_err(|_| format!("could not convert string to float: {s:?}"))?;
                Command::Speed(f)
            }
            "quit" => Command::Quit,
            "toggle_cheatmode" => Command::Cheat(Cheat::ToggleMode),
            // The words of the map language: `cmd victory`, `cmd add_units
            // a1 footman`...
            "cmd" => Command::Cheat(Cheat::Trigger(rest.join(" "))),
            other => return Err(format!("unknown command: 'cmd_{other}' ({line})")),
        };
        Ok((command, None))
    }

    /// Reads the words after `order`: the queue, the imperative, then the
    /// order.
    fn parse_order(words: &[&str], world: &World) -> Result<Command, String> {
        let [queue, imperative, order @ ..] = words else {
            return Err("list index out of range".to_owned());
        };
        let order = match order {
            ["reset_group", name] => Order::ResetGroup((*name).to_owned()),
            ["join_group", name] => Order::JoinGroup((*name).to_owned()),
            ["default", target] => Order::Default(OrderTarget::parse(world, target)?),
            [keyword, rest @ ..] => {
                let unknown = || format!("unknown order: {order:?}");
                let keyword = Keyword::parse(keyword).ok_or_else(unknown)?;
                let (o, rest) = match (keyword.is_complex(), rest) {
                    (false, rest) => (KeywordOrder::new(keyword), rest),
                    (true, [t, rest @ ..]) => (KeywordOrder::typed(keyword, *t), rest),
                    (true, []) => return Err(format!("an order without its type: {order:?}")),
                };
                let o = o.ok_or_else(unknown)?;
                Order::Keyword(match rest {
                    [] => o,
                    [t] => o.at(OrderTarget::parse(world, t)?),
                    _ => return Err(format!("too many words in an order: {order:?}")),
                })
            }
            [] => return Err("list index out of range".to_owned()),
        };
        Ok(Command::Order {
            order,
            // As the original reads them: `0` replaces, `1` is imperative.
            queue: if *queue == "0" {
                Queue::Replace
            } else {
                Queue::Append
            },
            imperative: *imperative == "1",
        })
    }
}

#[cfg(test)]
mod tests;
