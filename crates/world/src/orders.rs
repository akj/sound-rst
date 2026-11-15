//! Orders given to units (`worldorders.py`).
//!
//! An order is given as a [`Request`]: a keyword, a type for the keywords
//! that take one, and a target (`go 12`, `train footman`, `build farm
//! 25`). A unit keeps a queue of orders and executes the first one.

use crate::abort::{Abort, Invariant, Res};
use crate::command::{KeywordOrder, OrderTarget};
use crate::entity::{AiMode, Creature, EntityId, Place, PlayerId, SquareId, Target};
use crate::events::{EntityEvent, Event, Reason};
use crate::nofloat::{py_int, square_of_distance, to_int};
use crate::questions;
use crate::rules::{AirGround, Rules, TypeId, VIRTUAL_TIME_INTERVAL};
use crate::world::{NotEnoughSpace, TPlace, World};

/// The most orders a unit can queue.
pub const ORDERS_QUEUE_LIMIT: usize = 10;

macro_rules! keywords {
    ($($v:ident = $s:literal),* $(,)?) => {
        /// The kind of an order, by its keyword.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

    pub(crate) fn can_be_followed(self) -> bool {
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
/// A queued order keeps it as given. [`TargetRef::text`] writes it as the
/// map language does, which [`TargetRef::parse`] reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetRef {
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
    /// The target as the map language writes it.
    pub(crate) fn text(self, w: &World) -> String {
        match self {
            TargetRef::Square(s) => w.square(s).id.to_string(),
            TargetRef::SquareNamed(s) => w.square(s).name.clone(),
            TargetRef::Point { square, x, y } => format!("zoom-{}-{x}-{y}", w.square(square).id),
            TargetRef::Entity(e) => e.to_string(),
            TargetRef::Nothing => "None".to_owned(),
        }
    }

    /// The target as reading back its text gives it: an id that is a
    /// square's names the square.
    fn normalized(self, w: &World) -> TargetRef {
        match self {
            TargetRef::Entity(e) => w.square_by_obj_id(e.0).map_or(self, TargetRef::Square),
            t => t,
        }
    }

    /// Reads a target written as the map language, a line of text or a
    /// stored order write it: a point (`zoom-<square>-<x>-<y>`, the square
    /// by name or id), a square's name, a number, or any other word, which
    /// names nothing. The error says why a point cannot be read.
    pub(crate) fn parse(w: &World, word: &str) -> Result<TargetRef, String> {
        if word.starts_with("zoom") {
            let parts: Vec<&str> = word.split('-').collect();
            let [_, square, x, y] = parts[..] else {
                return Err(format!("{word:?} is not zoom-<square>-<x>-<y>"));
            };
            let (TargetRef::Square(square) | TargetRef::SquareNamed(square)) =
                TargetRef::parse(w, square)?
            else {
                return Err(format!("{word:?} names no square"));
            };
            let int = |s: &str| py_int(s).map_err(|_| format!("{s:?} in {word:?} is not a number"));
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
    #[cfg(test)]
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
            [word] => Some(Arg::Target(TargetRef::parse(w, word)?)),
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

    /// What follows the type, in words.
    fn args(&self, w: &World) -> Vec<String> {
        match &self.arg {
            None => Vec::new(),
            Some(Arg::Target(t)) => vec![t.text(w)],
            Some(Arg::Group(name)) => vec![name.clone()],
        }
    }

    /// The order a menu entry lists, without its target.
    pub(crate) fn listed(rules: &Rules, entry: &MenuEntry) -> Request {
        let type_name = entry.type_id.map(|t| rules.ty(t).name.clone());
        Request::of(entry.keyword, type_name, None)
    }

    /// The order in words, as a computer player keeps track of it:
    /// `["build", "farm", "25"]`.
    pub(crate) fn words(&self, w: &World) -> Vec<String> {
        let mut words = vec![self.keyword.as_str().to_owned()];
        words.extend(self.type_name.iter().cloned());
        words.extend(self.args(w));
        words
    }

    /// The first argument as text: a target as [`TargetRef::text`] writes
    /// it, or a group's name.
    fn first_arg(&self, w: &World) -> Res<String> {
        let first = self.args(w).into_iter().next();
        Ok(first.ok_or(Invariant::OrderWithoutItsTarget)?)
    }

    /// Whether the order lacks the target, or the group's name, that its
    /// keyword needs (see [`Keyword::nb_args`]).
    fn lacks_its_target(&self) -> bool {
        match self.keyword {
            Keyword::JoinGroup => self.arg.is_none(),
            k => k.nb_args() > 0 && self.given().is_none(),
        }
    }

    /// What the order was given to aim at. A group's name, which only
    /// `join_group` takes, aims at nothing.
    fn given(&self) -> Option<TargetRef> {
        match &self.arg {
            Some(Arg::Target(t)) => Some(*t),
            Some(Arg::Group(_)) | None => None,
        }
    }

    /// What the order becomes for a unit that takes it: an action done at
    /// once, or an order to queue. An order that lacks its target is refused
    /// before (see [`Request::lacks_its_target`]).
    fn taken(&self, w: &World) -> Res<Taken> {
        let type_id = || {
            self.type_name
                .as_deref()
                .and_then(|n| w.rules.type_id(n))
                .ok_or(Invariant::OrderWithoutItsType)
        };
        let target =
            || -> Res<TargetRef> { Ok(self.given().ok_or(Invariant::OrderWithoutItsTarget)?) };
        let aim = || target().map(|t| Aim::new(Some(t)));
        let now = Taken::Now;
        let queued = Taken::Queued;
        Ok(match self.keyword {
            Keyword::Stop => now(Immediate::Stop),
            Keyword::CancelTraining | Keyword::CancelUpgrading => now(Immediate::CancelLast),
            Keyword::CancelBuilding => now(Immediate::CancelBuilding),
            Keyword::EnableAutoGather => now(Immediate::AutoGather(true)),
            Keyword::DisableAutoGather => now(Immediate::AutoGather(false)),
            Keyword::EnableAutoRepair => now(Immediate::AutoRepair(true)),
            Keyword::DisableAutoRepair => now(Immediate::AutoRepair(false)),
            Keyword::ModeOffensive => now(Immediate::Stance(AiMode::Offensive)),
            Keyword::ModeDefensive => now(Immediate::Stance(AiMode::Defensive)),
            Keyword::RallyingPoint => now(Immediate::RallyingPoint(target()?.normalized(w))),
            Keyword::JoinGroup => now(Immediate::JoinGroup(self.first_arg(w)?)),
            Keyword::Enter => now(Immediate::Enter(target()?)),
            Keyword::Train => queued(OrderKind::Production(Production::new(Product::Train(
                type_id()?,
            )))),
            Keyword::Research => queued(OrderKind::Production(Production::new(Product::Research(
                type_id()?,
            )))),
            Keyword::UpgradeTo => queued(OrderKind::Production(Production::new(
                Product::UpgradeTo(type_id()?),
            ))),
            Keyword::Go => queued(OrderKind::Go {
                aim: aim()?,
                creation_time: None,
            }),
            Keyword::Attack => queued(OrderKind::Attack { aim: aim()? }),
            Keyword::Patrol => queued(OrderKind::Patrol {
                aim: aim()?,
                route: None,
            }),
            Keyword::Gather => queued(OrderKind::Gather {
                aim: aim()?,
                step: GatherStep::Start,
            }),
            Keyword::AutoAttack => queued(OrderKind::AutoAttack),
            Keyword::AutoExplore => queued(OrderKind::AutoExplore),
            Keyword::Wait => queued(OrderKind::Wait {
                aim: aim()?,
                must_deploy: false,
            }),
            Keyword::Block => queued(OrderKind::Block {
                aim: aim()?,
                step: BlockStep::Waiting,
            }),
            Keyword::Repair => queued(OrderKind::Repair {
                aim: aim()?,
                step: BuildStep::Waiting,
            }),
            Keyword::BuildPhaseTwo => queued(OrderKind::BuildPhaseTwo {
                aim: aim()?,
                step: BuildStep::Waiting,
            }),
            Keyword::Build => queued(OrderKind::Build {
                building: type_id()?,
                aim: aim()?,
            }),
            Keyword::Use => queued(OrderKind::Use {
                ability: type_id()?,
                aim: Aim::new(self.given()),
            }),
            Keyword::Load => queued(OrderKind::Load { aim: aim()? }),
            Keyword::LoadAll => queued(OrderKind::LoadAll { aim: aim()? }),
            Keyword::UnloadAll => queued(OrderKind::UnloadAll { aim: aim()? }),
        })
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

/// What a request becomes for the unit that takes it.
enum Taken {
    Now(Immediate),
    Queued(OrderKind),
}

/// An order done at once instead of being queued (see
/// [`Keyword::is_immediate`]).
enum Immediate {
    Stop,
    /// `cancel_training` or `cancel_upgrading`: cancels the last order.
    CancelLast,
    CancelBuilding,
    AutoGather(bool),
    AutoRepair(bool),
    Stance(AiMode),
    RallyingPoint(TargetRef),
    /// The name of the group to join.
    JoinGroup(String),
    /// The transport to enter.
    Enter(TargetRef),
}

/// A queued order's kind, with the state that only this kind has.
#[derive(Clone, Debug, Hash)]
pub(crate) enum OrderKind {
    /// `train`, `research` or `upgrade_to`.
    Production(Production),
    Go {
        aim: Aim,
        /// When it was queued. Two `go` orders are equal only if given at
        /// the same time.
        creation_time: Option<i64>,
    },
    Attack {
        aim: Aim,
    },
    Patrol {
        aim: Aim,
        /// The places patrolled, once queued as the first patrol of the
        /// queue. A later patrol adds its place to the first one's route.
        route: Option<Route>,
    },
    Gather {
        aim: Aim,
        step: GatherStep,
    },
    AutoAttack,
    AutoExplore,
    Wait {
        aim: Aim,
        /// Whether its next update deploys the unit first.
        must_deploy: bool,
    },
    Block {
        aim: Aim,
        step: BlockStep,
    },
    Repair {
        aim: Aim,
        step: BuildStep,
    },
    /// Working on a building site that a `build` order put down.
    BuildPhaseTwo {
        aim: Aim,
        step: BuildStep,
    },
    Build {
        building: TypeId,
        aim: Aim,
    },
    Use {
        ability: TypeId,
        aim: Aim,
    },
    Load {
        aim: Aim,
    },
    LoadAll {
        aim: Aim,
    },
    UnloadAll {
        aim: Aim,
    },
}

impl OrderKind {
    pub(crate) fn keyword(&self) -> Keyword {
        match self {
            OrderKind::Production(p) => match p.product {
                Product::Train(_) => Keyword::Train,
                Product::Research(_) => Keyword::Research,
                Product::UpgradeTo(_) => Keyword::UpgradeTo,
            },
            OrderKind::Go { .. } => Keyword::Go,
            OrderKind::Attack { .. } => Keyword::Attack,
            OrderKind::Patrol { .. } => Keyword::Patrol,
            OrderKind::Gather { .. } => Keyword::Gather,
            OrderKind::AutoAttack => Keyword::AutoAttack,
            OrderKind::AutoExplore => Keyword::AutoExplore,
            OrderKind::Wait { .. } => Keyword::Wait,
            OrderKind::Block { .. } => Keyword::Block,
            OrderKind::Repair { .. } => Keyword::Repair,
            OrderKind::BuildPhaseTwo { .. } => Keyword::BuildPhaseTwo,
            OrderKind::Build { .. } => Keyword::Build,
            OrderKind::Use { .. } => Keyword::Use,
            OrderKind::Load { .. } => Keyword::Load,
            OrderKind::LoadAll { .. } => Keyword::LoadAll,
            OrderKind::UnloadAll { .. } => Keyword::UnloadAll,
        }
    }

    /// The type of a complex order (`footman` in `train footman`).
    pub(crate) fn type_id(&self) -> Option<TypeId> {
        match self {
            OrderKind::Production(p) => Some(p.product.type_id()),
            OrderKind::Build { building, .. } => Some(*building),
            OrderKind::Use { ability, .. } => Some(*ability),
            OrderKind::Go { .. }
            | OrderKind::Attack { .. }
            | OrderKind::Patrol { .. }
            | OrderKind::Gather { .. }
            | OrderKind::AutoAttack
            | OrderKind::AutoExplore
            | OrderKind::Wait { .. }
            | OrderKind::Block { .. }
            | OrderKind::Repair { .. }
            | OrderKind::BuildPhaseTwo { .. }
            | OrderKind::Load { .. }
            | OrderKind::LoadAll { .. }
            | OrderKind::UnloadAll { .. } => None,
        }
    }

    /// The type a complex order pays for, and whether it pays only the
    /// difference with the unit's own type (`upgrade_to`).
    fn priced(&self) -> Option<(TypeId, bool)> {
        match self {
            OrderKind::Production(p) => Some((
                p.product.type_id(),
                matches!(p.product, Product::UpgradeTo(_)),
            )),
            OrderKind::Build { .. } | OrderKind::Use { .. } => self.type_id().map(|t| (t, false)),
            OrderKind::Go { .. }
            | OrderKind::Attack { .. }
            | OrderKind::Patrol { .. }
            | OrderKind::Gather { .. }
            | OrderKind::AutoAttack
            | OrderKind::AutoExplore
            | OrderKind::Wait { .. }
            | OrderKind::Block { .. }
            | OrderKind::Repair { .. }
            | OrderKind::BuildPhaseTwo { .. }
            | OrderKind::Load { .. }
            | OrderKind::LoadAll { .. }
            | OrderKind::UnloadAll { .. } => None,
        }
    }

    /// What the order aims at, for the kinds that take a target.
    pub(crate) fn aim(&self) -> Option<&Aim> {
        match self {
            OrderKind::Go { aim, .. }
            | OrderKind::Attack { aim }
            | OrderKind::Patrol { aim, .. }
            | OrderKind::Gather { aim, .. }
            | OrderKind::Wait { aim, .. }
            | OrderKind::Block { aim, .. }
            | OrderKind::Repair { aim, .. }
            | OrderKind::BuildPhaseTwo { aim, .. }
            | OrderKind::Build { aim, .. }
            | OrderKind::Use { aim, .. }
            | OrderKind::Load { aim }
            | OrderKind::LoadAll { aim }
            | OrderKind::UnloadAll { aim } => Some(aim),
            OrderKind::Production(_) | OrderKind::AutoAttack | OrderKind::AutoExplore => None,
        }
    }

    fn aim_mut(&mut self) -> Option<&mut Aim> {
        match self {
            OrderKind::Go { aim, .. }
            | OrderKind::Attack { aim }
            | OrderKind::Patrol { aim, .. }
            | OrderKind::Gather { aim, .. }
            | OrderKind::Wait { aim, .. }
            | OrderKind::Block { aim, .. }
            | OrderKind::Repair { aim, .. }
            | OrderKind::BuildPhaseTwo { aim, .. }
            | OrderKind::Build { aim, .. }
            | OrderKind::Use { aim, .. }
            | OrderKind::Load { aim }
            | OrderKind::LoadAll { aim }
            | OrderKind::UnloadAll { aim } => Some(aim),
            OrderKind::Production(_) | OrderKind::AutoAttack | OrderKind::AutoExplore => None,
        }
    }
}

/// What an order was given to aim at, and what that means now.
#[derive(Clone, Debug, Hash)]
pub(crate) struct Aim {
    /// The target as given; `None` for an order given without one, which
    /// fails when it is queued.
    given: Option<TargetRef>,
    /// What it aims at, as its player perceives or remembers it: set when
    /// the order is queued, and kept up to date while it runs.
    pub(crate) target: Option<Target>,
}

impl Aim {
    fn new(given: Option<TargetRef>) -> Aim {
        Aim {
            given,
            target: None,
        }
    }
}

/// What a production order makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Product {
    Train(TypeId),
    Research(TypeId),
    UpgradeTo(TypeId),
}

impl Product {
    pub(crate) fn type_id(self) -> TypeId {
        match self {
            Product::Train(t) | Product::Research(t) | Product::UpgradeTo(t) => t,
        }
    }
}

/// A `train`, `research` or `upgrade_to` order: what it makes and how far
/// it is.
#[derive(Clone, Debug, Hash)]
pub(crate) struct Production {
    pub(crate) product: Product,
    /// The time left, set when the order is queued and paid for.
    pub(crate) time_left: i64,
    /// The completeness last announced, in tenths.
    previous_completeness: Option<i64>,
}

impl Production {
    fn new(product: Product) -> Production {
        Production {
            product,
            time_left: 0,
            previous_completeness: None,
        }
    }
}

/// The route of a patrol.
#[derive(Clone, Debug, Hash)]
pub(crate) struct Route {
    /// The places patrolled, in order: `None` for a place that is not a
    /// square.
    pub(crate) places: Vec<Option<Place>>,
    /// The place it goes to next.
    index: usize,
}

/// The steps of a `gather` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GatherStep {
    /// Queued: its first update chooses between going to the deposit and
    /// bringing back the cargo it carries.
    Start,
    /// Carrying the cargo to a warehouse: the one chosen, once chosen.
    BringBack { storage: Option<EntityId> },
    /// Storing the cargo, until a time.
    Store { until: i64 },
    /// Going to the deposit.
    GoGather,
    /// Extracting, until a time.
    Extract { until: i64 },
}

/// The steps of a `block` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BlockStep {
    /// Queued, its target not yet accepted as an exit.
    Waiting,
    /// Going to the exit.
    Going,
    /// Blocking the exit.
    Blocking,
}

/// The steps of a `repair` or `build_phase_two` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BuildStep {
    /// Queued, its target not yet accepted as repairable.
    Waiting,
    /// Going to the building.
    Going,
    /// Working on it.
    Working,
}

/// How far an order is. `Deferred` and `Started` are production's
/// alone. `Complete` and `Impossible` are final: the order leaves the
/// queue at the unit's next update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Progress {
    /// Queued and not done; production that has not started.
    Pending,
    /// Production waiting for food before it starts.
    Deferred,
    /// Production that has taken its food and counts its time down.
    Started,
    /// Done.
    Complete,
    /// Failed. Production in this state has cost nothing: it was never
    /// paid for, or it was refunded.
    Impossible,
}

impl Progress {
    /// Whether it waits only to leave the queue.
    fn is_final(self) -> bool {
        match self {
            Progress::Complete | Progress::Impossible => true,
            Progress::Pending | Progress::Deferred | Progress::Started => false,
        }
    }
}

/// What a `use` order found before its effect.
enum UseNeed {
    /// The effect is to be done.
    Effect,
    /// Nothing to do: the order is complete.
    Nothing,
    /// No room where its units would go: the order is impossible.
    NoRoom,
}

/// An order in a unit's queue.
#[derive(Clone, Debug, Hash)]
pub struct Order {
    /// Unique in the world; tells two orders apart.
    pub uid: u64,
    /// The id of the `order` command that gave it, shared by the units of
    /// a group.
    pub id: Option<u64>,
    pub is_imperative: bool,
    progress: Progress,
    first_update: bool,
    pub(crate) kind: OrderKind,
}

impl Order {
    fn new(uid: u64, kind: OrderKind) -> Order {
        Order {
            uid,
            id: None,
            is_imperative: kind.keyword().imperative_by_default(),
            progress: Progress::Pending,
            first_update: true,
            kind,
        }
    }

    pub fn keyword(&self) -> Keyword {
        self.kind.keyword()
    }

    /// What it aims at now, for the kinds that take a target.
    pub fn target(&self) -> Option<&Target> {
        self.kind.aim().and_then(|a| a.target.as_ref())
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    pub fn is_complete(&self) -> bool {
        self.progress == Progress::Complete
    }

    pub fn is_impossible(&self) -> bool {
        self.progress == Progress::Impossible
    }

    /// Whether it waits only to leave the queue at the unit's next update.
    pub fn is_final(&self) -> bool {
        self.progress.is_final()
    }

    /// Production waiting for food.
    pub fn is_deferred(&self) -> bool {
        self.progress == Progress::Deferred
    }

    /// Marks it complete: it leaves the queue at the unit's next update.
    pub(crate) fn complete(&mut self) {
        self.progress = Progress::Complete;
    }

    /// The type of a complex order, by its name in `rules`.
    pub fn type_name<'r>(&self, rules: &'r Rules) -> Option<&'r str> {
        self.kind.type_id().map(|t| rules.ty(t).name.as_str())
    }

    /// The target as given, written as [`TargetRef::text`] writes it.
    pub fn args(&self, w: &World) -> Vec<String> {
        self.kind
            .aim()
            .and_then(|a| a.given)
            .map(|t| t.text(w))
            .into_iter()
            .collect()
    }

    /// The order in words, as given: `["build", "farm", "25"]`.
    pub fn words(&self, w: &World) -> Vec<String> {
        let mut words = vec![self.keyword().as_str().to_owned()];
        words.extend(self.type_name(&w.rules).map(str::to_owned));
        words.extend(self.args(w));
        words
    }
}

