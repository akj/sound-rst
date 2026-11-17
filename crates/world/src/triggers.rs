//! Map triggers, read once when the map loads into a typed condition and
//! action (the trigger language `lang_*`), and the commands of players
//! (`cmd_*`), from `worldplayerbase.py` and `worldplayercomputer.py`.

use crate::abort::Res;
use crate::command::{Cheat, Command, Order, Queue};
use crate::entity::{EntityId, Place, PlayerId, SquareId, Target};
use crate::events::{EntityEvent, Event};
use crate::map::MAX_START_UNITS;
use crate::msg::{Cue, Msg, MsgPart, cue, msg};
use crate::nofloat::{py_float, py_int};
use crate::orders::{Request, TargetRef};
use crate::phrases::Phrase;
use crate::player::{Objective, PlayerKind};
use crate::rules::{FactionId, Summoned, TypeId};
use crate::world::{NotEnoughSpace, World};

/// The deepest a trigger may nest parentheses. Dropping, cloning or
/// comparing an [`Expr`] goes one call deeper for each level, so this keeps
/// them far from the end of the stack. The original's triggers nest 3 deep.
pub const MAX_TRIGGER_DEPTH: usize = 32;

/// A word or a parenthesized list of a trigger. [`list_to_tree`] makes one
/// at most [`MAX_TRIGGER_DEPTH`] deep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Word(String),
    List(Vec<Expr>),
}

/// `_list_to_tree`: parentheses make nested lists. A list left open is
/// dropped, like the original. Nesting deeper than [`MAX_TRIGGER_DEPTH`] is
/// an error.
#[deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]
pub fn list_to_tree(words: &[String]) -> Result<Vec<Expr>, String> {
    let mut top: Vec<Expr> = Vec::new();
    let mut open: Vec<Vec<Expr>> = Vec::new();
    for w in words {
        match w.as_str() {
            "(" => {
                if open.len() >= MAX_TRIGGER_DEPTH {
                    return Err(format!(
                        "parentheses nested more than {MAX_TRIGGER_DEPTH} deep"
                    ));
                }
                open.push(Vec::new());
            }
            ")" => {
                let last = open.pop().ok_or("a ')' closes no '('")?;
                open.last_mut().unwrap_or(&mut top).push(Expr::List(last));
            }
            _ => open
                .last_mut()
                .unwrap_or(&mut top)
                .push(Expr::Word(w.clone())),
        }
    }
    Ok(top)
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A trigger of the map: when the condition holds, the action runs once.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Trigger {
    pub(crate) condition: Condition,
    pub(crate) action: Action,
}

/// What the first word of a condition or an action names: a function
/// `lang_*` of the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Name {
    Timer,
    Has,
    HasEntered,
    NoEnemyLeft,
    NoEnemyPlayerLeft,
    NoUnitLeft,
    NoBuildingLeft,
    Order,
    AddUnits,
    Victory,
    Defeat,
    CutScene,
    AddObjective,
    ObjectiveComplete,
    Ai,
    Faction,
}

impl Name {
    fn parse(word: &str) -> Option<Name> {
        Some(match word {
            "timer" => Name::Timer,
            "has" => Name::Has,
            "has_entered" => Name::HasEntered,
            "no_enemy_left" => Name::NoEnemyLeft,
            "no_enemy_player_left" => Name::NoEnemyPlayerLeft,
            "no_unit_left" => Name::NoUnitLeft,
            "no_building_left" => Name::NoBuildingLeft,
            "order" => Name::Order,
            "add_units" => Name::AddUnits,
            "victory" => Name::Victory,
            "defeat" => Name::Defeat,
            "cut_scene" => Name::CutScene,
            "add_objective" => Name::AddObjective,
            "objective_complete" => Name::ObjectiveComplete,
            "ai" => Name::Ai,
            "faction" => Name::Faction,
            _ => return None,
        })
    }
}

/// Whether a word of a map names a condition or an action of a trigger.
pub(crate) fn is_trigger_name(word: &str) -> bool {
    Name::parse(word).is_some()
}

