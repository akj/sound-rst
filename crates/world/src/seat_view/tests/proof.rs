//! The proof that a seat's view made only of what it was sent answers
//! every question the interface asks as the simulation itself would:
//! whole games are played, and on every turn, for every player, the seat's
//! view and the world are asked every question of [`SeatView`] with every
//! argument the interface passes. The world answers from its own state,
//! and computes the order menus, costs, requirements and default orders
//! with its own knowledge, the other source the shared rules take.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use super::*;
use crate::command::{Cheat, Command};
use crate::entity::Entity;
use crate::events::Event;
use crate::known::remembered_activity;
use crate::player::Controller;
use crate::setup::GameSetup;
use crate::test_games::{
    Case, allies_sharing_control, cheat_mode_and_defeat, computer, mods, sixteen,
};
use crate::world::{World, multiplayer_default_triggers};

/// The world's own answers to the questions of one player: what the seat's
/// view must agree with. Each reads the simulation's state for the player
/// directly.
#[derive(Clone, Copy)]
struct Truth<'a> {
    w: &'a World,
    me: PlayerId,
}

impl<'a> Truth<'a> {
    fn player(&self) -> PlayerId {
        self.me
    }

    fn time(&self) -> i64 {
        self.w.time
    }

    fn turn(&self) -> i64 {
        self.w.turn()
    }

