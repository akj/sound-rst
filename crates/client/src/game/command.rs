//! The commands a key of `bindings.txt` can give during a game, read once
//! when the bindings are loaded.

use std::num::NonZeroU32;

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
    /// Selects one `portion`th of the units of each type.
    Group {
        portion: NonZeroU32,
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
    /// Stops speech and gives up the news waiting, which stays in the log.
    Silence,
    /// Does nothing: what Shift alone does, so that the screen reader has
    /// it.
    Nothing,
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
    /// A watcher follows the next player (1) or the previous one (-1).
    FollowPlayer(i64),
    /// A listener to a recorded game plays it faster (1) or slower (-1).
    ListenSpeed(i64),
    /// A listener pauses the recorded game, or resumes it.
    ListenPause,
    /// A listener goes forward (or back, below 0) this many minutes of
    /// game time.
    ListenJump(i64),
    GameMenu,
    SayPlayers,
    /// The player's own faction.
    SayFaction,
    /// The other players: ally or enemy, and defeated.
    PlayersStatus,
    Say,
    SayAllies,
    /// Points the allies at the square under the cursor, on a server.
    Flare,
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
}

/// How a command reads its arguments.
type Parse = fn(&[String]) -> Result<GameCommand, String>;

use GameCommand as C;

/// Every command name the interface knows, with how it reads its arguments.
const COMMANDS: &[(&str, Parse)] = &[
    ("resource_status", |a| one_int(a, C::ResourceStatus)),
    ("all_resources_status", |a| none(a, C::AllResourcesStatus)),
    ("food_status", |a| none(a, C::FoodStatus)),
    ("unit_hp_status", |a| none(a, C::UnitHpStatus)),
    ("production_status", |a| none(a, C::ProductionStatus)),
    ("workers_status", |a| none(a, C::WorkersStatus)),
    ("fight_status", |a| none(a, C::FightStatus)),
    ("combat_status", |a| none(a, C::CombatStatus)),
    ("examine", |a| none(a, C::Examine)),
    ("unit_status", |a| none(a, C::UnitStatus)),
    ("select_square", |a| {
        arity(a, 2, usize::MAX)?;
        Ok(C::SelectSquare {
            dx: int(&a[0])?,
            dy: int(&a[1])?,
            no_collision: a[2..].iter().any(|a| a == "no_collision"),
        })
    }),
    ("select_scouted_square", |a| {
        one_int(a, C::SelectScoutedSquare)
    }),
    ("select_conflict_square", |a| {
        one_int(a, C::SelectConflictSquare)
    }),
    ("select_unknown_square", |a| {
        one_int(a, C::SelectUnknownSquare)
    }),
    ("select_resource_square", |a| {
        one_int(a, C::SelectResourceSquare)
    }),
    ("toggle_zoom", |a| none(a, C::ToggleZoom)),
    ("select_target", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::SelectTarget {
            inc: int(&a[0])?,
            types: rest(a).to_vec(),
        })
    }),
    ("select_order", |a| {
        arity(a, 1, usize::MAX)?;
        let has = |f: &str| rest(a).iter().any(|a| a == f);
        Ok(C::SelectOrder {
            inc: int(&a[0])?,
            menu: if has("inactive_included") {
                OrderMenu::Included
            } else if has("inactive_only") {
                OrderMenu::InactiveOnly
            } else {
                OrderMenu::Active
            },
        })
    }),
    ("order_shortcut", |a| none(a, C::OrderShortcut)),
    ("validate", |a| Ok(C::Validate(OrderFlags::from_args(a)))),
    ("default", |a| Ok(C::Default(OrderFlags::from_args(a)))),
    ("do_again", |a| {
        Ok(C::DoAgain {
            now: a.iter().any(|a| a == "now"),
            flags: OrderFlags::from_args(a),
        })
    }),
    ("select_unit", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::SelectUnit {
            inc: int(&a[0])?,
            filter: UnitFilter::from_args(rest(a)),
        })
    }),
    ("select_units", |a| {
        Ok(C::SelectUnits(UnitFilter::from_args(a)))
    }),
    ("command_unit", |a| none(a, C::CommandUnit)),
    ("group", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::Group {
            portion: positive(&a[0])?,
            filter: UnitFilter::from_args(rest(a)),
        })
    }),
    ("ungroup", |a| none(a, C::Ungroup)),
    ("recall_group", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::RecallGroup(a[0].clone()))
    }),
    ("set_group", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::SetGroup {
            name: a[0].clone(),
            flags: OrderFlags::from_args(rest(a)),
        })
    }),
    ("append_group", |a| {
        arity(a, 1, usize::MAX)?;
        Ok(C::AppendGroup {
            name: a[0].clone(),
            flags: OrderFlags::from_args(rest(a)),
        })
    }),
    ("escape", |a| none(a, C::Escape)),
    ("help", |a| one_int(a, C::Help)),
    ("fullscreen", |a| none(a, C::Fullscreen)),
    ("toggle_cheatmode", |a| none(a, C::ToggleCheatmode)),
    ("say_time", |a| none(a, C::SayTime)),
    ("toggle_tick", |a| none(a, C::ToggleTick)),
    ("toggle_talking_clock", |a| none(a, C::ToggleTalkingClock)),
    ("change_player", |a| none(a, C::ChangePlayer)),
    ("history_previous", |a| none(a, C::HistoryPrevious)),
    ("history_next", |a| none(a, C::HistoryNext)),
    ("history_stop", |a| none(a, C::HistoryStop)),
    ("silence", |a| none(a, C::Silence)),
    ("nothing", |a| none(a, C::Nothing)),
    ("unheard", |a| none(a, C::Unheard)),
    ("review_previous", |a| none(a, C::ReviewPrevious)),
    ("review_next", |a| none(a, C::ReviewNext)),
    ("review_category", |a| none(a, C::ReviewCategory)),
    ("copy_message", |a| none(a, C::CopyMessage)),
    ("jump_to_alert", |a| none(a, C::JumpToAlert)),
    ("objectives", |a| Ok(C::Objectives(optional_int(a)?))),
    ("follow_player", |a| Ok(C::FollowPlayer(optional_int(a)?))),
    ("listen_speed", |a| {
        Ok(C::ListenSpeed(optional_int(a)?.signum()))
    }),
    ("listen_pause", |a| none(a, C::ListenPause)),
    ("listen_jump", |a| {
        arity(a, 1, 1)?;
        match int(&a[0])? {
            0 => Err("0 minutes is no jump".into()),
            n => Ok(C::ListenJump(n.clamp(-600, 600))),
        }
    }),
    ("gamemenu", |a| none(a, C::GameMenu)),
    ("say_players", |a| none(a, C::SayPlayers)),
    ("say_faction", |a| none(a, C::SayFaction)),
    ("players_status", |a| none(a, C::PlayersStatus)),
    ("say", |a| none(a, C::Say)),
    ("say_allies", |a| none(a, C::SayAllies)),
    ("flare", |a| none(a, C::Flare)),
    ("chat_previous", |a| none(a, C::ChatPrevious)),
    ("chat_next", |a| none(a, C::ChatNext)),
    ("volume", |a| Ok(C::Volume(optional_int(a)?))),
    ("immersion", |a| none(a, C::Immersion)),
    ("rotate_left", |a| none(a, C::RotateLeft)),
    ("rotate_right", |a| none(a, C::RotateRight)),
    ("console", |a| none(a, C::Console)),
    ("reload_parameters", |a| none(a, C::ReloadParameters)),
    ("key_help", |a| none(a, C::KeyHelp)),
    ("cmd", |a| Ok(C::Cmd(a.join(" ")))),
];

