//! Map definitions: parsing, building the squares and resources, placing
//! the players, and writing a map back (`World._parse_map`,
//! `World._build_map`, `World.populate_map`, `World.save_map`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use regex::Regex;

use crate::entity::{Place, PlayerId, SquareId};
use crate::nofloat::{PRECISION, floordiv, py_float, py_int, to_int};
use crate::player::{Controller, ControllerKind, Player, PlayerKind, Trigger};
use crate::rules::AirGround;
use crate::square::Square;
use crate::triggers::{Expr, list_to_tree};
use crate::world::World;

/// The map cannot be loaded. The message says why, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapError(pub String);

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MapError {}

fn formatted(line: &str, msg: &str) -> String {
    if line.is_empty() {
        msg.to_owned()
    } else {
        format!("{msg} (in \"{line}\")")
    }
}

fn map_error(line: &str, msg: &str) -> MapError {
    MapError(formatted(line, msg))
}

/// One item of a start: units of a type on a square, or a forbidden
/// technology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartItem {
    Units {
        square: Option<String>,
        type_name: String,
        count: i64,
    },
    /// `-<type>`: the player may not build, train or research it.
    Forbidden(String),
}

/// A starting place: resources, units, and triggers of its player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub resources: Vec<i64>,
    pub items: Vec<StartItem>,
    pub triggers: Vec<Trigger>,
}

/// A map definition as parsed. Sizes are in squares here, except
/// `square_width`, which is multiplied by [`PRECISION`] when the map is
/// built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapDef {
    pub title: Vec<i64>,
    pub objective: Vec<i64>,
    pub intro: Vec<i64>,
    pub timer_coefficient: i64,
    /// Square, deposit type, quantity as written.
    pub map_objects: Vec<(String, String, String)>,
    pub computers_starts: Vec<Start>,
    pub players_starts: Vec<Start>,
    pub starting_units: Vec<String>,
    /// As written, for the editor; `None` without a `starting_resources`
    /// line.
    pub starting_resources: Option<String>,
    /// The `player` and `computer_only` lines, for the editor.
    pub specific_starts: Vec<String>,
    pub square_width: i64,
    pub nb_lines: i64,
    pub nb_columns: i64,
    pub nb_rows: i64,
    pub nb_meadows_by_square: i64,
    /// Exit type and starting squares of west-east passages.
    pub west_east: Vec<(String, Vec<String>)>,
    pub south_north: Vec<(String, Vec<String>)>,
    pub terrain: BTreeMap<String, String>,
    pub terrain_speed: BTreeMap<String, (i64, i64)>,
    pub terrain_cover: BTreeMap<String, (i64, i64)>,
    pub water_squares: BTreeSet<String>,
    pub no_air_squares: BTreeSet<String>,
    pub ground_squares: BTreeSet<String>,
    pub starting_squares: Vec<String>,
    pub additional_meadows: Vec<String>,
    pub remove_meadows: Vec<String>,
    pub high_grounds: Vec<String>,
    pub nb_players_min: i64,
    pub nb_players_max: i64,
    pub scenario: i64,
    pub global_food_limit: i64,
}

impl Default for MapDef {
    fn default() -> Self {
        MapDef {
            title: Vec::new(),
            objective: Vec::new(),
            intro: Vec::new(),
            timer_coefficient: 1,
            map_objects: Vec::new(),
            computers_starts: Vec::new(),
            players_starts: Vec::new(),
            starting_units: Vec::new(),
            starting_resources: None,
            specific_starts: Vec::new(),
            square_width: 12,
            nb_lines: 0,
            nb_columns: 0,
            nb_rows: 0,
            nb_meadows_by_square: 0,
            west_east: Vec::new(),
            south_north: Vec::new(),
            terrain: BTreeMap::new(),
            terrain_speed: BTreeMap::new(),
            terrain_cover: BTreeMap::new(),
            water_squares: BTreeSet::new(),
            no_air_squares: BTreeSet::new(),
            ground_squares: BTreeSet::new(),
            starting_squares: Vec::new(),
            additional_meadows: Vec::new(),
            remove_meadows: Vec::new(),
            high_grounds: Vec::new(),
            nb_players_min: 1,
            nb_players_max: 1,
            scenario: 0,
            global_food_limit: crate::world::GLOBAL_FOOD_LIMIT,
        }
    }
}

