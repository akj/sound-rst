//! Orders given to units (`worldorders.py`).
//!
//! An order is given as a [`Request`]: a keyword, a type for the keywords
//! that take one, and a target (`go 12`, `train footman`, `build farm
//! 25`). A unit keeps a queue of orders and executes the first one.

use crate::command::{KeywordOrder, OrderTarget};
use crate::entity::{EntityId, Place, PlayerId, SquareId, Target};
use crate::events::{EntityEvent, Reason};
use crate::nofloat::{py_int, square_of_distance, to_int};
use crate::rules::{AirGround, VIRTUAL_TIME_INTERVAL};
use crate::world::{Abort, Res, TPlace, World};

/// The most orders a unit can queue.
pub const ORDERS_QUEUE_LIMIT: usize = 10;

macro_rules! keywords {
    ($($v:ident = $s:literal),* $(,)?) => {
        /// The kind of an order, by its keyword.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Keyword { $($v,)* }

        impl Keyword {
            /// Every keyword, in the original's order (`ORDERS_DICT`).
            pub const ALL: &'static [Keyword] = &[$(Keyword::$v,)*];

            pub fn as_str(self) -> &'static str {
                match self { $(Keyword::$v => $s,)* }
            }

            pub fn parse(s: &str) -> Option<Keyword> {
                match s { $($s => Some(Keyword::$v),)* _ => None }
            }
        }
    };
}

keywords! {
    Stop = "stop",
    CancelTraining = "cancel_training",
    CancelUpgrading = "cancel_upgrading",
    CancelBuilding = "cancel_building",
    EnableAutoGather = "enable_auto_gather",
    DisableAutoGather = "disable_auto_gather",
    EnableAutoRepair = "enable_auto_repair",
    DisableAutoRepair = "disable_auto_repair",
    ModeOffensive = "mode_offensive",
    ModeDefensive = "mode_defensive",
    RallyingPoint = "rallying_point",
    JoinGroup = "join_group",
    Train = "train",
    Research = "research",
    UpgradeTo = "upgrade_to",
    Go = "go",
    Attack = "attack",
    Patrol = "patrol",
    Gather = "gather",
    AutoAttack = "auto_attack",
    AutoExplore = "auto_explore",
    Wait = "wait",
    Block = "block",
    Repair = "repair",
    BuildPhaseTwo = "build_phase_two",
    Build = "build",
    Use = "use",
    Load = "load",
    Enter = "enter",
    LoadAll = "load_all",
    UnloadAll = "unload_all",
}

impl Keyword {
    /// Orders with a type: `train footman`.
    pub fn is_complex(self) -> bool {
        matches!(
            self,
            Keyword::Train | Keyword::Research | Keyword::UpgradeTo | Keyword::Build | Keyword::Use
        )
    }

    /// Orders that act at once instead of being queued.
    pub fn is_immediate(self) -> bool {
        matches!(
            self,
            Keyword::Stop
                | Keyword::CancelTraining
                | Keyword::CancelUpgrading
                | Keyword::CancelBuilding
                | Keyword::EnableAutoGather
                | Keyword::DisableAutoGather
                | Keyword::EnableAutoRepair
                | Keyword::DisableAutoRepair
                | Keyword::ModeOffensive
                | Keyword::ModeDefensive
                | Keyword::RallyingPoint
                | Keyword::JoinGroup
                | Keyword::Enter
        )
    }

    fn is_production(self) -> bool {
        matches!(
            self,
            Keyword::Train | Keyword::Research | Keyword::UpgradeTo
        )
    }

    /// Giving this order does not cancel the previous ones.
    pub fn never_forget_previous(self) -> bool {
        self.is_immediate() || self.is_production()
    }

    fn imperative_by_default(self) -> bool {
        self.is_production()
            || matches!(
                self,
                Keyword::AutoAttack
                    | Keyword::AutoExplore
                    | Keyword::Wait
                    | Keyword::Block
                    | Keyword::Use
            )
    }

    /// The order that cancels this one when it is the last of the queue.
    pub fn cancel_order(self) -> &'static str {
        match self {
            Keyword::Train => "cancel_training",
            Keyword::Research | Keyword::UpgradeTo => "cancel_upgrading",
            _ => "stop",
        }
    }

    fn can_be_followed(self) -> bool {
        self != Keyword::UpgradeTo
    }

    /// The type list of the unit that a complex order chooses from.
    pub fn menu_attribute(self) -> Option<&'static str> {
        Some(match self {
            Keyword::Train => "can_train",
            Keyword::Research => "can_research",
            Keyword::UpgradeTo => "can_upgrade_to",
            Keyword::Build => "can_build",
            Keyword::Use => "can_use",
            _ => return None,
        })
    }

    /// The number of targets the order takes, for orders other than `use`.
    pub fn nb_args(self) -> usize {
        match self {
            Keyword::RallyingPoint
            | Keyword::JoinGroup
            | Keyword::Go
            | Keyword::Attack
            | Keyword::Patrol
            | Keyword::Gather
            | Keyword::Wait
            | Keyword::Block
            | Keyword::Repair
            | Keyword::BuildPhaseTwo
            | Keyword::Build
            | Keyword::Load
            | Keyword::Enter
            | Keyword::LoadAll
            | Keyword::UnloadAll => 1,
            _ => 0,
        }
    }
}

/// What an order aims at when it is given, before anything is resolved.
/// The ordering player's knowledge resolves it when the unit takes the
/// order ([`World::resolve_target`]): an entity it names may then be
/// perceived, remembered, or nothing.
///
/// The order as stored keeps its target as the text of
/// [`TargetRef::text`], which [`TargetRef::parse`] reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TargetRef {
    /// A square, written as its protocol id.
    Square(SquareId),
    /// A square, written as its name (`b1`), as the map language names
    /// squares: the stored order keeps the name it was given.
    SquareNamed(SquareId),
    /// A point of a square, in millimeters: `zoom-<square id>-<x>-<y>`.
    Point { square: SquareId, x: i64, y: i64 },
    /// An entity, by id. An id that is a square's protocol id names the
    /// square.
    Entity(EntityId),
    /// Nothing: the place inside a unit, or a word of the map language that
    /// names nothing. An order aimed at it has no target. Written `None`, as
    /// the original writes that place.
    Nothing,
}

impl TargetRef {
    /// The target as the order stores it.
    pub(crate) fn text(self, w: &World) -> String {
        match self {
            TargetRef::Square(s) => w.square(s).id.to_string(),
            TargetRef::SquareNamed(s) => w.square(s).name.clone(),
            TargetRef::Point { square, x, y } => format!("zoom-{}-{x}-{y}", w.square(square).id),
            TargetRef::Entity(e) => e.to_string(),
            TargetRef::Nothing => "None".to_owned(),
        }
    }

    /// Reads a target written as the map language, a line of text or a
    /// stored order write it: a point (`zoom-<square>-<x>-<y>`, the square
    /// by name or id), a square's name, a number, or any other word, which
    /// names nothing. The error is a point that cannot be read.
    pub(crate) fn parse(w: &World, word: &str) -> Res<TargetRef> {
        if word.starts_with("zoom") {
            let parts: Vec<&str> = word.split('-').collect();
            let [_, square, x, y] = parts[..] else {
                return Err(Abort::new("not enough values to unpack"));
            };
            let (TargetRef::Square(square) | TargetRef::SquareNamed(square)) =
                TargetRef::parse(w, square)?
            else {
                return Err(Abort::new("object has no attribute 'title'"));
            };
            let int = |s: &str| py_int(s).map_err(|_| Abort::new("invalid literal for int()"));
            let (x, y) = (int(x)?, int(y)?);
            return Ok(TargetRef::Point { square, x, y });
        }
        if let Some(s) = w.square_by_name(word) {
            return Ok(TargetRef::SquareNamed(s));
        }
        let Some(n) = word.parse::<u32>().ok().filter(|n| n.to_string() == word) else {
            return Ok(TargetRef::Nothing);
        };
        Ok(match w.square_by_obj_id(n) {
            Some(s) => TargetRef::Square(s),
            None => TargetRef::Entity(EntityId(n)),
        })
    }
}

impl From<OrderTarget> for TargetRef {
    fn from(t: OrderTarget) -> TargetRef {
        match t {
            OrderTarget::Square(s) => TargetRef::Square(s),
            OrderTarget::Point { square, x, y } => TargetRef::Point { square, x, y },
            OrderTarget::Entity(e) => TargetRef::Entity(e),
        }
    }
}

/// An order given to a unit: its keyword, the type for the keywords that
/// take one (see [`Keyword::is_complex`]), and what follows. A keyword
/// that takes a type always has one, and no other keyword has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    keyword: Keyword,
    type_name: Option<String>,
    arg: Option<Arg>,
}

/// What follows the keyword and the type of an order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Arg {
    Target(TargetRef),
    /// The name of a recallable group (`join_group`).
    Group(String),
}

impl Request {
    /// An order of a keyword without a type. `None` if the keyword needs
    /// one.
    pub(crate) fn plain(keyword: Keyword) -> Option<Request> {
        (!keyword.is_complex()).then_some(Request {
            keyword,
            type_name: None,
            arg: None,
        })
    }

    /// An order of a keyword with a type. `None` if the keyword takes none.
    pub(crate) fn typed(keyword: Keyword, type_name: impl Into<String>) -> Option<Request> {
        keyword.is_complex().then(|| Request {
            keyword,
            type_name: Some(type_name.into()),
            arg: None,
        })
    }

    /// The order, aimed at a target.
    pub(crate) fn at(self, target: TargetRef) -> Request {
        Request {
            arg: Some(Arg::Target(target)),
            ..self
        }
    }

    fn of(keyword: Keyword, type_name: Option<String>, target: Option<TargetRef>) -> Request {
        Request {
            keyword,
            type_name,
            arg: target.map(Arg::Target),
        }
    }

    pub(crate) fn stop() -> Request {
        Request::of(Keyword::Stop, None, None)
    }