    fn terrain(&self, q: SquareId) -> &'a str {
        &self.w.square(q).type_name
    }

    fn resources(&self) -> &'a [i64] {
        &self.w.player(self.me).resources
    }

    fn available_food(&self) -> i64 {
        self.w.player(self.me).available_food(self.w.food_limit())
    }

    fn used_food(&self) -> i64 {
        self.w.player(self.me).used_food
    }

    fn now(&self, e: &Entity) -> KnownEntity {
        KnownEntity::of(self.w, self.me, e, None)
    }

    fn own_units(&self) -> Vec<KnownEntity> {
        let w = self.w;
        w.player(self.me)
            .units
            .iter()
            .filter_map(|&u| w.entity(u))
            .map(|e| self.now(e))
            .collect()
    }

    fn controls(&self, p: PlayerId) -> bool {
        self.w.player(self.me).allied_control.contains(&p)
    }

    fn controlled_units(&self) -> Vec<KnownEntity> {
        let w = self.w;
        w.player(self.me)
            .allied_control
            .iter()
            .flat_map(|&p| &w.player(p).units)
            .filter_map(|&u| w.entity(u))
            .map(|e| self.now(e))
            .collect()
    }

    fn recallable_group(&self, name: &str) -> Vec<EntityId> {
        let w = self.w;
        let units = w.group_list(name).map_or(&[][..], Vec::as_slice);
        units
            .iter()
            .copied()
            .filter(|&u| {
                w.entity(u)
                    .and_then(|e| e.player)
                    .is_some_and(|p| self.controls(p))
            })
            .collect()
    }

    fn partly_observed_squares(&self) -> &'a BTreeSet<SquareId> {
        &self.w.player(self.me).partly_observed_squares
    }

    fn is_ally(&self, p: PlayerId) -> bool {
        self.allied(self.me, p)
    }

    fn objectives(&self) -> &'a [Objective] {
        &self.w.player(self.me).objectives
    }

    fn cheat_mode(&self) -> bool {
        self.w.player(self.me).cheatmode
    }

    fn has_victory(&self) -> bool {
        self.w.player(self.me).has_victory
    }

    fn players(&self) -> &'a [PlayerId] {
        &self.w.players
    }

    fn player_name(&self, p: PlayerId) -> Msg {
        self.w.player(p).name()
    }

    fn allied(&self, p: PlayerId, q: PlayerId) -> bool {
        self.w.player(p).allied.contains(&q)
    }

    fn defeated(&self) -> Vec<PlayerId> {
        self.w
            .all_players()
            .filter(|(_, p)| p.has_been_defeated)
            .map(|(id, _)| id)
            .collect()
    }

    fn perceived_entity(&self, id: EntityId) -> Option<&'a Entity> {
        let pl = self.w.player(self.me);
        pl.perception
            .contains(&id)
            .then(|| self.w.entity(id))
            .flatten()
    }

    /// The world's entity if the player perceives it, else the player's
    /// copy of it.
    fn known_entity(&self, id: EntityId) -> Option<&'a Entity> {
        self.perceived_entity(id).or_else(|| {
            let r = self.w.player(self.me).memory.get(&id)?;
            Some(&*r.snapshot)
        })
    }

    fn perceived(&self, id: EntityId) -> Option<KnownEntity> {
        self.perceived_entity(id).map(|e| self.now(e))
    }

    fn remembered(&self, id: EntityId) -> Option<KnownEntity> {
        let r = self.w.player(self.me).memory.get(&id)?;
        Some(KnownEntity::of(
            self.w,
            self.me,
            &r.snapshot,
            Some(r.time_stamp),
        ))
    }

    fn known(&self, id: EntityId) -> Option<KnownEntity> {
        self.perceived(id).or_else(|| self.remembered(id))
    }

    fn comrades(&self, owner: PlayerId, place: Option<Place>, except: EntityId) -> Comrades {
        let w = self.w;
        let perceived = w.player(self.me).perception.iter();
        questions::comrades(perceived.filter_map(|&u| w.entity(u)), owner, place, except)
    }

    fn remembered_activity(&self, unit: EntityId) -> Option<String> {
        let r = self.w.player(self.me).memory.get(&unit)?;
        remembered_activity(self.w, self.me, &r.snapshot).map(|a| a.name())
    }

    fn exit_is_blocked(&self, exit: EntityId, friendly_gates_open: bool) -> bool {
        let other_side = self
            .w
            .entity(exit)
            .and_then(|e| e.exit.as_ref())
            .and_then(|x| x.other_side);
        questions::exit_is_blocked(
            |id| self.known_entity(id),
            |p| self.is_ally(p),
            exit,
            other_side,
            friendly_gates_open,
        )
    }

    fn order_menu(&self, unit: EntityId, keyword: Keyword, strict: bool) -> Vec<String> {
        if self.w.entity(unit).is_none() {
            return Vec::new();
        }
        self.w.order_menu(unit, keyword, strict)
    }

    fn order_info(
        &self,
        unit: EntityId,
        keyword: Keyword,
        type_name: Option<&str>,
    ) -> Option<OrderInfo> {
        self.w.order_info(unit, keyword, type_name)
    }

    fn owner_has(&self, unit: EntityId, type_name: &str) -> Option<bool> {
        let owner = self.w.entity(unit)?.player?;
        self.controls(owner).then(|| self.w.has(owner, type_name))
    }

    fn activity(&self, unit: EntityId) -> Option<String> {
        self.perceived_entity(unit)?;
        self.w.activity(unit)
    }

    fn unit_upgrades(&self, unit: EntityId) -> Vec<String> {
        if self.perceived_entity(unit).is_none() {
            return Vec::new();
        }
        self.w.unit_upgrades(unit)
    }

    fn default_order(&self, unit: EntityId, target: OrderTarget) -> Result<Option<Keyword>, Abort> {
        self.w.default_keyword(unit, &TargetRef::from(target))
    }
}

/// The disagreements and the number of answers compared.
#[derive(Default)]
struct Tally {
    compared: u64,
    by_question: BTreeMap<&'static str, u64>,
    /// How often a case worth comparing came up: what the comparisons
    /// covered beyond empty answers.
    seen: BTreeMap<String, u64>,
    differences: Vec<String>,
}

impl Tally {
    fn saw(&mut self, what: impl Into<String>) {
        *self.seen.entry(what.into()).or_default() += 1;
    }

    fn same<T: PartialEq + Debug>(&mut self, question: &'static str, args: impl Debug, a: T, b: T) {
        self.compared += 1;
        *self.by_question.entry(question).or_default() += 1;
        if a != b && self.differences.len() < 40 {
            self.differences.push(format!(
                "{question}{args:?}: the world's {a:?}, the seat's {b:?}"
            ));
        }
    }
}

/// What the interface keeps of earlier turns, kept from the world as the
/// interface kept it before the seat's view kept it.
#[derive(Default)]
struct Kept {
    terrain: BTreeMap<SquareId, String>,
    upgrades: BTreeMap<EntityId, Vec<String>>,
}

