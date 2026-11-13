//! The world: the arena of squares, entities and players, and the update
//! loop (`world.py`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::ai::{AiFactory, ComputerAi, IdleAi};
use crate::collision::CollisionMatrix;
use crate::entity::{
    Action, ActionKind, COLLISION_RADIUS, Entity, EntityId, MemoryRef, Place, PlayerId, SquareId,
    Target,
};
use crate::events::{EntityEvent, Event};
use crate::map::MapDef;
use crate::pathfinding::{Graphs, PathCache};
use crate::player::{Player, PlayerKind};
use crate::pyrandom::PyRandom;
use crate::rules::{AiScripts, AirGround, EntityClass, Rules, VIRTUAL_TIME_INTERVAL};
use crate::square::Square;

/// `float("inf")` for path distances.
pub const INF: i64 = i64::MAX;

/// The food limit when the map does not set one.
pub const GLOBAL_FOOD_LIMIT: i64 = 80;

/// An error the original raises and catches with `except: exception("")`:
/// the work in progress stops (the update of one unit, one player's
/// update, one command) and the game goes on. The message goes to the
/// warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Abort(pub String);

impl Abort {
    pub(crate) fn new(s: impl Into<String>) -> Abort {
        Abort(s.into())
    }
}

/// The result of work the original may abandon with an exception.
pub type Res<T = ()> = Result<T, Abort>;

/// There is no free space for an entity where it should appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotEnoughSpace;

/// A default trigger of a game type: owners (`players`, `computers`,
/// `all`, `player1`...), condition and action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultTrigger {
    pub owners: String,
    pub condition: Vec<String>,
    pub action: Vec<String>,
}

/// The default triggers of multiplayer and training games: a player wins
/// when no enemy player is left and loses with its last building; a
/// computer loses with its last unit.
pub fn multiplayer_default_triggers() -> Vec<DefaultTrigger> {
    let t = |o: &str, c: &str, a: &str| DefaultTrigger {
        owners: o.into(),
        condition: vec![c.into()],
        action: vec![a.into()],
    };
    vec![
        t("players", "no_enemy_player_left", "victory"),
        t("players", "no_building_left", "defeat"),
        t("computers", "no_unit_left", "defeat"),
    ]
}

/// What the target of a reference is placed on, as `target.place` in the
/// original: a square is placed in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TPlace {
    Nowhere,
    World,
    At(Place),
}

/// The game world.
pub struct World {
    /// The rules of this game. Tests change them; a game does not.
    pub rules: Rules,
    pub ai_scripts: AiScripts,
    pub(crate) default_triggers: Vec<DefaultTrigger>,
    pub seed: i64,
    /// The world's own id: 1.
    pub id: u32,
    next_id: u32,
    next_order_id: u64,
    next_order_uid: u64,
    current_player_number: u32,
    pub random: PyRandom,
    /// World time in milliseconds.
    pub time: i64,
    pub squares: Vec<Square>,
    pub(crate) grid_names: BTreeMap<String, SquareId>,
    pub(crate) grid_xy: BTreeMap<(i64, i64), SquareId>,
    pub(crate) square_by_obj: BTreeMap<u32, SquareId>,
    entities: Vec<Option<Entity>>,
    pub(crate) active_objects: Vec<EntityId>,
    players_arena: Vec<Player>,
    /// The players still in the game, in creation order.
    pub players: Vec<PlayerId>,
    /// The players who left.
    pub ex_players: Vec<PlayerId>,
    harm_target_types: BTreeMap<(String, String), bool>,
    /// The map definition as parsed.
    pub map: MapDef,
    pub(crate) collision_ground: CollisionMatrix,
    pub(crate) collision_air: CollisionMatrix,
    pub(crate) graphs: Graphs,
    pub(crate) path_cache: PathCache,
    /// The squares each unit observes, during one player's perception update.
    pub(crate) observed_cache: Option<BTreeMap<(EntityId, bool), Vec<SquareId>>>,
    previous_slow_update: i64,
    /// Groups joined with `join_group`, shared by all players as in the
    /// original (where the dict is a class attribute).
    pub groups: Vec<(String, Vec<EntityId>)>,
    pending_commands: Vec<(PlayerId, String)>,
    warnings: Vec<String>,
    ai_factory: Option<Box<AiFactory>>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "World({})", self.seed)
    }
}