fn int(s: &str) -> Result<i64, String> {
    py_int(s).map_err(|_| format!("'{s}' is not a whole number"))
}

/// A whole number above 0.
fn positive(s: &str) -> Result<NonZeroU32, String> {
    u32::try_from(int(s)?)
        .ok()
        .and_then(NonZeroU32::new)
        .ok_or_else(|| format!("{s} is not above 0"))
}

/// The arguments' count must be between `min` and `max`.
fn arity(args: &[String], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min {
        Err(format!("{} argument(s) missing", min - args.len()))
    } else if args.len() > max {
        Err(format!("at most {max} argument(s), not {}", args.len()))
    } else {
        Ok(())
    }
}

/// A command that takes no argument.
fn none(args: &[String], c: GameCommand) -> Result<GameCommand, String> {
    arity(args, 0, 0).map(|()| c)
}

/// A command that takes one whole number.
fn one_int(args: &[String], f: fn(i64) -> GameCommand) -> Result<GameCommand, String> {
    arity(args, 1, 1)?;
    Ok(f(int(&args[0])?))
}

/// At most one whole number, 1 when it is left out.
fn optional_int(args: &[String]) -> Result<i64, String> {
    arity(args, 0, 1)?;
    match args.first() {
        Some(a) => int(a),
        None => Ok(1),
    }
}