/// The names of the trigger functions (`lang_*`), which a map may use.
pub(crate) const LANG_NAMES: &[&str] = &[
    "timer",
    "order",
    "has",
    "has_entered",
    "add_units",
    "no_enemy_left",
    "no_enemy_player_left",
    "no_unit_left",
    "no_building_left",
    "victory",
    "defeat",
    "cut_scene",
    "add_objective",
    "objective_complete",
    "ai",
    "faction",
];

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `convert_and_split_first_numbers`: the leading numbers (with
/// [`PRECISION`]) and the rest.
pub fn convert_and_split_first_numbers(words: &[String]) -> (Vec<i64>, Vec<String>) {
    let mut numbers = Vec::new();
    for (i, w) in words.iter().enumerate() {
        match to_int(w) {
            Ok(n) => numbers.push(n),
            Err(_) => return (numbers, words[i..].to_vec()),
        }
    }
    (numbers, Vec::new())
}

struct Patterns {
    strip_lines: Regex,
    random_choice: Regex,
    resources: Regex,
    paths: Regex,
    bridges: Regex,
    valid_word: Regex,
    square_name: Regex,
}

fn patterns() -> &'static Patterns {
    static P: std::sync::OnceLock<Patterns> = std::sync::OnceLock::new();
    P.get_or_init(|| Patterns {
        strip_lines: Regex::new(r"\s*\n\s*").expect("valid"),
        random_choice: Regex::new(r"(?ms)^#random_choice\n(.*?)\n#end_random_choice$")
            .expect("valid"),
        resources: Regex::new(r"(?m)^(goldmine|wood)s\s+([0-9]+)\s+(.*)$").expect("valid"),
        paths: Regex::new(r"(south_north|west_east)_paths").expect("valid"),
        bridges: Regex::new(r"(south_north|west_east)_bridges").expect("valid"),
        valid_word: Regex::new(r"^([a-z]+[0-9]+|[0-9]+(.[0-9]*)?|.[0-9]+)$").expect("valid"),
        square_name: Regex::new(r"^[a-z]+[0-9]+$").expect("valid"),
    })
}

impl World {
    fn check_squares(&mut self, line: &str, squares: &mut Vec<String>) {
        let mut kept = Vec::new();
        for sq in squares.drain(..) {
            if patterns().square_name.is_match(&sq) {
                kept.push(sq);
            } else {
                self.warn(formatted(line, &format!("{sq} is not a square name")));
            }
        }
        *squares = kept;
    }

    fn add_start_to(
        &self,
        resources: Vec<i64>,
        items: &[String],
        sq: Option<String>,
    ) -> Result<Start, MapError> {
        let mut sq = sq;
        let mut start = Vec::new();
        let mut multiplicator = 1;
        for x in items {
            let b = x.as_bytes();
            let first_letter = b.first().is_some_and(u8::is_ascii_alphabetic);
            let is_a_square = first_letter && {
                let Some(second) = b.get(1) else {
                    return Err(MapError(format!("string index out of range: {x}")));
                };
                second.is_ascii_digit() && (b.len() == 2 || (b.len() == 3 && b[2].is_ascii_digit()))
            };
            if is_a_square {
                sq = Some(x.clone());
                multiplicator = 1;
            } else if x.starts_with('-') {
                start.push(StartItem::Forbidden(x.clone()));
            } else if is_digits(x) {
                multiplicator = py_int(x).map_err(|_| MapError(format!("invalid number: {x}")))?;
            } else {
                start.push(StartItem::Units {
                    square: sq.clone(),
                    type_name: x.clone(),
                    count: multiplicator,
                });
                multiplicator = 1;
            }
        }
        Ok(Start {
            resources,
            items: start,
            triggers: Vec::new(),
        })
    }

    fn add_trigger(&mut self, tree: Vec<Expr>, words_line: &str) -> Result<(), MapError> {
        let [owners, condition, action]: [Expr; 3] = tree.try_into().map_err(|_| {
            MapError(format!(
                "a trigger needs owners, a condition and an action: {words_line}"
            ))
        })?;
        let owners: Vec<String> = match owners {
            Expr::Word(w) => vec![w],
            Expr::List(l) => l
                .into_iter()
                .map(|e| match e {
                    Expr::Word(w) => w,
                    Expr::List(_) => String::new(),
                })
                .collect(),
        };
        let trigger = Trigger { condition, action };
        let context = format!("trigger {words_line}");
        for o in owners {
            match o.as_str() {
                "computers" => {
                    for s in &mut self.map.computers_starts {
                        s.triggers.push(trigger.clone());
                    }
                }
                "players" => {
                    for s in &mut self.map.players_starts {
                        s.triggers.push(trigger.clone());
                    }
                }
                "all" => {
                    for s in self
                        .map
                        .computers_starts
                        .iter_mut()
                        .chain(self.map.players_starts.iter_mut())
                    {
                        s.triggers.push(trigger.clone());
                    }
                }
                _ => {
                    let (kind, digit) = o.split_at(o.len().saturating_sub(1));
                    let list = match kind {
                        "computer" => Some(&mut self.map.computers_starts),
                        "player" => Some(&mut self.map.players_starts),
                        _ => None,
                    };
                    let done = list.and_then(|l| {
                        let n = py_int(digit).ok()? - 1;
                        let len = l.len() as i64;
                        let i = if n < 0 { n + len } else { n };
                        if i < 0 || i >= len {
                            return None;
                        }
                        l[i as usize].triggers.push(trigger.clone());
                        Some(())
                    });
                    if done.is_none() {
                        self.warn(formatted(&context, &format!("{o} is unknown")));
                    }
                }
            }
        }
        Ok(())
    }