impl World {
    /// A world without a map yet. `default_triggers` are used when the map
    /// has no trigger of its own.
    pub fn new(
        rules: Rules,
        ai_scripts: AiScripts,
        default_triggers: Vec<DefaultTrigger>,
        seed: i64,
    ) -> World {
        let mut w = World {
            rules,
            ai_scripts,
            default_triggers,
            seed,
            id: 0,
            next_id: 0,
            next_order_id: 0,
            next_order_uid: 0,
            current_player_number: 0,
            random: PyRandom::new(seed),
            time: 0,
            squares: Vec::new(),
            grid_names: BTreeMap::new(),
            grid_xy: BTreeMap::new(),
            square_by_obj: BTreeMap::new(),
            entities: Vec::new(),
            active_objects: Vec::new(),
            players_arena: Vec::new(),
            players: Vec::new(),
            ex_players: Vec::new(),
            harm_target_types: BTreeMap::new(),
            map: MapDef::default(),
            collision_ground: CollisionMatrix::default(),
            collision_air: CollisionMatrix::default(),
            graphs: Graphs::default(),
            path_cache: PathCache::default(),
            observed_cache: None,
            previous_slow_update: 0,
            groups: Vec::new(),
            pending_commands: Vec::new(),
            warnings: Vec::new(),
            ai_factory: None,
        };
        w.id = w.get_next_id();
        w
    }

    /// Installs the AI created for computer players added from now on.
    pub fn set_ai_factory(&mut self, factory: Box<AiFactory>) {
        self.ai_factory = Some(factory);
    }

    pub(crate) fn make_ai(&self, kind: &PlayerKind) -> Box<dyn ComputerAi> {
        let script = match kind {
            PlayerKind::Computer { ai_type } => self.ai_scripts.get(ai_type),
            _ => None,
        };
        match &self.ai_factory {
            Some(f) => f(kind, script),
            None => Box::new(IdleAi),
        }
    }

    // ids

    pub(crate) fn get_next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn get_next_order_id(&mut self) -> u64 {
        self.next_order_id += 1;
        self.next_order_id
    }

    pub(crate) fn next_order_uid(&mut self) -> u64 {
        self.next_order_uid += 1;
        self.next_order_uid
    }

    pub(crate) fn get_next_player_number(&mut self) -> u32 {
        self.current_player_number += 1;
        self.current_player_number
    }

    /// World time in turns of [`VIRTUAL_TIME_INTERVAL`].
    pub fn turn(&self) -> i64 {
        self.time / VIRTUAL_TIME_INTERVAL
    }

    // warnings

    pub(crate) fn warn(&mut self, s: impl Into<String>) {
        self.warnings.push(s.into());
    }

    /// The warnings since the last call: what the original logs.
    pub fn drain_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    // access