impl Kept {
    fn note(&mut self, v: &Truth, w: &World) {
        let pl = w.player(v.player());
        for &s in pl
            .observed_squares
            .iter()
            .chain(&pl.partly_observed_squares)
        {
            self.terrain.insert(s, v.terrain(s).to_owned());
        }
        for &id in &pl.perception {
            if let Some(e) = v.perceived(id)
                && e.is_creature()
                && e.player.is_some()
            {
                self.upgrades.insert(id, v.unit_upgrades(id));
            }
        }
        self.upgrades.retain(|&id, _| v.known(id).is_some());
    }
}

/// Asks the world and the seat's view every question, with every argument
/// the interface passes, and a few it does not. The map, the rules and the
/// names, sent once, are compared with the world by [`compare_start`].
fn compare(t: &mut Tally, w: &World, v: &Truth, s: &SeatView, kept: &Kept, label: &str) {
    let me = v.player();
    let pl = w.player(me);
    let at = |what: &str| format!("{label} {what}");
    if pl.cheatmode {
        t.saw("turns of a seat in cheat mode");
    }
    if pl.cheat_has_all {
        t.saw("turns of a seat that has everything");
    }
    if pl.has_been_defeated {
        t.saw("turns of a defeated seat");
    }
    t.saw(format!(
        "unperceived controlled units: {}",
        s.message().unperceived_units.len().min(1)
    ));
    for _ in &s.message().unperceived_units {
        t.saw("unperceived controlled unit");
    }

    // 1. The game, known to everyone.
    t.same("player", at(""), v.player(), s.player());
    t.same("time", at(""), v.time(), s.time());
    t.same("turn", at(""), v.turn(), s.turn());
    // The terrain the interface asks for: the squares observed now, fully
    // or partly. Of the others it keeps what it last saw.
    for &q in pl
        .observed_squares
        .iter()
        .chain(&pl.partly_observed_squares)
    {
        t.same("terrain", q, v.terrain(q), s.terrain(q));
    }
    let squares = (0..w.squares.len() as u32).map(SquareId);
    for q in squares {
        let k = kept.terrain.get(&q).map_or("", String::as_str);
        if !k.is_empty() && k != v.terrain(q) {
            t.saw("square whose terrain changed since the seat saw it");
        }
        t.same("terrain as last seen", q, k, s.terrain(q));
    }
    let exits: Vec<EntityId> = s.game_start().map.exits.iter().map(|x| x.id).collect();

    // 2. The player's own state.
    t.same("resources", at(""), v.resources(), s.resources());
    t.same(
        "available_food",
        at(""),
        v.available_food(),
        s.available_food(),
    );
    t.same("used_food", at(""), v.used_food(), s.used_food());
    let own = v.own_units();
    t.same(
        "own_units",
        at(""),
        &own,
        &s.own_units().collect::<Vec<_>>(),
    );
    let controlled = v.controlled_units();
    t.same(
        "controlled_units",
        at(""),
        &controlled,
        &s.controlled_units().collect::<Vec<_>>(),
    );
    let players: Vec<PlayerId> = w.all_players().map(|(p, _)| p).collect();
    for &p in &players {
        t.same("controls", p, v.controls(p), s.controls(p));
        t.same("is_ally", p, v.is_ally(p), s.is_ally(p));
        t.same("player_name", p, v.player_name(p), s.player_name(p));
        for &q in &players {
            t.same("allied", (p, q), v.allied(p, q), s.allied(p, q));
        }
    }
    let mut names: Vec<&str> = w.groups.iter().map(|(n, _)| n.as_str()).collect();
    names.push("no such group");
    for n in names {
        if !v.recallable_group(n).is_empty() {
            t.saw("recallable group with units");
        }
        t.same(
            "recallable_group",
            n,
            v.recallable_group(n),
            s.recallable_group(n),
        );
    }
    t.same(
        "partly_observed_squares",
        at(""),
        v.partly_observed_squares(),
        s.partly_observed_squares(),
    );
    t.same("objectives", at(""), v.objectives(), s.objectives());
    t.same("cheat_mode", at(""), v.cheat_mode(), s.cheat_mode());
    t.same("has_victory", at(""), v.has_victory(), s.has_victory());

    // 3. What everyone may know of the other players.
    t.same("players", at(""), v.players(), s.players());
    if !v.defeated().is_empty() {
        t.saw("defeated player");
    }
    t.same("defeated", at(""), &v.defeated()[..], s.defeated());

    // 4. Entities the player perceives or remembers, and every other id.
    let ids: Vec<EntityId> = w
        .entities()
        .map(|e| e.id)
        .chain([EntityId(u32::MAX)])
        .collect();
    for &id in &ids {
        let known = v.known(id);
        t.same("perceived", id, v.perceived(id), s.perceived(id));
        t.same("remembered", id, v.remembered(id), s.remembered(id));
        t.same("known", id, &known, &s.known(id));
        t.same("activity", id, v.activity(id), s.activity(id));
        t.same(
            "remembered_activity",
            id,
            v.remembered_activity(id),
            s.remembered_activity(id),
        );
        t.same(
            "unit_upgrades",
            id,
            v.unit_upgrades(id),
            s.unit_upgrades(id),
        );
        if let Some(e) = &known {
            t.saw(if e.is_memory() {
                "remembered entity"
            } else {
                "perceived entity"
            });
            if e.activity.is_some() {
                t.saw(if e.is_memory() {
                    "remembered activity"
                } else {
                    "activity"
                });
            }
            if !e.upgrades.is_empty() {
                t.saw("perceived entity with upgrades");
            }
            if e.exit.as_ref().is_some_and(|x| !x.blockers.is_empty()) {
                t.saw("exit with a blocker");
            }
        }
        if let Some(e) = known {
            if let Some(owner) = e.player {
                t.same(
                    "comrades",
                    (owner, e.place, id),
                    v.comrades(owner, e.place, id),
                    s.comrades(owner, e.place, id),
                );
            }
            if e.is_memory() {
                let k = kept.upgrades.get(&id).cloned().unwrap_or_default();
                t.same(
                    "upgrades as last seen",
                    id,
                    &k[..],
                    s.remembered_upgrades(id),
                );
            }
        }
    }
    for &x in &exits {
        if v.exit_is_blocked(x, false) {
            t.saw("exit blocked");
        }
        if v.known(x)
            .and_then(|e| e.exit)
            .is_some_and(|e| e.is_blocked_by_forests)
        {
            t.saw("exit closed by forests");
        }
        for open in [false, true] {
            t.same(
                "exit_is_blocked",
                (x, open),
                v.exit_is_blocked(x, open),
                s.exit_is_blocked(x, open),
            );
        }
    }

    // 5. Questions computed on demand, for the units the player controls.
    let rules = &s.game_start().rules;
    for u in &controlled {
        let ally = u.player != Some(me);
        if ally {
            t.saw("ally's unit controlled");
        }
        if !v.order_menu(u.id, Keyword::Train, true).is_empty() {
            t.saw("unit with a train menu");
        }
        let ty = rules
            .unit_type(&u.type_name)
            .filter(|_| !u.is_building_site());
        for &k in Keyword::ALL {
            for strict in [true, false] {
                t.same(
                    "order_menu",
                    (u.id, k, strict),
                    v.order_menu(u.id, k, strict),
                    s.order_menu(u.id, k, strict),
                );
            }
            let mut types: Vec<Option<&str>> = vec![None];
            if let (Some(attr), Some(ty)) = (k.menu_attribute(), ty) {
                let list = questions::type_list(Some(ty), attr);
                types.extend(list.iter().map(|t| Some(t.as_str())));
                for name in list {
                    let requirements = rules.unit_type(name).map_or(&[][..], |t| &t.requirements);
                    for r in requirements.iter().chain([name]) {
                        t.same(
                            "owner_has",
                            (u.id, r),
                            v.owner_has(u.id, r),
                            s.owner_has(u.id, r),
                        );
                    }
                }
            }
            types.push(Some("no_such_type"));
            for ty in types {
                t.same(
                    "order_info",
                    (u.id, k, ty),
                    v.order_info(u.id, k, ty),
                    s.order_info(u.id, k, ty),
                );
            }
        }
        for target in targets(w, v, u) {
            let answer = v.default_order(u.id, target);
            let kind = match &answer {
                Ok(Some(k)) => k.as_str(),
                Ok(None) => "nothing",
                Err(_) => "error",
            };
            t.saw(format!(
                "default order {kind}{}",
                if ally { ", ally's unit" } else { "" }
            ));
            t.same(
                "default_order",
                (u.id, target),
                v.default_order(u.id, target),
                s.default_order(u.id, target),
            );
        }
    }
    // A requirement is not known for a unit of a player not controlled.
    for e in pl.perception.iter().filter_map(|&id| v.perceived(id)) {
        if e.player.is_some_and(|p| !v.controls(p)) {
            t.same(
                "owner_has",
                (e.id, "farm"),
                v.owner_has(e.id, "farm"),
                s.owner_has(e.id, "farm"),
            );
        }
    }
}