/// Costs and arguments of an order a unit could receive, for menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderInfo {
    pub cost: Vec<i64>,
    pub food_cost: i64,
    pub nb_args: usize,
}

/// An order a unit may receive, as its order menu lists it: the kind of
/// order and, for a kind that takes one, the type it makes or uses
/// (`footman` in `train footman`) and what that type requires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub keyword: Keyword,
    pub type_id: Option<TypeId>,
    /// The requirements of the type, by the class names the rules give
    /// them: a type, or a class that types derive from.
    pub requirements: Vec<String>,
}

impl MenuEntry {
    /// An order without a type: `None` for a kind that takes one.
    pub fn plain(keyword: Keyword) -> Option<MenuEntry> {
        (!keyword.is_complex()).then(|| MenuEntry {
            keyword,
            type_id: None,
            requirements: Vec::new(),
        })
    }

    /// An order of a kind that takes a type: `None` for another kind.
    pub fn typed(rules: &Rules, keyword: Keyword, t: TypeId) -> Option<MenuEntry> {
        keyword.is_complex().then(|| MenuEntry {
            keyword,
            type_id: Some(t),
            requirements: rules.ty(t).requirements.clone(),
        })
    }

    /// The entry as the original's menus wrote it: `go`, `train footman`.
    pub fn text(&self, rules: &Rules) -> String {
        match self.type_id {
            Some(t) => format!("{} {}", self.keyword.as_str(), rules.ty(t).name),
            None => self.keyword.as_str().to_owned(),
        }
    }
}