    /// The entity with this id. Deleted entities stay, without a place, as
    /// in the original.
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id.0 as usize).and_then(Option::as_ref)
    }

    /// The entity with this id, to change it. Tests use this; a game does
    /// not.
    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities
            .get_mut(id.0 as usize)
            .and_then(Option::as_mut)
    }

    pub(crate) fn e(&self, id: EntityId) -> &Entity {
        self.entity(id)
            .expect("an entity id always names an entity")
    }

    pub(crate) fn em(&mut self, id: EntityId) -> &mut Entity {
        self.entity_mut(id)
            .expect("an entity id always names an entity")
    }

    /// Every entity ever registered, in id order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter_map(Option::as_ref)
    }

    pub fn square(&self, id: SquareId) -> &Square {
        &self.squares[id.0 as usize]
    }

    pub(crate) fn sqm(&mut self, id: SquareId) -> &mut Square {
        &mut self.squares[id.0 as usize]
    }

    /// The square called `name` (`a1`...).
    pub fn square_by_name(&self, name: &str) -> Option<SquareId> {
        self.grid_names.get(name).copied()
    }

    /// The square at a column and a row, from 0.
    pub fn square_at(&self, col: i64, row: i64) -> Option<SquareId> {
        self.grid_xy.get(&(col, row)).copied()
    }

    /// The square with this protocol id.
    pub fn square_by_obj_id(&self, id: u32) -> Option<SquareId> {
        self.square_by_obj.get(&id).copied()
    }

    pub fn player(&self, id: PlayerId) -> &Player {
        &self.players_arena[id.0 as usize]
    }

    /// The player, to change it. Tests use this; a game does not.
    pub fn player_mut(&mut self, id: PlayerId) -> &mut Player {
        &mut self.players_arena[id.0 as usize]
    }

    pub(crate) fn pl(&self, id: PlayerId) -> &Player {
        &self.players_arena[id.0 as usize]
    }

    pub(crate) fn plm(&mut self, id: PlayerId) -> &mut Player {
        &mut self.players_arena[id.0 as usize]
    }

    /// Every player created, including those who left.
    pub fn all_players(&self) -> impl Iterator<Item = (PlayerId, &Player)> {
        self.players_arena
            .iter()
            .enumerate()
            .map(|(i, p)| (PlayerId(i as u32), p))
    }

    pub(crate) fn add_player(&mut self, p: Player) -> PlayerId {
        self.players_arena.push(p);
        PlayerId(self.players_arena.len() as u32 - 1)
    }

    pub fn square_width(&self) -> i64 {
        self.map.square_width
    }

    pub fn food_limit(&self) -> i64 {
        self.map.global_food_limit
    }

    pub(crate) fn collision(&self, a: AirGround) -> &CollisionMatrix {
        match a {
            AirGround::Air => &self.collision_air,
            _ => &self.collision_ground,
        }
    }

    pub(crate) fn collision_mut(&mut self, a: AirGround) -> &mut CollisionMatrix {
        match a {
            AirGround::Air => &mut self.collision_air,
            _ => &mut self.collision_ground,
        }
    }

    /// The square containing a point.
    pub fn get_place_from_xy(&self, x: i64, y: i64) -> Option<SquareId> {
        let w = self.square_width();
        self.square_at(
            crate::nofloat::floordiv(x, w),
            crate::nofloat::floordiv(y, w),
        )
    }

    pub fn get_subsquare_id_from_xy(&self, x: i64, y: i64) -> (i64, i64) {
        crate::square::subsquare_id(x, y, self.square_width())
    }

    // targets

    /// Whether a remembered entity is still in its owner's memory. A
    /// forgotten one has no place any more.
    pub(crate) fn memory_is_current(&self, m: &MemoryRef) -> bool {
        self.pl(m.owner)
            .memory
            .get(&m.snapshot.id)
            .is_some_and(|r| Arc::ptr_eq(&r.snapshot, &m.snapshot))
    }

    /// The entity a reference shows: the live one, or the remembered copy.
    pub(crate) fn t_entity<'a>(&'a self, t: &'a Target) -> Option<&'a Entity> {
        match t {
            Target::Entity(id) => self.entity(*id),
            Target::Memory(m) => Some(&m.snapshot),
            _ => None,
        }
    }

    /// `target.place`.
    pub(crate) fn t_place(&self, t: &Target) -> TPlace {
        match t {
            Target::Square(_) => TPlace::World,
            Target::Zoom(z) => TPlace::At(Place::Square(z.place)),
            Target::Entity(id) => match self.e(*id).place {
                Some(p) => TPlace::At(p),
                None => TPlace::Nowhere,
            },
            Target::Memory(m) => {
                if !self.memory_is_current(m) {
                    return TPlace::Nowhere;
                }
                match m.snapshot.place {
                    Some(p) => TPlace::At(p),
                    None => TPlace::Nowhere,
                }
            }
            Target::Xy(..) => TPlace::Nowhere,
        }
    }

    /// `target.place` as a place, if it is one.
    pub(crate) fn t_place_of(&self, t: &Target) -> Option<Place> {
        match self.t_place(t) {
            TPlace::At(p) => Some(p),
            _ => None,
        }
    }

    /// `target.x, target.y`.
    pub(crate) fn t_xy(&self, t: &Target) -> (i64, i64) {
        match t {
            Target::Square(s) => (self.square(*s).x, self.square(*s).y),
            Target::Zoom(z) => (z.x, z.y),
            Target::Entity(id) => (self.e(*id).x, self.e(*id).y),
            Target::Memory(m) => (m.snapshot.x, m.snapshot.y),
            Target::Xy(x, y) => (*x, *y),
        }
    }

    /// `target.radius`: squares have none.
    pub(crate) fn t_radius(&self, t: &Target) -> i64 {
        self.t_entity(t).map_or(0, Entity::radius)
    }

    /// `getattr(target, "id", None)` as text.
    pub(crate) fn t_id_text(&self, t: &Target) -> Option<String> {
        t.id_text(|s| self.square(s).id)
    }

    // entity lifecycle

    /// Whether the original's class has an `update` method, which puts its
    /// instances in the active objects.
    fn is_active_class(class: EntityClass) -> bool {
        !matches!(class, EntityClass::Meadow | EntityClass::Exit)
    }

    /// The position a new entity takes on `place`: `(x, y)`, or the place's
    /// center, moved to free space if it has collision.
    pub(crate) fn initial_position(
        &mut self,
        collision: bool,
        airground: AirGround,
        place: Place,
        xy: Option<(i64, i64)>,
    ) -> Result<(i64, i64), NotEnoughSpace> {
        let (x, y) = match (xy, place) {
            (Some(xy), _) => xy,
            (None, Place::Square(s)) => (self.square(s).x, self.square(s).y),
            (None, Place::Inside(c)) => (self.e(c).x, self.e(c).y),
        };
        if !collision {
            return Ok((x, y));
        }
        match place {
            Place::Square(s) => self
                .find_free_space(s, airground, x, y)
                .ok_or(NotEnoughSpace),
            Place::Inside(_) => Ok((x, y)),
        }
    }

    /// Registers a new entity at a position already found free: gives it an
    /// id and puts it on the place (`Entity.__init__` after the space
    /// search).
    pub(crate) fn insert_entity(
        &mut self,
        mut e: Entity,
        place: Place,
        x: i64,
        y: i64,
        o: i64,
    ) -> EntityId {
        let id = EntityId(self.get_next_id());
        e.id = id;
        e.x = x;
        e.y = y;
        e.o = o;
        e.place = None;
        let idx = id.0 as usize;
        if self.entities.len() <= idx {
            self.entities.resize_with(idx + 1, || None);
        }
        let active = Self::is_active_class(e.class);
        self.entities[idx] = Some(e);
        self.enter(place, id);
        if active {
            self.active_objects.push(id);
        }
        let ent = self.em(id);
        ent.previous_place = None;
        ent.place = Some(place);
        if ent.is_creature() {
            ent.action = Some(Action {
                kind: ActionKind::Plain,
                target: None,
            });
        }
        self.occupy_space(id);
        if self.e(id).stats.speed != 0 {
            self.em(id).is_moving = true;
        }
        id
    }

    /// Gives a new id to an entity stored outside the arena (a resurrected
    /// unit) and stores it, without a place.
    pub(crate) fn register_unplaced(&mut self, mut e: Entity) -> EntityId {
        let id = EntityId(self.get_next_id());
        e.id = id;
        e.place = None;
        let idx = id.0 as usize;
        if self.entities.len() <= idx {
            self.entities.resize_with(idx + 1, || None);
        }
        let active = Self::is_active_class(e.class);
        self.entities[idx] = Some(e);
        if active {
            self.active_objects.push(id);
        }
        id
    }

    fn enter(&mut self, place: Place, id: EntityId) {
        match place {
            Place::Square(s) => self.sqm(s).objects.push(id),
            Place::Inside(c) => {
                if let Some(inside) = self.em(c).inside.as_mut() {
                    inside.push(id);
                }
            }
        }
    }

    fn leave(&mut self, place: Place, id: EntityId) {
        let list = match place {
            Place::Square(s) => Some(&mut self.sqm(s).objects),
            Place::Inside(c) => self.em(c).inside.as_mut(),
        };
        if let Some(list) = list
            && let Some(i) = list.iter().position(|&o| o == id)
        {
            list.remove(i);
        }
    }

    pub(crate) fn occupy_space(&mut self, id: EntityId) {
        let e = self.e(id);
        if e.stats.collision == 0 {
            return;
        }
        if let Some(Place::Square(_)) = e.place {
            let (a, x, y) = (e.airground_type, e.x, e.y);
            self.collision_mut(a).add(x, y);
        }
    }

    pub(crate) fn free_space(&mut self, id: EntityId) {
        let e = self.e(id);
        if let Some(Place::Square(_)) = e.place {
            let (a, x, y) = (e.airground_type, e.x, e.y);
            self.collision_mut(a).remove(x, y);
        }
    }

    /// `move_to`: moves an entity to a place, or out of the world with
    /// `None`. With collision, the entity takes the nearest free space; if
    /// there is none, it stays where it is.
    pub(crate) fn move_to(
        &mut self,
        id: EntityId,
        new_place: Option<Place>,
        xy: Option<(i64, i64)>,
        o: i64,
    ) -> Result<(), NotEnoughSpace> {
        let (mut x, mut y) = match (xy, new_place) {
            (Some(xy), _) => xy,
            (None, Some(Place::Square(s))) => (self.square(s).x, self.square(s).y),
            (None, Some(Place::Inside(c))) => (self.e(c).x, self.e(c).y),
            (None, None) => (0, 0),
        };
        let collision = self.e(id).stats.collision != 0;
        if let (Some(np), true) = (new_place, collision) {
            match np {
                Place::Square(s) => {
                    self.free_space(id);
                    let a = self.e(id).airground_type;
                    let found = self.find_free_space(s, a, x, y);
                    self.occupy_space(id);
                    match found {
                        Some((fx, fy)) => {
                            x = fx;
                            y = fy;
                        }
                        None => {
                            if self.e(id).place.is_some() {
                                return Ok(());
                            }
                            return Err(NotEnoughSpace);
                        }
                    }
                }
                Place::Inside(_) => {}
            }
        }
        if collision {
            self.free_space(id);
        }
        {
            let e = self.em(id);
            e.x = x;
            e.y = y;
            e.o = o;
        }
        if new_place != self.e(id).place {
            self.move_to_new_place(id, new_place);
        }
        if collision {
            self.occupy_space(id);
        }
        if self.e(id).stats.speed != 0 {
            self.em(id).is_moving = true;
        }
        Ok(())
    }

    fn move_to_new_place(&mut self, id: EntityId, new_place: Option<Place>) {
        let old = self.e(id).place;
        if let Some(p) = old {
            self.leave(p, id);
            if new_place.is_none() {
                self.unregister_entity(id);
            }
        }
        {
            let e = self.em(id);
            e.previous_place = old;
            e.place = new_place;
        }
        if let Some(np) = new_place {
            self.enter(np, id);
            if self.e(id).is_creature() {
                self.em(id).action = Some(Action {
                    kind: ActionKind::Plain,
                    target: None,
                });
            }
        }
    }

    fn unregister_entity(&mut self, id: EntityId) {
        if let Some(i) = self.active_objects.iter().position(|&o| o == id) {
            self.active_objects.remove(i);
        }
    }

    /// `Entity.delete`: unblocks the exit and leaves the world.
    pub(crate) fn entity_delete_base(&mut self, id: EntityId) {
        self.unblock(id);
        let _ = self.move_to(id, None, Some((0, 0)), 90);
    }

    /// `delete()` of any entity, with the behavior of its class.
    pub(crate) fn delete(&mut self, id: EntityId) {
        match self.e(id).class {
            EntityClass::Exit => self.exit_delete(id),
            c if c.is_creature() => {
                self.entity_delete_base(id);
                self.set_player(id, None);
            }
            _ => self.entity_delete_base(id),
        }
    }

    /// `Exit.delete`. In the original, the exit is removed from its
    /// square's exits, then setting the other side's `other_side` raises
    /// (the property has no setter), so nothing else happens.
    fn exit_delete(&mut self, id: EntityId) {
        if let Some(Place::Square(s)) = self.e(id).place {
            let exits = &mut self.sqm(s).exits;
            if let Some(i) = exits.iter().position(|&e| e == id) {
                exits.remove(i);
            }
        }
        self.warn("Exit.delete: can't set attribute 'other_side'");
    }

    pub(crate) fn unblock(&mut self, id: EntityId) {
        if let Some(exit) = self.e(id).blocked_exit {
            if let Some(data) = self.em(exit).exit.as_mut()
                && let Some(i) = data.blockers.iter().position(|&b| b == id)
            {
                data.blockers.remove(i);
            }
            self.em(id).blocked_exit = None;
        }
    }

    /// `block(e)`: units block only one exit at a time; buildings replace
    /// the exit they block.
    pub(crate) fn block(&mut self, id: EntityId, exit: EntityId) {
        if self.e(id).is_unit() && self.e(id).blocked_exit.is_some() {
            return;
        }
        self.em(id).blocked_exit = Some(exit);
        if let Some(data) = self.em(exit).exit.as_mut() {
            data.blockers.push(id);
        }
    }

    // events

    /// `notify`: tells the players perceiving the entity (or its container)
    /// that something happened, or every player if `universal`.
    pub(crate) fn notify(&mut self, id: EntityId, event: EntityEvent, universal: bool) {
        let emitter = match self.e(id).place {
            Some(Place::Inside(c)) => c,
            _ => id,
        };
        if self.e(emitter).place.is_none() {
            return;
        }
        for p in self.players.clone() {
            if universal || self.pl(p).perception.contains(&emitter) {
                self.send_event(p, emitter, event.clone());
            }
        }
    }

    pub(crate) fn send_event(&mut self, p: PlayerId, o: EntityId, event: EntityEvent) {
        if self.pl(p).is_local_human() {
            let entity = Box::new(self.e(o).clone());
            self.plm(p).events.push(Event::Entity { entity, event });
        }
    }

    /// `player.push(...)`: only a local human records events.
    pub(crate) fn push(&mut self, p: PlayerId, event: Event) {
        if self.pl(p).is_local_human() {
            self.plm(p).events.push(event);
        }
    }

    /// The events of a player since the last call, oldest first.
    pub fn drain_events(&mut self, p: PlayerId) -> Vec<Event> {
        self.plm(p).drain_events()
    }

    // harm

    /// Whether units of the first type can harm units of the second type
    /// (`harm_target_type`).
    pub fn can_harm(&mut self, unit_type: &str, other_type: &str) -> bool {
        let key = (unit_type.to_owned(), other_type.to_owned());
        if let Some(&r) = self.harm_target_types.get(&key) {
            return r;
        }
        let result = match (
            self.rules.unit_type(unit_type),
            self.rules.unit_type(other_type),
        ) {
            (_, None) => false,
            (None, Some(_)) => {
                // `unit.harm_target_type` on None raises; the original
                // catches it in the caller.
                false
            }
            (Some(unit), Some(other)) => {
                let mut r = true;
                for t in &unit.harm_target_type {
                    let fails = (t == "healable" && other.stats.is_healable == 0)
                        || (t == "building" && other.stats.is_a_building == 0)
                        || ((t == "air" || t == "ground")
                            && other.airground_type.ground_or_air() != t)
                        || (t == "unit" && other.stats.is_a_unit == 0)
                        || (t == "undead" && other.stats.is_undead == 0);
                    if fails {
                        r = false;
                        break;
                    }
                }
                r
            }
        };
        self.harm_target_types.insert(key, result);
        result
    }

    // update

    /// Queues a text command for a player. Commands run in order at the
    /// start of the next [`World::update`].
    pub fn queue_command(&mut self, player: PlayerId, command: &str) {
        self.pending_commands.push((player, command.to_owned()));
    }

    /// Runs the queued commands, then advances the world by one turn of
    /// [`VIRTUAL_TIME_INTERVAL`].
    pub fn update(&mut self) {
        for (p, cmd) in std::mem::take(&mut self.pending_commands) {
            if let Err(Abort(msg)) = self.execute_command(p, &cmd) {
                self.warn(format!("command {cmd:?}: {msg}"));
            }
        }
        self.tick();
    }

    fn tick(&mut self) {
        self.update_terrain_of_squares();
        self.update_buckets();
        self.update_cloaking();
        self.update_detection();
        for p in self.players.clone() {
            if self.players.contains(&p)
                && let Err(Abort(msg)) = self.player_update(p)
            {
                self.warn(msg);
            }
        }
        let mut active = std::mem::take(&mut self.active_objects);
        self.random.shuffle(&mut active);
        self.active_objects = active;
        for o in self.active_objects.clone() {
            if self.e(o).place.is_some()
                && let Err(Abort(msg)) = self.entity_update(o)
            {
                self.warn(msg);
            }
        }
        if self.time >= self.previous_slow_update + 1000 {
            for o in self.active_objects.clone() {
                if self.e(o).place.is_some()
                    && let Err(Abort(msg)) = self.entity_slow_update(o)
                {
                    self.warn(msg);
                }
            }
            for p in self.players.clone() {
                if self.players.contains(&p)
                    && let Err(Abort(msg)) = self.player_slow_update(p)
                {
                    self.warn(msg);
                }
            }
            self.previous_slow_update += 1000;
        }
        for p in self.players.clone() {
            let gone: Vec<EntityId> = self
                .pl(p)
                .perception
                .iter()
                .copied()
                .filter(|&o| self.e(o).place.is_none())
                .collect();
            for o in gone {
                self.plm(p).perception.remove(&o);
            }
        }
        self.time += VIRTUAL_TIME_INTERVAL;
        if self.true_playing_players().is_empty() {
            // The list shrinks while it is walked, so every other player
            // quits, as in the original.
            let mut i = 0;
            while i < self.players.len() {
                let p = self.players[i];
                self.quit_game(p);
                i += 1;
            }
        }
    }

    pub(crate) fn entity_update(&mut self, id: EntityId) -> Res {
        if self.e(id).is_creature() {
            self.creature_update(id)
        } else {
            Ok(())
        }
    }

    pub(crate) fn entity_slow_update(&mut self, id: EntityId) -> Res {
        let e = self.e(id);
        match e.class {
            EntityClass::Deposit => {
                if e.resource_regen != 0 && e.stats.qty < e.qty_max {
                    let q = (e.stats.qty + e.resource_regen).min(e.qty_max);
                    self.em(id).stats.qty = q;
                }
                Ok(())
            }
            EntityClass::Corpse => {
                if e.time_limit.is_some_and(|t| self.time >= t) {
                    self.delete(id);
                }
                Ok(())
            }
            c if c.is_creature() => {
                self.regenerate(id);
                let e = self.e(id);
                if e.time_limit.is_some_and(|t| self.time >= t) {
                    self.die(id, None)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The players who are not neutral.
    pub fn true_players(&self) -> Vec<PlayerId> {
        self.players
            .iter()
            .copied()
            .filter(|&p| !self.pl(p).neutral)
            .collect()
    }

    /// The true players still playing.
    pub fn true_playing_players(&self) -> Vec<PlayerId> {
        self.true_players()
            .into_iter()
            .filter(|&p| self.pl(p).is_playing())
            .collect()
    }

    /// Whether the players still playing belong to at least two camps.
    pub fn at_least_two_camps(&self) -> bool {
        let playing = self.true_playing_players();
        let Some(&first) = playing.first() else {
            return false;
        };
        let first_camp = self.pl(first).allied.clone();
        playing.iter().any(|p| !first_camp.contains(p))
    }

    /// The players whose AI is not `timers` (`cpu_intensive_players`), who
    /// play in turns.
    pub fn cpu_intensive_players(&self) -> Vec<PlayerId> {
        self.players
            .iter()
            .copied()
            .filter(|&p| self.pl(p).is_cpu_intensive())
            .collect()
    }

    /// The number of human players in the game.
    pub fn current_nb_human_players(&self) -> usize {
        self.players
            .iter()
            .filter(|&&p| self.pl(p).is_human())
            .count()
    }

    /// The units of a player whose position is in the buckets around a
    /// point (computed at the start of the update).
    pub(crate) fn potential_neighbors(&self, p: PlayerId, x: i64, y: i64) -> Vec<EntityId> {
        let a = crate::playerlogic::BUCKET_SIDE;
        let (bx, by) = (
            crate::nofloat::floordiv(x, a),
            crate::nofloat::floordiv(y, a),
        );
        let mut result = Vec::new();
        let buckets = &self.pl(p).buckets;
        for dx in [0, 1, -1] {
            for dy in [0, 1, -1] {
                if let Some(b) = buckets.get(&(bx + dx, by + dy)) {
                    result.extend_from_slice(b);
                }
            }
        }
        result
    }

    pub(crate) fn update_buckets(&mut self) {
        for p in self.players.clone() {
            let a = crate::playerlogic::BUCKET_SIDE;
            let mut buckets: BTreeMap<(i64, i64), Vec<EntityId>> = BTreeMap::new();
            for &u in &self.pl(p).units {
                let e = self.e(u);
                let k = (
                    crate::nofloat::floordiv(e.x, a),
                    crate::nofloat::floordiv(e.y, a),
                );
                buckets.entry(k).or_default().push(u);
            }
            self.plm(p).buckets = buckets;
        }
    }

    pub(crate) fn update_cloaking(&mut self) {
        for p in self.players.clone() {
            for u in self.pl(p).units.clone() {
                if self.e(u).stats.is_cloakable != 0 {
                    self.em(u).is_cloaked = false;
                }
            }
        }
        for p in self.players.clone() {
            for u in self.pl(p).units.clone() {
                let e = self.e(u);
                if e.stats.is_a_cloaker == 0 {
                    continue;
                }
                let r2 = e.stats.cloaking_range * e.stats.cloaking_range;
                let (ux, uy) = (e.x, e.y);
                for vp in self.pl(p).allied.clone() {
                    for vu in self.potential_neighbors(vp, ux, uy) {
                        let v = self.e(vu);
                        if v.stats.is_cloakable == 0 || v.is_cloaked {
                            continue;
                        }
                        if crate::nofloat::square_of_distance(v.x, v.y, ux, uy) < r2 {
                            self.em(vu).is_cloaked = true;
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn update_detection(&mut self) {
        for p in self.players.clone() {
            self.plm(p).detected_units = BTreeSet::new();
        }
        for p in self.players.clone() {
            for u in self.pl(p).units.clone() {
                let e = self.e(u);
                if e.stats.is_a_detector == 0 {
                    continue;
                }
                let r2 = e.stats.detection_range * e.stats.detection_range;
                let (ux, uy) = (e.x, e.y);
                for ep in self.players.clone() {
                    if self.pl(p).allied.contains(&ep) {
                        continue;
                    }
                    for iu in self.potential_neighbors(ep, ux, uy) {
                        let i = self.e(iu);
                        if !(i.is_invisible() || i.is_cloaked) {
                            continue;
                        }
                        if self.pl(p).detected_units.contains(&iu) {
                            continue;
                        }
                        if crate::nofloat::square_of_distance(i.x, i.y, ux, uy) < r2 {
                            for a in self.pl(p).allied.clone() {
                                self.plm(a).detected_units.insert(iu);
                            }
                        }
                    }
                }
            }
        }
    }

    // digest

    /// A text describing the state of the world, to compare two runs: time,
    /// random generator, players and every placed entity.
    pub fn get_objects_string(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "time {} random {:016x}",
            self.time,
            self.random.state_fingerprint()
        );
        for (pid, p) in self.all_players() {
            let _ = writeln!(
                s,
                "player {} in_game {} resources {:?} food {}/{} units {:?} upgrades {:?} victory {} defeat {}",
                pid.0,
                self.players.contains(&pid),
                p.resources,
                p.used_food,
                p.food,
                p.units.iter().map(|u| u.0).collect::<Vec<_>>(),
                p.upgrades,
                p.has_victory,
                p.has_been_defeated
            );
        }
        for e in self.entities() {
            let Some(place) = e.place else { continue };
            let place = match place {
                Place::Square(sq) => self.square(sq).name.clone(),
                Place::Inside(c) => format!("in {}", c.0),
            };
            let _ = writeln!(
                s,
                "{} {} {} {} {} {} hp {} mana {} qty {} player {:?} orders {}",
                e.id.0,
                e.type_name,
                place,
                e.x,
                e.y,
                e.o,
                e.hp,
                e.mana,
                e.stats.qty,
                e.player.map(|p| p.0),
                e.orders
                    .iter()
                    .map(|o| o.keyword.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        s
    }

    /// A short digest of [`World::get_objects_string`].
    pub fn get_digest(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.get_objects_string().bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    pub(crate) fn collision_radius() -> i64 {
        COLLISION_RADIUS
    }
}
