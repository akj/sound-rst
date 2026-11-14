//! Map triggers, the trigger language (`lang_*`) and the text commands of
//! players (`cmd_*`), from `worldplayerbase.py` and `worldplayercomputer.py`.

use crate::entity::{EntityId, Place, PlayerId, Target};
use crate::events::{EntityEvent, Event};
use crate::map::MAX_START_UNITS;
use crate::msg::{Msg, MsgPart, msg};
use crate::msgparts as mp;
use crate::nofloat::{py_float, py_int};
use crate::player::{Objective, PlayerKind};
use crate::world::{Abort, NotEnoughSpace, Res, World};

/// A word or a parenthesized list of a trigger.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Expr {
    Word(String),
    List(Vec<Expr>),
}

impl Expr {
    fn word(&self) -> Res<&str> {
        match self {
            Expr::Word(w) => Ok(w),
            Expr::List(_) => Err(Abort::new("expected a word, found a list")),
        }
    }

    fn list(&self) -> Res<&[Expr]> {
        match self {
            Expr::List(l) => Ok(l),
            Expr::Word(_) => Err(Abort::new("expected a list, found a word")),
        }
    }

    /// The words of a list, as an order.
    fn words(&self) -> Res<Vec<String>> {
        self.list()?
            .iter()
            .map(|e| e.word().map(str::to_owned))
            .collect()
    }
}

