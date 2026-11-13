//! The commands a key of `bindings.txt` can give during a game, read once
//! when the bindings are loaded.

use soundrts_world::nofloat::py_int;

use crate::bindings::BindingError;

/// `queue_order` and `imperative` among the arguments of an order command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OrderFlags {
    /// Add to the unit's orders instead of replacing them.
    pub queue: bool,
    pub imperative: bool,
}

impl OrderFlags {
    fn from_args(args: &[String]) -> OrderFlags {
        OrderFlags {
            queue: args.iter().any(|a| a == "queue_order"),
            imperative: args.iter().any(|a| a == "imperative"),
        }
    }
}

/// Which units a unit selection command considers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitFilter {
    /// Only on the current square (or zoom).
    pub local: bool,
    /// Only units without orders.
    pub idle: bool,
    /// Include units without a menu.
    pub even_if_no_menu: bool,
    /// The `keyboard` names of the style (`worker`, `soldier1`...); none
    /// means any type.
    pub keyboard_types: Vec<String>,
}

impl UnitFilter {
    fn from_args(args: &[String]) -> UnitFilter {
        let flag = |f: &str| args.iter().any(|a| a == f);
        UnitFilter {
            local: flag("local"),
            idle: flag("idle"),
            even_if_no_menu: flag("even_if_no_menu"),
            keyboard_types: args
                .iter()
                .filter(|a| !matches!(a.as_str(), "local" | "idle" | "even_if_no_menu"))
                .cloned()
                .collect(),
        }
    }
}

/// Which entries the order menu lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderMenu {
    /// The orders the group may give now.
    Active,
    /// Every order, given now or later.
    Included,
    /// Only the orders the group may not give yet (said without help).
    InactiveOnly,
}

/// A command of the game interface.
#[derive(Clone, Debug, PartialEq)]
pub enum GameCommand {
    ResourceStatus(i64),
    FoodStatus,
    UnitHpStatus,
    Examine,
    UnitStatus,
    SelectSquare {
        dx: i64,
        dy: i64,
        no_collision: bool,
    },
    SelectScoutedSquare(i64),
    SelectConflictSquare(i64),
    SelectUnknownSquare(i64),
    SelectResourceSquare(i64),
    ToggleZoom,
    SelectTarget {
        inc: i64,
        types: Vec<String>,
    },
    SelectOrder {
        inc: i64,
        menu: OrderMenu,
    },
    OrderShortcut,
    Validate(OrderFlags),
    Default(OrderFlags),
    DoAgain {
        now: bool,
        flags: OrderFlags,
    },
    SelectUnit {
        inc: i64,
        filter: UnitFilter,
    },
    SelectUnits(UnitFilter),
    CommandUnit,
    Group {
        portion: i64,
        filter: UnitFilter,
    },
    Ungroup,
    RecallGroup(String),
    SetGroup {
        name: String,
        flags: OrderFlags,
    },
    AppendGroup {
        name: String,
        flags: OrderFlags,
    },
    Escape,
    Help(i64),
    Fullscreen,
    ToggleCheatmode,
    SayTime,
    ToggleTick,
    ToggleTalkingClock,
    ChangePlayer,
    HistoryPrevious,
    HistoryNext,
    HistoryStop,
    Objectives,
    GameMenu,
    SayPlayers,
    Say,
    Volume(i64),
    Immersion,
    RotateLeft,
    RotateRight,
    Console,
    ReloadParameters,
    /// A cheat command, as typed in the console.
    Cmd(String),
    /// A known command with arguments it cannot take: pressing its key
    /// reports the error, as upstream's call raised then.
    Broken {
        name: String,
        error: String,
    },
}