    /// Parses a map definition (`_parse_map`).
    pub(crate) fn parse_map(&mut self, map_definition: &str) -> Result<(), MapError> {
        let nb_res = self.rules.nb_of_resource_types();
        let mut triggers: Vec<Vec<String>> = Vec::new();
        let mut starting_resources = vec![0; nb_res];
        let p = patterns();
        let s = crate::defs::preprocess(map_definition);
        let s = s.replace('(', " ( ").replace(')', " ) ");
        let s = p.strip_lines.replace_all(&s, "\n").into_owned();
        let s = p
            .random_choice
            .replace_all(&s, |c: &regex::Captures| {
                let choices: Vec<&str> = c[1].split("\n#end_choice\n").collect();
                let i = self.random.choice_index(choices.len()).unwrap_or(0);
                choices[i].to_owned()
            })
            .into_owned();
        let s = p.resources.replace_all(&s, "${1} ${2} ${3}").into_owned();
        let s = p.paths.replace_all(&s, "${1} path").into_owned();
        let s = p.bridges.replace_all(&s, "${1} bridge").into_owned();
        let classnames: BTreeSet<String> = self
            .rules
            .classnames()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let ai_names: BTreeSet<String> = self
            .ai_scripts
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        for line in s.split('\n') {
            let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            let Some(w) = words.first().cloned() else {
                continue;
            };
            if w.starts_with(';') {
                continue;
            }
            let w = w.as_str();
            if !matches!(
                w,
                "south_north" | "west_east" | "terrain" | "speed" | "cover"
            ) {
                for word in &words[1..] {
                    for part in word.split(',') {
                        let part = part.strip_prefix('-').unwrap_or(part);
                        let known = p.valid_word.is_match(part)
                            || LANG_NAMES.contains(&part)
                            || classnames.contains(part)
                            || ai_names.contains(part)
                            || matches!(part, "(" | ")" | "all" | "players" | "computers")
                            || crate::orders::Keyword::parse(part).is_some();
                        if !known {
                            return Err(map_error(line, &format!("unknown: {part}")));
                        }
                    }
                }
            }
            let int_list = |words: &[String]| -> Result<Vec<i64>, MapError> {
                words
                    .iter()
                    .map(|x| {
                        py_int(x).map_err(|_| {
                            map_error(line, &format!("invalid literal for int(): {x}"))
                        })
                    })
                    .collect()
            };
            match w {
                "title" => self.map.title = int_list(&words[1..])?,
                "objective" => self.map.objective = int_list(&words[1..])?,
                "intro" => self.map.intro = int_list(&words[1..])?,
                "square_width"
                | "nb_rows"
                | "nb_columns"
                | "nb_lines"
                | "nb_players_min"
                | "nb_players_max"
                | "scenario"
                | "nb_meadows_by_square"
                | "global_food_limit"
                | "timer_coefficient" => {
                    let v = words
                        .get(1)
                        .and_then(|x| py_int(x).ok())
                        .ok_or_else(|| map_error(line, &format!("{w} must be an integer")))?;
                    let m = &mut self.map;
                    match w {
                        "square_width" => m.square_width = v,
                        "nb_rows" => {
                            m.nb_rows = v;
                            m.nb_columns = v;
                        }
                        "nb_columns" => m.nb_columns = v,
                        "nb_lines" => m.nb_lines = v,
                        "nb_players_min" => m.nb_players_min = v,
                        "nb_players_max" => m.nb_players_max = v,
                        "scenario" => m.scenario = v,
                        "nb_meadows_by_square" => m.nb_meadows_by_square = v,
                        "global_food_limit" => m.global_food_limit = v,
                        _ => m.timer_coefficient = v,
                    }
                    if w == "nb_rows" {
                        self.warn("nb_rows is deprecated, use nb_columns instead");
                    }
                }
                "south_north" | "west_east" => {
                    let t = words
                        .get(1)
                        .cloned()
                        .ok_or_else(|| map_error(line, "list index out of range"))?;
                    let mut squares = words[2..].to_vec();
                    self.check_squares(line, &mut squares);
                    if w == "south_north" {
                        self.map.south_north.push((t, squares));
                    } else {
                        self.map.west_east.push((t, squares));
                    }
                }
                "starting_squares" | "additional_meadows" | "remove_meadows" | "high_grounds" => {
                    let mut squares = words[1..].to_vec();
                    self.check_squares(line, &mut squares);
                    let m = &mut self.map;
                    let list = match w {
                        "starting_squares" => &mut m.starting_squares,
                        "additional_meadows" => &mut m.additional_meadows,
                        "remove_meadows" => &mut m.remove_meadows,
                        _ => &mut m.high_grounds,
                    };
                    list.extend(squares);
                }
                "starting_resources" => {
                    self.map.starting_resources = Some(words[1..].join(" "));
                    starting_resources = Vec::new();
                    for c in &words[1..] {
                        let v = to_int(c).map_err(|_| {
                            map_error(line, &format!("expected an integer but found {c}"))
                        })?;
                        starting_resources.push(v);
                    }
                }
                _ if self.rules.get_words(w, "class").as_deref()
                    == Some(&["deposit".to_owned()][..]) =>
                {
                    for sq in words.iter().skip(2) {
                        self.map
                            .map_objects
                            .push((sq.clone(), w.to_owned(), words[1].clone()));
                    }
                }
                "starting_units" => self.map.starting_units.extend(words[1..].iter().cloned()),
                "player" | "computer_only" | "computer" => {
                    self.map.specific_starts.push(words.join(" "));
                    let (resources, units) = convert_and_split_first_numbers(&words[1..]);
                    if resources.len() != nb_res {
                        self.warn(format!(
                            "the map have {} resources while the rules have {} resources",
                            resources.len(),
                            nb_res
                        ));
                    }
                    let start = self.add_start_to(resources, &units, None)?;
                    if w == "player" {
                        self.map.players_starts.push(start);
                    } else {
                        self.map.computers_starts.push(start);
                    }
                }
                "trigger" => triggers.push(words[1..].to_vec()),
                "terrain" => {
                    let t = words
                        .get(1)
                        .cloned()
                        .ok_or_else(|| map_error(line, "list index out of range"))?;
                    let mut squares = words[2..].to_vec();
                    self.check_squares(line, &mut squares);
                    for sq in squares {
                        self.map.terrain.insert(sq, t.clone());
                    }
                }
                "speed" | "cover" => {
                    let mut t = Vec::new();
                    for x in words.iter().skip(1).take(2) {
                        let f = py_float(x).map_err(|_| {
                            map_error(line, &format!("could not convert string to float: {x}"))
                        })?;
                        t.push(
                            crate::nofloat::float_to_int(f * 100.0)
                                .map_err(|_| map_error(line, "invalid value"))?,
                        );
                    }
                    let [a, b] = t[..] else {
                        return Err(map_error(line, "two values are expected"));
                    };
                    let mut squares = words.iter().skip(3).cloned().collect();
                    self.check_squares(line, &mut squares);
                    let target = if w == "speed" {
                        &mut self.map.terrain_speed
                    } else {
                        &mut self.map.terrain_cover
                    };
                    for sq in squares {
                        target.insert(sq, (a, b));
                    }
                }
                "water" | "ground" | "no_air" => {
                    let mut squares = words[1..].to_vec();
                    self.check_squares(line, &mut squares);
                    let m = &mut self.map;
                    let set = match w {
                        "water" => &mut m.water_squares,
                        "ground" => &mut m.ground_squares,
                        _ => &mut m.no_air_squares,
                    };
                    set.extend(squares);
                }
                _ => self.warn(formatted(line, &format!("unknown command: {w}"))),
            }
        }
        for sq in self.map.starting_squares.clone() {
            let units = self.map.starting_units.clone();
            let start = self.add_start_to(starting_resources.clone(), &units, Some(sq))?;
            self.map.players_starts.push(start);
        }
        if self.map.nb_players_min > self.map.nb_players_max {
            return Err(MapError("nb_players_min > nb_players_max".into()));
        }
        if (self.map.players_starts.len() as i64) < self.map.nb_players_max {
            return Err(MapError(
                "not enough starting places for nb_players_max".into(),
            ));
        }
        let trees: Vec<(Vec<Expr>, String)> =
            if triggers.is_empty() && !self.default_triggers.is_empty() {
                self.default_triggers
                    .iter()
                    .map(|t| {
                        let words = |l: &[String]| {
                            Expr::List(l.iter().map(|w| Expr::Word(w.clone())).collect())
                        };
                        (
                            vec![
                                Expr::Word(t.owners.clone()),
                                words(&t.condition),
                                words(&t.action),
                            ],
                            t.owners.clone(),
                        )
                    })
                    .collect()
            } else {
                let mut out = Vec::new();
                for t in &triggers {
                    let tree = list_to_tree(t)
                        .map_err(|e| MapError(format!("{e} in trigger {}", t.join(" "))))?;
                    out.push((tree, t.join(" ")));
                }
                out
            };
        for (tree, line) in trees {
            self.add_trigger(tree, &line)?;
        }
        Ok(())
    }