/// The data sent once against the world's own state, read as the world's
/// view read it before that data existed: both views now read the data
/// sent, so comparing them with each other could not find it wrong.
fn compare_start(t: &mut Tally, w: &World, s: &SeatView) {
    for (i, sq) in w.squares.iter().enumerate() {
        let m = s.square(SquareId(i as u32));
        t.same(
            "square, against the world",
            i,
            (
                sq.id,
                (sq.col, sq.row),
                sq.name.clone(),
                sq.title.clone(),
                ((sq.x, sq.y), (sq.xmin, sq.ymin), (sq.xmax, sq.ymax)),
                (sq.high_ground, sq.is_water),
                sq.neighbors.clone(),
            ),
            (
                m.protocol_id(),
                (m.col(), m.row()),
                m.name().to_owned(),
                m.title().clone(),
                (m.center(), m.min(), m.max()),
                (m.high_ground(), m.is_water()),
                m.neighbors().to_vec(),
            ),
        );
        t.same(
            "square_at, against the world",
            (sq.col, sq.row),
            w.square_at(sq.col, sq.row),
            s.square_at(sq.col, sq.row),
        );
    }
    let grid = (
        w.squares.iter().map(|s| s.col).max().unwrap_or(0),
        w.squares.iter().map(|s| s.row).max().unwrap_or(0),
    );
    for (col, row) in [(-1, 0), (0, -1), (grid.0 + 1, 0), (0, grid.1 + 1)] {
        t.same(
            "square_at off the map, against the world",
            (col, row),
            w.square_at(col, row),
            s.square_at(col, row),
        );
    }
    t.same("grid_max, against the world", (), grid, s.grid_max());
    t.same(
        "square_width, against the world",
        (),
        w.square_width(),
        s.square_width(),
    );
    t.same(
        "square_count, against the world",
        (),
        w.squares.len(),
        s.square_count(),
    );
    t.same(
        "map_objective, against the world",
        (),
        &w.map.objective[..],
        s.map_objective(),
    );
    t.same(
        "map_intro, against the world",
        (),
        &w.map.intro[..],
        s.map_intro(),
    );
    t.same(
        "cheats_allowed, against the world",
        (),
        w.allow_cheatmode,
        s.cheats_allowed(),
    );
    for e in w.entities() {
        let other = e
            .exit
            .as_ref()
            .and_then(|x| x.other_side)
            .and_then(|o| w.entity(o)?.place);
        t.same(
            "other_side_of_exit, against the world",
            e.id,
            other,
            s.other_side_of_exit(e.id),
        );
    }
    for (p, pl) in w.all_players() {
        t.same(
            "player_name, against the world",
            p,
            pl.name(),
            s.player_name(p),
        );
    }
    for (i, ty) in w.rules.types().iter().enumerate() {
        let id = TypeId(i as u32);
        t.same("unit_type_by_id", id, ty, s.unit_type_by_id(id));
        t.same("unit_type", &ty.name, Some(ty), s.unit_type(&ty.name));
    }
    t.same(
        "unit_type",
        "no_such_type",
        None,
        s.unit_type("no_such_type"),
    );
}