/// `_list_to_tree`: parentheses make nested lists.
pub fn list_to_tree(words: &[String]) -> Result<Vec<Expr>, String> {
    let mut cache: Vec<Vec<Expr>> = vec![Vec::new()];
    for w in words {
        match w.as_str() {
            "(" => cache.push(Vec::new()),
            ")" => {
                if cache.len() < 2 {
                    return Err("list index out of range".into());
                }
                let last = cache.pop().expect("checked");
                cache.last_mut().expect("checked").push(Expr::List(last));
            }
            _ => cache
                .last_mut()
                .expect("never empty")
                .push(Expr::Word(w.clone())),
        }
    }
    Ok(cache.swap_remove(0))
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn msg_of(numbers: &[i64]) -> Msg {
    numbers
        .iter()
        .map(|&n| match u64::try_from(n) {
            Ok(n) => MsgPart::Num(n),
            Err(_) => MsgPart::Text(n.to_string()),
        })
        .collect()
}

impl World {
    // triggers

    pub(crate) fn run_triggers(&mut self, p: PlayerId) -> Res {
        if !self.pl(p).is_playing() {
            return Ok(());
        }
        for t in self.pl(p).triggers.clone() {
            if self.my_eval(p, &t.condition)? {
                self.my_eval(p, &t.action)?;
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

    /// `my_eval`: runs `lang_<first word>` with the rest; false for an
    /// unknown name. A lone word is read as its first letter, like the
    /// original indexing a string, and names nothing.
    fn my_eval(&mut self, p: PlayerId, l: &Expr) -> Res<bool> {
        let items = match l {
            Expr::Word(w) => {
                if w.is_empty() {
                    return Err(Abort::new("string index out of range"));
                }
                return Ok(false);
            }
            Expr::List(items) => items,
        };
        let name = items
            .first()
            .ok_or_else(|| Abort::new("list index out of range"))?
            .word()
            .map_err(|_| Abort::new("can only concatenate str (not \"list\") to str"))?
            .to_owned();
        let args = &items[1..];
        self.lang(p, &name, args)
    }

    pub(crate) fn lang(&mut self, p: PlayerId, name: &str, args: &[Expr]) -> Res<bool> {
        let arg_word = |i: usize| -> Res<String> {
            args.get(i)
                .ok_or_else(|| Abort::new("list index out of range"))?
                .word()
                .map(str::to_owned)
        };
        match name {
            "timer" => {
                let s = arg_word(0)?;
                let f = py_float(&s)
                    .map_err(|_| Abort::new(format!("could not convert string to float: {s:?}")))?;
                let lhs = (self.time / 1000) as f64;
                Ok(lhs >= f * self.map.timer_coefficient as f64)
            }
            "order" => {
                self.lang_order(p, args)?;
                Ok(false)
            }
            "has" => {
                let mut nb = 1;
                for x in args {
                    let x = x.word()?;
                    if is_digits(x) {
                        nb = py_int(x).map_err(|_| Abort::new("invalid number"))?;
                    } else {
                        for &u in &self.pl(p).units {
                            if *self.e(u).type_name == *x {
                                nb -= 1;
                                if nb == 0 {
                                    break;
                                }
                            }
                        }
                        if nb != 0 {
                            return Ok(false);
                        }
                        nb = 1;
                    }
                }
                Ok(true)
            }
            "has_entered" => {
                let Some(&first) = self.players.first() else {
                    return Err(Abort::new("list index out of range"));
                };
                for x in args {
                    let x = x.word()?;
                    let s = self
                        .square_by_name(x)
                        .ok_or_else(|| Abort::new(format!("KeyError: '{x}'")))?;
                    for &o in &self.square(s).objects {
                        if self.pl(first).units.contains(&o) && self.e(o).stats.presence != 0 {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            "add_units" => {
                let items: Vec<String> = args
                    .iter()
                    .map(|e| e.word().map(str::to_owned))
                    .collect::<Res<_>>()?;
                self.lang_add_units(p, &items, None, 0, None, true);
                Ok(false)
            }
            "no_enemy_left" => Ok(!self
                .players
                .iter()
                .any(|&op| self.player_is_an_enemy(p, op) && self.pl(op).is_playing())),
            "no_enemy_player_left" => Ok(!self
                .true_players()
                .iter()
                .any(|&op| self.player_is_an_enemy(p, op) && self.pl(op).is_playing())),
            "no_unit_left" => Ok(self.pl(p).units.is_empty()),
            "no_building_left" => Ok(!self
                .pl(p)
                .units
                .iter()
                .any(|&u| self.e(u).stats.provides_survival != 0)),
            "victory" => {
                self.victory(p);
                Ok(false)
            }
            "defeat" => {
                self.defeat(p, false);
                Ok(false)
            }
            "cut_scene" => {
                let words: Vec<String> = args
                    .iter()
                    .map(|e| e.word().map(str::to_owned))
                    .collect::<Res<_>>()?;
                self.push(p, Event::Sequence(words));
                Ok(false)
            }
            "add_objective" => {
                let n = arg_word(0)?;
                let mut description = Vec::new();
                for x in &args[1..] {
                    let x = x.word()?;
                    description.push(
                        py_int(x)
                            .map_err(|_| Abort::new(format!("invalid literal for int(): {x}")))?,
                    );
                }
                if !self.pl(p).objectives.iter().any(|o| o.number == n) {
                    let mut m = msg(mp::NEW_OBJECTIVE);
                    m.extend(msg_of(&description));
                    self.plm(p).objectives.push(Objective {
                        number: n,
                        description,
                    });
                    self.send_voice_important(p, m);
                }
                Ok(false)
            }
            "objective_complete" => {
                let n = arg_word(0)?;
                if let Some(i) = self.pl(p).objectives.iter().position(|o| o.number == n) {
                    let mut m = msg(mp::OBJECTIVE_COMPLETE);
                    m.extend(msg_of(&self.pl(p).objectives[i].description));
                    self.send_voice_important(p, m);
                    self.plm(p).objectives.remove(i);
                    if self.pl(p).objectives.is_empty() {
                        self.send_voice_important(p, msg(mp::MISSION_COMPLETE));
                        self.victory(p);
                    }
                }
                Ok(false)
            }
            "ai" => {
                let t = arg_word(0)?;
                self.set_ai(p, &t)?;
                Ok(false)
            }
            "faction" => {
                let f = args.first().map(Expr::word).transpose()?;
                match f {
                    Some(f) if self.rules.factions().iter().any(|x| x == f) => {
                        self.plm(p).faction = f.to_owned();
                    }
                    _ => {
                        let words: Vec<String> = args
                            .iter()
                            .filter_map(|e| e.word().ok().map(str::to_owned))
                            .collect();
                        self.warn(format!("unknown faction: {}", words.join(" ")));
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// `set_ai`: changes the AI type of a computer player.
    fn set_ai(&mut self, p: PlayerId, ai_type: &str) -> Res {
        if !matches!(self.pl(p).kind, PlayerKind::Computer { .. }) {
            return Ok(());
        }
        self.plm(p).kind = PlayerKind::Computer {
            ai_type: ai_type.to_owned(),
        };
        self.computer_set_ai(p, ai_type)
    }

    fn lang_order(&mut self, p: PlayerId, args: &[Expr]) -> Res {
        let [select, orders] = args else {
            return Err(Abort::new("not enough values to unpack"));
        };
        let select = select.list()?;
        let orders: Vec<Vec<String>> =
            orders.list()?.iter().map(Expr::words).collect::<Res<_>>()?;
        let mut default_square = "a1".to_owned();
        let mut n: i64 = 1;
        for x in select {
            let x = x.word()?;
            if self.square_by_name(x).is_some() {
                default_square = x.to_owned();
                n = 1;
            } else if is_digits(x) {
                n = py_int(x).map_err(|_| Abort::new("invalid number"))?;
            } else {
                let s = self
                    .square_by_name(&default_square)
                    .ok_or_else(|| Abort::new(format!("KeyError: '{default_square}'")))?;
                // The list may change while orders are given, as in the
                // original, which walks it by index.
                let mut i = 0;
                while let Some(&o) = self.square(s).objects.get(i) {
                    if *self.e(o).type_name == *x && self.e(o).player == Some(p) {
                        for order in &orders {
                            self.take_order(o, order.clone(), false, false, None)?;
                        }
                        n -= 1;
                        if n == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                n = 1;
            }
        }
        Ok(())
    }

    /// `lang_add_units`: adds units, upgrades, on squares named in the
    /// list (`a1 2 footman b3 archer`). Summons use a target, raised dead
    /// use corpses.
    pub(crate) fn lang_add_units(
        &mut self,
        p: PlayerId,
        items: &[String],
        target: Option<&Target>,
        decay: i64,
        mut corpses: Option<Vec<EntityId>>,
        notify: bool,
    ) {
        let Some(mut square) = self.square_by_name("a1") else {
            self.warn("KeyError: 'a1'");
            return;
        };
        let mut nb: i64 = 1;
        for i in items {
            if let Some(s) = self.square_by_name(i) {
                square = s;
                nb = 1;
            } else if is_digits(i) {
                nb = py_int(i).unwrap_or(1).min(MAX_START_UNITS);
            } else {
                match self
                    .rules
                    .unit_type(i)
                    .map(|t| (t.is_upgrade(), t.is_ability()))
                {
                    Some((true, _)) => {
                        self.plm(p).upgrades.push(i.clone());
                        self.send_voice_important(p, msg(mp::OK));
                    }
                    Some((_, true)) => {
                        self.send_voice_important(p, msg(mp::BEEP));
                        self.warn("cannot add an ability");
                    }
                    Some(_) => {
                        for _ in 0..nb {
                            if !self.check_count_limit(p, i) {
                                break;
                            }
                            match self.add_unit_for_lang(
                                p,
                                i,
                                square,
                                target,
                                decay,
                                corpses.as_mut(),
                                notify,
                            ) {
                                Ok(()) => {}
                                Err(AddUnitError::NotEnoughSpace) => {
                                    self.warn("not enough space");
                                    // The original deletes the player's last
                                    // unit here, which is not the one that
                                    // failed.
                                    if let Some(&last) = self.pl(p).units.last() {
                                        self.delete(last);
                                    }
                                }
                                Err(AddUnitError::Other(m)) => {
                                    self.warn(format!("couldn't add unit: {i}: {m}"))
                                }
                            }
                        }
                    }
                    None => self.send_voice_important(p, msg(mp::BEEP)),
                }
                nb = 1;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_unit_for_lang(
        &mut self,
        p: PlayerId,
        type_name: &str,
        mut square: crate::entity::SquareId,
        target: Option<&Target>,
        decay: i64,
        corpses: Option<&mut Vec<EntityId>>,
        notify: bool,
    ) -> Result<(), AddUnitError> {
        let tid = self
            .rules
            .type_id(type_name)
            .ok_or(AddUnitError::Other("no type".into()))?;
        if self
            .rules
            .ty(tid)
            .entity_class()
            .is_none_or(|c| c == crate::rules::EntityClass::Deposit)
        {
            return Err(AddUnitError::Other(
                "__init__() takes 3 positional arguments".into(),
            ));
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
            let anywhere = self.rules.ty(tid).stats.is_buildable_anywhere != 0;
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
        self.em(u).building_land = land;
        if decay != 0 {
            let t = self.time + decay;
            self.em(u).time_limit = Some(t);
        }
        if notify {
            self.notify(u, EntityEvent::Added, false);
        }
        Ok(())
    }

    // commands

    /// Runs a text command of a player now (`execute_command`): `order`,
    /// `control`, `cmd`, `speed`, `quit`, `neutral_quit`,
    /// `toggle_cheatmode`. `cmd` and `toggle_cheatmode` run only if
    /// [`World::allow_cheatmode`] is set.
    pub fn execute_command(&mut self, p: PlayerId, data: &str) -> Result<(), Abort> {
        let args: Vec<String> = data.split_whitespace().map(str::to_owned).collect();
        let Some(first) = args.first() else {
            return Err(Abort::new("list index out of range"));
        };
        let cmd = first.to_lowercase();
        let rest = args[1..].to_vec();
        match cmd.as_str() {
            "toggle_cheatmode" | "cmd" if !self.allow_cheatmode => {
                self.warn(format!("cheat mode is not allowed: {data}"));
            }
            "toggle_cheatmode" => {
                let c = self.pl(p).cheatmode;
                self.plm(p).cheatmode = !c;
            }
            "cmd" => {
                let e = Expr::List(rest.into_iter().map(Expr::Word).collect());
                self.my_eval(p, &e)?;
            }
            "speed" => {
                if self.players.first() == Some(&p) {
                    let s = rest
                        .first()
                        .ok_or_else(|| Abort::new("list index out of range"))?;
                    let f = py_float(s).map_err(|_| {
                        Abort::new(format!("could not convert string to float: {s:?}"))
                    })?;
                    for op in self.players.clone() {
                        self.push(op, Event::Speed(f));
                    }
                } else if !self.players.contains(&p) {
                    return Err(Abort::new("x not in list"));
                } else {
                    self.warn("non admin client tried to change game speed");
                }
            }
            "quit" => self.defeat(p, true),
            "neutral_quit" => {
                if self.players.contains(&p) {
                    self.quit_game(p);
                }
            }
            "order" => {
                if let Err(Abort(m)) = self.cmd_order(p, rest.clone()) {
                    self.warn(format!("problem with order: {rest:?}: {m}"));
                }
            }
            "control" => self.cmd_control(p, &rest)?,
            _ => self.warn(format!("unknown command: 'cmd_{cmd}' ({data})")),
        }
        Ok(())
    }

    fn reset_group(&mut self, name: &str) {
        if let Some(i) = self.groups.iter().position(|(n, _)| n == name) {
            for u in self.groups[i].1.clone() {
                self.em(u).group = None;
            }
            self.groups[i].1 = Vec::new();
        }
    }

    fn cmd_order(&mut self, p: PlayerId, mut args: Vec<String>) -> Res {
        self.plm(p).group_had_enough_mana = false;
        let order_id = self.get_next_order_id();
        let take = |args: &mut Vec<String>| -> Res<String> {
            if args.is_empty() {
                return Err(Abort::new("list index out of range"));
            }
            Ok(args.remove(0))
        };
        let forget_previous = take(&mut args)? == "0";
        let imperative = take(&mut args)? == "1";
        let first = args
            .first()
            .cloned()
            .ok_or_else(|| Abort::new("list index out of range"))?;
        if first == "reset_group" {
            let name = args
                .get(1)
                .cloned()
                .ok_or_else(|| Abort::new("list index out of range"))?;
            self.reset_group(&name);
            return Ok(());
        }
        let selection = self.pl(p).group.clone();
        for u in selection.clone().unwrap_or_default() {
            if let Some(g) = self.e(u).group.clone() {
                let list = self.group_list(&g).cloned().unwrap_or_default();
                if Some(&list) != selection.as_ref() {
                    if let Some((_, l)) = self.groups.iter_mut().find(|(n, _)| *n == g)
                        && let Some(i) = l.iter().position(|&x| x == u)
                    {
                        l.remove(i);
                    }
                    self.em(u).group = None;
                }
            }
            if self
                .e(u)
                .player
                .is_some_and(|up| self.pl(p).allied_control.contains(&up))
            {
                let r = if args[0] == "default" {
                    match args.get(1).cloned() {
                        Some(t) => self.take_default_order(
                            u,
                            &t,
                            forget_previous,
                            imperative,
                            Some(order_id),
                        ),
                        None => Err(Abort::new("list index out of range")),
                    }
                } else {
                    self.take_order_shared(
                        u,
                        &mut args,
                        forget_previous,
                        imperative,
                        Some(order_id),
                    )
                };
                if let Err(Abort(m)) = r {
                    self.warn(format!("problem with order: {args:?}: {m}"));
                }
            }
        }
        Ok(())
    }

    /// `take_order` with the order list shared between the units of a
    /// group: an imperative `go` that becomes `attack` stays `attack` for
    /// the next units, as in the original.
    fn take_order_shared(
        &mut self,
        u: EntityId,
        args: &mut [String],
        forget_previous: bool,
        imperative: bool,
        order_id: Option<u64>,
    ) -> Res {
        if imperative
            && args.first().map(String::as_str) == Some("go")
            && let Some(up) = self.e(u).player
        {
            let id = args
                .get(1)
                .cloned()
                .ok_or_else(|| Abort::new("list index out of range"))?;
            let target = self.get_object_by_id(up, &id)?;
            if target
                .as_ref()
                .and_then(|t| self.t_entity(t))
                .is_some_and(|e| e.player.is_some())
            {
                args[0] = "attack".into();
            }
        }
        self.take_order(u, args.to_vec(), forget_previous, imperative, order_id)
    }

    fn cmd_control(&mut self, p: PlayerId, args: &[String]) -> Res {
        self.plm(p).group = Some(Vec::new());
        let ids = crate::group::decode(&args.join(" ")).map_err(Abort::new)?;
        let mut units = Vec::new();
        for ap in self.pl(p).allied_control.clone() {
            units.extend(self.pl(ap).units.iter().copied());
        }
        let mut group = Vec::new();
        for id in ids {
            if let Some(&u) = units.iter().find(|u| u.0.to_string() == id) {
                group.push(u);
            }
        }
        self.plm(p).group = Some(group);
        Ok(())
    }
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
}