    /// Parses and builds a map from its definition text
    /// (`load_and_build_map`). The rules and computer scripts must already
    /// include the map's own, if it has any.
    pub fn load_and_build_map(&mut self, map_definition: &str) -> Result<(), MapError> {
        let text = map_definition.replace("\r\n", "\n").replace('\r', "\n");
        self.parse_map(&text)?;
        self.map.square_width *= PRECISION;
        self.build_map()
    }

    fn build_map(&mut self) -> Result<(), MapError> {
        self.create_squares_and_grid()?;
        self.create_resources()?;
        self.arrange_all_resources_symmetrically();
        self.create_passages()?;
        self.create_graphs();
        Ok(())
    }

    fn create_squares_and_grid(&mut self) -> Result<(), MapError> {
        let sw = self.map.square_width;
        if self.map.nb_columns > 26 {
            return Err(MapError(
                "string index out of range: too many columns".into(),
            ));
        }
        for col in 0..self.map.nb_columns {
            for row in 0..self.map.nb_lines {
                let id = self.get_next_id();
                let mut sq = Square::new(id, col, row, sw);
                let name = sq.name.clone();
                sq.high_ground = self.map.high_grounds.contains(&name);
                if let Some(t) = self.map.terrain.get(&name) {
                    sq.type_name = t.clone();
                }
                if let Some(&t) = self.map.terrain_speed.get(&name) {
                    sq.terrain_speed = t;
                }
                if let Some(&t) = self.map.terrain_cover.get(&name) {
                    sq.terrain_cover = t;
                }
                if self.map.water_squares.contains(&name) {
                    sq.is_water = true;
                    sq.is_ground = self.map.ground_squares.contains(&name);
                }
                if self.map.no_air_squares.contains(&name) {
                    sq.is_air = false;
                }
                let sid = SquareId(self.squares.len() as u32);
                self.grid_names.insert(name, sid);
                self.grid_xy.insert((col, row), sid);
                self.square_by_obj.insert(id, sid);
                self.squares.push(sq);
            }
        }
        for i in 0..self.squares.len() {
            let (col, row) = (self.squares[i].col, self.squares[i].row);
            let strict: Vec<SquareId> = [(0, 1), (0, -1), (1, 0), (-1, 0)]
                .iter()
                .filter_map(|(dc, dr)| self.square_at(col + dc, row + dr))
                .collect();
            let all: Vec<SquareId> = [
                (0, 1),
                (0, -1),
                (1, 0),
                (-1, 0),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ]
            .iter()
            .filter_map(|(dc, dr)| self.square_at(col + dc, row + dr))
            .collect();
            self.squares[i].strict_neighbors = strict;
            self.squares[i].neighbors = all;
        }
        let xmax = self.map.nb_columns * sw;
        let res = World::collision_radius() * 2 / 3;
        self.collision_ground = crate::collision::CollisionMatrix::new(xmax, res);
        self.collision_air = crate::collision::CollisionMatrix::new(xmax, res);
        Ok(())
    }