/// The targets of a default order the interface may give `u`: its square,
/// a point of it and one outside it, the square named by its protocol id,
/// every entity the player knows on the unit's square and around, and an
/// id that names nothing.
fn targets(w: &World, v: &Truth, u: &KnownEntity) -> Vec<OrderTarget> {
    let here = match u.place {
        Some(Place::Square(q)) => q,
        Some(Place::Inside(c)) => v
            .known(c)
            .and_then(|c| c.place)
            .and_then(Place::square)
            .unwrap_or(SquareId(0)),
        None => SquareId(0),
    };
    let sq = w.square(here);
    let (x, y) = (sq.x, sq.y);
    let mut out = vec![
        OrderTarget::Square(here),
        OrderTarget::Point { square: here, x, y },
        OrderTarget::Point {
            square: here,
            x: sq.xmax,
            y,
        },
        OrderTarget::Entity(EntityId(sq.id)),
        OrderTarget::Entity(EntityId(u32::MAX)),
    ];
    let near: BTreeSet<SquareId> = sq.neighbors.iter().copied().chain([here]).collect();
    let pl = w.player(v.player());
    let known = pl.perception.iter().chain(pl.memory.keys());
    for &id in known {
        if v.known(id)
            .and_then(|e| e.place)
            .and_then(|p| match p {
                Place::Square(q) => Some(q),
                Place::Inside(c) => v.known(c)?.place?.square(),
            })
            .is_some_and(|q| near.contains(&q))
        {
            out.push(OrderTarget::Entity(id));
        }
    }
    out
}