/// Every command name the interface knows.
pub const COMMAND_NAMES: &[&str] = &[
    "resource_status",
    "food_status",
    "unit_hp_status",
    "examine",
    "unit_status",
    "select_square",
    "select_scouted_square",
    "select_conflict_square",
    "select_unknown_square",
    "select_resource_square",
    "toggle_zoom",
    "select_target",
    "select_order",
    "order_shortcut",
    "validate",
    "default",
    "do_again",
    "select_unit",
    "select_units",
    "command_unit",
    "group",
    "ungroup",
    "recall_group",
    "set_group",
    "append_group",
    "escape",
    "help",
    "fullscreen",
    "toggle_cheatmode",
    "say_time",
    "toggle_tick",
    "toggle_talking_clock",
    "change_player",
    "history_previous",
    "history_next",
    "history_stop",
    "objectives",
    "gamemenu",
    "say_players",
    "say",
    "volume",
    "immersion",
    "rotate_left",
    "rotate_right",
    "console",
    "reload_parameters",
    "cmd",
];

fn int(s: &str) -> Result<i64, String> {
    py_int(s).map_err(|_| format!("invalid literal for int() with base 10: '{s}'"))
}

/// The arguments' count must be between `min` and `max`.
fn arity(args: &[String], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min {
        Err(format!("missing {} required argument(s)", min - args.len()))
    } else if args.len() > max {
        Err(format!(
            "takes {max} argument(s) but {} were given",
            args.len()
        ))
    } else {
        Ok(())
    }
}

impl GameCommand {
    /// The command `name` with `args`. An unknown name is an error; known
    /// names with unfit arguments give [`GameCommand::Broken`].
    pub fn parse(name: &str, args: Vec<String>) -> Result<GameCommand, BindingError> {
        if !COMMAND_NAMES.contains(&name) {
            return Err(BindingError(format!("'{name}' is not a command")));
        }
        Ok(
            Self::parse_known(name, &args).unwrap_or_else(|error| GameCommand::Broken {
                name: name.to_owned(),
                error,
            }),
        )
    }