/// When the action of a trigger runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Condition {
    /// `timer <seconds>`: from this second of the game on. The seconds the
    /// map gives are multiplied by its timer coefficient.
    Timer { from_second: i64 },
    /// `has [<count>] <type>...`: the player has at least the count of
    /// units of each type, 1 if none is given. A count of 0 asks for no
    /// unit of the type.
    Has(Vec<(i64, TypeId)>),
    /// `has_entered <square>...`: a present unit of the first player is on
    /// one of the squares.
    HasEntered(Vec<SquareId>),
    /// `no_enemy_left`: no enemy plays, neutral computers included.
    NoEnemyLeft,
    /// `no_enemy_player_left`: no enemy plays but neutral computers.
    NoEnemyPlayerLeft,
    /// `no_unit_left`: the player has no unit.
    NoUnitLeft,
    /// `no_building_left`: the player has no unit that keeps it in the
    /// game.
    NoBuildingLeft,
}

/// What a trigger does when its condition holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Action {
    /// `order (<units>) ((<order>)...)`: each unit of the player that the
    /// first list chooses takes the orders in turn.
    Order {
        units: Vec<Units>,
        orders: Vec<Request>,
    },
    /// `add_units <units>`: adds the units, or gives an upgrade.
    AddUnits(Vec<Units>),
    /// `victory`
    Victory,
    /// `defeat`
    Defeat,
    /// `cut_scene <phrases and sounds>`: the player hears them in turn.
    CutScene(Msg),
    /// `add_objective <number> <phrases and sounds>`
    AddObjective { number: String, description: Msg },
    /// `objective_complete <number>`
    ObjectiveComplete(String),
    /// `ai <type>`: a computer player plays with another AI type.
    Ai(String),
    /// `faction <faction>`
    Faction(FactionId),
}

/// Units of a type on a square, as `add_units` and `order` name them:
/// `a1 2 footman` is 2 footmen on `a1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Units {
    square: SquareId,
    count: i64,
    ty: TypeId,
}

impl Trigger {
    /// Reads the condition and the action of a trigger. The error says
    /// what the words lack, or the square, type or faction that does not
    /// exist.
    pub(crate) fn read(w: &World, condition: &Expr, action: &Expr) -> Result<Trigger, String> {
        Ok(Trigger {
            condition: Condition::read(w, condition)?,
            action: Action::read(w, action)?,
        })
    }
}

/// The first word of a list, which names a condition or an action, and the
/// rest of the list.
fn split_name(e: &Expr) -> Result<(&str, &[Expr]), String> {
    let items = match e {
        Expr::List(items) => items,
        Expr::Word(w) => return Err(format!("{w} is not in parentheses")),
    };
    let (first, args) = items.split_first().ok_or("an empty list")?;
    Ok((word(first)?, args))
}

fn word(e: &Expr) -> Result<&str, String> {
    match e {
        Expr::Word(w) => Ok(w),
        Expr::List(_) => Err("a list where a word is expected".to_owned()),
    }
}

fn words(args: &[Expr]) -> Result<Vec<&str>, String> {
    args.iter().map(word).collect()
}

/// The first word after the name, which the condition or action needs.
fn first_word<'a>(name: &str, args: &[&'a str]) -> Result<&'a str, String> {
    args.first()
        .copied()
        .ok_or_else(|| format!("{name} lacks a word"))
}

fn number(x: &str) -> Result<i64, String> {
    py_int(x).map_err(|_| format!("{x} is not a number"))
}

fn square_named(w: &World, x: &str) -> Result<SquareId, String> {
    w.square_by_name(x).ok_or_else(|| format!("no square {x}"))
}

/// Whether a word is written as a square's name: letters, then digits,
/// such as `z9`.
fn looks_like_a_square(x: &str) -> bool {
    let digits = x.trim_start_matches(|c: char| c.is_ascii_lowercase());
    digits.len() < x.len() && is_digits(digits)
}

/// The type a word names. A word written as a square's name is said to
/// name no square.
fn type_named(w: &World, x: &str) -> Result<TypeId, String> {
    w.rules.type_id(x).ok_or_else(|| {
        if looks_like_a_square(x) {
            format!("no square {x}")
        } else {
            format!("no type {x}")
        }
    })
}

/// What the words of an action that says something name.
fn named(words: &[&str]) -> Result<Msg, String> {
    MsgPart::all_named(words.iter().copied())
        .map_err(|word| format!("{word} is not a name or a count"))
}