    fn meadow_squares(&self) -> Vec<String> {
        let mut m = Vec::new();
        for name in self.grid_names.keys() {
            for _ in 0..self.map.nb_meadows_by_square {
                m.push(name.clone());
            }
        }
        m.extend(self.map.additional_meadows.iter().cloned());
        for sq in &self.map.remove_meadows {
            if let Some(i) = m.iter().position(|x| x == sq) {
                m.remove(i);
            }
        }
        m
    }

    fn create_resources(&mut self) -> Result<(), MapError> {
        for (z, cls, n) in self.map.map_objects.clone() {
            let Some(s) = self.square_by_name(&z) else {
                self.warn(format!("{z} is not a valid coordinate"));
                continue;
            };
            let tid = self
                .rules
                .type_id(&cls)
                .ok_or_else(|| MapError(format!("unknown deposit type {cls}")))?;
            if self.can_receive(s, AirGround::Ground, None) {
                let d = self.create_deposit(tid, s, &n).map_err(|e| MapError(e.0))?;
                let m = self.create_meadow(s, None);
                self.em(d).building_land = Some(m);
                self.delete(m);
            }
        }
        for z in self.meadow_squares() {
            let s = self
                .square_by_name(&z)
                .ok_or_else(|| MapError(format!("KeyError: '{z}'")))?;
            self.create_meadow(s, None);
        }
        Ok(())
    }

    fn arrange_all_resources_symmetrically(&mut self) {
        let xc = floordiv(self.map.nb_columns * 10, 2);
        let yc = floordiv(self.map.nb_lines * 10, 2);
        for i in 0..self.squares.len() {
            self.arrange_resources_symmetrically(SquareId(i as u32), xc, yc);
        }
    }