/// The arguments after the first.
fn rest(args: &[String]) -> &[String] {
    args.get(1..).unwrap_or_default()
}

/// Whether `name` is a command the interface knows.
pub fn is_command(name: &str) -> bool {
    COMMANDS.iter().any(|(n, _)| *n == name)
}

impl GameCommand {
    /// The command `name` with `args`: an error for an unknown name, or
    /// arguments the command cannot take.
    pub fn parse(name: &str, args: Vec<String>) -> Result<GameCommand, BindingError> {
        let Some((_, parse)) = COMMANDS.iter().find(|(n, _)| *n == name) else {
            return Err(BindingError(format!("'{name}' is not a command")));
        };
        parse(&args).map_err(|e| BindingError(format!("{name}: {e}")))
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
            C::FollowPlayer(i) => format!("watching, follow the {} player", direction(*i)),
            C::ListenSpeed(i) if *i < 0 => "listening, play slower".into(),
            C::ListenSpeed(_) => "listening, play faster".into(),
            C::ListenPause => "listening, pause or resume".into(),
            C::ListenJump(n) => {
                let way = if *n < 0 { "back" } else { "forward" };
                let n = n.unsigned_abs();
                let unit = if n == 1 { "minute" } else { "minutes" };
                format!("listening, go {way} {n} {unit}")
            }
            C::SayPlayers => "say the names of the players".into(),
            C::SayFaction => "say your faction".into(),
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
                let part = match portion.get() {
                    1 => "all".to_owned(),
                    2 => "half".to_owned(),
                    4 => "a quarter of".to_owned(),
                    n => format!("a {} of", ordinal(n.into())),
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
            C::Silence => "stop speech and the news waiting".into(),
            C::Nothing => "do nothing".into(),
            C::Unheard => "say what you missed".into(),
            C::ReviewPrevious => "previous message of the category".into(),
            C::ReviewNext => "next message of the category".into(),
            C::ReviewCategory => "change the category of messages".into(),
            C::CopyMessage => "copy the message you review, or the last one said".into(),
            C::Say => "write to every player".into(),
            C::SayAllies => "write to your allies".into(),
            C::Flare => "point your allies at this square".into(),
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
        assert_eq!(
            parse("help x"),
            Err(BindingError("help: 'x' is not a whole number".into()))
        );
        assert_eq!(
            parse("examine now"),
            Err(BindingError("examine: at most 0 argument(s), not 1".into()))
        );
        assert_eq!(
            parse("select_square 1"),
            Err(BindingError("select_square: 1 argument(s) missing".into()))
        );
        assert_eq!(
            parse("group 0 local"),
            Err(BindingError("group: 0 is not above 0".into()))
        );
        assert!(parse("dance").is_err());
    }

    #[test]
    fn each_command_name_has_one_row() {
        for (i, (name, _)) in COMMANDS.iter().enumerate() {
            assert!(
                !COMMANDS[..i].iter().any(|(n, _)| n == name),
                "{name} has two rows, and the second is never read"
            );
        }
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
        assert_eq!(
            describe("follow_player"),
            "watching, follow the next player"
        );
        assert_eq!(
            describe("follow_player -1"),
            "watching, follow the previous player"
        );
        assert_eq!(describe("flare"), "point your allies at this square");
    }

    #[test]
    fn each_command_of_the_games_keys_has_a_description_of_its_own() {
        use crate::bindings::{Bindings, CLIENT_BINDINGS};
        use crate::package::TextFile;
        let path = crate::testkit::game_root().join("res/ui/bindings.txt");
        let files = [
            TextFile {
                path: "res/ui/bindings.txt".to_owned(),
                text: std::fs::read_to_string(path).unwrap(),
            },
            TextFile {
                path: "client".to_owned(),
                text: CLIENT_BINDINGS.to_owned(),
            },
        ];
        let mut b = Bindings::new();
        b.load(&files, &GameCommand::parse, &crate::log::Log::silent());
        let mut seen: Vec<(String, GameCommand)> = Vec::new();
        for (_, c) in b.iter() {
            let d = c.description();
            match seen.iter().find(|(s, _)| *s == d) {
                Some((_, other)) => assert_eq!(other, c, "two commands are \"{d}\""),
                None => seen.push((d, c.clone())),
            }
        }
        assert_eq!(seen.len(), 187);
    }
}