impl World {
    fn find_order(&self, unit: EntityId, uid: u64) -> Option<usize> {
        self.entity(unit)?
            .orders()
            .iter()
            .position(|o| o.uid == uid)
    }

    fn ord(&self, unit: EntityId, uid: u64) -> Option<&Order> {
        let i = self.find_order(unit, uid)?;
        self.e(unit).orders().get(i)
    }

    fn ordm(&mut self, unit: EntityId, uid: u64) -> Option<&mut Order> {
        let i = self.find_order(unit, uid)?;
        self.cm(unit)?.orders.get_mut(i)
    }

    /// The kind of a queued order, to change its state; `None` once the
    /// order has left the queue. A uid always names an order of the same
    /// kind, so the callers' patterns on the kind match whenever this
    /// finds the order.
    fn kind_mut(&mut self, unit: EntityId, uid: u64) -> Option<&mut OrderKind> {
        self.ordm(unit, uid).map(|o| &mut o.kind)
    }

    /// `==` between orders, as the original's `__eq__`: `go` orders are
    /// equal for the same target given at the same time, `build` orders for
    /// the same type, target and command; other orders are never equal.
    pub(crate) fn order_eq(&self, a: &Order, b: &Order) -> bool {
        let same_target =
            || a.target().and_then(Target::identity) == b.target().and_then(Target::identity);
        match &a.kind {
            OrderKind::Go { creation_time, .. } => {
                matches!(&b.kind, OrderKind::Go { creation_time: tb, .. } if same_target() && creation_time == tb)
            }
            OrderKind::Build { building, .. } => {
                matches!(&b.kind, OrderKind::Build { building: bb, .. } if building == bb && same_target() && a.id == b.id)
            }
            OrderKind::Production(_)
            | OrderKind::Attack { .. }
            | OrderKind::Patrol { .. }
            | OrderKind::Gather { .. }
            | OrderKind::AutoAttack
            | OrderKind::AutoExplore
            | OrderKind::Wait { .. }
            | OrderKind::Block { .. }
            | OrderKind::Repair { .. }
            | OrderKind::BuildPhaseTwo { .. }
            | OrderKind::Use { .. }
            | OrderKind::Load { .. }
            | OrderKind::LoadAll { .. }
            | OrderKind::UnloadAll { .. } => false,
        }
    }

    /// `x in list`: the same order or an equal one.
    fn order_in(&self, list: &[Order], o: &Order) -> bool {
        list.iter().any(|x| x.uid == o.uid || self.order_eq(x, o))
    }

    // costs

    fn priced(&self, o: &Order) -> Option<(&crate::rules::UnitType, bool)> {
        o.kind
            .priced()
            .map(|(t, upgrade)| (self.rules.ty(t), upgrade))
    }

    pub(crate) fn order_cost(&self, unit: EntityId, o: &Order) -> Vec<i64> {
        questions::priced_cost(&self.rules, &self.e(unit), self.priced(o))
    }

    pub(crate) fn order_food_cost(&self, unit: EntityId, o: &Order) -> i64 {
        questions::priced_food_cost(&self.e(unit), self.priced(o))
    }

    fn production_time_cost(&self, unit: EntityId, p: &Production) -> i64 {
        let t = self.rules.ty(p.product.type_id());
        match p.product {
            Product::UpgradeTo(_) => t.stats.time_cost - self.e(unit).stats.time_cost,
            Product::Train(_) | Product::Research(_) => t.stats.time_cost,
        }
    }