    fn create_passages(&mut self) -> Result<(), MapError> {
        const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz";
        for (t, squares) in self.map.west_east.clone() {
            for i in squares {
                let mut portal = false;
                let first = i.chars().next().unwrap_or(' ');
                let mut col = ALPHABET.find(first).map_or(0, |c| c as i64 + 1);
                if col == self.map.nb_columns {
                    col = 0;
                    portal = true;
                }
                let letter = ALPHABET
                    .chars()
                    .nth(col as usize)
                    .ok_or_else(|| MapError("string index out of range".into()))?;
                let j = format!("{letter}{}", &i[1..]);
                let si = self
                    .square_by_name(&i)
                    .ok_or_else(|| MapError(format!("KeyError: '{i}'")))?;
                match self.square_by_name(&j) {
                    None => self.warn(format!(
                        "couldn't create a west-east passage from {i} to {j}"
                    )),
                    Some(sj) => {
                        let (a, b) = (self.square(si).east_side(), self.square(sj).west_side());
                        self.passage((si, a.0, a.1, a.2), (sj, b.0, b.1, b.2), portal, &t);
                    }
                }
            }
        }
        for (t, squares) in self.map.south_north.clone() {
            for i in squares {
                let mut portal = false;
                let mut line =
                    py_int(&i[1..]).map_err(|_| MapError(format!("invalid square {i}")))? + 1;
                if line == self.map.nb_lines + 1 {
                    line = 1;
                    portal = true;
                }
                let j = format!("{}{line}", &i[..1]);
                let si = self
                    .square_by_name(&i)
                    .ok_or_else(|| MapError(format!("KeyError: '{i}'")))?;
                match self.square_by_name(&j) {
                    None => self.warn(format!(
                        "couldn't create a south-north passage from {i} to {j}"
                    )),
                    Some(sj) => {
                        let (a, b) = (self.square(si).north_side(), self.square(sj).south_side());
                        self.passage((si, a.0, a.1, a.2), (sj, b.0, b.1, b.2), portal, &t);
                    }
                }
            }
        }
        Ok(())
    }

    // players

    /// Creates the player of a controller (`create_player`).
    fn create_player(&mut self, controller: &Controller) -> Result<PlayerId, MapError> {
        let pid = self.new_player(controller)?;
        self.players.push(pid);
        Ok(pid)
    }

    /// Creates a player without adding it to the players in the game, as
    /// the original's player classes do when called directly.
    pub(crate) fn new_player(&mut self, controller: &Controller) -> Result<PlayerId, MapError> {
        let kind = match &controller.kind {
            ControllerKind::Human { .. } => PlayerKind::Human,
            ControllerKind::Computer { ai_type } if ai_type == "ai2" => PlayerKind::Computer2,
            ControllerKind::Computer { ai_type } => PlayerKind::Computer {
                ai_type: ai_type.clone(),
            },
            ControllerKind::Neutral => PlayerKind::Computer {
                ai_type: "timers".into(),
            },
        };
        let faction = if controller.faction == "random_faction" {
            let factions = self.rules.factions();
            let i = self.random.choice_index(factions.len()).ok_or_else(|| {
                MapError("Cannot choose from an empty sequence (no faction)".into())
            })?;
            factions[i].clone()
        } else {
            controller.faction.clone()
        };
        let number = if controller.is_neutral() {
            None
        } else {
            Some(self.get_next_player_number())
        };
        let obj_id = self.get_next_id();
        let mut player = Player::new(obj_id, kind.clone(), controller.clone(), faction);
        player.number = number;
        let pid = self.add_player(player);
        self.plm(pid).allied = vec![pid];
        self.plm(pid).allied_control = vec![pid];
        if let PlayerKind::Computer { ai_type } = &kind
            && ai_type != "timers"
            && self
                .ai_scripts
                .get(&self.faction_ai_type(pid, ai_type))
                .is_none()
        {
            self.warn(format!("KeyError: no AI called {ai_type}"));
        }
        if kind != PlayerKind::Human {
            let ai = self.make_ai(&kind);
            self.plm(pid).ai = Some(ai);
        }
        Ok(pid)
    }

    /// `faction_ai_type`: the faction's own version of an AI type.
    pub(crate) fn faction_ai_type(&self, p: PlayerId, ai_type: &str) -> String {
        self.rules.equivalent_type(ai_type, &self.pl(p).faction)
    }

    fn init_alliance(&mut self, p: PlayerId) {
        let alliance = self.pl(p).controller.alliance.clone();
        let Some(a) = alliance.filter(|a| a != "None") else {
            return;
        };
        for op in self.players.clone() {
            if self.pl(op).controller.alliance.as_deref() == Some(a.as_str()) {
                self.plm(p).allied.push(op);
            }
        }
    }