/// Plays a case, and compares both views of every player on every turn.
/// Returns the tally.
fn play(case: &Case) -> Tally {
    let mut w = case.world(read);
    let players: Vec<PlayerId> = w.all_players().map(|(p, _)| p).collect();
    let start = Arc::new(GameStart::of(&w));
    let mut seats: Vec<(SeatView, Kept)> = players
        .iter()
        .map(|&me| {
            let mut kept = Kept::default();
            kept.note(&Truth { w: &w, me }, &w);
            let mut s = SeatView::new(Arc::clone(&start), w.turn_message(me, Vec::new()));
            s.remember();
            (s, kept)
        })
        .collect();
    let mut t = Tally::default();
    compare_start(&mut t, &w, &seats[0].0);
    for turn in 0..case.turns {
        (case.script)(&mut w, turn);
        w.update();
        let digest = turn.is_multiple_of(100).then(|| w.digest_u64());
        for (&p, (s, kept)) in players.iter().zip(&mut seats) {
            let events: Vec<Event> = w.drain_events(p);
            s.install(w.turn_message(p, events));
            s.remember();
            let v = Truth { w: &w, me: p };
            kept.note(&v, &w);
            let label = format!("{} turn {} player {}", case.name, turn + 1, p.0);
            compare(&mut t, &w, &v, s, kept, &label);
        }
        if let Some(d) = digest {
            assert_eq!(d, w.digest_u64(), "asking the views changed the world");
        }
        if w.true_playing_players().is_empty() {
            break;
        }
    }
    compare_start(&mut t, &w, &seats[0].0);
    let end = GameStart::of(&w);
    assert_eq!(end.map, start.map, "{}: the map changed", case.name);
    assert_eq!(
        end.player_names, start.player_names,
        "{}: a name changed",
        case.name
    );
    for (&p, (s, _)) in players.iter().zip(&mut seats) {
        t.same("stats before the end", (case.name, p), None, s.stats());
        s.install_end(w.game_end(p));
        let stats = &w.player(p).stats;
        t.same("stats", (case.name, p), Some(stats), s.stats());
    }
    t
}

fn report(cases: &[Case]) {
    let mut total = Tally::default();
    for case in cases {
        let t = play(case);
        eprintln!("{}: {} answers compared", case.name, t.compared);
        total.compared += t.compared;
        for (q, n) in t.by_question {
            *total.by_question.entry(q).or_default() += n;
        }
        for (q, n) in t.seen {
            *total.seen.entry(q).or_default() += n;
        }
        total.differences.extend(t.differences);
    }
    eprintln!("{} answers compared in all", total.compared);
    for (q, n) in &total.by_question {
        eprintln!("  {q}: {n}");
    }
    eprintln!("cases covered:");
    for (q, n) in &total.seen {
        eprintln!("  {q}: {n}");
    }
    assert!(
        total.differences.is_empty(),
        "the seat's view and the world disagree:\n{}",
        total.differences.join("\n")
    );
}

