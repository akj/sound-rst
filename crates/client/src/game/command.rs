//! The commands a key of `bindings.txt` can give during a game, read once
//! when the bindings are loaded.

use soundrts_world::Queue;
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

    pub fn queue(self) -> Queue {
        if self.queue {
            Queue::Append
        } else {
            Queue::Replace
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
    /// Every resource the rules define.
    AllResourcesStatus,
    FoodStatus,
    UnitHpStatus,
    /// What the player's units produce, soonest done first.
    ProductionStatus,
    /// What the workers do: idle, gathering each resource, building.
    WorkersStatus,
    /// The strength of each side on the selected square.
    FightStatus,
    /// How the first unit controlled fights, its upgrades included.
    CombatStatus,
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
    /// What did I miss: the unheard news by category, then each of it.
    Unheard,
    /// The previous message of the category under review.
    ReviewPrevious,
    ReviewNext,
    /// Reviews the next category.
    ReviewCategory,
    /// Copies the message under review, or the one said last.
    CopyMessage,
    /// Moves the cursor to the square of the last alert, then the one
    /// before.
    JumpToAlert,
    /// The next (1) or previous (-1) objective, with whether it is done.
    Objectives(i64),
    GameMenu,
    SayPlayers,
    /// The other players: ally or enemy, and defeated.
    PlayersStatus,
    Say,
    SayAllies,
    ChatPrevious,
    ChatNext,
    Volume(i64),
    Immersion,
    RotateLeft,
    RotateRight,
    Console,
    ReloadParameters,
    /// Turns key help on: keys say what they do instead of doing it.
    KeyHelp,
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
    "all_resources_status",
    "food_status",
    "unit_hp_status",
    "production_status",
    "workers_status",
    "fight_status",
    "combat_status",
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
    "unheard",
    "review_previous",
    "review_next",
    "review_category",
    "copy_message",
    "jump_to_alert",
    "objectives",
    "gamemenu",
    "say_players",
    "players_status",
    "say",
    "say_allies",
    "chat_previous",
    "chat_next",
    "volume",
    "immersion",
    "rotate_left",
    "rotate_right",
    "console",
    "reload_parameters",
    "key_help",
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
            "all_resources_status" => none(C::AllResourcesStatus)?,
            "food_status" => none(C::FoodStatus)?,
            "unit_hp_status" => none(C::UnitHpStatus)?,
            "production_status" => none(C::ProductionStatus)?,
            "workers_status" => none(C::WorkersStatus)?,
            "fight_status" => none(C::FightStatus)?,
            "combat_status" => none(C::CombatStatus)?,
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
            "unheard" => none(C::Unheard)?,
            "review_previous" => none(C::ReviewPrevious)?,
            "review_next" => none(C::ReviewNext)?,
            "review_category" => none(C::ReviewCategory)?,
            "copy_message" => none(C::CopyMessage)?,
            "jump_to_alert" => none(C::JumpToAlert)?,
            "objectives" => {
                arity(args, 0, 1)?;
                C::Objectives(match args.first() {
                    Some(a) => int(a)?,
                    None => 1,
                })
            }
            "gamemenu" => none(C::GameMenu)?,
            "say_players" => none(C::SayPlayers)?,
            "players_status" => none(C::PlayersStatus)?,
            "say" => none(C::Say)?,
            "say_allies" => none(C::SayAllies)?,
            "chat_previous" => none(C::ChatPrevious)?,
            "chat_next" => none(C::ChatNext)?,
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
            "key_help" => none(C::KeyHelp)?,
            "cmd" => C::Cmd(args.join(" ")),
            other => unreachable!("{other} is in COMMAND_NAMES"),
        })
    }
}

/// "next" or "previous", by the sign of a step.
fn direction(inc: i64) -> &'static str {
    if inc < 0 { "previous" } else { "next" }
}

/// "first", "second"... for `n` counted from 1.
fn ordinal(n: i64) -> String {
    const WORDS: [&str; 10] = [
        "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
        "tenth",
    ];
    usize::try_from(n)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| WORDS.get(i))
        .map_or_else(|| format!("number {n}"), |w| (*w).to_owned())
}

/// A unit type of the style's `keyboard` names, singular or plural:
/// `soldier3` is "soldier of type 3".
fn unit_type(name: &str, plural: bool) -> String {
    let s = if plural { "s" } else { "" };
    match name.strip_prefix("soldier") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
            format!("soldier{s} of type {n}")
        }
        _ => format!("{name}{s}"),
    }
}