/// Reads units as `add_units` and `order` name them: a square says where
/// the next types are, a count how many of the next type there are, 1 if
/// none is given. Before any square, the units are on `square`, or on
/// `a1`.
fn read_units(
    w: &World,
    words: &[&str],
    square: Option<SquareId>,
    count: impl Fn(&str) -> Result<i64, String>,
) -> Result<Vec<Units>, String> {
    let mut square = square.or_else(|| w.square_by_name("a1"));
    let mut n = 1;
    let mut units = Vec::new();
    for &x in words {
        if let Some(s) = w.square_by_name(x) {
            square = Some(s);
            n = 1;
        } else if is_digits(x) {
            n = count(x)?;
        } else {
            let ty = type_named(w, x)?;
            let square = square.ok_or("no square a1")?;
            units.push(Units {
                square,
                count: n,
                ty,
            });
            n = 1;
        }
    }
    Ok(units)
}

/// Reads the units that `add_units` adds (`a1 2 footman b3 archer`).
/// Before any square, they go on `square`, or on `a1`. A count is at most
/// [`MAX_START_UNITS`], and a count too large to read is 1.
pub(crate) fn units_to_add(
    w: &World,
    words: &[&str],
    square: Option<SquareId>,
) -> Result<Vec<Units>, String> {
    read_units(w, words, square, |x| {
        Ok(py_int(x).unwrap_or(1).min(MAX_START_UNITS))
    })
}

/// Reads an order that a trigger gives (`go b1`, `train footman`). It
/// names a type that exists, and its target is not a word that names
/// nothing.
fn read_request(w: &World, words: &[&str]) -> Result<Request, String> {
    let owned: Vec<String> = words.iter().map(|&x| x.to_owned()).collect();
    let r = Request::parse(w, &owned)?;
    if let Some(t) = r.type_name() {
        type_named(w, t)?;
    }
    if r.target() == Some(&TargetRef::Nothing) {
        let target = words.last().copied().unwrap_or_default();
        return Err(if looks_like_a_square(target) {
            format!("no square {target}")
        } else {
            format!("{target} names no target")
        });
    }
    Ok(r)
}

impl Condition {
    fn read(w: &World, e: &Expr) -> Result<Condition, String> {
        let (name, args) = split_name(e)?;
        let args = words(args)?;
        Ok(match Name::parse(name) {
            Some(Name::Timer) => {
                let s = first_word(name, &args)?;
                let seconds = py_float(s)
                    .ok()
                    .filter(|f| f.is_finite())
                    .ok_or_else(|| format!("{s} is not a number"))?;
                // The time counts whole seconds: it reaches the given time
                // at the first whole second at or past it.
                let from = (seconds * w.map.timer_coefficient as f64).ceil();
                Condition::Timer {
                    from_second: from as i64,
                }
            }
            Some(Name::Has) => {
                let mut needed = Vec::new();
                let mut count = 1;
                for x in args {
                    if is_digits(x) {
                        count = number(x)?;
                    } else {
                        needed.push((count, type_named(w, x)?));
                        count = 1;
                    }
                }
                Condition::Has(needed)
            }
            Some(Name::HasEntered) => Condition::HasEntered(
                args.iter()
                    .map(|x| square_named(w, x))
                    .collect::<Result<_, _>>()?,
            ),
            Some(Name::NoEnemyLeft) => Condition::NoEnemyLeft,
            Some(Name::NoEnemyPlayerLeft) => Condition::NoEnemyPlayerLeft,
            Some(Name::NoUnitLeft) => Condition::NoUnitLeft,
            Some(Name::NoBuildingLeft) => Condition::NoBuildingLeft,
            _ => return Err(format!("{name} is not a condition")),
        })
    }
}

impl Action {
    /// Reads an action of a map, or one typed in the console.
    pub(crate) fn read(w: &World, e: &Expr) -> Result<Action, String> {
        let (name, args) = split_name(e)?;
        let kind = Name::parse(name);
        if kind == Some(Name::Order) {
            let [Expr::List(choice), Expr::List(orders)] = args else {
                return Err("order takes the units, then the orders, each a list".to_owned());
            };
            let units = read_units(w, &words(choice)?, None, number)?;
            let orders = orders
                .iter()
                .map(|o| match o {
                    Expr::List(o) => read_request(w, &words(o)?),
                    Expr::Word(o) => Err(format!("the order {o} is not in parentheses")),
                })
                .collect::<Result<_, _>>()?;
            return Ok(Action::Order { units, orders });
        }
        let args = words(args)?;
        Ok(match kind {
            Some(Name::AddUnits) => Action::AddUnits(units_to_add(w, &args, None)?),
            Some(Name::Victory) => Action::Victory,
            Some(Name::Defeat) => Action::Defeat,
            Some(Name::CutScene) => Action::CutScene(named(&args)?),
            Some(Name::AddObjective) => {
                let number = first_word(name, &args)?.to_owned();
                let description = named(args.get(1..).unwrap_or_default())?;
                Action::AddObjective {
                    number,
                    description,
                }
            }
            Some(Name::ObjectiveComplete) => {
                Action::ObjectiveComplete(first_word(name, &args)?.to_owned())
            }
            Some(Name::Ai) => Action::Ai(first_word(name, &args)?.to_owned()),
            Some(Name::Faction) => {
                let f = first_word(name, &args)?;
                let id = w
                    .rules
                    .faction_id(f)
                    .ok_or_else(|| format!("no faction {f}"))?;
                Action::Faction(id)
            }
            _ => return Err(format!("{name} is not an action")),
        })
    }
}