    /// The costs and the number of targets of an order a unit could
    /// receive (`keyword` or `keyword type`), as the menu shows them.
    pub fn order_info(
        &self,
        unit: EntityId,
        keyword: Keyword,
        type_id: Option<TypeId>,
    ) -> Option<OrderInfo> {
        questions::order_info(&self.rules, &self.entity(unit)?, keyword, type_id)
    }

    // permissions and menus

    /// Whether a unit may receive an order now (`cls.is_allowed(unit,
    /// *args)`), with the name of its type for `train`, `build`... A
    /// complex order without its type, or with a type the rules lack, is
    /// not allowed.
    pub(crate) fn is_order_allowed(
        &self,
        unit: EntityId,
        kw: Keyword,
        type_name: Option<&str>,
    ) -> bool {
        let t = type_name.and_then(|n| self.rules.type_id(n));
        questions::is_order_allowed(self, self.e(unit), kw, t)
    }

    /// The menu entries of one kind of order for a unit: `go`, or
    /// `train footman`, `train archer`... With `strict`, the requirements
    /// must be met too.
    pub fn order_menu(&self, unit: EntityId, kw: Keyword, strict: bool) -> Vec<MenuEntry> {
        questions::order_menu(self, self.e(unit), kw, strict)
    }

    /// Every menu entry of a unit, in the original's order of order kinds.
    pub fn full_order_menu(&self, unit: EntityId, strict: bool) -> Vec<MenuEntry> {
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
        // A unit without an owner takes no order. `enter` tells only a unit
        // with an owner to load.
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        if imperative {
            self.imperative_go(p, &mut request);
        }
        let kw = request.keyword;
        if self.e(unit).is_inside()
            || !self.is_order_allowed(unit, kw, request.type_name())
            || request.lacks_its_target()
        {
            self.notify(unit, EntityEvent::OrderImpossible(None), false);
            return Ok(());
        }
        let taken = request.taken(self)?;
        if forget_previous && !kw.never_forget_previous() {
            self.cancel_all_orders(unit, true);
        }
        let uid = self.next_order_uid();
        match taken {
            Taken::Now(immediate) => self.immediate_order(unit, immediate),
            Taken::Queued(kind) => {
                let mut order = Order::new(uid, kind);
                order.id = order_id;
                if imperative {
                    order.is_imperative = true;
                }
                self.queue_order(unit, order)
            }
        }
    }

    /// An imperative `go` toward a unit that has an owner becomes `attack`.
    /// The target is resolved through what `p` perceives or remembers now.
    pub(crate) fn imperative_go(&self, p: PlayerId, request: &mut Request) {
        if request.keyword != Keyword::Go {
            return;
        }
        let Some(t) = request.target() else {
            return;
        };
        if self
            .resolve_target(p, t)
            .as_ref()
            .and_then(|t| self.t_entity(t))
            .is_some_and(|e| e.player().is_some())
        {
            request.keyword = Keyword::Attack;
        }
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
        if let Some(request) = self.default_keyword(unit, &target).and_then(Request::plain) {
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
        while let Some(o) = self.cm(unit).and_then(|c| c.orders.pop()) {
            self.order_cancel(unit, &o, unpay);
        }
    }

    fn order_cancel(&mut self, unit: EntityId, o: &Order, unpay: bool) {
        // Only production pays when it is queued.
        let OrderKind::Production(_) = o.kind else {
            return;
        };
        let Some(p) = self.e(unit).player() else {
            return;
        };
        // An impossible order was never paid for, or was refunded when it
        // failed; a complete one has delivered what it paid for. Either
        // waits in the queue until the unit's next update.
        if o.progress.is_final() {
            self.notify(unit, EntityEvent::OrderOk, false);
            return;
        }
        if unpay {
            let cost = self.order_cost(unit, o);
            self.unpay(p, &cost);
        }
        if o.progress == Progress::Started {
            let food = self.order_food_cost(unit, o);
            self.plm(p).used_food -= food;
        }
        self.notify(unit, EntityEvent::OrderOk, false);
    }

    fn queue_order(&mut self, unit: EntityId, order: Order) -> Res {
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let food = self.order_food_cost(unit, &order);
        let limit = self.food_limit();
        if self.e(unit).orders().len() >= ORDERS_QUEUE_LIMIT {
            self.notify(
                unit,
                EntityEvent::OrderImpossible(Some(Reason::TheQueueIsFull)),
                false,
            );
        } else if self.e(unit).orders().is_empty()
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
            // A unit with an owner is a creature.
            if let Some(c) = self.cm(unit) {
                c.orders.push(order);
            }
            self.order_on_queued(unit, uid)?;
        }
        Ok(())
    }