    /// `populate_map`: creates the players of the controllers (with
    /// `random_starts`, at random starting places), their alliances, a
    /// neutral player for each `computer_only` start, and their units. With
    /// `equivalents`, the factions replace start units with their own.
    pub fn populate_map(
        &mut self,
        controllers: &[Controller],
        random_starts: bool,
        equivalents: bool,
    ) -> Result<Vec<PlayerId>, MapError> {
        let starts: Vec<Start> = if random_starts {
            self.random
                .sample(&self.map.players_starts.clone(), controllers.len())
                .ok_or_else(|| MapError("Sample larger than population or is negative".into()))?
        } else {
            self.map
                .players_starts
                .iter()
                .take(controllers.len())
                .cloned()
                .collect()
        };
        let mut created = Vec::new();
        for c in controllers {
            created.push(self.create_player(c)?);
        }
        for p in self.players.clone() {
            self.init_alliance(p);
        }
        for _ in 0..self.map.computers_starts.len() {
            self.create_player(&Controller::neutral())?;
        }
        let all_starts: Vec<Start> = starts
            .into_iter()
            .chain(self.map.computers_starts.clone())
            .collect();
        for (p, start) in self.players.clone().into_iter().zip(all_starts) {
            self.init_position(p, &start, equivalents)?;
        }
        Ok(created)
    }

    fn init_position(
        &mut self,
        p: PlayerId,
        start: &Start,
        equivalents: bool,
    ) -> Result<(), MapError> {
        let faction = self.pl(p).faction.clone();
        let resources = self.rules.normalized_cost_or_resources(&start.resources);
        let mut units: Vec<(SquareId, i64, String)> = Vec::new();
        let mut upgrades = Vec::new();
        let mut forbidden = Vec::new();
        for item in &start.items {
            match item {
                StartItem::Forbidden(t) => forbidden.push(t[1..].to_owned()),
                StartItem::Units {
                    square,
                    type_name,
                    count,
                } => {
                    let mut t = type_name.clone();
                    if equivalents && self.rules.unit_type(&t).is_some() {
                        t = self.rules.equivalent_type(&t, &faction);
                    }
                    match self.rules.unit_type(&t) {
                        Some(ut) if ut.is_upgrade() => upgrades.push(t),
                        None => self.warn("couldn't create an initial unit"),
                        Some(_) => {
                            let s = square
                                .as_deref()
                                .and_then(|n| self.square_by_name(n))
                                .ok_or_else(|| MapError(format!("KeyError: {square:?}")))?;
                            units.push((s, *count, t));
                        }
                    }
                }
            }
        }
        {
            let pl = self.plm(p);
            pl.upgrades = upgrades;
            pl.forbidden_techs = forbidden;
            pl.resources = resources.clone();
            for (i, &q) in resources.iter().enumerate() {
                pl.stats.add(
                    crate::player::StatEvent::Gathered,
                    crate::player::StatTarget::Resource(i),
                    q,
                );
            }
        }
        for (s, n, t) in units {
            for _ in 0..n {
                self.add_start_unit(p, &t, s);
            }
        }
        self.plm(p).triggers = start.triggers.clone();
        Ok(())
    }

    // save