    pub(crate) fn auto_explore() -> Request {
        Request::of(Keyword::AutoExplore, None, None)
    }

    pub(crate) fn go(target: TargetRef) -> Request {
        Request::of(Keyword::Go, None, Some(target))
    }

    pub(crate) fn gather(target: TargetRef) -> Request {
        Request::of(Keyword::Gather, None, Some(target))
    }

    pub(crate) fn repair(target: TargetRef) -> Request {
        Request::of(Keyword::Repair, None, Some(target))
    }

    pub(crate) fn load(target: TargetRef) -> Request {
        Request::of(Keyword::Load, None, Some(target))
    }

    pub(crate) fn build_phase_two(target: TargetRef) -> Request {
        Request::of(Keyword::BuildPhaseTwo, None, Some(target))
    }

    pub(crate) fn train(type_name: impl Into<String>) -> Request {
        Request::of(Keyword::Train, Some(type_name.into()), None)
    }

    pub(crate) fn research(type_name: impl Into<String>) -> Request {
        Request::of(Keyword::Research, Some(type_name.into()), None)
    }

    pub(crate) fn upgrade_to(type_name: impl Into<String>) -> Request {
        Request::of(Keyword::UpgradeTo, Some(type_name.into()), None)
    }

    pub(crate) fn build(type_name: impl Into<String>, target: TargetRef) -> Request {
        Request::of(Keyword::Build, Some(type_name.into()), Some(target))
    }

    /// `use` of an ability; [`Request::at`] aims it.
    pub(crate) fn use_ability(ability: impl Into<String>) -> Request {
        Request::of(Keyword::Use, Some(ability.into()), None)
    }

    pub(crate) fn join_group(name: impl Into<String>) -> Request {
        Request {
            keyword: Keyword::JoinGroup,
            type_name: None,
            arg: Some(Arg::Group(name.into())),
        }
    }

    /// Reads an order of the map language: `go b1`, `train footman`,
    /// `join_group 1`. The error says why the words are not an order.
    pub(crate) fn parse(w: &World, words: &[String]) -> Result<Request, String> {
        let Some((first, rest)) = words.split_first() else {
            return Err("an order without a keyword".to_owned());
        };
        let keyword = Keyword::parse(first).ok_or_else(|| format!("unknown order: {words:?}"))?;
        let (type_name, rest) = match (keyword.is_complex(), rest.split_first()) {
            (false, _) => (None, rest),
            (true, Some((t, rest))) => (Some(t.clone()), rest),
            (true, None) => return Err(format!("an order without its type: {words:?}")),
        };
        let arg = match rest {
            [] => None,
            [name] if keyword == Keyword::JoinGroup => Some(Arg::Group(name.clone())),
            [word] => Some(Arg::Target(TargetRef::parse(w, word).map_err(|a| a.0)?)),
            _ => return Err(format!("too many words in an order: {words:?}")),
        };
        Ok(Request {
            keyword,
            type_name,
            arg,
        })
    }

    pub(crate) fn keyword(&self) -> Keyword {
        self.keyword
    }

    pub(crate) fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }

    /// The target, if the order has one.
    pub(crate) fn target(&self) -> Option<&TargetRef> {
        match &self.arg {
            Some(Arg::Target(t)) => Some(t),
            _ => None,
        }
    }

    /// What follows the type, as the order stores it.
    fn args(&self, w: &World) -> Vec<String> {
        match &self.arg {
            None => Vec::new(),
            Some(Arg::Target(t)) => vec![t.text(w)],
            Some(Arg::Group(name)) => vec![name.clone()],
        }
    }

    /// The order in words, as a computer player keeps track of it:
    /// `["build", "farm", "25"]`.
    pub(crate) fn words(&self, w: &World) -> Vec<String> {
        let mut words = vec![self.keyword.as_str().to_owned()];
        words.extend(self.type_name.iter().cloned());
        words.extend(self.args(w));
        words
    }

    /// The order as a menu lists it: `go`, `train footman`.
    fn menu_entry(&self) -> String {
        match &self.type_name {
            Some(t) => format!("{} {t}", self.keyword.as_str()),
            None => self.keyword.as_str().to_owned(),
        }
    }

    /// The order as a unit stores it.
    fn stored(&self, uid: u64, w: &World) -> Order {
        Order::new(uid, self.keyword, self.type_name.clone(), self.args(w))
    }
}

impl From<&KeywordOrder> for Request {
    fn from(o: &KeywordOrder) -> Request {
        Request::of(
            o.keyword(),
            o.type_name().map(str::to_owned),
            o.target().copied().map(TargetRef::from),
        )
    }
}

/// The step an order is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    None,
    /// Gather: carrying the cargo to a warehouse.
    BringBack,
    /// Gather: storing the cargo.
    Store,
    /// Gather: going to the deposit.
    GoGather,
    /// Gather: extracting.
    Gather,
    /// Block: going to the exit.
    GoBlock,
    /// Block: blocking the exit.
    Block,
    /// Repair and build: going to the building.
    GoBuild,
    /// Repair and build: working.
    Build,
}

/// An order in a unit's queue.
#[derive(Clone, Debug, Hash)]
pub struct Order {
    /// Unique in the world; tells two orders apart.
    pub uid: u64,
    pub keyword: Keyword,
    /// The arguments after the keyword (and after the type of a complex
    /// order).
    pub args: Vec<String>,
    /// The type of a complex order (`footman` in `train footman`).
    pub type_name: Option<String>,
    /// The id of the `order` command that gave it, shared by the units of
    /// a group.
    pub id: Option<u64>,
    pub target: Option<Target>,
    pub is_impossible: bool,
    pub is_complete: bool,
    pub is_imperative: bool,
    /// Production waiting for food.
    pub is_deferred: bool,
    first_update: bool,
    pub mode: Mode,
    delay: i64,
    storage: Option<EntityId>,
    /// Production time left.
    pub time: i64,
    has_started: bool,
    previous_completeness: Option<i64>,
    creation_time: Option<i64>,
    /// Patrol: the places patrolled, in order.
    pub patrol_targets: Option<Vec<Option<Place>>>,
    patrol_index: usize,
    must_deploy: bool,
}

impl Order {
    fn new(uid: u64, keyword: Keyword, type_name: Option<String>, args: Vec<String>) -> Order {
        Order {
            uid,
            keyword,
            args,
            type_name,
            id: None,
            target: None,
            is_impossible: false,
            is_complete: false,
            is_imperative: keyword.imperative_by_default(),
            is_deferred: false,
            first_update: true,
            mode: Mode::None,
            delay: 0,
            storage: None,
            time: 0,
            has_started: false,
            previous_completeness: None,
            creation_time: None,
            patrol_targets: None,
            patrol_index: 0,
            must_deploy: false,
        }
    }
}

/// Costs and arguments of an order a unit could receive, for menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderInfo {
    pub cost: Vec<i64>,
    pub food_cost: i64,
    pub nb_args: usize,
}

const EFFECTS: &[&str] = &[
    "teleportation",
    "recall",
    "conversion",
    "summon",
    "raise_dead",
    "resurrection",
];

impl World {
    fn find_order(&self, unit: EntityId, uid: u64) -> Option<usize> {
        self.entity(unit)?.orders.iter().position(|o| o.uid == uid)
    }

    fn ord(&self, unit: EntityId, uid: u64) -> Option<&Order> {
        let i = self.find_order(unit, uid)?;
        Some(&self.e(unit).orders[i])
    }

    fn ordm(&mut self, unit: EntityId, uid: u64) -> Option<&mut Order> {
        let i = self.find_order(unit, uid)?;
        Some(&mut self.em(unit).orders[i])
    }

    /// `==` between orders, as the original's `__eq__`: `go` orders are
    /// equal for the same target given at the same time, `build` orders for
    /// the same type, target and command; other orders are never equal.
    pub(crate) fn order_eq(&self, a: &Order, b: &Order) -> bool {
        match a.keyword {
            Keyword::Go => {
                b.keyword == Keyword::Go
                    && a.target.as_ref().and_then(|t| self.t_id_text(t))
                        == b.target.as_ref().and_then(|t| self.t_id_text(t))
                    && a.creation_time == b.creation_time
            }
            Keyword::Build => {
                b.keyword == Keyword::Build
                    && a.type_name == b.type_name
                    && a.target.as_ref().and_then(|t| self.t_id_text(t))
                        == b.target.as_ref().and_then(|t| self.t_id_text(t))
                    && a.id == b.id
            }
            _ => false,
        }
    }

    /// `x in list`: the same order or an equal one.
    fn order_in(&self, list: &[Order], o: &Order) -> bool {
        list.iter().any(|x| x.uid == o.uid || self.order_eq(x, o))
    }

    // costs

    fn complex_type(&self, o: &Order) -> Res<&crate::rules::UnitType> {
        o.type_name
            .as_deref()
            .and_then(|n| self.rules.unit_type(n))
            .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'cost'"))
    }

    pub(crate) fn order_cost(&self, unit: EntityId, o: &Order) -> Res<Vec<i64>> {
        if !o.keyword.is_complex() {
            return Ok(vec![0; crate::rules::MAX_NB_OF_RESOURCE_TYPES]);
        }
        let t = self.complex_type(o)?;
        if o.keyword == Keyword::UpgradeTo {
            let uc = self.entity_cost(unit);
            return Ok(t
                .cost
                .iter()
                .enumerate()
                .map(|(i, &c)| c - uc.get(i).copied().unwrap_or(0))
                .collect());
        }
        Ok(t.cost.clone())
    }

    pub(crate) fn order_food_cost(&self, unit: EntityId, o: &Order) -> Res<i64> {
        if !o.keyword.is_complex() {
            return Ok(0);
        }
        let t = self.complex_type(o)?;
        if o.keyword == Keyword::UpgradeTo {
            return Ok(t.stats.food_cost - self.e(unit).stats.food_cost);
        }
        Ok(t.stats.food_cost)
    }