    fn immediate_order(&mut self, unit: EntityId, order: Immediate) -> Res {
        let ok = EntityEvent::OrderOk;
        match order {
            Immediate::Stop => {
                self.cancel_all_orders(unit, true);
                self.stop(unit);
                self.notify(unit, ok, false);
            }
            Immediate::CancelLast => {
                if let Some(o) = self.cm(unit).and_then(|c| c.orders.pop()) {
                    self.order_cancel(unit, &o, true);
                }
            }
            Immediate::CancelBuilding => {
                let cost = self
                    .e(unit)
                    .site()
                    .as_ref()
                    .map(|s| self.rules.ty(s.building_type).cost.clone())
                    .ok_or(Invariant::CancelOfNoSite)?;
                if let Some(p) = self.e(unit).player() {
                    self.unpay(p, &cost);
                }
                self.die(unit, None)?;
            }
            // `is_order_allowed` gives the switches of auto gathering and
            // repairing only to workers, and the stances only to workers and
            // soldiers.
            Immediate::AutoGather(on) => {
                if let Some(w) = self.cm(unit).and_then(Creature::work_mut) {
                    w.auto_gather = on;
                }
                self.notify(unit, ok, false);
            }
            Immediate::AutoRepair(on) => {
                if let Some(w) = self.cm(unit).and_then(Creature::work_mut) {
                    w.auto_repair = on;
                }
                self.notify(unit, ok, false);
            }
            Immediate::Stance(mode) => {
                if let Some(c) = self.cm(unit) {
                    c.set_stance(mode);
                }
                self.notify(unit, ok, false);
            }
            Immediate::RallyingPoint(target) => {
                if let Some(c) = self.cm(unit) {
                    c.rallying_point = Some(target);
                }
                self.notify(unit, ok, false);
            }
            Immediate::JoinGroup(name) => {
                if !self.groups.iter().any(|(n, _)| *n == name) {
                    self.groups.push((name.clone(), Vec::new()));
                }
                let target_list = self.group_list(&name).cloned().unwrap_or_default();
                let current = self
                    .c(unit)
                    .and_then(|c| c.group.as_ref())
                    .and_then(|g| self.group_list(g))
                    .cloned();
                if current.as_ref() != Some(&target_list) {
                    if let Some(c) = self.cm(unit) {
                        c.group = Some(name.clone());
                    }
                    if let Some((_, l)) = self.groups.iter_mut().find(|(n, _)| *n == name) {
                        l.push(unit);
                    }
                    self.notify(unit, ok, false);
                }
            }
            Immediate::Enter(transport) => {
                let Some(p) = self.e(unit).player() else {
                    return Ok(());
                };
                // Only a unit with an owner, perceived now, can be told to
                // load: a square, a deposit or a memory cannot.
                let tid = match self.resolve_target(p, &transport) {
                    Some(Target::Entity(t)) if self.e(t).player().is_some() => t,
                    _ => {
                        self.notify(unit, EntityEvent::OrderImpossible(None), false);
                        return Ok(());
                    }
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
        }
        Ok(())
    }

    pub(crate) fn group_list(&self, name: &str) -> Option<&Vec<EntityId>> {
        self.groups.iter().find(|(n, _)| n == name).map(|(_, l)| l)
    }

    // order state

    fn mark_as_impossible(&mut self, unit: EntityId, uid: u64, reason: Option<Reason>) {
        if let Some(o) = self.ordm(unit, uid) {
            o.progress = Progress::Impossible;
        }
        self.notify(unit, EntityEvent::OrderImpossible(reason), false);
        if let Some(c) = self.cm(unit) {
            c.distance_to_goal = crate::world::INF;
        }
    }

    fn mark_as_complete(&mut self, unit: EntityId, uid: u64) {
        if let Some(o) = self.ordm(unit, uid) {
            o.complete();
        }
        if let Some(c) = self.cm(unit) {
            c.distance_to_goal = 0;
        }
    }

    fn set_target(&mut self, unit: EntityId, uid: u64, t: Option<Target>) {
        if let Some(aim) = self.kind_mut(unit, uid).and_then(OrderKind::aim_mut) {
            aim.target = t.clone();
        }
        // A reserved project is the same object as the order.
        if let Some(p) = self.e(unit).player() {
            for b in &mut self.plm(p).budget {
                if b.uid == uid
                    && let Some(aim) = b.kind.aim_mut()
                {
                    aim.target = t.clone();
                }
            }
        }
    }

    fn target_of(&self, unit: EntityId, uid: u64) -> Option<Target> {
        self.ord(unit, uid).and_then(|o| o.target().cloned())
    }

    fn update_target(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(t) = self.target_of(unit, uid) else {
            return Ok(());
        };
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let nt = self.updated_target(p, &t)?;
        self.set_target(unit, uid, nt);
        Ok(())
    }

    /// What the target an order was given means to the unit's player now.
    fn resolve_given(&self, unit: EntityId, aim: &Aim) -> Res<Option<Target>> {
        let Some(p) = self.e(unit).player() else {
            return Ok(None);
        };
        let given = aim.given.ok_or(Invariant::OrderWithoutItsTarget)?;
        Ok(self.resolve_target(p, &given))
    }

    // queueing

    fn order_on_queued(&mut self, unit: EntityId, uid: u64) -> Res {
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let ok = EntityEvent::OrderOk;
        match &o.kind {
            OrderKind::Production(production) => {
                let cost = self.order_cost(unit, &o);
                let food = self.order_food_cost(unit, &o);
                if let Some(r) = self.check_if_enough_resources(unit, &cost, food) {
                    self.mark_as_impossible(unit, uid, Some(r));
                    return Ok(());
                }
                if let Some(p) = self.e(unit).player() {
                    self.pay(p, &cost);
                }
                let tc = self.production_time_cost(unit, production);
                if let Some(OrderKind::Production(p)) = self.kind_mut(unit, uid) {
                    p.time_left = tc;
                }
            }
            OrderKind::Go { aim, .. } => {
                let now = self.time;
                if let Some(OrderKind::Go { creation_time, .. }) = self.kind_mut(unit, uid) {
                    *creation_time = Some(now);
                }
                let mut t = self.resolve_given(unit, aim)?;
                if t.is_none() {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                if let Some(e) = t.as_ref().and_then(|t| self.t_entity(t))
                    && e.is_exit()
                    && let Some(os) = e.exit().and_then(|x| x.other_side)
                {
                    t = self.e(os).square().map(Target::Square);
                }
                self.set_target(unit, uid, t);
                self.notify(unit, ok, false);
            }
            OrderKind::Attack { aim } => {
                let t = self.resolve_given(unit, aim)?;
                let none = t.is_none();
                self.set_target(unit, uid, t);
                if none {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
            }
            OrderKind::Patrol { aim, .. } => {
                let mut t = self.resolve_given(unit, aim)?;
                if !matches!(t, Some(Target::Square(_))) {
                    let Some(tt) = t.clone() else {
                        self.mark_as_impossible(unit, uid, None);
                        return Ok(());
                    };
                    let p = self.e(unit).player().expect("checked by take_order");
                    t = match self.t_place(&tt) {
                        TPlace::At(Place::Square(s)) => {
                            self.resolve_target(p, &TargetRef::Square(s))
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
                let place = match &t {
                    Some(Target::Square(s)) => Some(Place::Square(*s)),
                    _ => None,
                };
                // A patrol queued after a first one adds its place to the
                // first one's route.
                if let Some(Order {
                    kind:
                        OrderKind::Patrol {
                            route: Some(route), ..
                        },
                    ..
                }) = self.cm(unit).and_then(|c| c.orders.first_mut())
                {
                    route.places.push(place);
                    self.mark_as_complete(unit, uid);
                } else {
                    let here = self.e(unit).place;
                    if let Some(OrderKind::Patrol { route, .. }) = self.kind_mut(unit, uid) {
                        *route = Some(Route {
                            places: vec![here, place],
                            index: 0,
                        });
                    }
                }
            }
            OrderKind::Gather { aim, .. } => {
                let t = self.resolve_given(unit, aim)?;
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
                self.set_gather_step(unit, uid, GatherStep::Start);
            }
            OrderKind::AutoAttack => {}
            OrderKind::AutoExplore => {
                let Some(p) = self.e(unit).player() else {
                    return Ok(());
                };
                if self.pl(p).places_to_explore.is_none() {
                    let mut places: Vec<SquareId> = Vec::new();
                    for name in self.map.starting_squares.clone() {
                        let s = self
                            .square_by_name(&name)
                            .ok_or(Invariant::UnknownStartingSquare)?;
                        places.push(s);
                    }
                    self.random.shuffle(&mut places);
                    self.plm(p).places_to_explore = Some(places);
                }
            }
            OrderKind::Wait { aim, .. } => {
                let t = self.resolve_given(unit, aim)?;
                self.set_target(unit, uid, t);
                if let Some(OrderKind::Wait { must_deploy, .. }) = self.kind_mut(unit, uid) {
                    *must_deploy = true;
                }
            }
            OrderKind::Block { aim, .. } => {
                let t = self.resolve_given(unit, aim)?;
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
                if let Some(OrderKind::Block { step, .. }) = self.kind_mut(unit, uid) {
                    *step = BlockStep::Going;
                }
            }
            OrderKind::Repair { aim, .. } | OrderKind::BuildPhaseTwo { aim, .. } => {
                let t = self.resolve_given(unit, aim)?;
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
                if let Some(
                    OrderKind::Repair { step, .. } | OrderKind::BuildPhaseTwo { step, .. },
                ) = self.kind_mut(unit, uid)
                {
                    *step = BuildStep::Going;
                }
            }
            OrderKind::Build { building, aim } => {
                self.build_on_queued(unit, uid, *building, aim)?
            }
            OrderKind::Use { ability, aim } => self.use_on_queued(unit, uid, *ability, aim)?,
            OrderKind::Load { aim } => {
                let t = self.resolve_given(unit, aim)?;
                let p = self.e(unit).player();
                let same_player = t
                    .as_ref()
                    .and_then(|t| self.t_entity(t))
                    .is_some_and(|e| e.player() == p);
                self.set_target(unit, uid, t);
                if !same_player {
                    self.mark_as_impossible(unit, uid, None);
                    return Ok(());
                }
                self.notify(unit, ok, false);
            }
            OrderKind::LoadAll { aim } | OrderKind::UnloadAll { aim } => {
                let t = self.resolve_given(unit, aim)?;
                let t = match t {
                    Some(Target::Square(s)) => Some(Target::Square(s)),
                    // A unit inside something is not on a square to load or
                    // unload at.
                    Some(other) => match self.t_place(&other) {
                        TPlace::At(Place::Square(s)) => Some(Target::Square(s)),
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
        }
        Ok(())
    }

    fn build_on_queued(&mut self, unit: EntityId, uid: u64, building: TypeId, aim: &Aim) -> Res {
        let bt = self.rules.ty(building).clone();
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let mut t = self.resolve_given(unit, aim)?;
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
                        .and_then(|e| e.building_land())
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
            let cost = self.order_cost(unit, &o);
            let food = self.order_food_cost(unit, &o);
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
            let cost = self.order_cost(unit, &o);
            self.pay(p, &cost);
            self.plm(p).budget.push(o);
        }
        Ok(())
    }

    fn use_on_queued(&mut self, unit: EntityId, uid: u64, ability: TypeId, aim: &Aim) -> Res {
        let t = self.rules.ty(ability).clone();
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        if t.effect_target == ["ask"] {
            let target = match aim.given {
                Some(given) => self.resolve_target(p, &given),
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
                .ok_or(Invariant::NoSquare)?;
            self.set_target(
                unit,
                uid,
                Some(Target::Square(crate::entity::SquareId(i as u32))),
            );
        } else if t.effect_target == ["self"] {
            self.set_target(unit, uid, Some(Target::Entity(unit)));
        }
        if self.c(unit).map_or(0, |c| c.mana) < t.stats.mana_cost {
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
            // What has no mana has 0.
            if self.c(u).map_or(0, |c| c.mana) > mana {
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
        match &o.kind {
            OrderKind::Production(production) => self.production_execute(unit, &o, production),
            OrderKind::Go { .. } => self.go_execute(unit, uid),
            OrderKind::Attack { .. } => {
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
                    if let Some(c) = self.cm(unit) {
                        c.action = Some(crate::entity::Action {
                            kind: crate::entity::ActionKind::Attack,
                            target: Some(t),
                        });
                    }
                } else if self.e(unit).is_idle() {
                    self.move_to_or_fail(unit, uid, Some(t))?;
                }
                Ok(())
            }
            OrderKind::Patrol { route, .. } => {
                let Some(route) = route.as_ref().filter(|r| !r.places.is_empty()) else {
                    return Ok(());
                };
                let i = route.index;
                let next = route.places[i];
                if self.e(unit).place == next {
                    if let Some(OrderKind::Patrol {
                        route: Some(route), ..
                    }) = self.kind_mut(unit, uid)
                    {
                        route.index = (i + 1) % route.places.len();
                    }
                    self.deploy(unit)?;
                } else if self.e(unit).is_idle() {
                    let t = match next {
                        Some(Place::Square(s)) => Some(Target::Square(s)),
                        _ => None,
                    };
                    self.move_to_or_fail(unit, uid, t)?;
                }
                Ok(())
            }
            OrderKind::Gather { step, .. } => self.gather_execute(unit, uid, *step),
            OrderKind::AutoAttack => {
                if self.e(unit).is_idle() {
                    self.start_moving_to_enemy(unit)?;
                }
                Ok(())
            }
            OrderKind::AutoExplore => self.auto_explore(unit),
            OrderKind::Wait { aim, must_deploy } => {
                if *must_deploy {
                    self.deploy(unit)?;
                    if let Some(OrderKind::Wait { must_deploy, .. }) = self.kind_mut(unit, uid) {
                        *must_deploy = false;
                    }
                }
                let Some(p) = self.e(unit).player() else {
                    return Ok(());
                };
                if self.time_has_come(p, aim.target.as_ref()) {
                    self.mark_as_complete(unit, uid);
                }
                Ok(())
            }
            OrderKind::Block { step, .. } => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let Some(t) = t else {
                    if *step == BlockStep::Going {
                        return Err(Abort::new("'NoneType' object has no attribute 'place'"));
                    }
                    return Err(Abort::new(
                        "'NoneType' object has no attribute 'add_blocker'",
                    ));
                };
                let exit = t.entity_id().expect("an exit");
                match *step {
                    BlockStep::Going => {
                        if self.near_enough(unit, &t) {
                            if let Some(OrderKind::Block { step, .. }) = self.kind_mut(unit, uid) {
                                *step = BlockStep::Blocking;
                            }
                            self.stop(unit);
                            self.move_on_border(unit, exit);
                        } else if self.e(unit).is_idle() {
                            self.move_to_or_fail(unit, uid, Some(t))?;
                        }
                    }
                    BlockStep::Blocking => self.block(unit, exit),
                    BlockStep::Waiting => {}
                }
                Ok(())
            }
            OrderKind::Repair { step, .. } | OrderKind::BuildPhaseTwo { step, .. } => {
                self.update_target(unit, uid)?;
                let t = self.target_of(unit, uid);
                let done = match &t {
                    None => true,
                    Some(t) => match t {
                        Target::Entity(id) => self.is_fully_repaired(*id),
                        Target::Memory(m) => {
                            // A remembered entity of any kind: what has
                            // no hit points has 0.
                            let e = &m.snapshot;
                            !e.is_building_site()
                                && e.stats.is_repairable != 0
                                && e.creature().map_or(0, |c| c.hp) == e.stats.hp_max
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
                let set_step = |w: &mut World, new: BuildStep| {
                    if let Some(
                        OrderKind::Repair { step, .. } | OrderKind::BuildPhaseTwo { step, .. },
                    ) = w.kind_mut(unit, uid)
                    {
                        *step = new;
                    }
                };
                match *step {
                    BuildStep::Going => {
                        if self.near_enough(unit, &t) {
                            set_step(self, BuildStep::Working);
                            self.stop(unit);
                        } else if self.e(unit).is_idle() {
                            self.move_to_or_fail(unit, uid, Some(t))?;
                        }
                    }
                    BuildStep::Working => {
                        if !self.near_enough(unit, &t) {
                            set_step(self, BuildStep::Going);
                            self.stop(unit);
                        } else if let Some(id) = t.entity_id() {
                            self.be_built(id, unit)?;
                        }
                    }
                    BuildStep::Waiting => {}
                }
                Ok(())
            }
            OrderKind::Build { building, .. } => self.build_execute(unit, uid, *building),
            OrderKind::Use { ability, .. } => self.use_execute(unit, uid, *ability),
            OrderKind::Load { .. } => {
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
            OrderKind::LoadAll { .. } => self.load_all_execute(unit, uid, true),
            OrderKind::UnloadAll { .. } => self.load_all_execute(unit, uid, false),
        }
    }

    /// `load_all` (`load`) or `unload_all`.
    fn load_all_execute(&mut self, unit: EntityId, uid: u64, load: bool) -> Res {
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
                && (load || sq.is_ground);
            if near {
                self.mark_as_complete(unit, uid);
                if load {
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
            if load {
                self.load_all(unit, None);
            } else {
                self.unload_all(unit, None);
            }
        }
        Ok(())
    }

    fn move_to_or_fail(&mut self, unit: EntityId, uid: u64, target: Option<Target>) -> Res {
        if self.e(unit).stats.speed == 0 {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        let smart = self
            .e(unit)
            .player()
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
        let grouped = self
            .ord(unit, uid)
            .is_some_and(|o| matches!(o.kind, OrderKind::Go { .. } | OrderKind::Patrol { .. }));
        if self.e(unit).is_idle() && grouped {
            let p = self.e(unit).player().expect("smart units have a player");
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
        let Some(p) = self.e(unit).player() else {
            return true;
        };
        for &u in &self.pl(p).units {
            let ue = self.e(u);
            if u != unit
                && ue.orders().first().is_some_and(|f| self.order_eq(f, me))
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
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let here = self.e(unit).place;
        for u in self.pl(p).units.clone() {
            let ue = self.e(u);
            if ue.orders().first().is_some_and(|f| self.order_eq(f, &me)) && ue.place == here {
                self.start_moving_to(u, target.as_ref(), false)?;
            }
        }
        Ok(())
    }

    fn production_execute(&mut self, unit: EntityId, o: &Order, production: &Production) -> Res {
        let uid = o.uid;
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let food = self.order_food_cost(unit, o);
        match o.progress {
            Progress::Pending | Progress::Deferred => {
                let limit = self.food_limit();
                let can_start =
                    food == 0 || self.pl(p).available_food(limit) >= self.pl(p).used_food + food;
                if can_start {
                    if let Some(o) = self.ordm(unit, uid) {
                        o.progress = Progress::Started;
                    }
                    self.plm(p).used_food += food;
                    self.notify_completeness(unit, uid);
                } else if o.progress == Progress::Pending {
                    if let Some(o) = self.ordm(unit, uid) {
                        o.progress = Progress::Deferred;
                    }
                    self.notify(unit, EntityEvent::ProductionDeferred, false);
                }
            }
            Progress::Started => {
                if production.time_left > 0 {
                    if let Some(OrderKind::Production(p)) = self.kind_mut(unit, uid) {
                        p.time_left -= VIRTUAL_TIME_INTERVAL;
                    }
                    self.notify_completeness(unit, uid);
                } else if self.production_complete(unit, o, production.product)?
                    && let Some(o) = self.ordm(unit, uid)
                {
                    o.complete();
                }
            }
            // A final order leaves the queue before it would run again.
            Progress::Complete | Progress::Impossible => {}
        }
        Ok(())
    }

    fn notify_completeness(&mut self, unit: EntityId, uid: u64) {
        // Read again: the caller may have just counted the time down.
        let Some(Order {
            kind: OrderKind::Production(production),
            ..
        }) = self.ord(unit, uid).cloned()
        else {
            return;
        };
        let tc = self.production_time_cost(unit, &production);
        if tc == 0 {
            return;
        }
        let time = production.time_left;
        let t = if time < 0 {
            0
        } else if time > tc {
            tc
        } else {
            time
        };
        let c = (tc - t) * 10 / tc;
        if Some(c) != production.previous_completeness {
            self.notify(unit, EntityEvent::Completeness(c), false);
            if let Some(OrderKind::Production(p)) = self.kind_mut(unit, uid) {
                p.previous_completeness = Some(c);
            }
        }
    }

    /// Delivers what a production order makes. False when it could not: the
    /// order is then refunded and impossible.
    fn production_complete(&mut self, unit: EntityId, o: &Order, product: Product) -> Res<bool> {
        let uid = o.uid;
        let Some(p) = self.e(unit).player() else {
            return Ok(true);
        };
        match product {
            Product::Train(type_id) => {
                let t = self.rules.ty(type_id).clone();
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
                        _ => return Err(Invariant::ActingInside.into()),
                    }
                };
                let Some((place, Some((x, y)))) = found else {
                    self.order_cancel(unit, o, true);
                    self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                    return Ok(false);
                };
                // The space was just found, and the type is a creature's: a
                // deposit cannot be ordered trained.
                let u = self
                    .create_creature(type_id, Some(p), Place::Square(place), x, y)
                    .map_err(|_| Invariant::NoRoomWhereRoomWas)?;
                let food = self.order_food_cost(unit, o);
                self.plm(p).used_food -= food;
                self.notify(u, EntityEvent::Complete, false);
                if let Some(t) = self.c(unit).and_then(|c| c.rallying_point) {
                    self.take_default_order(u, t, true, false, None)?;
                }
            }
            Product::Research(type_id) => {
                self.upgrade_player(type_id, p)?;
                self.notify(unit, EntityEvent::ResearchComplete, false);
            }
            Product::UpgradeTo(type_id) => {
                let t = self.rules.ty(type_id).clone();
                let e = self.e(unit).clone();
                let Some(place) = e.place else {
                    return Ok(true);
                };
                let (mut x, mut y) = (e.x, e.y);
                // A unit with a production order is a creature.
                let (hp, rallying_point, group) = e.creature().map_or((0, None, None), |c| {
                    (c.hp, c.rallying_point, c.group.clone())
                });
                let hp_max = e.stats.hp_max;
                let leave_meadow =
                    e.stats.is_buildable_anywhere == 0 && t.stats.is_buildable_anywhere != 0;
                let consume_meadow =
                    e.stats.is_buildable_anywhere != 0 && t.stats.is_buildable_anywhere == 0;
                let blocked_exit = e.blocked_exit();
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
                            self.order_cancel(unit, o, true);
                            self.mark_as_impossible(unit, uid, Some(Reason::CannotBuildHere));
                            return Ok(false);
                        }
                    }
                }
                // Taken out first, the units inside stay their player's when
                // the old building is deleted.
                let inside = self
                    .cm(unit)
                    .and_then(|c| c.inside.as_mut())
                    .map(std::mem::take);
                self.delete(unit);
                // The old building's spot was just freed, and the type is a
                // creature's: a deposit cannot be ordered upgraded to.
                let nu = self
                    .create_creature(type_id, Some(p), place, x, y)
                    .map_err(|_| Invariant::NoRoomWhereRoomWas)?;
                if let Some(ex) = blocked_exit {
                    self.block(nu, ex);
                }
                // `nu` was just created as a creature.
                if let Some(c) = self.cm(nu) {
                    if hp != hp_max {
                        c.hp = hp;
                    }
                    c.rallying_point = rallying_point;
                }
                // They go into the new building, or out beside it if they no
                // longer fit: without collision if the square is full, as
                // from a building destroyed.
                for o in inside.unwrap_or_default() {
                    if self.have_enough_space(nu, self.e(o)) {
                        let _ = self.move_to(o, Some(Place::Inside(nu)), Some((0, 0)), 90);
                    } else {
                        let _ = self.move_to(o, Some(place), Some((x, y)), 90);
                        if self.e(o).place == Some(Place::Inside(unit)) {
                            self.em(o).stats.collision = 0;
                            let _ = self.move_to(o, Some(place), Some((x, y)), 90);
                        }
                        self.notify(o, EntityEvent::Exit, false);
                    }
                }
                self.replace_unit(unit, nu, group);
                self.notify(nu, EntityEvent::Complete, false);
                if leave_meadow && let Place::Square(s) = place {
                    self.create_meadow(s, Some((x, y)));
                }
            }
        }
        Ok(true)
    }

    /// Makes what named `old`, in the recallable group `group`, name `new`,
    /// which took its place: the recallable groups, the units a player
    /// controls, and the interface of each player that may give it orders,
    /// told by an event.
    fn replace_unit(&mut self, old: EntityId, new: EntityId, group: Option<String>) {
        let replace = |units: &mut Vec<EntityId>| {
            for u in units.iter_mut().filter(|u| **u == old) {
                *u = new;
            }
        };
        for (_, units) in &mut self.groups {
            replace(units);
        }
        if let Some(c) = self.cm(new) {
            c.group = group;
        }
        let Some(owner) = self.e(new).player() else {
            return;
        };
        for p in self.players.clone() {
            if let Some(units) = self.plm(p).group.as_mut() {
                replace(units);
            }
            if self.pl(p).allied_control.contains(&owner) {
                self.push(p, Event::Replaced { old, new });
            }
        }
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

    fn set_gather_step(&mut self, unit: EntityId, uid: u64, new: GatherStep) {
        if let Some(OrderKind::Gather { step, .. }) = self.kind_mut(unit, uid) {
            *step = new;
        }
    }

    /// What a worker carries. Another kind carries nothing.
    pub(crate) fn set_cargo(&mut self, unit: EntityId, cargo: Option<(i64, i64)>) {
        if let Some(w) = self.cm(unit).and_then(Creature::work_mut) {
            w.cargo = cargo;
        }
    }

    fn gather_execute(&mut self, unit: EntityId, uid: u64, step: GatherStep) -> Res {
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        // Only a worker can be given a gather order (`basic_abilities`):
        // what has no work carries nothing.
        if step == GatherStep::Start {
            let s = if self.e(unit).work().is_some_and(|w| w.cargo.is_some()) {
                GatherStep::BringBack { storage: None }
            } else {
                GatherStep::GoGather
            };
            self.set_gather_step(unit, uid, s);
        }
        self.update_target(unit, uid)?;
        // Read again after the target's update; the order may have left the
        // queue.
        let Some(Order {
            kind: OrderKind::Gather { aim, step },
            ..
        }) = self.ord(unit, uid).cloned()
        else {
            return Ok(());
        };
        match step {
            GatherStep::BringBack { storage: None } => {
                let cargo = self
                    .e(unit)
                    .work()
                    .and_then(|w| w.cargo)
                    .ok_or(Invariant::BringingBackNothing)?;
                let Some(here) = self.e(unit).place else {
                    return Ok(());
                };
                let storage = self.nearest_warehouse(p, here, cargo.0, false)?;
                self.set_gather_step(unit, uid, GatherStep::BringBack { storage });
                match storage {
                    None => {
                        self.set_cargo(unit, None);
                        self.mark_as_impossible(unit, uid, None);
                    }
                    Some(s) => self.start_moving_to(unit, Some(&Target::Entity(s)), false)?,
                }
            }
            GatherStep::BringBack {
                storage: Some(storage),
            } => {
                let st = Target::Entity(storage);
                if self.near_enough(unit, &st) {
                    let until = self.time + 1000;
                    self.set_gather_step(unit, uid, GatherStep::Store { until });
                    let r = self.e(unit).work().and_then(|w| w.cargo).map_or(0, |c| c.0);
                    self.notify(unit, EntityEvent::Store(r), false);
                    self.stop(unit);
                } else if self.e(unit).is_idle() {
                    self.start_moving_to(unit, Some(&st), false)?;
                    if self.e(unit).is_idle() {
                        self.set_gather_step(unit, uid, GatherStep::BringBack { storage: None });
                    }
                }
            }
            GatherStep::Store { until } => {
                if self.time > until {
                    if let Some((r, q)) = self.e(unit).work().and_then(|w| w.cargo) {
                        self.store(p, r, q);
                    }
                    self.set_cargo(unit, None);
                    self.set_gather_step(unit, uid, GatherStep::GoGather);
                }
            }
            GatherStep::GoGather => match aim.target {
                None => {
                    self.mark_as_impossible(unit, uid, None);
                    self.deploy(unit)?;
                }
                Some(t) => {
                    if self.near_enough(unit, &t) {
                        let et = self
                            .t_entity(&t)
                            .and_then(|e| e.type_id())
                            .map_or(0, |ty| self.rules.ty(ty).stats.extraction_time);
                        let until = self.time + et;
                        self.set_gather_step(unit, uid, GatherStep::Extract { until });
                        self.stop(unit);
                    } else if self.e(unit).is_idle() {
                        self.move_to_or_fail(unit, uid, Some(t))?;
                    }
                }
            },
            GatherStep::Extract { until } => match aim.target {
                None => self.mark_as_impossible(unit, uid, None),
                Some(t) => {
                    if !self.near_enough(unit, &t) {
                        self.set_gather_step(unit, uid, GatherStep::GoGather);
                        self.stop(unit);
                    } else if self.time > until {
                        let dep = self.t_entity(&t).cloned().expect("a deposit");
                        let ty = dep.type_id().map(|ty| self.rules.ty(ty));
                        let cargo = (
                            ty.and_then(|t| t.resource_type).unwrap_or(0),
                            ty.map_or(0, |t| t.stats.extraction_qty),
                        );
                        self.set_cargo(unit, Some(cargo));
                        if let Some(id) = t.entity_id() {
                            self.extract_resource(id)?;
                        }
                        self.set_gather_step(unit, uid, GatherStep::BringBack { storage: None });
                    }
                }
            },
            // Left above for one of the other steps.
            GatherStep::Start => {}
        }
        Ok(())
    }

    fn build_execute(&mut self, unit: EntityId, uid: u64, building: TypeId) -> Res {
        self.update_target(unit, uid)?;
        let Some(o) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let Some(t) = o.target().cloned() else {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        };
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
        let here = self.e(unit).place;
        let at_target = match &t {
            Target::Square(s) => here == Some(Place::Square(*s)),
            _ => self.t_place_of(&t).is_some() && self.t_place_of(&t) == here,
        };
        if at_target {
            self.free_resources(p, &o)?;
            let bt = self.rules.ty(building);
            let airground = bt.airground_type;
            let (tx, ty) = self.t_xy(&t);
            let Some(Place::Square(s)) = here else {
                return Err(Invariant::ActingInside.into());
            };
            if self.find_free_space(s, airground, tx, ty).is_none() {
                self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                return Ok(());
            }
            if self.check_count_limit(p, building) {
                self.put_building_site(unit, uid, building, t)?;
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
        type_id: TypeId,
        target: Target,
    ) -> Res {
        let Some(p) = self.e(unit).player() else {
            return Ok(());
        };
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
        // The land of a building that needs it is a meadow: a remembered
        // one was made its entity above.
        if remember_land {
            match &target {
                Target::Entity(e) => self.delete(*e),
                _ => return Err(Invariant::LandIsNotAnEntity.into()),
            }
        }
        // The caller found free space at this point.
        let site = self
            .create_building_site(p, place, x, y, type_id)
            .map_err(|_| Invariant::NoRoomWhereRoomWas)?;
        // `site` was just created as a creature.
        if remember_land && let Some(c) = self.cm(site) {
            c.building_land = target.entity_id();
        }
        if is_exit && let Some(e) = target.entity_id() {
            self.block(site, e);
        }
        let Some(order) = self.ord(unit, uid).cloned() else {
            return Ok(());
        };
        let phase_two = || OrderKind::BuildPhaseTwo {
            aim: Aim::new(Some(TargetRef::Entity(site))),
            step: BuildStep::Waiting,
        };
        for u in self.pl(p).units.clone() {
            if u == unit {
                continue;
            }
            for n in 0..self.e(u).orders().len() {
                if self.order_eq(&self.e(u).orders()[n], &order) {
                    let nuid = self.next_order_uid();
                    if let Some(slot) = self.cm(u).and_then(|c| c.orders.get_mut(n)) {
                        *slot = Order::new(nuid, phase_two());
                    }
                    self.order_on_queued(u, nuid)?;
                }
            }
        }
        let nuid = self.next_order_uid();
        if let Some(slot) = self
            .find_order(unit, uid)
            .and_then(|i| self.cm(unit)?.orders.get_mut(i))
        {
            *slot = Order::new(nuid, phase_two());
            self.order_on_queued(unit, nuid)?;
        }
        Ok(())
    }

    // use

    fn use_execute(&mut self, unit: EntityId, uid: u64, ability: TypeId) -> Res {
        self.update_target(unit, uid)?;
        let Some(o) = self.ord(unit, uid) else {
            return Ok(());
        };
        if o.target().is_none() {
            self.mark_as_impossible(unit, uid, None);
            return Ok(());
        }
        let t = self.rules.ty(ability).clone();
        let effect = t.effect.clone().ok_or(Invariant::UnknownAbility)?;
        let kind = effect.first().cloned().unwrap_or_default();
        match self.use_need(unit, uid, &kind, &t)? {
            UseNeed::Effect => {}
            UseNeed::Nothing => {
                self.mark_as_complete(unit, uid);
                return Ok(());
            }
            UseNeed::NoRoom => {
                self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
                return Ok(());
            }
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
        if self.c(unit).map_or(0, |c| c.mana) < t.stats.mana_cost {
            self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughMana));
            return Ok(());
        }
        if !self.use_effect_execute(unit, uid, &kind, &t, &effect)? {
            self.mark_as_impossible(unit, uid, Some(Reason::NotEnoughSpace));
            return Ok(());
        }
        if let Some(c) = self.cm(unit) {
            c.mana -= t.stats.mana_cost;
        }
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
        let p = self.e(unit).player();
        let r = t.stats.effect_radius;
        match kind {
            "teleportation" => {
                let (x, y) = (self.e(unit).x, self.e(unit).y);
                self.get_objects(x, y, r, |e| {
                    e.player() == p && p.is_some() && e.stats.is_teleportable != 0
                })
            }
            "recall" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| {
                    e.player() == p && p.is_some() && e.stats.is_teleportable != 0
                })
            }
            "raise_dead" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| e.is_corpse())
            }
            "resurrection" => {
                let (x, y) = self.t_xy(target);
                self.get_objects(x, y, r, |e| {
                    e.corpse().is_some_and(|c| c.unit.kind.player == p)
                })
            }
            _ => Vec::new(),
        }
    }

    /// Whether a `use` order has its effect to do. A teleportation aimed
    /// at a unit is aimed at its square from then on.
    fn use_need(
        &mut self,
        unit: EntityId,
        uid: u64,
        kind: &str,
        t: &crate::rules::UnitType,
    ) -> Res<UseNeed> {
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
                    return Ok(UseNeed::Nothing);
                }
                if !types.iter().any(|&a| self.can_receive(s, a, None)) {
                    return Ok(UseNeed::NoRoom);
                }
                Ok(UseNeed::Effect)
            }
            "recall" => {
                let units = self.effect_targets(unit, kind, t, &target);
                if units.is_empty() {
                    return Ok(UseNeed::Nothing);
                }
                if matches!(target, Target::Square(s) if self.e(unit).place == Some(Place::Square(s)))
                {
                    return Ok(UseNeed::Nothing);
                }
                let mut types: Vec<AirGround> = Vec::new();
                for u in &units {
                    let a = self.e(*u).airground_type;
                    if !types.contains(&a) {
                        types.push(a);
                    }
                }
                let Some(Place::Square(here)) = self.e(unit).place else {
                    return Err(Invariant::ActingInside.into());
                };
                if !types.iter().any(|&a| self.can_receive(here, a, None)) {
                    return Ok(UseNeed::NoRoom);
                }
                Ok(UseNeed::Effect)
            }
            "conversion" if self.is_an_enemy(unit, &target) => Ok(UseNeed::Effect),
            "conversion" => Ok(UseNeed::Nothing),
            "summon" => Ok(UseNeed::Effect),
            "raise_dead" | "resurrection" => {
                if self.effect_targets(unit, kind, t, &target).is_empty() {
                    Ok(UseNeed::Nothing)
                } else {
                    Ok(UseNeed::Effect)
                }
            }
            _ => Err(Invariant::UnknownAbility.into()),
        }
    }

    fn use_effect_execute(
        &mut self,
        unit: EntityId,
        uid: u64,
        kind: &str,
        t: &crate::rules::UnitType,
        effect: &[String],
    ) -> Res<bool> {
        let target = self.target_of(unit, uid).expect("checked");
        let Some(p) = self.e(unit).player() else {
            return Ok(true);
        };
        // The rules drop an effect whose numbers they cannot read.
        let decay = || -> Res<i64> {
            let s = effect.get(1).ok_or(Invariant::EffectUnread)?;
            Ok(to_int(s).map_err(|_| Invariant::EffectUnread)?)
        };
        match kind {
            "teleportation" => {
                let Target::Square(s) = target else {
                    return Ok(true);
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
                    .ok_or(Invariant::EffectUnread)?;
                let (tx, ty) = self.t_xy(&target);
                let mut corpses = self.effect_targets(unit, kind, t, &target);
                corpses.sort_by_key(|&c| square_of_distance(tx, ty, self.e(c).x, self.e(c).y));
                let mut risen = false;
                for _ in 0..n {
                    if corpses.is_empty() {
                        break;
                    }
                    let c = corpses.remove(0);
                    // The corpses chosen above.
                    let Some(dead) = self.e(c).corpse().map(|c| c.unit.clone()) else {
                        continue;
                    };
                    if !dead
                        .kind
                        .type_id()
                        .is_some_and(|t| self.check_count_limit(p, t))
                    {
                        continue;
                    }
                    // The next corpses are no nearer to free space.
                    match self.resurrect(c, *dead, p) {
                        Ok(()) => risen = true,
                        Err(NotEnoughSpace) => return Ok(risen),
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn resurrect(
        &mut self,
        corpse: EntityId,
        mut u: crate::entity::Entity<Creature>,
        p: crate::entity::PlayerId,
    ) -> Result<(), NotEnoughSpace> {
        u.kind.player = None;
        u.kind.hp = u.stats.hp_max / 3;
        let id = self.register_unplaced(u.into_entity());
        self.set_player(id, Some(p));
        let c = self.e(corpse);
        let (cp, cx, cy) = (c.place, c.x, c.y);
        if self.move_to(id, cp, Some((cx, cy)), 90).is_err() {
            // The unit never had a place: it leaves the player and the
            // objects updated each turn.
            self.set_player(id, None);
            self.unregister_entity(id);
            return Err(NotEnoughSpace);
        }
        let decay = self.e(id).stats.decay;
        let time = self.time;
        if decay != 0
            && let Some(c) = self.cm(id)
        {
            c.time_limit = Some(time + decay);
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