/// The units a filter keeps, as "idle workers and soldiers on this
/// square". The seven soldier types together are "soldiers".
fn units(filter: &UnitFilter, plural: bool) -> String {
    let soldiers: Vec<String> = (1..=7).map(|i| format!("soldier{i}")).collect();
    let all_soldiers = soldiers.iter().all(|s| filter.keyboard_types.contains(s));
    let mut names: Vec<String> = Vec::new();
    for t in &filter.keyboard_types {
        let name = if all_soldiers && soldiers.contains(t) {
            if plural { "soldiers" } else { "soldier" }.to_owned()
        } else {
            unit_type(t, plural)
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        names.push(if plural { "units" } else { "unit" }.to_owned());
    }
    let mut out = String::new();
    if filter.idle {
        out.push_str("idle ");
    }
    out.push_str(&names.join(" and "));
    if filter.local {
        out.push_str(" on this square");
    }
    if filter.even_if_no_menu {
        out.push_str(", even without a menu");
    }
    out
}

/// ", after the other orders" and ", without stopping to fight".
fn order_flags(flags: OrderFlags) -> String {
    let mut out = String::new();
    if flags.queue {
        out.push_str(", after the other orders");
    }
    if flags.imperative {
        out.push_str(", without stopping to fight");
    }
    out
}

/// A move of the cursor: "move one square north", "move 5 squares
/// south-west".
fn square_move(dx: i64, dy: i64, no_collision: bool) -> String {
    let north_south = match dy.signum() {
        1 => "north",
        -1 => "south",
        _ => "",
    };
    let east_west = match dx.signum() {
        1 => "east",
        -1 => "west",
        _ => "",
    };
    let way = match (north_south, east_west) {
        ("", "") => return "stay on this square".to_owned(),
        ("", w) | (w, "") => w.to_owned(),
        (n, e) => format!("{n}-{e}"),
    };
    let distance = match dx.unsigned_abs().max(dy.unsigned_abs()) {
        1 => "one square".to_owned(),
        n => format!("{n} squares"),
    };
    let path = if no_collision {
        ", even without a path"
    } else {
        ""
    };
    format!("move {distance} {way}{path}")
}

impl GameCommand {
    /// What the command does, said plainly, action first: "select the
    /// next worker". Key help says it. English for now: every description
    /// is here, and a new command cannot compile without one.
    pub fn description(&self) -> String {
        use GameCommand as C;
        match self {
            C::ResourceStatus(r) => format!(
                "say the amount of the {} resource",
                ordinal(r.saturating_add(1))
            ),
            C::AllResourcesStatus => "say every resource".into(),
            C::FoodStatus => "say the food".into(),
            C::UnitHpStatus => "say the hit points of the controlled units".into(),
            C::ProductionStatus => "say what is in production".into(),
            C::WorkersStatus => "say what the workers do".into(),
            C::FightStatus => "say how the fight on this square goes".into(),
            C::CombatStatus => "say how the controlled unit fights".into(),
            C::Examine => "describe the target or the square".into(),
            C::UnitStatus => "say the controlled units and follow them".into(),
            C::SayTime => "say the time".into(),
            C::Objectives(i) => format!("{} objective", direction(*i)),
            C::SayPlayers => "say the names of the players".into(),
            C::PlayersStatus => "say who is an ally or an enemy".into(),
            C::SelectSquare {
                dx,
                dy,
                no_collision,
            } => square_move(*dx, *dy, *no_collision),
            C::SelectScoutedSquare(i) => {
                format!("go to the {} explored square", direction(*i))
            }
            C::SelectConflictSquare(i) => {
                format!("go to the {} square with an enemy", direction(*i))
            }
            C::SelectUnknownSquare(i) => {
                format!("go to the {} unexplored square", direction(*i))
            }
            C::SelectResourceSquare(i) => {
                format!("go to the {} square with resources", direction(*i))
            }
            C::ToggleZoom => "zoom in or out of the square".into(),
            C::JumpToAlert => "go to the square of the last alert".into(),
            C::SelectTarget { inc, types } => match types.as_slice() {
                [] => format!("{} target", direction(*inc)),
                [useful] if useful == "useful" => {
                    format!("{} useful target", direction(*inc))
                }
                types => format!("{} target: {}", direction(*inc), types.join(" or ")),
            },
            C::SelectOrder { inc, menu } => match menu {
                OrderMenu::Included => format!("{} order", direction(*inc)),
                OrderMenu::Active => format!("{} available order", direction(*inc)),
                OrderMenu::InactiveOnly => {
                    format!("{} order not yet available", direction(*inc))
                }
            },
            C::OrderShortcut => "choose an order by its shortcut".into(),
            C::Validate(flags) => format!("confirm the order{}", order_flags(*flags)),
            C::Default(flags) => format!("give the default order{}", order_flags(*flags)),
            C::DoAgain { now: false, flags } => {
                format!("select the last order again{}", order_flags(*flags))
            }
            C::DoAgain { now: true, flags } => format!(
                "give the last order again{}, or select it if it needs a target",
                order_flags(*flags)
            ),
            C::Escape => "cancel the order, or leave the zoom".into(),
            C::SelectUnit { inc, filter } => {
                format!("select the {} {}", direction(*inc), units(filter, false))
            }
            C::SelectUnits(filter) => format!("select all {}", units(filter, true)),
            C::CommandUnit => "control the target, if it is yours".into(),
            C::Group { portion, filter } => {
                let part = match portion {
                    1 => "all".to_owned(),
                    2 => "half".to_owned(),
                    4 => "a quarter of".to_owned(),
                    n => format!("a {} of", ordinal(*n)),
                };
                let what = if filter.keyboard_types.is_empty() {
                    let idle = if filter.idle { "idle " } else { "" };
                    format!("the {idle}units of the same types")
                } else {
                    let all_over = UnitFilter {
                        local: false,
                        ..filter.clone()
                    };
                    format!("the {}", units(&all_over, true))
                };
                let here = if filter.local { " on this square" } else { "" };
                format!("control {part} {what}{here}")
            }
            C::Ungroup => "control only the first unit".into(),
            C::RecallGroup(name) => format!("recall group {name}"),
            C::SetGroup { name, .. } => format!("make the controlled units group {name}"),
            C::AppendGroup { name, .. } => {
                format!("add the controlled units to group {name}")
            }
            C::Help(i) => format!("{} line of help", direction(*i)),
            C::HistoryPrevious => "previous message".into(),
            C::HistoryNext => "next message".into(),
            C::HistoryStop => "skip the message being said".into(),
            C::Unheard => "say what you missed".into(),
            C::ReviewPrevious => "previous message of the category".into(),
            C::ReviewNext => "next message of the category".into(),
            C::ReviewCategory => "change the category of messages".into(),
            C::CopyMessage => "copy the message you review, or the last one said".into(),
            C::Say => "write to every player".into(),
            C::SayAllies => "write to your allies".into(),
            C::ChatPrevious => "previous chat message".into(),
            C::ChatNext => "next chat message".into(),
            C::KeyHelp => "turn key help on or off".into(),
            C::GameMenu => "open the game menu".into(),
            C::Fullscreen => "switch full screen".into(),
            C::ToggleCheatmode => "switch cheat mode".into(),
            C::ToggleTick => "switch the tick at each turn".into(),
            C::ToggleTalkingClock => "switch the talking clock".into(),
            C::ChangePlayer => "play as the next player".into(),
            C::Volume(n) => match n {
                1 => "raise the volume".into(),
                -1 => "lower the volume".into(),
                n if *n > 0 => format!("raise the volume {n} times"),
                n => format!("lower the volume {} times", n.unsigned_abs()),
            },
            C::Immersion => "go into first person".into(),
            C::RotateLeft => "turn left".into(),
            C::RotateRight => "turn right".into(),
            C::Console => "open the console".into(),
            C::ReloadParameters => "reload the parameters".into(),
            C::Cmd(cmd) => format!("run the console command {cmd}"),
            C::Broken { name, .. } => format!("{name}, with arguments it cannot take"),
        }
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

    fn describe(line: &str) -> String {
        parse(line).unwrap().description()
    }

    #[test]
    fn a_description_says_the_action_with_its_arguments() {
        assert_eq!(describe("select_unit 1 worker"), "select the next worker");
        assert_eq!(
            describe("select_unit -1 worker idle"),
            "select the previous idle worker"
        );
        assert_eq!(
            describe("select_units local soldier3"),
            "select all soldiers of type 3 on this square"
        );
        assert_eq!(
            describe("resource_status 0"),
            "say the amount of the first resource"
        );
        assert_eq!(describe("history_previous"), "previous message");
        assert_eq!(describe("select_target 1 useful"), "next useful target");
        assert_eq!(
            describe("select_square 0 5 no_collision"),
            "move 5 squares north, even without a path"
        );
        assert_eq!(
            describe("validate imperative queue_order"),
            "confirm the order, after the other orders, without stopping to fight"
        );
        assert_eq!(
            describe("group 2 local"),
            "control half the units of the same types on this square"
        );
        assert_eq!(describe("volume -1"), "lower the volume");
        assert_eq!(describe("objectives"), "next objective");
        assert_eq!(describe("objectives -1"), "previous objective");
    }

    #[test]
    fn each_command_of_the_games_keys_has_a_description_of_its_own() {
        use crate::bindings::{Bindings, CLIENT_BINDINGS};
        let path = crate::testkit::upstream_root().join("res/ui/bindings.txt");
        let text = format!(
            "{}\n{CLIENT_BINDINGS}",
            std::fs::read_to_string(path).unwrap()
        );
        let mut b = Bindings::new();
        b.load(&text, &GameCommand::parse, &crate::log::Log::silent());
        let mut seen: Vec<(String, GameCommand)> = Vec::new();
        for (_, c) in b.iter() {
            let d = c.description();
            match seen.iter().find(|(s, _)| *s == d) {
                Some((_, other)) => assert_eq!(other, c, "two commands are \"{d}\""),
                None => seen.push((d, c.clone())),
            }
        }
        assert_eq!(seen.len(), 175);
    }
}