    fn order_time_cost(&self, unit: EntityId, o: &Order) -> Res<i64> {
        let t = self.complex_type(o)?;
        if o.keyword == Keyword::UpgradeTo {
            return Ok(t.stats.time_cost - self.e(unit).stats.time_cost);
        }
        Ok(t.stats.time_cost)
    }

    fn use_effect(&self, o: &Order) -> Res<Vec<String>> {
        self.complex_type(o)?
            .effect
            .clone()
            .ok_or_else(|| Abort::new("'NoneType' object is not subscriptable"))
    }

    /// The costs and the number of targets of an order a unit could
    /// receive (`keyword` or `keyword type`), as the menu shows them.
    pub fn order_info(
        &self,
        unit: EntityId,
        keyword: Keyword,
        type_name: Option<&str>,
    ) -> Option<OrderInfo> {
        let o = Order::new(0, keyword, type_name.map(str::to_owned), Vec::new());
        let nb_args = if keyword == Keyword::Use {
            let t = self.complex_type(&o).ok()?;
            usize::from(t.effect_target == ["ask"])
        } else {
            keyword.nb_args()
        };
        Some(OrderInfo {
            cost: self.order_cost(unit, &o).ok()?,
            food_cost: self.order_food_cost(unit, &o).ok()?,
            nb_args,
        })
    }

    // permissions and menus

    fn additional_condition(&self, unit: EntityId, kw: Keyword, type_name: &str) -> bool {
        match kw {
            Keyword::Research => {
                let Some(p) = self.e(unit).player else {
                    return false;
                };
                if self.pl(p).upgrades.iter().any(|u| u == type_name) {
                    return false;
                }
                for &u in &self.pl(p).units {
                    for w in &self.e(u).orders {
                        if w.keyword == Keyword::Research
                            && w.type_name.as_deref() == Some(type_name)
                        {
                            return false;
                        }
                    }
                }
                true
            }
            Keyword::UpgradeTo => self.e(unit).orders.is_empty(),
            Keyword::Use => self
                .rules
                .get_words(type_name, "effect")
                .and_then(|e| e.first().cloned())
                .is_some_and(|e| EFFECTS.contains(&e.as_str())),
            _ => true,
        }
    }

    fn is_almost_allowed(&self, unit: EntityId, kw: Keyword, type_name: &str) -> bool {
        let Some(attr) = kw.menu_attribute() else {
            return false;
        };
        let e = self.e(unit);
        if !self.type_list(unit, attr).iter().any(|t| t == type_name) {
            return false;
        }
        let Some(p) = e.player else { return false };
        !self.pl(p).forbidden_techs.iter().any(|t| t == type_name)
            && e.orders.last().is_none_or(|o| o.keyword.can_be_followed())
            && self.additional_condition(unit, kw, type_name)
            && self.check_count_limit(p, type_name)
    }

    fn complex_is_allowed(&self, unit: EntityId, kw: Keyword, type_name: &str) -> bool {
        if !self.is_almost_allowed(unit, kw, type_name) {
            return false;
        }
        let Some(p) = self.e(unit).player else {
            return false;
        };
        let req = self
            .rules
            .unit_type(type_name)
            .map(|t| t.requirements.clone())
            .unwrap_or_default();
        self.has_all(p, &req)
    }

    /// Whether a unit may receive an order now (`cls.is_allowed(unit,
    /// *args)`), with its type for `train`, `build`... A complex order
    /// without its type raises in the original.
    pub(crate) fn is_order_allowed(
        &self,
        unit: EntityId,
        kw: Keyword,
        type_name: Option<&str>,
    ) -> Res<bool> {
        if kw.is_complex() {
            let t = type_name.ok_or_else(|| {
                Abort::new("is_allowed() missing 1 required positional argument: 'type_name'")
            })?;
            return Ok(self.complex_is_allowed(unit, kw, t));
        }
        Ok(self.order_is_allowed(unit, kw))
    }

    /// Whether a unit may receive an order without a type.
    pub(crate) fn order_is_allowed(&self, unit: EntityId, kw: Keyword) -> bool {
        let e = self.e(unit);
        match kw {
            Keyword::Stop => !e.is_idle() || !e.orders.is_empty(),
            Keyword::CancelTraining | Keyword::CancelUpgrading => e
                .orders
                .last()
                .is_some_and(|o| o.keyword.cancel_order() == kw.as_str()),
            Keyword::EnableAutoGather => e.auto_gather == Some(false),
            Keyword::DisableAutoGather => e.auto_gather == Some(true),
            Keyword::EnableAutoRepair => e.auto_repair == Some(false),
            Keyword::DisableAutoRepair => e.auto_repair == Some(true),
            Keyword::ModeOffensive => {
                e.can_switch_ai_mode() && e.ai_mode == Some(crate::entity::AiMode::Defensive)
            }
            Keyword::ModeDefensive => {
                e.can_switch_ai_mode() && e.ai_mode == Some(crate::entity::AiMode::Offensive)
            }
            Keyword::RallyingPoint => !self.order_menu(unit, Keyword::Train, false).is_empty(),
            Keyword::AutoAttack | Keyword::AutoExplore | Keyword::Wait => {
                e.player.is_some_and(|p| !self.pl(p).is_human())
            }
            Keyword::Load | Keyword::LoadAll | Keyword::UnloadAll => e.stats.transport_capacity > 0,
            Keyword::Enter => true,
            k if k.is_complex() => false,
            k => e.basic_abilities().contains(&k.as_str()),
        }
    }

    /// The menu entries of one kind of order for a unit: `go`, or
    /// `train footman`, `train archer`... With `strict`, the requirements
    /// must be met too.
    pub fn order_menu(&self, unit: EntityId, kw: Keyword, strict: bool) -> Vec<String> {
        self.menu_requests(unit, kw, strict)
            .iter()
            .map(Request::menu_entry)
            .collect()
    }

    /// The orders of one kind that [`World::order_menu`] lists, without
    /// their target.
    pub(crate) fn menu_requests(&self, unit: EntityId, kw: Keyword, strict: bool) -> Vec<Request> {
        let Some(attr) = kw.menu_attribute() else {
            return Request::plain(kw)
                .filter(|_| self.order_is_allowed(unit, kw))
                .into_iter()
                .collect();
        };
        self.type_list(unit, attr)
            .into_iter()
            .filter(|t| {
                if strict {
                    self.complex_is_allowed(unit, kw, t)
                } else {
                    self.is_almost_allowed(unit, kw, t)
                }
            })
            .filter_map(|t| Request::typed(kw, t))
            .collect()
    }

    /// Every menu entry of a unit, in the original's order of order kinds.
    pub fn full_order_menu(&self, unit: EntityId, strict: bool) -> Vec<String> {
        Keyword::ALL
            .iter()
            .flat_map(|&k| self.order_menu(unit, k, strict))
            .collect()
    }

    // giving orders

    /// `take_order`: gives an order to a unit. An imperative `go` toward a
    /// unit becomes `attack`.
    pub(crate) fn take_order(
        &mut self,
        unit: EntityId,
        mut request: Request,
        forget_previous: bool,
        imperative: bool,
        order_id: Option<u64>,
    ) -> Res {
        let Some(p) = self.e(unit).player else {
            return Err(Abort::new(
                "'NoneType' object has no attribute 'get_object_by_id'",
            ));
        };
        if imperative {
            self.imperative_go(p, &mut request)?;
        }
        if self.e(unit).is_inside() {
            self.notify(unit, EntityEvent::OrderImpossible(None), false);
            return Ok(());
        }
        let kw = request.keyword;
        if !self.is_order_allowed(unit, kw, request.type_name())? {
            self.notify(unit, EntityEvent::OrderImpossible(None), false);
            return Ok(());
        }
        if forget_previous && !kw.never_forget_previous() {
            self.cancel_all_orders(unit, true);
        }
        let uid = self.next_order_uid();
        let mut order = request.stored(uid, self);
        order.id = order_id;
        if imperative {
            order.is_imperative = true;
        }
        self.immediate_action(unit, order)
    }

    /// An imperative `go` toward a unit that has an owner becomes `attack`.
    /// The target is resolved through what `p` perceives or remembers now.
    pub(crate) fn imperative_go(&self, p: PlayerId, request: &mut Request) -> Res {
        if request.keyword != Keyword::Go {
            return Ok(());
        }
        let t = request
            .target()
            .ok_or_else(|| Abort::new("list index out of range"))?;
        if self
            .resolve_target(p, t)?
            .as_ref()
            .and_then(|t| self.t_entity(t))
            .is_some_and(|e| e.player.is_some())
        {
            request.keyword = Keyword::Attack;
        }
        Ok(())
    }

    /// `take_default_order`: the order a click on a target means.
    pub(crate) fn take_default_order(
        &mut self,
        unit: EntityId,
        target: TargetRef,
        forget_previous: bool,
        imperative: bool,
        order_id: Option<u64>,
    ) -> Res {
        // A default order is always one without a type.
        if let Some(request) = self
            .default_keyword(unit, &target)?
            .and_then(Request::plain)
        {
            self.take_order(
                unit,
                request.at(target),
                forget_previous,
                imperative,
                order_id,
            )?;
        }
        Ok(())
    }

    /// Cancels the orders of a unit, last first.
    pub fn cancel_all_orders(&mut self, unit: EntityId, unpay: bool) {
        while let Some(o) = self.em(unit).orders.pop() {
            self.order_cancel(unit, &o, unpay);
        }
    }

    fn order_cancel(&mut self, unit: EntityId, o: &Order, unpay: bool) {
        if !o.keyword.is_production() {
            return;
        }
        let Some(p) = self.e(unit).player else { return };
        // An impossible order was never paid for, or was refunded when it
        // failed; a complete one has delivered what it paid for. Either
        // waits in the queue until the unit's next update.
        if o.is_impossible || o.is_complete {
            self.notify(unit, EntityEvent::OrderOk, false);
            return;
        }
        if unpay && let Ok(cost) = self.order_cost(unit, o) {
            self.unpay(p, &cost);
        }
        if o.has_started
            && let Ok(food) = self.order_food_cost(unit, o)
        {
            self.plm(p).used_food -= food;
        }
        self.notify(unit, EntityEvent::OrderOk, false);
    }