#[test]
fn a_seat_answers_from_what_it_is_sent_as_the_world_does() {
    report(&[
        allies_sharing_control(600),
        cheat_mode_and_defeat(1000),
        mods(400),
        sixteen(60),
    ]);
}

/// The same games, longer: run by hand with `--ignored`.
#[test]
#[ignore = "long: run by hand"]
fn the_longer_games_agree_too() {
    report(&[
        allies_sharing_control(3000),
        cheat_mode_and_defeat(3000),
        mods(2000),
        sixteen(400),
    ]);
}

/// An ally's catapult goes into the ally's tower: its owner still
/// perceives it, the seat only remembers it as it went in. Hurt since, the
/// ally's peasant would repair it, and the seat says so too: a unit of the
/// owner is read as the owner perceives it, which the seat is sent.
#[test]
fn a_default_order_reads_the_owners_own_unit_as_it_is_now() {
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    let mut ally = Controller::human("ally", false);
    ally.alliance = Some("1".into());
    let setup = GameSetup {
        rules: read("res/rules.txt"),
        ai: read("res/ai.txt"),
        map: read("res/multi/jl7.txt"),
        seed: 2,
        controllers: vec![me, ally, computer("timers", "2")],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: false,
        random_starts: true,
        equivalents: true,
    };
    let (mut w, seats) = setup.build_world().unwrap();
    let (me, ally) = (seats[0], seats[1]);
    for p in [me, ally] {
        w.player_mut(p).allied_control = vec![me, ally];
    }
    w.update();
    let peasant = super::unit(&w, ally, "peasant");
    let q = w.entity(peasant).unwrap().square().unwrap();
    let (x, y) = (w.square(q).x, w.square(q).y);
    let make = |w: &mut World, kind: &str| {
        let t = w.rules.type_id(kind).unwrap();
        w.create_creature(t, Some(ally), Place::Square(q), x, y)
            .unwrap()
    };
    let catapult = make(&mut w, "catapult");
    let tower = make(&mut w, "guardtower");
    w.update();
    w.load(tower, catapult);
    w.update();
    assert!(w.player(me).memory.contains_key(&catapult));
    w.entity_mut(catapult).unwrap().hp = 1000;

    let v = Truth { w: &w, me };
    let s = seat(&w, me);
    let target = OrderTarget::Entity(catapult);
    assert_eq!(v.default_order(peasant, target), Ok(Some(Keyword::Repair)));
    assert_eq!(s.default_order(peasant, target), Ok(Some(Keyword::Repair)));
}

/// In cheat mode a player perceives only what is on the squares: its own
/// catapult in its own tower is remembered as it went in, by the world as
/// by the seat, though it has been hurt since.
#[test]
fn in_cheat_mode_the_seats_own_unit_inside_is_remembered() {
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    let setup = GameSetup {
        rules: read("res/rules.txt"),
        ai: read("res/ai.txt"),
        map: read("res/multi/jl7.txt"),
        seed: 2,
        controllers: vec![me, computer("timers", "2")],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: true,
        random_starts: true,
        equivalents: true,
    };
    let (mut w, seats) = setup.build_world().unwrap();
    let me = seats[0];
    w.queue(me, Command::Cheat(Cheat::ToggleMode));
    w.update();
    assert!(w.player(me).cheatmode);
    let peasant = super::unit(&w, me, "peasant");
    let q = w.entity(peasant).unwrap().square().unwrap();
    let (x, y) = (w.square(q).x, w.square(q).y);
    let make = |w: &mut World, kind: &str| {
        let t = w.rules.type_id(kind).unwrap();
        w.create_creature(t, Some(me), Place::Square(q), x, y)
            .unwrap()
    };
    let catapult = make(&mut w, "catapult");
    let tower = make(&mut w, "guardtower");
    w.update();
    w.load(tower, catapult);
    w.update();
    assert!(w.player(me).memory.contains_key(&catapult));
    w.entity_mut(catapult).unwrap().hp = 1000;

    let v = Truth { w: &w, me };
    let s = seat(&w, me);
    let target = OrderTarget::Entity(catapult);
    assert_eq!(v.default_order(peasant, target), Ok(Some(Keyword::Go)));
    assert_eq!(s.default_order(peasant, target), Ok(Some(Keyword::Go)));
}