impl World {
    // triggers

    pub(crate) fn run_triggers(&mut self, p: PlayerId) -> Res {
        if !self.pl(p).is_playing() {
            return Ok(());
        }
        for t in self.pl(p).triggers.clone() {
            if self.holds(p, &t.condition) {
                self.run_action(p, &t.action)?;
                if !self.pl(p).is_playing() {
                    break;
                }
                let triggers = &mut self.plm(p).triggers;
                if let Some(i) = triggers.iter().position(|x| *x == t) {
                    triggers.remove(i);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn holds(&self, p: PlayerId, condition: &Condition) -> bool {
        match condition {
            &Condition::Timer { from_second } => self.time / 1000 >= from_second,
            Condition::Has(needed) => needed.iter().all(|&(count, ty)| {
                let mut left = count;
                for &u in &self.pl(p).units {
                    if self.e(u).type_id() == Some(ty) {
                        left -= 1;
                        if left == 0 {
                            break;
                        }
                    }
                }
                left == 0
            }),
            Condition::HasEntered(squares) => {
                // The triggers of a player run while it is in the game.
                let Some(&first) = self.players.first() else {
                    return false;
                };
                squares.iter().any(|&s| {
                    self.square(s)
                        .objects
                        .iter()
                        .any(|&o| self.pl(first).units.contains(&o) && self.e(o).stats.presence)
                })
            }
            Condition::NoEnemyLeft => !self
                .players
                .iter()
                .any(|&op| self.player_is_an_enemy(p, op) && self.pl(op).is_playing()),
            Condition::NoEnemyPlayerLeft => !self
                .true_players()
                .iter()
                .any(|&op| self.player_is_an_enemy(p, op) && self.pl(op).is_playing()),
            Condition::NoUnitLeft => self.pl(p).units.is_empty(),
            Condition::NoBuildingLeft => !self
                .pl(p)
                .units
                .iter()
                .any(|&u| self.e(u).stats.provides_survival),
        }
    }

    pub(crate) fn run_action(&mut self, p: PlayerId, action: &Action) -> Res {
        match action {
            Action::Order { units, orders } => self.give_orders(p, units, orders)?,
            Action::AddUnits(units) => self.add_units(p, units),
            Action::Victory => self.victory(p),
            Action::Defeat => self.defeat(p, false),
            Action::CutScene(m) => self.push(p, Event::Sequence(m.clone())),
            Action::AddObjective {
                number,
                description,
            } => {
                if !self.pl(p).objectives.iter().any(|o| o.number == *number) {
                    let mut m = msg(Phrase::NewObjective);
                    m.extend(description.iter().cloned());
                    self.plm(p).objectives.push(Objective {
                        number: number.clone(),
                        description: description.clone(),
                    });
                    self.send_voice_important(p, m);
                }
            }
            Action::ObjectiveComplete(number) => {
                if let Some(i) = self
                    .pl(p)
                    .objectives
                    .iter()
                    .position(|o| o.number == *number)
                {
                    let mut m = msg(Phrase::ObjectiveComplete);
                    m.extend(self.pl(p).objectives[i].description.iter().cloned());
                    self.send_voice_important(p, m);
                    self.plm(p).objectives.remove(i);
                    if self.pl(p).objectives.is_empty() {
                        self.send_voice_important(p, msg(Phrase::MissionComplete));
                        self.victory(p);
                    }
                }
            }
            Action::Ai(ai_type) => self.set_ai(p, ai_type),
            &Action::Faction(f) => self.plm(p).faction = Some(f),
        }
        Ok(())
    }

    /// `set_ai`: changes the AI type of a computer player, which keeps
    /// its skill level.
    fn set_ai(&mut self, p: PlayerId, ai_type: &str) {
        let PlayerKind::Computer { skill, .. } = self.pl(p).kind else {
            return;
        };
        self.plm(p).kind = PlayerKind::Computer {
            ai_type: ai_type.to_owned(),
            skill,
        };
        self.computer_set_ai(p, ai_type);
    }

    /// Gives the orders to the units of the player that `units` choose.
    fn give_orders(&mut self, p: PlayerId, units: &[Units], orders: &[Request]) -> Res {
        for u in units {
            let mut n = u.count;
            // The list may change while orders are given, as in the
            // original, which walks it by index.
            let mut i = 0;
            while let Some(&o) = self.square(u.square).objects.get(i) {
                if self.e(o).type_id() == Some(u.ty) && self.e(o).player() == Some(p) {
                    for order in orders {
                        self.take_order(o, order.clone(), false, false, None)?;
                    }
                    n -= 1;
                    if n == 0 {
                        break;
                    }
                }
                i += 1;
            }
        }
        Ok(())
    }

    /// `lang_add_units`: adds units, or gives upgrades, each unit with an
    /// event that it is added.
    pub(crate) fn add_units(&mut self, p: PlayerId, units: &[Units]) {
        for u in units {
            let add = Added {
                square: u.square,
                target: None,
                decay: 0,
                notify: true,
            };
            self.add_of_type(p, u.ty, u.count, &add, None);
        }
    }

    /// Adds the units a summon or a raising of the dead names, at the
    /// target, or in place of the corpses: as [`World::add_units`]
    /// with types and counts.
    pub(crate) fn add_summoned(
        &mut self,
        p: PlayerId,
        items: &[Summoned],
        target: Option<&Target>,
        decay: i64,
        mut corpses: Option<Vec<EntityId>>,
    ) {
        let Some(square) = self.square_by_name("a1") else {
            self.warn("add_units: the map has no square a1");
            return;
        };
        let add = Added {
            square,
            target,
            decay,
            notify: false,
        };
        for s in items {
            self.add_of_type(p, s.ty, s.count, &add, corpses.as_mut());
        }
    }

    /// Adds `nb` units of type `t`, or the upgrade `t`. An ability cannot
    /// be added.
    fn add_of_type(
        &mut self,
        p: PlayerId,
        t: TypeId,
        nb: i64,
        add: &Added<'_>,
        mut corpses: Option<&mut Vec<EntityId>>,
    ) {
        if self.rules.ty(t).is_upgrade() {
            self.plm(p).upgrades.add(t);
            self.send_voice_important(p, msg(Phrase::Ok));
            return;
        }
        if self.rules.ty(t).is_ability() {
            self.send_voice_important(p, cue(Cue::Beep));
            self.warn("cannot add an ability");
            return;
        }
        for _ in 0..nb {
            if !self.check_count_limit(p, t) {
                break;
            }
            match self.add_unit_for_lang(p, t, add, corpses.as_deref_mut()) {
                Ok(()) => {}
                // The unit that found no space was never created.
                Err(AddUnitError::NotEnoughSpace) => self.warn("not enough space"),
                Err(AddUnitError::Other(m)) => {
                    let name = &self.rules.ty(t).name;
                    let w = format!("couldn't add unit: {name}: {m}");
                    self.warn(w);
                }
            }
        }
    }

    fn add_unit_for_lang(
        &mut self,
        p: PlayerId,
        tid: TypeId,
        add: &Added<'_>,
        corpses: Option<&mut Vec<EntityId>>,
    ) -> Result<(), AddUnitError> {
        let &Added {
            mut square,
            target,
            decay,
            notify,
        } = add;
        if self
            .rules
            .ty(tid)
            .entity_class()
            .is_none_or(|c| c == crate::rules::EntityClass::Deposit)
        {
            return Err(AddUnitError::Other("not a unit or a building".into()));
        }
        let mut land = None;
        let (place, x, y) = if let Some(corpses) = corpses {
            if corpses.is_empty() {
                return Ok(());
            }
            let c = corpses.remove(0);
            let ce = self.e(c);
            let (place, x, y) = (ce.place, ce.x, ce.y);
            self.delete(c);
            let place = place.ok_or(AddUnitError::Other("the corpse has no place".into()))?;
            (place, x, y)
        } else if let Some(t) = target {
            let (x, y) = self.t_xy(t);
            let place = match t {
                Target::Square(s) => Place::Square(*s),
                _ => self
                    .t_place_of(t)
                    .ok_or(AddUnitError::Other("the target has no place".into()))?,
            };
            (place, x, y)
        } else {
            let anywhere = self.rules.ty(tid).stats.is_buildable_anywhere;
            let (x, y, l) = self.find_and_remove_meadow(square, anywhere);
            land = l;
            (Place::Square(square), x, y)
        };
        if let Place::Square(s) = place {
            square = s;
        }
        let _ = square;
        let u = self
            .create_creature(tid, Some(p), place, x, y)
            .map_err(|NotEnoughSpace| AddUnitError::NotEnoughSpace)?;
        let time = self.time;
        // `u` was created as a creature just above.
        if let Some(c) = self.cm(u) {
            c.building_land = land;
            if decay != 0 {
                c.time_limit = Some(time + decay);
            }
        }
        if notify {
            self.notify(u, EntityEvent::Added, false);
        }
        Ok(())
    }

    // commands

    /// Runs a command of a player now (`execute_command`). Cheats run only
    /// if [`World::allow_cheatmode`] is set.
    pub(crate) fn execute_command(&mut self, p: PlayerId, command: &Command) -> Res {
        match command {
            Command::Control(units) => self.cmd_control(p, units),
            Command::Order {
                order,
                queue,
                imperative,
            } => self.cmd_order(p, order, *queue, *imperative),
            &Command::Speed(f) => {
                if self.players.first() == Some(&p) {
                    for op in self.players.clone() {
                        self.push(op, Event::Speed(f));
                    }
                } else {
                    self.warn("non admin client tried to change game speed");
                }
            }
            Command::Quit => self.defeat(p, true),
            Command::Cheat(cheat) if !self.allow_cheatmode => {
                self.warn(format!("cheat mode is not allowed: {cheat:?}"));
            }
            Command::Cheat(Cheat::ToggleMode) => {
                let c = self.pl(p).cheatmode;
                self.plm(p).cheatmode = !c;
            }
            Command::Cheat(Cheat::Victory) => self.victory(p),
            Command::Cheat(Cheat::AddUnits { square, units }) => {
                let words: Vec<&str> = units.split_whitespace().collect();
                match units_to_add(self, &words, *square) {
                    Ok(units) => self.add_units(p, &units),
                    Err(m) => self.refuse_cheat(p, &m),
                }
            }
            Command::Cheat(Cheat::Trigger(text)) => {
                let words = text.split_whitespace();
                let e = Expr::List(words.map(|w| Expr::Word(w.to_owned())).collect());
                match Action::read(self, &e) {
                    Ok(action) => self.run_action(p, &action)?,
                    Err(m) => self.refuse_cheat(p, &m),
                }
            }
            Command::Cheat(
                cheat @ (Cheat::Resources
                | Cheat::AllTechs
                | Cheat::InfiniteMana
                | Cheat::PlayAs(_)),
            ) => {
                self.warn(format!("the host carries out this cheat: {cheat:?}"));
            }
        }
        Ok(())
    }

    /// A cheat typed in the console names nothing, or lacks a word: the
    /// player hears a beep, and nothing changes.
    fn refuse_cheat(&mut self, p: PlayerId, why: &str) {
        self.warn(format!("cheat refused: {why}"));
        self.send_voice_important(p, cue(Cue::Beep));
    }

    fn reset_group(&mut self, name: &str) {
        if let Some(i) = self.groups.iter().position(|(n, _)| n == name) {
            for u in self.groups[i].1.clone() {
                // Only creatures join a group.
                if let Some(c) = self.cm(u) {
                    c.group = None;
                }
            }
            self.groups[i].1 = Vec::new();
        }
    }

    fn cmd_order(&mut self, p: PlayerId, order: &Order, queue: Queue, imperative: bool) {
        self.plm(p).group_had_enough_mana = false;
        let order_id = self.get_next_order_id();
        let forget_previous = queue == Queue::Replace;
        let mut given = match order {
            Order::ResetGroup(name) => {
                self.reset_group(name);
                return;
            }
            Order::Default(t) => Given::Default(TargetRef::from(*t)),
            Order::Keyword(o) => Given::Order(Request::from(o)),
            Order::JoinGroup(name) => Given::Order(Request::join_group(name.clone())),
        };
        let selection = self.pl(p).group.clone();
        for u in selection.clone().unwrap_or_default() {
            if let Some(g) = self.c(u).and_then(|c| c.group.clone()) {
                let list = self.group_list(&g).cloned().unwrap_or_default();
                if Some(&list) != selection.as_ref() {
                    if let Some((_, l)) = self.groups.iter_mut().find(|(n, _)| *n == g)
                        && let Some(i) = l.iter().position(|&x| x == u)
                    {
                        l.remove(i);
                    }
                    if let Some(c) = self.cm(u) {
                        c.group = None;
                    }
                }
            }
            if self
                .e(u)
                .player()
                .is_some_and(|up| self.pl(p).allied_control.contains(&up))
            {
                let r = match &mut given {
                    Given::Default(t) => {
                        self.take_default_order(u, *t, forget_previous, imperative, Some(order_id))
                    }
                    Given::Order(request) => self.take_order_shared(
                        u,
                        request,
                        forget_previous,
                        imperative,
                        Some(order_id),
                    ),
                };
                if let Err(e) = r {
                    let words = match &given {
                        Given::Default(t) => vec!["default".to_owned(), t.text(self)],
                        Given::Order(request) => request.words(self),
                    };
                    let to = self.what(u);
                    self.warn(format!(
                        "the order {words:?} given to {to} stopped at turn {}: {e}",
                        self.turn()
                    ));
                }
            }
        }
    }

    /// `take_order` with the order shared between the units of a group: an
    /// imperative `go` that becomes `attack` stays `attack` for the next
    /// units, as in the original.
    fn take_order_shared(
        &mut self,
        u: EntityId,
        request: &mut Request,
        forget_previous: bool,
        imperative: bool,
        order_id: Option<u64>,
    ) -> Res {
        if imperative && let Some(up) = self.e(u).player() {
            self.imperative_go(up, request);
        }
        self.take_order(u, request.clone(), forget_previous, imperative, order_id)
    }

    fn cmd_control(&mut self, p: PlayerId, ids: &[EntityId]) {
        let mut units = Vec::new();
        for ap in self.pl(p).allied_control.clone() {
            units.extend(self.pl(ap).units.iter().copied());
        }
        let group = ids.iter().copied().filter(|u| units.contains(u)).collect();
        self.plm(p).group = Some(group);
    }
}

/// What an `order` command gives each unit chosen.
enum Given {
    /// The order each unit chooses for the target.
    Default(TargetRef),
    Order(Request),
}

/// Where and how units are added: on `square`, or at the target, or in
/// place of corpses; for `decay` thousandths of a second if not 0; with an
/// event that each is added if `notify`.
struct Added<'a> {
    square: SquareId,
    target: Option<&'a Target>,
    decay: i64,
    notify: bool,
}

enum AddUnitError {
    NotEnoughSpace,
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn parentheses_make_nested_lists() {
        let tree = list_to_tree(&words(
            "player1 ( timer 0 ) ( order ( a1 footman ) ( ( go b1 ) ) )",
        ))
        .unwrap();
        let w = |s: &str| Expr::Word(s.into());
        assert_eq!(
            tree,
            vec![
                w("player1"),
                Expr::List(vec![w("timer"), w("0")]),
                Expr::List(vec![
                    w("order"),
                    Expr::List(vec![w("a1"), w("footman")]),
                    Expr::List(vec![Expr::List(vec![w("go"), w("b1")])]),
                ]),
            ]
        );
        assert!(list_to_tree(&words(")")).is_err());
    }

    #[test]
    fn a_list_left_open_is_dropped_and_deep_nesting_is_an_error() {
        let w = |s: &str| Expr::Word(s.into());
        assert_eq!(list_to_tree(&words("a ( b ( c")).unwrap(), [w("a")]);
        let nested = |depth: usize| {
            let mut t = vec!["(".to_owned(); depth];
            t.extend(vec![")".to_owned(); depth]);
            t
        };
        assert_eq!(list_to_tree(&nested(MAX_TRIGGER_DEPTH)).unwrap().len(), 1);
        assert_eq!(
            list_to_tree(&nested(MAX_TRIGGER_DEPTH + 1)),
            Err("parentheses nested more than 32 deep".to_owned())
        );
    }
}