    fn parse_known(name: &str, args: &[String]) -> Result<GameCommand, String> {
        use GameCommand as C;
        let none = |c: GameCommand| arity(args, 0, 0).map(|()| c);
        let one_int = |f: fn(i64) -> GameCommand| -> Result<GameCommand, String> {
            arity(args, 1, 1)?;
            Ok(f(int(&args[0])?))
        };
        let rest = || args.get(1..).unwrap_or_default();
        Ok(match name {
            "resource_status" => one_int(C::ResourceStatus)?,
            "food_status" => none(C::FoodStatus)?,
            "unit_hp_status" => none(C::UnitHpStatus)?,
            "examine" => none(C::Examine)?,
            "unit_status" => none(C::UnitStatus)?,
            "select_square" => {
                arity(args, 2, usize::MAX)?;
                C::SelectSquare {
                    dx: int(&args[0])?,
                    dy: int(&args[1])?,
                    no_collision: args[2..].iter().any(|a| a == "no_collision"),
                }
            }
            "select_scouted_square" => one_int(C::SelectScoutedSquare)?,
            "select_conflict_square" => one_int(C::SelectConflictSquare)?,
            "select_unknown_square" => one_int(C::SelectUnknownSquare)?,
            "select_resource_square" => one_int(C::SelectResourceSquare)?,
            "toggle_zoom" => none(C::ToggleZoom)?,
            "select_target" => {
                arity(args, 1, usize::MAX)?;
                C::SelectTarget {
                    inc: int(&args[0])?,
                    types: rest().to_vec(),
                }
            }
            "select_order" => {
                arity(args, 1, usize::MAX)?;
                let has = |f: &str| rest().iter().any(|a| a == f);
                C::SelectOrder {
                    inc: int(&args[0])?,
                    menu: if has("inactive_included") {
                        OrderMenu::Included
                    } else if has("inactive_only") {
                        OrderMenu::InactiveOnly
                    } else {
                        OrderMenu::Active
                    },
                }
            }
            "order_shortcut" => none(C::OrderShortcut)?,
            "validate" => C::Validate(OrderFlags::from_args(args)),
            "default" => C::Default(OrderFlags::from_args(args)),
            "do_again" => C::DoAgain {
                now: args.iter().any(|a| a == "now"),
                flags: OrderFlags::from_args(args),
            },
            "select_unit" => {
                arity(args, 1, usize::MAX)?;
                C::SelectUnit {
                    inc: int(&args[0])?,
                    filter: UnitFilter::from_args(rest()),
                }
            }
            "select_units" => C::SelectUnits(UnitFilter::from_args(args)),
            "command_unit" => none(C::CommandUnit)?,
            "group" => {
                arity(args, 1, usize::MAX)?;
                C::Group {
                    portion: int(&args[0])?,
                    filter: UnitFilter::from_args(rest()),
                }
            }
            "ungroup" => none(C::Ungroup)?,
            "recall_group" => {
                arity(args, 1, usize::MAX)?;
                C::RecallGroup(args[0].clone())
            }
            "set_group" => {
                arity(args, 1, usize::MAX)?;
                C::SetGroup {
                    name: args[0].clone(),
                    flags: OrderFlags::from_args(rest()),
                }
            }
            "append_group" => {
                arity(args, 1, usize::MAX)?;
                C::AppendGroup {
                    name: args[0].clone(),
                    flags: OrderFlags::from_args(rest()),
                }
            }
            "escape" => none(C::Escape)?,
            "help" => one_int(C::Help)?,
            "fullscreen" => none(C::Fullscreen)?,
            "toggle_cheatmode" => none(C::ToggleCheatmode)?,
            "say_time" => none(C::SayTime)?,
            "toggle_tick" => none(C::ToggleTick)?,
            "toggle_talking_clock" => none(C::ToggleTalkingClock)?,
            "change_player" => none(C::ChangePlayer)?,
            "history_previous" => none(C::HistoryPrevious)?,
            "history_next" => none(C::HistoryNext)?,
            "history_stop" => none(C::HistoryStop)?,
            "objectives" => none(C::Objectives)?,
            "gamemenu" => none(C::GameMenu)?,
            "say_players" => none(C::SayPlayers)?,
            "say" => none(C::Say)?,
            "volume" => {
                arity(args, 0, 1)?;
                C::Volume(match args.first() {
                    Some(a) => int(a)?,
                    None => 1,
                })
            }
            "immersion" => none(C::Immersion)?,
            "rotate_left" => none(C::RotateLeft)?,
            "rotate_right" => none(C::RotateRight)?,
            "console" => none(C::Console)?,
            "reload_parameters" => none(C::ReloadParameters)?,
            "cmd" => C::Cmd(args.join(" ")),
            other => unreachable!("{other} is in COMMAND_NAMES"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<GameCommand, BindingError> {
        let mut w = line.split_whitespace();
        let name = w.next().unwrap();
        GameCommand::parse(name, w.map(str::to_owned).collect())
    }

    #[test]
    fn arguments_are_read_as_upstream_calls_would_take_them() {
        assert_eq!(
            parse("select_square 0 -5 no_collision"),
            Ok(GameCommand::SelectSquare {
                dx: 0,
                dy: -5,
                no_collision: true
            })
        );
        assert_eq!(
            parse("select_units local worker soldier1"),
            Ok(GameCommand::SelectUnits(UnitFilter {
                local: true,
                idle: false,
                even_if_no_menu: false,
                keyboard_types: vec!["worker".into(), "soldier1".into()],
            }))
        );
        assert_eq!(
            parse("validate imperative queue_order"),
            Ok(GameCommand::Validate(OrderFlags {
                queue: true,
                imperative: true
            }))
        );
        assert_eq!(parse("volume"), Ok(GameCommand::Volume(1)));
        assert_eq!(parse("volume -1"), Ok(GameCommand::Volume(-1)));
        assert_eq!(
            parse("cmd a footman 3"),
            Ok(GameCommand::Cmd("a footman 3".into()))
        );
        assert!(matches!(parse("help x"), Ok(GameCommand::Broken { .. })));
        assert!(matches!(
            parse("examine now"),
            Ok(GameCommand::Broken { .. })
        ));
        assert!(matches!(
            parse("select_square 1"),
            Ok(GameCommand::Broken { .. })
        ));
        assert!(parse("dance").is_err());
    }
}