    /// The map as a definition text (`save_map`), for the map editor.
    pub fn save_map(&self) -> String {
        use std::fmt::Write;
        let sorted = |mut names: Vec<String>| -> Vec<String> {
            names.sort_by_key(|n| (n[..1].to_owned(), py_int(&n[1..]).unwrap_or(0)));
            names
        };
        let join = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>().join(" ");
        let mut f = String::new();
        let m = &self.map;
        let _ = writeln!(f, "title {}", join(&m.title));
        let _ = writeln!(f, "objective {}", join(&m.objective));
        f.push('\n');
        let _ = writeln!(f, "square_width {}", floordiv(m.square_width, PRECISION));
        let _ = writeln!(f, "nb_columns {}", m.nb_columns);
        let _ = writeln!(f, "nb_lines {}", m.nb_lines);
        f.push('\n');
        let _ = writeln!(f, "nb_players_min {}", m.nb_players_min);
        let _ = writeln!(f, "nb_players_max {}", m.nb_players_max);
        let _ = writeln!(
            f,
            "starting_squares {}",
            sorted(m.starting_squares.clone()).join(" ")
        );
        let _ = writeln!(f, "starting_units {}", m.starting_units.join(" "));
        // The original's attribute is an empty list until the map sets it.
        let _ = writeln!(
            f,
            "starting_resources {}",
            m.starting_resources.as_deref().unwrap_or("[]")
        );
        let _ = writeln!(f, "global_food_limit {}", m.global_food_limit);
        for line in &m.specific_starts {
            let _ = writeln!(f, "{line}");
        }
        f.push('\n');
        let mut res: BTreeSet<(String, i64)> = BTreeSet::new();
        for s in &self.squares {
            for &o in &s.objects {
                let e = self.e(o);
                if e.type_id
                    .and_then(|t| self.rules.ty(t).resource_type)
                    .is_some()
                {
                    res.insert((e.type_name.to_string(), floordiv(e.stats.qty, PRECISION)));
                }
            }
        }
        let mut res: Vec<(String, i64)> = res.into_iter().collect();
        res.sort_by(|a, b| (a.0.as_str(), -a.1).cmp(&(b.0.as_str(), -b.1)));
        for (t, q) in res {
            let mut names = Vec::new();
            for s in &self.squares {
                for &o in &s.objects {
                    let e = self.e(o);
                    if *e.type_name == *t && floordiv(e.stats.qty, PRECISION) == q {
                        names.push(s.name.clone());
                    }
                }
            }
            let _ = writeln!(f, "{t} {q} {}", sorted(names).join(" "));
        }
        let _ = writeln!(f, "\nnb_meadows_by_square 0");
        let counts: BTreeSet<usize> = (0..self.squares.len())
            .map(|i| self.nb_meadows(SquareId(i as u32)))
            .filter(|&n| n > 0)
            .collect();
        for n in counts {
            let names = sorted(
                (0..self.squares.len())
                    .filter(|&i| self.nb_meadows(SquareId(i as u32)) == n)
                    .map(|i| self.squares[i].name.clone())
                    .collect(),
            );
            if n == 1 {
                let _ = writeln!(f, "; 1 meadow");
            } else {
                let _ = writeln!(f, "; {n} meadows");
            }
            for _ in 0..n {
                let _ = writeln!(f, "additional_meadows {}", names.join(" "));
            }
        }
        f.push('\n');
        let types: BTreeSet<&str> = self
            .squares
            .iter()
            .map(|s| s.type_name.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        for t in types {
            let names = sorted(
                self.squares
                    .iter()
                    .filter(|s| s.type_name == t)
                    .map(|s| s.name.clone())
                    .collect(),
            );
            let _ = writeln!(f, "terrain {t} {}", names.join(" "));
        }
        let names_where = |pred: &dyn Fn(&Square) -> bool| {
            sorted(
                self.squares
                    .iter()
                    .filter(|s| pred(s))
                    .map(|s| s.name.clone())
                    .collect(),
            )
            .join(" ")
        };
        let _ = writeln!(f, "high_grounds {}", names_where(&|s| s.high_ground));
        let _ = writeln!(f, "water {}", names_where(&|s| s.is_water));
        let _ = writeln!(f, "ground {}", names_where(&|s| s.is_ground && s.is_water));
        let _ = writeln!(f, "no_air {}", names_where(&|s| !s.is_air));
        let pyf = |x: i64| {
            let v = x as f64 / 100.0;
            if v.fract() == 0.0 {
                format!("{v:.1}")
            } else {
                format!("{v}")
            }
        };
        let covers: BTreeSet<(i64, i64)> = self
            .squares
            .iter()
            .map(|s| s.terrain_cover)
            .filter(|&c| c != (0, 0))
            .collect();
        for c in covers {
            let names = names_where(&|s| s.terrain_cover == c);
            let _ = writeln!(f, "cover {} {} {names}", pyf(c.0), pyf(c.1));
        }
        let speeds: BTreeSet<(i64, i64)> = self
            .squares
            .iter()
            .map(|s| s.terrain_speed)
            .filter(|&c| c != (100, 100))
            .collect();
        for c in speeds {
            let names = names_where(&|s| s.terrain_speed == c);
            let _ = writeln!(f, "speed {} {} {names}", pyf(c.0), pyf(c.1));
        }
        f.push('\n');
        let mut we: Vec<(String, Vec<String>)> = Vec::new();
        let mut sn: Vec<(String, Vec<String>)> = Vec::new();
        for s in &self.squares {
            for &e in &s.exits {
                let Some(os) = self.exit_other_side(e) else {
                    continue;
                };
                let Some(Place::Square(o)) = self.e(os).place else {
                    continue;
                };
                let o = self.square(o);
                let delta = (o.col - s.col, o.row - s.row);
                let list = match delta {
                    (1, 0) => &mut we,
                    (0, 1) => &mut sn,
                    _ => continue,
                };
                let tn = self.e(e).type_name.to_string();
                match list.iter_mut().find(|(k, _)| *k == tn) {
                    Some((_, v)) => v.push(s.name.clone()),
                    None => list.push((tn, vec![s.name.clone()])),
                }
            }
        }
        for (tn, v) in we {
            let _ = writeln!(f, "west_east {tn} {}", sorted(v).join(" "));
        }
        for (tn, v) in sn {
            let _ = writeln!(f, "south_north {tn} {}", sorted(v).join(" "));
        }
        f
    }
}