    fn immediate_action(&mut self, unit: EntityId, order: Order) -> Res {
        if order.keyword.is_immediate() {
            return self.immediate_order(unit, order);
        }
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let food = self.order_food_cost(unit, &order)?;
        let limit = self.food_limit();
        if self.e(unit).orders.len() >= ORDERS_QUEUE_LIMIT {
            self.notify(
                unit,
                EntityEvent::OrderImpossible(Some(Reason::TheQueueIsFull)),
                false,
            );
        } else if self.e(unit).orders.is_empty()
            && food != 0
            && self.pl(p).available_food(limit) < self.pl(p).used_food + food
        {
            self.notify(
                unit,
                EntityEvent::OrderImpossible(Some(Reason::NotEnoughFood)),
                false,
            );
        } else {
            let uid = order.uid;
            self.em(unit).orders.push(order);
            self.order_on_queued(unit, uid)?;
        }
        Ok(())
    }

    fn arg0(o: &Order) -> Res<String> {
        o.args
            .first()
            .cloned()
            .ok_or_else(|| Abort::new("list index out of range"))
    }

    fn immediate_order(&mut self, unit: EntityId, order: Order) -> Res {
        let ok = EntityEvent::OrderOk;
        match order.keyword {
            Keyword::Stop => {
                self.cancel_all_orders(unit, true);
                self.stop(unit);
                self.notify(unit, ok, false);
            }
            Keyword::CancelTraining | Keyword::CancelUpgrading => {
                if let Some(o) = self.em(unit).orders.pop() {
                    self.order_cancel(unit, &o, true);
                }
            }
            Keyword::CancelBuilding => {
                let cost = self
                    .e(unit)
                    .site
                    .as_ref()
                    .map(|s| self.rules.ty(s.building_type).cost.clone())
                    .ok_or_else(|| Abort::new("'BuildingSite' object has no attribute 'type'"))?;
                if let Some(p) = self.e(unit).player {
                    self.unpay(p, &cost);
                }
                self.die(unit, None)?;
            }
            Keyword::EnableAutoGather => {
                self.em(unit).auto_gather = Some(true);
                self.notify(unit, ok, false);
            }
            Keyword::DisableAutoGather => {
                self.em(unit).auto_gather = Some(false);
                self.notify(unit, ok, false);
            }
            Keyword::EnableAutoRepair => {
                self.em(unit).auto_repair = Some(true);
                self.notify(unit, ok, false);
            }
            Keyword::DisableAutoRepair => {
                self.em(unit).auto_repair = Some(false);
                self.notify(unit, ok, false);
            }
            Keyword::ModeOffensive => {
                self.em(unit).ai_mode = Some(crate::entity::AiMode::Offensive);
                self.notify(unit, ok, false);
            }
            Keyword::ModeDefensive => {
                self.em(unit).ai_mode = Some(crate::entity::AiMode::Defensive);
                self.notify(unit, ok, false);
            }
            Keyword::RallyingPoint => {
                self.em(unit).rallying_point = Some(Self::arg0(&order)?);
                self.notify(unit, ok, false);
            }
            Keyword::JoinGroup => {
                let name = Self::arg0(&order)?;
                if !self.groups.iter().any(|(n, _)| *n == name) {
                    self.groups.push((name.clone(), Vec::new()));
                }
                let target_list = self.group_list(&name).cloned().unwrap_or_default();
                let current = self
                    .e(unit)
                    .group
                    .as_ref()
                    .and_then(|g| self.group_list(g))
                    .cloned();
                if current.as_ref() != Some(&target_list) {
                    self.em(unit).group = Some(name.clone());
                    if let Some((_, l)) = self.groups.iter_mut().find(|(n, _)| *n == name) {
                        l.push(unit);
                    }
                    self.notify(unit, ok, false);
                }
            }
            Keyword::Enter => {
                let Some(p) = self.e(unit).player else {
                    return Ok(());
                };
                let id = Self::arg0(&order)?;
                let target = self.get_object_by_id(p, &id)?;
                let tid = match &target {
                    Some(Target::Entity(t)) => *t,
                    _ => return Err(Abort::new("the target cannot take orders")),
                };
                self.take_order(
                    tid,
                    Request::load(TargetRef::Entity(unit)),
                    false,
                    false,
                    None,
                )?;
                let water_target = self.e(tid).airground_type == AirGround::Water;
                if !(self.e(unit).airground_type == AirGround::Ground && water_target) {
                    let go = Request::go(TargetRef::Entity(tid));
                    self.take_order(unit, go, true, false, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn group_list(&self, name: &str) -> Option<&Vec<EntityId>> {
        self.groups.iter().find(|(n, _)| n == name).map(|(_, l)| l)
    }

    // order state

    fn mark_as_impossible(&mut self, unit: EntityId, uid: u64, reason: Option<Reason>) {
        if let Some(o) = self.ordm(unit, uid) {
            o.is_impossible = true;
        }
        self.notify(unit, EntityEvent::OrderImpossible(reason), false);
        self.em(unit).distance_to_goal = crate::world::INF;
    }

    fn mark_as_complete(&mut self, unit: EntityId, uid: u64) {
        if let Some(o) = self.ordm(unit, uid) {
            o.is_complete = true;
        }
        self.em(unit).distance_to_goal = 0;
    }

    fn set_target(&mut self, unit: EntityId, uid: u64, t: Option<Target>) {
        if let Some(o) = self.ordm(unit, uid) {
            o.target = t.clone();
        }
        // A reserved project is the same object as the order.
        if let Some(p) = self.e(unit).player {
            for b in &mut self.plm(p).budget {
                if b.uid == uid {
                    b.target = t.clone();
                }
            }
        }
    }

    fn target_of(&self, unit: EntityId, uid: u64) -> Option<Target> {
        self.ord(unit, uid).and_then(|o| o.target.clone())
    }

    fn update_target(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(t) = self.target_of(unit, uid) else {
            return Ok(());
        };
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let nt = self.updated_target(p, &t)?;
        self.set_target(unit, uid, nt);
        Ok(())
    }

    fn get_target_arg(&mut self, unit: EntityId, uid: u64) -> Res<Option<Target>> {
        let Some(p) = self.e(unit).player else {
            return Ok(None);
        };
        let a = self
            .ord(unit, uid)
            .map(Self::arg0)
            .transpose()?
            .unwrap_or_default();
        self.get_object_by_id(p, &a)
    }

    // queueing

    fn order_on_queued(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let ok = EntityEvent::OrderOk;
        match o.keyword {
            Keyword::Train | Keyword::Research | Keyword::UpgradeTo => {
                let cost = self.order_cost(unit, &o)?;
                let food = self.order_food_cost(unit, &o)?;
                if let Some(r) = self.check_if_enough_resources(unit, &cost, food) {
                    self.mark_as_impossible(unit, uid, Some(r));
                    return Ok(());
                }
                if let Some(p) = self.e(unit).player {
                    self.pay(p, &cost);
                }
                let tc = self.order_time_cost(unit, &o)?;
                if let Some(o) = self.ordm(unit, uid) {
                    o.time = tc;
                }
            }
            Keyword::Go => {
                let now = self.time;
                if let Some(o) = self.ordm(unit, uid) {
                    o.creation_time = Some(now);
                }
                let mut t = self.get_target_arg(unit, uid)?;
                if t.is_none() {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                if let Some(e) = t.as_ref().and_then(|t| self.t_entity(t))
                    && e.is_exit()
                    && let Some(os) = e.exit.as_ref().and_then(|x| x.other_side)
                {
                    t = self.e(os).square().map(Target::Square);
                }
                self.set_target(unit, uid, t);
                self.notify(unit, ok, false);
            }
            Keyword::Attack => {
                let t = self.get_target_arg(unit, uid)?;
                let none = t.is_none();
                self.set_target(unit, uid, t);
                if none {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
            }
            Keyword::Patrol => {
                let mut t = self.get_target_arg(unit, uid)?;
                if !matches!(t, Some(Target::Square(_))) {
                    let Some(tt) = t.clone() else {
                        self.mark_as_impossible(unit, uid, None);
                        return Ok(());
                    };
                    let p = self.e(unit).player.expect("checked by take_order");
                    t = match self.t_place(&tt) {
                        TPlace::At(Place::Square(s)) => {
                            self.resolve_target(p, &TargetRef::Square(s))?
                        }
                        TPlace::At(Place::Inside(_)) => None,
                        _ => {
                            self.mark_as_impossible(unit, uid, None);
                            return Ok(());
                        }
                    };
                }
                self.set_target(unit, uid, t.clone());
                self.notify(unit, ok, false);
                let place_of = |t: &Option<Target>| match t {
                    Some(Target::Square(s)) => Some(Place::Square(*s)),
                    _ => None,
                };
                let first_has_targets =
                    self.e(unit).orders.first().is_some_and(|f| {
                        f.keyword == Keyword::Patrol && f.patrol_targets.is_some()
                    });
                if first_has_targets {
                    if let Some(f) = self.em(unit).orders.first_mut()
                        && let Some(tl) = f.patrol_targets.as_mut()
                    {
                        tl.push(place_of(&t));
                    }
                    self.mark_as_complete(unit, uid);
                } else {
                    let here = self.e(unit).place;
                    if let Some(o) = self.ordm(unit, uid) {
                        o.patrol_targets = Some(vec![here, place_of(&t)]);
                        o.patrol_index = 0;
                    }
                }
            }
            Keyword::Gather => {
                let t = self.get_target_arg(unit, uid)?;
                let is_deposit = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.is_deposit());
                self.set_target(unit, uid, t);
                if !is_deposit {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
                if let Some(o) = self.ordm(unit, uid) {
                    o.mode = Mode::None;
                }
            }
            Keyword::AutoAttack => {}
            Keyword::AutoExplore => {
                let Some(p) = self.e(unit).player else {
                    return Ok(());
                };
                if self.pl(p).places_to_explore.is_none() {
                    let mut places: Vec<SquareId> = Vec::new();
                    for name in self.map.starting_squares.clone() {
                        let s = self
                            .square_by_name(&name)
                            .ok_or_else(|| Abort::new(format!("KeyError: {name}")))?;
                        places.push(s);
                    }
                    self.random.shuffle(&mut places);
                    self.plm(p).places_to_explore = Some(places);
                }
            }
            Keyword::Wait => {
                let t = self.get_target_arg(unit, uid)?;
                self.set_target(unit, uid, t);
                if let Some(o) = self.ordm(unit, uid) {
                    o.must_deploy = true;
                }
            }
            Keyword::Block => {
                let t = self.get_target_arg(unit, uid)?;
                let is_exit = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.is_exit());
                self.set_target(unit, uid, t);
                if !is_exit {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
                if let Some(o) = self.ordm(unit, uid) {
                    o.mode = Mode::GoBlock;
                }
            }
            Keyword::Repair | Keyword::BuildPhaseTwo => {
                let t = self.get_target_arg(unit, uid)?;
                let repairable = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.stats.is_repairable != 0);
                self.set_target(unit, uid, t);
                if !repairable {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
                if let Some(o) = self.ordm(unit, uid) {
                    o.mode = Mode::GoBuild;
                }
            }
            Keyword::Build => self.build_on_queued(unit, uid)?,
            Keyword::Use => self.use_on_queued(unit, uid)?,
            Keyword::Load => {
                let t = self.get_target_arg(unit, uid)?;
                let p = self.e(unit).player;
                let same_player = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.player == p);
                self.set_target(unit, uid, t);
                if !same_player {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
            }
            Keyword::LoadAll | Keyword::UnloadAll => {
                let t = self.get_target_arg(unit, uid)?;
                let t = match t {
                    Some(Target::Square(s)) => Some(Target::Square(s)),
                    Some(other) => match self.t_place(&other) {
                        TPlace::At(Place::Square(s)) => Some(Target::Square(s)),
                        TPlace::At(Place::Inside(_)) => {
                            // An interior has no id: the next update finds no
                            // target and the order fails.
                            self.set_target(unit, uid, None);
                            self.notify(unit, ok, false);
                            return Ok(());
                        }
                        _ => None,
                    },
                    None => None,
                };
                let none = t.is_none();
                self.set_target(unit, uid, t);
                if none {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
            }
            _ => {}
        }
        Ok(())
    }

    fn build_on_queued(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let bt = self.complex_type(&o)?.clone();
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let mut t = self.get_target_arg(unit, uid)?;
        let cannot = Some(Reason::CannotBuildHere);
        if bt.stats.is_buildable_on_exits_only != 0 {
            let is_exit = t
                .as_ref()
                .and_then(|t| self.t_entity(t))
                .is_some_and(|e| e.is_exit());
            if !is_exit {
                t = match &t {
                    Some(Target::Square(s)) => self.square_exit(*s).map(Target::Entity),
                    Some(Target::Zoom(z)) => self.zoom_exit(z).map(Target::Entity),
                    _ => None,
                };
                if t.is_none() {
                    self.set_target(unit, uid, None);
                    self.mark_as_impossible(unit, uid, cannot);
                    return Ok(());
                }
            }
            let ex = t.as_ref().and_then(|t| t.entity_id()).expect("an exit");
            if self.exit_is_blocked(ex, crate::pathfinding::Viewer::Nobody, false, false) {
                self.set_target(unit, uid, t);
                self.mark_as_impossible(unit, uid, cannot);
                return Ok(());
            }
        } else if bt.stats.is_buildable_near_water_only != 0
            && !t.as_ref().is_some_and(|t| self.target_is_near_water(t))
        {
            self.set_target(unit, uid, t);
            self.mark_as_impossible(unit, uid, cannot);
            return Ok(());
        } else if bt.stats.is_buildable_anywhere != 0 {
            t = match &t {
                Some(t) => self.target_any_land(t),
                None => None,
            };
            if t.is_none() {
                self.set_target(unit, uid, None);
                self.mark_as_impossible(unit, uid, cannot);
                return Ok(());
            }
        } else {
            let is_land = t
                .as_ref()
                .and_then(|t| self.t_entity(t))
                .is_some_and(|e| e.is_a_building_land());
            if !is_land {
                t = match &t {
                    Some(Target::Square(s)) => self.square_building_land(*s).map(Target::Entity),
                    Some(Target::Zoom(z)) => self.zoom_building_land(z).map(Target::Entity),
                    Some(other) => self
                        .t_entity(other)
                        .and_then(|e| e.building_land)
                        .map(Target::Entity),
                    None => None,
                };
                if t.is_none() {
                    self.set_target(unit, uid, None);
                    self.mark_as_impossible(unit, uid, cannot);
                    return Ok(());
                }
            }
        }
        self.set_target(unit, uid, t.clone());
        let o = self.ord(unit, uid).cloned().expect("queued");
        let reserved = self.order_in(&self.pl(p).budget, &o);
        if !reserved {
            let cost = self.order_cost(unit, &o)?;
            let food = self.order_food_cost(unit, &o)?;
            if let Some(r) = self.check_if_enough_resources(unit, &cost, food) {
                self.mark_as_impossible(unit, uid, Some(r));
                return Ok(());
            }
        }
        let target = t.expect("set above");
        let stage = self.next_stage(unit, Some(&target), false)?;
        let is_unit_place =
            matches!(&target, Target::Square(s) if self.e(unit).place == Some(Place::Square(*s)));
        if stage.is_none() && !is_unit_place {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        self.notify(unit, EntityEvent::OrderOk, false);
        let o = self.ord(unit, uid).cloned().expect("queued");
        if !self.order_in(&self.pl(p).budget, &o) {
            let cost = self.order_cost(unit, &o)?;
            self.pay(p, &cost);
            self.plm(p).budget.push(o);
        }
        Ok(())
    }

    fn use_on_queued(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let t = self.complex_type(&o)?.clone();
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        if t.effect_target == ["ask"] {
            let target = match o.args.first() {
                Some(a) => self.get_object_by_id(p, a)?,
                None => None,
            };
            let none = target.is_none();
            self.set_target(unit, uid, target);
            if none {
                self.mark_as_impossible(unit, uid, None);
                return Ok(());
            }
        } else if t.effect_target == ["random"] {
            let i = self
                .random
                .choice_index(self.squares.len())
                .ok_or_else(|| Abort::new("Cannot choose from an empty sequence"))?;
            self.set_target(
                unit,
                uid,
                Some(Target::Square(crate::entity::SquareId(i as u32))),
            );
        } else if t.effect_target == ["self"] {
            self.set_target(unit, uid, Some(Target::Entity(unit)));
        }
        if self.e(unit).mana < t.stats.mana_cost {
            if self.group_has_enough_mana(p, t.stats.mana_cost) {
                self.mark_as_complete(unit, uid);
            } else {
                self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughMana));
            }
            return Ok(());
        }
        self.notify(unit, EntityEvent::OrderOk, false);
        Ok(())
    }

    fn group_has_enough_mana(&mut self, p: crate::entity::PlayerId, mana: i64) -> bool {
        if self.pl(p).group_had_enough_mana {
            return true;
        }
        let group = self.pl(p).group.clone().unwrap_or_default();
        for u in group {
            if self.e(u).mana > mana {
                self.plm(p).group_had_enough_mana = true;
                return true;
            }
        }
        false
    }

    // execution

    /// `Order.update`: the first update stops the unit, then the order
    /// executes.
    pub(crate) fn order_update(&mut self, unit: EntityId, uid: u64) -> Res {
        if self.ord(unit, uid).is_some_and(|o| o.first_update) {
            self.stop(unit);
            if let Some(o) = self.ordm(unit, uid) {
                o.first_update = false;
            }
        }
        self.order_execute(unit, uid)
    }

    fn order_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        match o.keyword {
            Keyword::Train | Keyword::Research | Keyword::UpgradeTo => {
                self.production_execute(unit, uid)
            }
            Keyword::Go => self.go_execute(unit, uid),
            Keyword::Attack => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let vulnerable = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.stats.is_vulnerable != 0);
                if !vulnerable {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                let t = t.expect("vulnerable");
                if self.near_enough_to_aim(unit, &t)? {
                    self.em(unit).action = Some(crate::entity::Action {
                        kind: crate::entity::ActionKind::Attack,
                        target: Some(t),
                    });
                } else if self.e(unit).is_idle() {
                    self.move_to_or_fail(unit, uid, Some(t))?;
                }
                Ok(())
            }
            Keyword::Patrol => {
                let targets = o.patrol_targets.clone().unwrap_or_default();
                if targets.is_empty() {
                    return Ok(());
                }
                let i = o.patrol_index;
                if self.e(unit).place == targets[i] {
                    if let Some(o) = self.ordm(unit, uid) {
                        o.patrol_index = (i + 1) % targets.len();
                    }
                    self.deploy(unit)?;
                } else if self.e(unit).is_idle() {
                    let t = match targets[i] {
                        Some(Place::Square(s)) => Some(Target::Square(s)),
                        _ => None,
                    };
                    self.move_to_or_fail(unit, uid, t)?;
                }
                Ok(())
            }
            Keyword::Gather => self.gather_execute(unit, uid),
            Keyword::AutoAttack => {
                if self.e(unit).is_idle() {
                    self.start_moving_to_enemy(unit)?;
                }
                Ok(())
            }
            Keyword::AutoExplore => self.auto_explore(unit),
            Keyword::Wait => {
                if o.must_deploy {
                    self.deploy(unit)?;
                    if let Some(o) = self.ordm(unit, uid) {
                        o.must_deploy = false;
                    }
                }
                let Some(p) = self.e(unit).player else {
                    return Ok(());
                };
                if self.time_has_come(p, o.target.as_ref())? {
                    self.mark_as_complete(unit, uid);
                }
                Ok(())
            }
            Keyword::Block => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let mode = self.ord(unit, uid).map(|o| o.mode);
                let Some(t) = t else {
                    if mode == Some(Mode::GoBlock) {
                        return Err(Abort::new("'NoneType' object has no attribute 'place'"));
                    }
                    return Err(Abort::new(
                        "'NoneType' object has no attribute 'add_blocker'",
                    ));
                };
                let exit = t.entity_id().expect("an exit");
                match mode {
                    Some(Mode::GoBlock) => {
                        if self.near_enough(unit, &t) {
                            if let Some(o) = self.ordm(unit, uid) {
                                o.mode = Mode::Block;
                            }
                            self.stop(unit);
                            self.move_on_border(unit, exit);
                        } else if self.e(unit).is_idle() {
                            self.move_to_or_fail(unit, uid, Some(t))?;
                        }
                    }
                    Some(Mode::Block) => self.block(unit, exit),
                    _ => {}
                }
                Ok(())
            }
            Keyword::Repair | Keyword::BuildPhaseTwo => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let done = match &t {
                    None => true,
                    Some(t) => match t {
                        Target::Entity(id) => self.is_fully_repaired(*id),
                        Target::Memory(m) => {
                            let e = &m.snapshot;
                            !e.is_building_site()
                                && e.stats.is_repairable != 0
                                && e.hp == e.stats.hp_max
                        }
                        _ => false,
                    },
                };
                if done {
                    self.mark_as_complete(unit, uid);
                    self.stop(unit);
                    return Ok(());
                }
                let t = t.expect("not done");
                match o.mode {
                    Mode::GoBuild => {
                        if self.near_enough(unit, &t) {
                            if let Some(o) = self.ordm(unit, uid) {
                                o.mode = Mode::Build;
                            }
                            self.stop(unit);
                        } else if self.e(unit).is_idle() {
                            self.move_to_or_fail(unit, uid, Some(t))?;
                        }
                    }
                    Mode::Build => {
                        if !self.near_enough(unit, &t) {
                            if let Some(o) = self.ordm(unit, uid) {
                                o.mode = Mode::GoBuild;
                            }
                            self.stop(unit);
                        } else if let Some(id) = t.entity_id() {
                            self.be_built(id, unit)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            Keyword::Build => self.build_execute(unit, uid),
            Keyword::Use => self.use_execute(unit, uid),
            Keyword::Load => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let te = t.as_ref().and_then(|t| self.t_entity(t)).cloned();
                let Some(te) = te.filter(|te| self.have_enough_space(unit, te)) else {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                };
                let t = t.expect("checked");
                let here = self.e(unit).place;
                let tplace = self.t_place_of(&t);
                if self.e(unit).airground_type == AirGround::Water {
                    let near = match (here, tplace) {
                        (Some(Place::Square(h)), Some(Place::Square(tp))) => {
                            self.square(h).strict_neighbors.contains(&tp)
                                && !self.square(tp).high_ground
                        }
                        _ => false,
                    };
                    if near {
                        self.mark_as_complete(unit, uid);
                        self.load(unit, te.id);
                    } else {
                        self.mark_as_impossible(unit, uid, None);
                    }
                } else if here != tplace {
                    if self.e(unit).stats.speed != 0 {
                        let dest = match tplace {
                            Some(Place::Square(s)) => Some(Target::Square(s)),
                            _ => None,
                        };
                        self.move_to_or_fail(unit, uid, dest)?;
                    } else {
                        self.mark_as_complete(unit, uid);
                    }
                } else {
                    self.mark_as_complete(unit, uid);
                    self.load(unit, te.id);
                }
                Ok(())
            }
            Keyword::LoadAll | Keyword::UnloadAll => {
                self.update_target(unit, uid)?;
                let Some(Target::Square(s)) = self.target_of(unit, uid) else {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                };
                let here = self.e(unit).place;
                if self.e(unit).airground_type == AirGround::Water {
                    let sq = self.square(s);
                    let near = matches!(here, Some(Place::Square(h)) if self.square(h).strict_neighbors.contains(&s))
                        && !sq.high_ground
                        && (o.keyword == Keyword::LoadAll || sq.is_ground);
                    if near {
                        self.mark_as_complete(unit, uid);
                        if o.keyword == Keyword::LoadAll {
                            self.load_all(unit, Some(Place::Square(s)));
                        } else {
                            self.unload_all(unit, Some(s));
                        }
                    } else {
                        self.mark_as_impossible(unit, uid, None);
                    }
                } else if here != Some(Place::Square(s)) {
                    self.move_to_or_fail(unit, uid, Some(Target::Square(s)))?;
                } else {
                    self.mark_as_complete(unit, uid);
                    if o.keyword == Keyword::LoadAll {
                        self.load_all(unit, None);
                    } else {
                        self.unload_all(unit, None);
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn move_to_or_fail(&mut self, unit: EntityId, uid: u64, target: Option<Target>) -> Res {
        if self.e(unit).stats.speed == 0 {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        let smart = self
            .e(unit)
            .player
            .is_some_and(|p| self.pl(p).smart_units());
        if smart {
            self.smart_move_to_or_fail(unit, uid, target)
        } else {
            self.start_moving_to(unit, target.as_ref(), false)?;
            if self.e(unit).is_idle() {
                self.mark_as_impossible(unit, uid, None);
                self.deploy(unit)?;
            }
            Ok(())
        }
    }

    fn smart_move_to_or_fail(&mut self, unit: EntityId, uid: u64, target: Option<Target>) -> Res {
        self.start_moving_to(unit, target.as_ref(), true)?;
        let kw = self.ord(unit, uid).map(|o| o.keyword);
        let grouped = matches!(kw, Some(Keyword::Go | Keyword::Patrol));
        if self.e(unit).is_idle() && grouped {
            let p = self.e(unit).player.expect("smart units have a player");
            let next_square = match &target {
                Some(t) => self.next_square(unit, t, false)?,
                None => None,
            };
            let here = self.e(unit).place;
            // Only a square can hold units and have a menace; any other next
            // stage makes the balance divide by zero (1000).
            let ns_place = match &next_square {
                Some(Target::Square(s)) => Some(Place::Square(*s)),
                _ => None,
            };
            let ns_square = match &next_square {
                Some(Target::Square(s)) => Some(*s),
                _ => None,
            };
            let menace = ns_square.map_or(0, |s| self.enemy_menace(p, s));
            if menace == 0 {
                self.start_moving_to(unit, next_square.as_ref(), false)?;
            } else if next_square
                .as_ref()
                .zip(target.as_ref())
                .is_some_and(|(a, b)| a.same(b))
            {
                if self.balance(p, &[ns_place, here], None, 10) > 11
                    || self.group_is_ready(unit, uid)
                {
                    self.grouped_attack(unit, uid, next_square)?;
                } else {
                    self.deploy(unit)?;
                }
                return Ok(());
            } else if self.balance(p, &[ns_place, here], None, 10) > 11 {
                self.grouped_attack(unit, uid, next_square)?;
            } else {
                self.deploy(unit)?;
                return Ok(());
            }
        }
        if self.e(unit).is_idle() {
            self.mark_as_impossible(unit, uid, None);
            self.deploy(unit)?;
        }
        Ok(())
    }

    fn group_is_ready(&self, unit: EntityId, uid: u64) -> bool {
        let Some(me) = self.ord(unit, uid) else {
            return true;
        };
        let Some(p) = self.e(unit).player else {
            return true;
        };
        for &u in &self.pl(p).units {
            let ue = self.e(u);
            if u != unit
                && ue.orders.first().is_some_and(|f| self.order_eq(f, me))
                && ue.place != self.e(unit).place
            {
                return false;
            }
        }
        true
    }

    fn grouped_attack(&mut self, unit: EntityId, uid: u64, target: Option<Target>) -> Res {
        self.notify(unit, EntityEvent::Attack, false);
        let Some(me) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let here = self.e(unit).place;
        for u in self.pl(p).units.clone() {
            let ue = self.e(u);
            if ue.orders.first().is_some_and(|f| self.order_eq(f, &me)) && ue.place == here {
                self.start_moving_to(u, target.as_ref(), false)?;
            }
        }
        Ok(())
    }

    fn production_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let food = self.order_food_cost(unit, &o)?;
        if !o.has_started {
            let limit = self.food_limit();
            let can_start =
                food == 0 || self.pl(p).available_food(limit) >= self.pl(p).used_food + food;
            if can_start {
                if let Some(o) = self.ordm(unit, uid) {
                    o.has_started = true;
                    o.is_deferred = false;
                }
                self.plm(p).used_food += food;
                self.notify_completeness(unit, uid)?;
            } else if !o.is_deferred {
                if let Some(o) = self.ordm(unit, uid) {
                    o.is_deferred = true;
                }
                self.notify(unit, EntityEvent::ProductionDeferred, false);
            }
        } else if o.time > 0 {
            if let Some(o) = self.ordm(unit, uid) {
                o.time -= VIRTUAL_TIME_INTERVAL;
            }
            self.notify_completeness(unit, uid)?;
        } else {
            self.production_complete(unit, uid)?;
            if let Some(o) = self.ordm(unit, uid) {
                o.is_complete = true;
            }
        }
        Ok(())
    }

    fn notify_completeness(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let tc = self.order_time_cost(unit, &o)?;
        if tc == 0 {
            return Ok(());
        }
        let t = if o.time < 0 {
            0
        } else if o.time > tc {
            tc
        } else {
            o.time
        };
        let c = (tc - t) * 10 / tc;
        if Some(c) != o.previous_completeness {
            self.notify(unit, EntityEvent::Completeness(c), false);
            if let Some(o) = self.ordm(unit, uid) {
                o.previous_completeness = Some(c);
            }
        }
        Ok(())
    }

    fn production_complete(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let t = self.complex_type(&o)?.clone();
        let type_id = self.rules.type_id(&t.name).expect("a type");
        match o.keyword {
            Keyword::Train => {
                let found = if t.airground_type == AirGround::Water
                    && self.e(unit).stats.is_buildable_near_water_only != 0
                {
                    match self.nearest_water(unit) {
                        None => None,
                        Some(w) => {
                            let (x, y) = (self.square(w).x, self.square(w).y);
                            Some((w, self.find_free_space(w, t.airground_type, x, y)))
                        }
                    }
                } else {
                    match self.e(unit).place {
                        Some(Place::Square(s)) => {
                            let (x, y) = (self.e(unit).x, self.e(unit).y);
                            Some((s, self.find_free_space(s, t.airground_type, x, y)))
                        }
                        _ => {
                            return Err(Abort::new(
                                "'Inside' object has no attribute 'find_free_space'",
                            ));
                        }
                    }
                };
                let Some((place, Some((x, y)))) = found else {
                    self.order_cancel(unit, &o, true);
                    self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                    return Ok(());
                };
                let food = self.order_food_cost(unit, &o)?;
                self.plm(p).used_food -= food;
                let u = self
                    .create_creature(type_id, Some(p), Place::Square(place), x, y)
                    .map_err(|_| Abort::new("NotEnoughSpaceError"))?;
                self.notify(u, EntityEvent::Complete, false);
                // The rallying point keeps the text of the target it was
                // given, which reads back as that target.
                let rp = self.e(unit).rallying_point.clone();
                match rp {
                    Some(rp) => {
                        let t = TargetRef::parse(self, &rp)?;
                        self.take_default_order(u, t, true, false, None)?;
                    }
                    None => {
                        // `get_object_by_id(None)` finds nothing.
                    }
                }
            }
            Keyword::Research => {
                self.upgrade_player(&t.name, p)?;
                self.notify(unit, EntityEvent::ResearchComplete, false);
            }
            Keyword::UpgradeTo => {
                let e = self.e(unit).clone();
                let Some(place) = e.place else { return Ok(()) };
                let (mut x, mut y) = (e.x, e.y);
                let (hp, hp_max) = (e.hp, e.stats.hp_max);
                let leave_meadow =
                    e.stats.is_buildable_anywhere == 0 && t.stats.is_buildable_anywhere != 0;
                let consume_meadow =
                    e.stats.is_buildable_anywhere != 0 && t.stats.is_buildable_anywhere == 0;
                let blocked_exit = e.blocked_exit;
                if consume_meadow {
                    let meadow = match place {
                        Place::Square(s) => self.find_nearest_meadow(s, unit),
                        Place::Inside(_) => None,
                    };
                    match meadow {
                        Some(m) => {
                            x = self.e(m).x;
                            y = self.e(m).y;
                            self.delete(m);
                        }
                        None => {
                            self.notify(unit, EntityEvent::OrderImpossible(None), false);
                            return Ok(());
                        }
                    }
                }
                self.delete(unit);
                let nu = self
                    .create_creature(type_id, Some(p), place, x, y)
                    .map_err(|_| Abort::new("NotEnoughSpaceError"))?;
                if let Some(ex) = blocked_exit {
                    self.block(nu, ex);
                }
                if hp != hp_max {
                    self.em(nu).hp = hp;
                }
                self.notify(nu, EntityEvent::Complete, false);
                if leave_meadow && let Place::Square(s) = place {
                    self.create_meadow(s, Some((x, y)));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn go_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        self.update_target(unit, uid)?;
        let Some(t) = self.target_of(unit, uid) else {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        };
        if self.is_in_position(unit, &t) {
            self.hold(unit, t)?;
            self.mark_as_complete(unit, uid);
        } else if self.near_enough(unit, &t) {
            if let Some(tid) = t.entity_id()
                && self.e(tid).is_creature()
                && self.have_enough_space(tid, self.e(unit))
            {
                self.load(tid, unit);
            }
            self.mark_as_complete(unit, uid);
        } else if self.e(unit).is_idle() {
            self.move_to_or_fail(unit, uid, Some(t))?;
        }
        Ok(())
    }

    fn gather_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        if self.ord(unit, uid).is_some_and(|o| o.mode == Mode::None) {
            let m = if self.e(unit).cargo.is_some() {
                Mode::BringBack
            } else {
                Mode::GoGather
            };
            if let Some(o) = self.ordm(unit, uid) {
                o.mode = m;
            }
        }
        self.update_target(unit, uid)?;
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        match o.mode {
            Mode::BringBack => match o.storage {
                None => {
                    let cargo = self
                        .e(unit)
                        .cargo
                        .ok_or_else(|| Abort::new("'NoneType' object is not subscriptable"))?;
                    let Some(here) = self.e(unit).place else {
                        return Ok(());
                    };
                    let storage = self.nearest_warehouse(p, here, cargo.0, false)?;
                    if let Some(o) = self.ordm(unit, uid) {
                        o.storage = storage;
                    }
                    match storage {
                        None => {
                            self.em(unit).cargo = None;
                            self.mark_as_impossible(unit, uid, None);
                        }
                        Some(s) => self.start_moving_to(unit, Some(&Target::Entity(s)), false)?,
                    }
                }
                Some(storage) => {
                    let st = Target::Entity(storage);
                    if self.near_enough(unit, &st) {
                        let delay = self.time + 1000;
                        if let Some(o) = self.ordm(unit, uid) {
                            o.mode = Mode::Store;
                            o.delay = delay;
                        }
                        let r = self.e(unit).cargo.map_or(0, |c| c.0);
                        self.notify(unit, EntityEvent::Store(r), false);
                        self.stop(unit);
                    } else if self.e(unit).is_idle() {
                        self.start_moving_to(unit, Some(&st), false)?;
                        if self.e(unit).is_idle()
                            && let Some(o) = self.ordm(unit, uid)
                        {
                            o.storage = None;
                        }
                    }
                }
            },
            Mode::Store => {
                if self.time > o.delay {
                    if let Some((r, q)) = self.e(unit).cargo {
                        self.store(p, r, q);
                    }
                    self.em(unit).cargo = None;
                    if let Some(o) = self.ordm(unit, uid) {
                        o.mode = Mode::GoGather;
                    }
                }
            }
            Mode::GoGather => match o.target.clone() {
                None => {
                    self.mark_as_impossible(unit, uid, None);
                    self.deploy(unit)?;
                }
                Some(t) => {
                    if self.near_enough(unit, &t) {
                        let et = self
                            .t_entity(&t)
                            .and_then(|e| e.type_id)
                            .map_or(0, |ty| self.rules.ty(ty).stats.extraction_time);
                        let delay = self.time + et;
                        if let Some(o) = self.ordm(unit, uid) {
                            o.mode = Mode::Gather;
                            o.delay = delay;
                        }
                        self.stop(unit);
                    } else if self.e(unit).is_idle() {
                        self.move_to_or_fail(unit, uid, Some(t))?;
                    }
                }
            },
            Mode::Gather => match o.target.clone() {
                None => self.mark_as_impossible(unit, uid, None),
                Some(t) => {
                    if !self.near_enough(unit, &t) {
                        if let Some(o) = self.ordm(unit, uid) {
                            o.mode = Mode::GoGather;
                        }
                        self.stop(unit);
                    } else if self.time > o.delay {
                        let dep = self.t_entity(&t).cloned().expect("a deposit");
                        let ty = dep.type_id.map(|ty| self.rules.ty(ty));
                        let cargo = (
                            ty.and_then(|t| t.resource_type).unwrap_or(0),
                            ty.map_or(0, |t| t.stats.extraction_qty),
                        );
                        self.em(unit).cargo = Some(cargo);
                        if let Some(id) = t.entity_id() {
                            self.extract_resource(id)?;
                        }
                        if let Some(o) = self.ordm(unit, uid) {
                            o.mode = Mode::BringBack;
                            o.storage = None;
                        }
                    }
                }
            },
            _ => {}
        }
        Ok(())
    }

    fn build_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        self.update_target(unit, uid)?;
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let Some(t) = o.target.clone() else {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        };
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let here = self.e(unit).place;
        let at_target = match &t {
            Target::Square(s) => here == Some(Place::Square(*s)),
            _ => self.t_place_of(&t).is_some() && self.t_place_of(&t) == here,
        };
        if at_target {
            self.free_resources(p, &o)?;
            let bt = self.complex_type(&o)?.clone();
            let (tx, ty) = self.t_xy(&t);
            let Some(Place::Square(s)) = here else {
                return Err(Abort::new(
                    "'Inside' object has no attribute 'find_free_space'",
                ));
            };
            if self.find_free_space(s, bt.airground_type, tx, ty).is_none() {
                self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                return Ok(());
            }
            if self.check_count_limit(p, &bt.name) {
                self.put_building_site(unit, uid, &bt.name, t)?;
            } else {
                self.mark_as_impossible(unit, uid, Some(Reason::CountLimitReached));
            }
        } else if self.e(unit).is_idle() {
            self.move_to_or_fail(unit, uid, Some(t))?;
        }
        Ok(())
    }

    fn put_building_site(
        &mut self,
        unit: EntityId,
        uid: u64,
        type_name: &str,
        target: Target,
    ) -> Res {
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let type_id = self.rules.type_id(type_name).expect("a type");
        let bt = self.rules.ty(type_id).clone();
        let target = match target {
            Target::Memory(m) => Target::Entity(m.snapshot.id),
            t => t,
        };
        let (x, y) = self.t_xy(&target);
        let place = match &target {
            Target::Square(s) => Place::Square(*s),
            _ => self
                .t_place_of(&target)
                .ok_or_else(|| Abort::new("'NoneType' object has no attribute 'place'"))?,
        };
        let is_exit = target.entity_id().is_some_and(|e| self.e(e).is_exit());
        let remember_land = !(is_exit || bt.stats.is_buildable_anywhere != 0);
        if remember_land {
            match &target {
                Target::Entity(e) => self.delete(*e),
                _ => return Err(Abort::new("this target cannot be deleted")),
            }
        }
        let site = self
            .create_building_site(p, place, x, y, type_id)
            .map_err(|_| Abort::new("NotEnoughSpaceError"))?;
        if remember_land {
            self.em(site).building_land = target.entity_id();
        }
        if is_exit && let Some(e) = target.entity_id() {
            self.block(site, e);
        }
        let Some(order) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let phase_two = Request::build_phase_two(TargetRef::Entity(site));
        for u in self.pl(p).units.clone() {
            if u == unit {
                continue;
            }
            for n in 0..self.e(u).orders.len() {
                if self.order_eq(&self.e(u).orders[n], &order) {
                    let nuid = self.next_order_uid();
                    let new = phase_two.stored(nuid, self);
                    self.em(u).orders[n] = new;
                    self.order_on_queued(u, nuid)?;
                }
            }
        }
        let nuid = self.next_order_uid();
        let new = phase_two.stored(nuid, self);
        if let Some(i) = self.find_order(unit, uid) {
            self.em(unit).orders[i] = new;
            self.order_on_queued(unit, nuid)?;
        }
        Ok(())
    }

    // use

    fn use_execute(&mut self, unit: EntityId, uid: u64) -> Res {
        self.update_target(unit, uid)?;
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        if o.target.is_none() {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        let t = self.complex_type(&o)?.clone();
        let effect = self.use_effect(&o)?;
        let kind = effect.first().cloned().unwrap_or_default();
        if self.use_is_not_necessary(unit, uid, &kind, &t)? {
            self.mark_as_complete(unit, uid);
            return Ok(());
        }
        let target = self.target_of(unit, uid).expect("checked");
        let (tx, ty) = self.t_xy(&target);
        let (ux, uy) = (self.e(unit).x, self.e(unit).y);
        let r = t.stats.effect_range;
        if square_of_distance(tx, ty, ux, uy) > r * r {
            self.move_to_or_fail(unit, uid, Some(target))?;
            return Ok(());
        }
        self.stop(unit);
        if kind == "conversion" && matches!(target, Target::Memory(_)) {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        if self.e(unit).mana < t.stats.mana_cost {
            self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughMana));
            return Ok(());
        }
        self.use_effect_execute(unit, uid, &kind, &t, &effect)?;
        self.em(unit).mana -= t.stats.mana_cost;
        let universal = t.stats.universal_notification != 0;
        self.notify(unit, EntityEvent::UseComplete(t.name.clone()), universal);
        self.mark_as_complete(unit, uid);
        Ok(())
    }

    fn effect_targets(
        &self,
        unit: EntityId,
        kind: &str,
        t: &crate::rules::UnitType,
        target: &Target,
    ) -> Vec<EntityId> {
        let p = self.e(unit).player;
        let r = t.stats.effect_radius;
        match kind {
            "teleportation" => {
                let (x, y) = (self.e(unit).x, self.e(unit).y);
                self.get_objects(x, y, r, |e| {
                    e.player == p && p.is_some() && e.stats.is_teleportable != 0
                })
            }
            "recall" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| {
                    e.player == p && p.is_some() && e.stats.is_teleportable != 0
                })
            }
            "raise_dead" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| e.is_corpse())
            }
            "resurrection" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| {
                    e.is_corpse() && e.corpse_unit.as_ref().is_some_and(|u| u.player == p)
                })
            }
            _ => Vec::new(),
        }
    }

    fn use_is_not_necessary(
        &mut self,
        unit: EntityId,
        uid: u64,
        kind: &str,
        t: &crate::rules::UnitType,
    ) -> Res<bool> {
        let target = self.target_of(unit, uid).expect("checked");
        match kind {
            "teleportation" => {
                let units = self.effect_targets(unit, kind, t, &target);
                let mut types: Vec<AirGround> = Vec::new();
                for u in &units {
                    let a = self.e(*u).airground_type;
                    if !types.contains(&a) {
                        types.push(a);
                    }
                }
                let target = match target {
                    Target::Square(s) => Target::Square(s),
                    other => {
                        let np = match self.t_place(&other) {
                            TPlace::At(Place::Square(s)) => Target::Square(s),
                            _ => return Err(Abort::new("the target has no square")),
                        };
                        self.set_target(unit, uid, Some(np.clone()));
                        np
                    }
                };
                let Target::Square(s) = target else {
                    unreachable!("a square")
                };
                if self.e(unit).place == Some(Place::Square(s)) {
                    return Ok(true);
                }
                if !types.iter().any(|&a| self.can_receive(s, a, None)) {
                    self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                    return Ok(true);
                }
                Ok(false)
            }
            "recall" => {
                let units = self.effect_targets(unit, kind, t, &target);
                if units.is_empty() {
                    return Ok(true);
                }
                if matches!(target, Target::Square(s) if self.e(unit).place == Some(Place::Square(s)))
                {
                    return Ok(true);
                }
                let mut types: Vec<AirGround> = Vec::new();
                for u in &units {
                    let a = self.e(*u).airground_type;
                    if !types.contains(&a) {
                        types.push(a);
                    }
                }
                let Some(Place::Square(here)) = self.e(unit).place else {
                    return Err(Abort::new("'Inside' object has no attribute 'can_receive'"));
                };
                if !types.iter().any(|&a| self.can_receive(here, a, None)) {
                    self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                    return Ok(true);
                }
                Ok(false)
            }
            "conversion" => Ok(!self.is_an_enemy(unit, &target)),
            "summon" => Ok(false),
            "raise_dead" | "resurrection" => {
                Ok(self.effect_targets(unit, kind, t, &target).is_empty())
            }
            other => Err(Abort::new(format!(
                "'UseOrder' object has no attribute '{other}_is_not_necessary'"
            ))),
        }
    }

    fn use_effect_execute(
        &mut self,
        unit: EntityId,
        uid: u64,
        kind: &str,
        t: &crate::rules::UnitType,
        effect: &[String],
    ) -> Res {
        let target = self.target_of(unit, uid).expect("checked");
        let Some(p) = self.e(unit).player else {
            return Ok(());
        };
        let decay = || -> Res<i64> {
            let s = effect
                .get(1)
                .ok_or_else(|| Abort::new("list index out of range"))?;
            to_int(s).map_err(|_| Abort::new(format!("could not convert string to float: {s:?}")))
        };
        match kind {
            "teleportation" => {
                let Target::Square(s) = target else {
                    return Ok(());
                };
                for u in self.effect_targets(unit, kind, t, &Target::Entity(unit)) {
                    let a = self.e(u).airground_type;
                    if self.can_receive(s, a, None) {
                        let _ = self.move_to(u, Some(Place::Square(s)), None, 90);
                    }
                }
            }
            "recall" => {
                let nearest_water = self.nearest_water(unit);
                for u in self.effect_targets(unit, kind, t, &target) {
                    let Some(Place::Square(mut place)) = self.e(unit).place else {
                        continue;
                    };
                    let a = self.e(u).airground_type;
                    if a == AirGround::Water && !self.square(place).is_water {
                        match nearest_water {
                            Some(w) => place = w,
                            None => continue,
                        }
                    }
                    if self.can_receive(place, a, None) {
                        let _ = self.move_to(u, Some(Place::Square(place)), None, 90);
                    }
                }
            }
            "conversion" => {
                if let Some(id) = target.entity_id() {
                    self.set_player(id, Some(p));
                }
            }
            "summon" => {
                let d = decay()?;
                let items = effect.get(2..).unwrap_or(&[]).to_vec();
                self.lang_add_units(p, &items, Some(&target), d, None, false);
            }
            "raise_dead" => {
                let d = decay()?;
                let (tx, ty) = self.t_xy(&target);
                let mut corpses = self.effect_targets(unit, kind, t, &target);
                corpses.sort_by_key(|&c| square_of_distance(tx, ty, self.e(c).x, self.e(c).y));
                let items = effect.get(2..).unwrap_or(&[]).to_vec();
                self.lang_add_units(p, &items, None, d, Some(corpses), false);
            }
            "resurrection" => {
                let n = effect
                    .get(1)
                    .and_then(|s| crate::nofloat::py_int(s).ok())
                    .ok_or_else(|| Abort::new("invalid literal for int()"))?;
                let (tx, ty) = self.t_xy(&target);
                let mut corpses = self.effect_targets(unit, kind, t, &target);
                corpses.sort_by_key(|&c| square_of_distance(tx, ty, self.e(c).x, self.e(c).y));
                for _ in 0..n {
                    if corpses.is_empty() {
                        break;
                    }
                    let c = corpses.remove(0);
                    let Some(dead) = self.e(c).corpse_unit.clone() else {
                        continue;
                    };
                    if !self.check_count_limit(p, &dead.type_name) {
                        continue;
                    }
                    self.resurrect(c, *dead, p)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn resurrect(
        &mut self,
        corpse: EntityId,
        mut u: crate::entity::Entity,
        p: crate::entity::PlayerId,
    ) -> Res {
        u.player = None;
        u.hp = u.stats.hp_max / 3;
        let id = self.register_unplaced(u);
        self.set_player(id, Some(p));
        let c = self.e(corpse);
        let (cp, cx, cy) = (c.place, c.x, c.y);
        if self.move_to(id, cp, Some((cx, cy)), 90).is_err() {
            // The unit never had a place: it leaves the player and the
            // objects updated each turn.
            self.set_player(id, None);
            self.unregister_entity(id);
            return Err(Abort::new("NotEnoughSpaceError"));
        }
        let decay = self.e(id).stats.decay;
        if decay != 0 {
            let t = self.time + decay;
            self.em(id).time_limit = Some(t);
        }
        self.delete(corpse);
        Ok(())
    }

    /// The effect range of a `use` order's type, for the interface: targets
    /// farther than 12 squares need not avoid collisions.
    pub fn use_effect_range(&self, type_name: &str) -> Option<i64> {
        self.rules
            .unit_type(type_name)
            .map(|t| t.stats.effect_range)
    }
}

#[cfg(test)]
mod tests;
