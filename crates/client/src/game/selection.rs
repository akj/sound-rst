//! Choosing what to act on and acting: the target, the units controlled,
//! the order, and sending the order to the game.

use std::num::NonZeroU32;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::{Keyword, MenuEntry};
use soundrts_world::rules::TypeId;
use soundrts_world::{Command, EntityId, Event, Order, OrderTarget, TurnMessage};

use crate::audio::stereo;
use crate::client::Client;
use crate::net::words::{self, text};
use crate::sound::StereoVolume;

use super::command::{OrderFlags, OrderMenu, UnitFilter};
use super::entity::EntityView;
use super::focus::Zoom;
use super::order::{OrderTypeView, PRECISION};
use super::squares::{remove_duplicates, summary};
use super::{BEEP_SOUND, GameInterface};

/// What an order with a target aims at: the selected object, else the
/// point of the zoom on its square, else the selected square.
enum UiTarget<'a> {
    Entity(&'a EntityView),
    Zoom(&'a Zoom, soundrts_world::SquareId),
    Square(soundrts_world::SquareId),
}

/// The unit types a selection command asks for: any type, or these (none
/// at all when the keys name types the style does not have).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeFilter {
    Any,
    These(Vec<String>),
}

impl TypeFilter {
    pub(super) fn accepts(&self, type_name: &str) -> bool {
        match self {
            TypeFilter::Any => true,
            TypeFilter::These(t) => t.iter().any(|x| x == type_name),
        }
    }
}

impl GameInterface {
    // target

    /// The selection order of a target: by kind, then title length, then
    /// distance.
    fn priority(&self, cx: &Client, v: &EntityView) -> (f64, usize, f64) {
        let mut p = 10.0;
        if self.an_order_requiring_a_target_is_selected() {
            if self
                .order
                .as_ref()
                .is_some_and(|o| o.keyword == Keyword::Build)
                && v.model.is_a_building_land
            {
                p = if self.is_an_exit(cx, v) { 0.5 } else { 0.0 };
            }
        } else if self.is_an_enemy(&v.model) {
            p = 0.5;
        } else if let Some(rt) = self.resource_type(&v.model).filter(|_| v.qty() > 0) {
            p = 1.0 + rt as f64 / 100.0;
        } else if v.model.is_repairable && v.hp() < v.hp_max() {
            p = 2.0;
        } else if v.model.is_a_building_land {
            p = 3.0;
        } else if v.model.is_exit() {
            p = 4.0;
        }
        (p, self.title(cx, v).len(), self.distance_to(v))
    }

    fn object_choices(&self, cx: &Client, inc: i64, types: &[String]) -> Vec<EntityId> {
        let mut choices = Vec::new();
        for v in self.dobjets.values() {
            if self.is_visible(cx, v)
                && (types.is_empty()
                    || types.iter().any(|t| t == v.type_name())
                    || types.iter().any(|t| t == "useful") && self.is_a_useful_target(cx, v))
            {
                choices.push((self.priority(cx, v), v.id()));
            }
        }
        choices.sort_by(|a, b| {
            a.0.0
                .total_cmp(&b.0.0)
                .then(a.0.1.cmp(&b.0.1))
                .then(a.0.2.total_cmp(&b.0.2))
        });
        let mut ids: Vec<EntityId> = choices.into_iter().map(|(_, id)| id).collect();
        if inc == -1 {
            ids.reverse();
        }
        ids
    }

    /// The selected object.
    fn target_view(&self) -> Option<&EntityView> {
        self.dobjets.get(&self.target?)
    }

    /// Says the target with a beep from its direction.
    pub(crate) fn say_target(&self, cx: &mut Client) {
        let Some(v) = self.target_view() else { return };
        let mut d = self.positional_description(cx, v);
        if self.an_order_requiring_a_target_is_selected()
            && let Some(o) = &self.order
        {
            d.extend(msg(mp::COMMA));
            d.extend(o.title.iter().cloned());
        }
        let (mut l, mut r) = stereo(self.x, self.y, v.x(), v.y(), self.o, 1.0, true);
        if l.max(r) < 0.2 {
            (l, r) = (0.2, 0.2);
        }
        let m = [msg(mp::POSITIONAL_BEEP), d].concat();
        cx.voice.item_stereo(&m, l, r);
    }

    pub(crate) fn cmd_examine(&self, cx: &mut Client) {
        if self.target.is_some() {
            self.say_target(cx);
        } else if self.zoom.is_some() {
            self.say_zoom(cx, &Vec::new());
        } else {
            self.say_square(cx, self.place, &Vec::new());
        }
    }

    pub(crate) fn cmd_select_target(&mut self, cx: &mut Client, inc: i64, types: &[String]) {
        let choices = self.object_choices(cx, inc, types);
        if choices.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            self.target = None;
        } else {
            let sel = self
                .target
                .and_then(|t| choices.iter().position(|&c| c == t))
                .map_or(0, |i| i + 1);
            self.target = Some(choices[if sel >= choices.len() { 0 } else { sel }]);
            self.say_target(cx);
        }
    }

    // units

    /// The units the player controls, on the map (not inside), perceived.
    /// Without `even_if_no_menu`, only those that can be given an order.
    pub fn units(&self, cx: &Client, even_if_no_menu: bool, sort: bool) -> Vec<EntityId> {
        let view = self.view();
        let mut result: Vec<EntityId> = Vec::new();
        for e in view.controlled_units() {
            let u = e.id;
            if e.is_inside() || !self.dobjets.contains_key(&u) {
                continue;
            }
            let has_menu = even_if_no_menu || {
                let t = view.unit_type(&e.type_name);
                let own = e.own.as_ref();
                t.is_some_and(|t| !t.can_train.is_empty() || !t.can_upgrade_to.is_empty())
                    || own.is_some_and(|o| !o.orders.is_empty() || o.has_basic_abilities)
                    || !self.view_menu(u, false).is_empty()
            };
            if has_menu {
                result.push(u);
            }
        }
        if sort {
            let key = |id: &EntityId| {
                let v = &self.dobjets[id];
                let t: Vec<u64> = self
                    .short_title(cx, v)
                    .iter()
                    .map(|p| match p {
                        MsgPart::Num(n) => *n,
                        MsgPart::Text(_) | MsgPart::Literal(_) => 0,
                    })
                    .collect();
                (t, v.model.number)
            };
            result.sort_by_key(key);
        }
        result
    }

    /// Puts each unit that took the place of another, by an upgrade, in
    /// its place wherever the interface names it between turns: the group,
    /// the group last sent, the order picked and the order to do again,
    /// and the selected object if the turn's message `m` knows it. Done
    /// before the turn's entities are taken, which keep the selected object
    /// only if it is still known. The commands not yet sent followed when
    /// the turn came (see [`GameInterface::follow_replaced_in_commands`]).
    pub(super) fn follow_replaced_units(&mut self, m: &TurnMessage) {
        for e in &m.events {
            let &Event::Replaced { old, new } = e else {
                continue;
            };
            let follow = |u: &mut EntityId| {
                if *u == old {
                    *u = new;
                }
            };
            self.group.iter_mut().for_each(follow);
            self.previous_group.iter_mut().flatten().for_each(follow);
            for o in self.order.iter_mut().chain(&mut self.previous_order) {
                follow(&mut o.unit);
            }
            let known = m.perceived.iter().chain(&m.remembered).any(|e| e.id == new);
            if self.target == Some(old) && known {
                self.target = Some(new);
            }
        }
    }

    /// Keeps in the group the units still known and controlled.
    pub fn update_group(&mut self) {
        let view = &self.seat;
        let group = std::mem::take(&mut self.group);
        self.group = group
            .into_iter()
            .filter(|u| {
                self.dobjets.get(u).is_some_and(|v| {
                    v.player()
                        .is_some_and(|vp| vp == self.me || view.controls(vp))
                })
            })
            .collect();
    }

    /// The types named by `keyboard` in style, from the keys' names.
    pub(super) fn arrange(&self, cx: &Client, filter: &UnitFilter) -> TypeFilter {
        if filter.keyboard_types.is_empty() {
            return TypeFilter::Any;
        }
        let style = cx.res.style();
        TypeFilter::These(
            style
                .objects()
                .filter(|x| style.has(x, "keyboard"))
                .filter(|x| {
                    style
                        .words(x, "keyboard")
                        .first()
                        .is_some_and(|k| filter.keyboard_types.contains(k))
                })
                .map(str::to_owned)
                .collect(),
        )
    }

    fn regroup(&mut self, cx: &mut Client, portion: NonZeroU32, filter: &UnitFilter) {
        self.update_group();
        if let Some(&first) = self.group.first() {
            let initial = &self.dobjets[&first];
            let initial_type = initial.type_name().to_owned();
            let local_place = if self.is_in(initial, self.place) {
                self.place
            } else {
                self.outer_square(initial.place())
            };
            let types = match self.arrange(cx, filter) {
                TypeFilter::These(t) => t,
                TypeFilter::Any => remove_duplicates(
                    &self
                        .group
                        .iter()
                        .map(|id| self.dobjets[id].type_name().to_owned())
                        .collect::<Vec<_>>(),
                ),
            };
            let units = self.units(cx, true, false);
            let mut group = Vec::new();
            for t in &types {
                let m: Vec<EntityId> = units
                    .iter()
                    .copied()
                    .filter(|x| {
                        let v = &self.dobjets[x];
                        v.type_name() == t
                            && (!filter.local
                                || match &self.zoom {
                                    Some(z) => z.contains(v.model.x, v.model.y),
                                    None => self.is_in(v, local_place),
                                })
                            && (!filter.idle || v.orders().is_empty())
                    })
                    .collect();
                group.extend_from_slice(&m[..m.len() / portion.get() as usize]);
            }
            if !group.contains(&first) && types.contains(&initial_type) {
                group.pop();
                group.push(first);
            }
            self.group = group;
        }
        self.say_group(cx, &Vec::new())
    }

    pub(crate) fn cmd_group(&mut self, cx: &mut Client, portion: NonZeroU32, filter: &UnitFilter) {
        self.regroup(cx, portion, filter)
    }

    pub(crate) fn cmd_ungroup(&mut self, cx: &mut Client) {
        if self.group.len() > 1 {
            self.group.truncate(1);
        }
        self.say_group(cx, &Vec::new())
    }

    fn command_unit(&mut self, cx: &mut Client, unit: EntityId, silent: bool) {
        if !silent {
            let v = &self.dobjets[&unit];
            let m = [
                self.ext_title(cx, v),
                self.orders_txt(cx, v),
                msg(mp::AWAITING_YOUR_ORDERS),
            ]
            .concat();
            cx.voice.item(&m);
        }
        self.group = vec![unit];
    }

    pub(crate) fn cmd_command_unit(&mut self, cx: &mut Client) {
        if let Some(t) = self.target
            && self.units(cx, true, false).contains(&t)
        {
            self.command_unit(cx, t, false);
        }
    }

    fn select_unit(&mut self, cx: &mut Client, inc: i64, filter: &UnitFilter, silent: bool) {
        let types = self.arrange(cx, filter);
        let units: Vec<EntityId> = self
            .units(cx, filter.even_if_no_menu, true)
            .into_iter()
            .filter(|x| {
                let v = &self.dobjets[x];
                types.accepts(v.type_name())
                    && (!filter.local
                        || match &self.zoom {
                            Some(z) => z.contains(v.model.x, v.model.y),
                            None => self.is_in(v, self.place),
                        })
                    && (!filter.idle || v.orders().is_empty())
            })
            .collect();
        if units.is_empty() {
            self.group = Vec::new();
            return;
        }
        let len = units.len() as i64;
        let mut sel = units
            .iter()
            .position(|u| self.group.contains(u))
            .map_or(-1, |i| i as i64);
        sel += inc;
        if sel < 0 {
            sel = len - 1;
        }
        if sel >= len {
            sel = 0;
        }
        self.command_unit(cx, units[sel as usize], silent);
        self.order = None;
    }

    pub(crate) fn cmd_select_unit(&mut self, cx: &mut Client, inc: i64, filter: &UnitFilter) {
        self.select_unit(cx, inc, filter, false)
    }

    pub(crate) fn cmd_select_units(&mut self, cx: &mut Client, filter: &UnitFilter) {
        self.select_unit(cx, 1, filter, true);
        self.regroup(cx, NonZeroU32::MIN, filter)
    }

    /// Says the units controlled and their orders, after `prefix`.
    pub(crate) fn say_group(&mut self, cx: &mut Client, prefix: &Msg) {
        self.update_group();
        if self.group.len() == 1 {
            let v = &self.dobjets[&self.group[0]];
            let m = [
                prefix.clone(),
                msg(mp::YOU_CONTROL),
                self.ext_title(cx, v),
                self.orders_txt(cx, v),
            ]
            .concat();
            cx.voice.item(&m);
        } else if self.group.len() > 1 {
            let mut orders = Vec::new();
            let mut titles = Vec::new();
            for x in &self.group {
                if let Some(v) = self.dobjets.get(x) {
                    orders.push(self.orders_txt(cx, v));
                    titles.push(self.short_title(cx, v));
                }
            }
            let mut m = [
                prefix.clone(),
                msg(mp::COMMA),
                msg(mp::YOU_CONTROL),
                summary(&titles, false),
            ]
            .concat();
            if remove_duplicates(&orders).len() == 1 {
                m.extend(msg(mp::COMMA));
                m.extend(orders[0].iter().cloned());
            } else if remove_duplicates(&titles).len() == 1 {
                m.extend(msg(mp::COMMA));
                m.extend(summary(&orders, false));
            }
            cx.voice.item(&m);
        } else {
            let m = [prefix.clone(), msg(mp::COMMA), msg(mp::NO_UNIT_CONTROLLED)].concat();
            cx.voice.item(&m);
        }
    }

    pub(crate) fn cmd_unit_status(&mut self, cx: &mut Client) {
        let place_title = match self.group.first().and_then(|g| self.dobjets.get(g)) {
            Some(v) => v
                .place()
                .map(|p| self.place_title(cx, p))
                .unwrap_or_default(),
            None => self
                .place
                .map(|p| self.view().square(p).title().clone())
                .unwrap_or_default(),
        };
        self.say_group(cx, &place_title);
        if !self.group.is_empty() {
            self.follow_mode = true;
            self.follow_if_needed(cx);
        }
    }

    /// The health of the one unit controlled, or of the group as a whole:
    /// "35 hit points on 50 , 1 badly hurt , 2 footman and 1 archer". A
    /// unit is badly hurt below half its hit points.
    pub(crate) fn cmd_unit_hp_status(&self, cx: &mut Client) {
        if self.group.len() == 1
            && let Some(v) = self.dobjets.get(&self.group[0])
        {
            let d = self.description(cx, v);
            cx.voice.item(&d);
        } else if self.group.len() > 1 {
            let (mut hp, mut hp_max, mut hurt) = (0, 0, 0);
            let mut titles = Vec::new();
            for v in self.group.iter().filter_map(|u| self.dobjets.get(u)) {
                hp += v.model.hp;
                hp_max += v.model.hp_max;
                if v.model.hp * 2 < v.model.hp_max {
                    hurt += 1;
                }
                titles.push(self.short_title(cx, v));
            }
            let mut m = [
                nb2msg(hp / PRECISION),
                msg(mp::HITPOINTS_ON),
                nb2msg(hp_max / PRECISION),
            ]
            .concat();
            if hurt > 0 {
                m.extend(msg(mp::COMMA));
                m.extend(nb2msg(hurt));
                m.extend(text(words::BADLY_HURT));
            }
            m.extend(msg(mp::COMMA));
            m.extend(summary(&titles, false));
            cx.voice.item(&m);
        }
    }

    // recallable groups

    pub(crate) fn cmd_set_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) {
        self.send_order(cx, Order::ResetGroup(name.to_owned()), flags);
        self.send_order(cx, Order::JoinGroup(name.to_owned()), flags);
    }

    pub(crate) fn cmd_append_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) {
        self.send_order(cx, Order::JoinGroup(name.to_owned()), flags);
    }

    pub(crate) fn cmd_recall_group(&mut self, cx: &mut Client, name: &str) {
        self.group = self.view().recallable_group(name);
        self.say_group(cx, &Vec::new())
    }

    /// The orders of the group's units, in menu order.
    pub fn orders(&self, cx: &Client, menu: OrderMenu) -> Vec<OrderTypeView> {
        let mut result: Vec<OrderTypeView> = Vec::new();
        let mut done: Vec<MenuEntry> = Vec::new();
        for (u, id) in self.group_now() {
            if !self.dobjets.contains_key(&u) {
                continue;
            }
            let entries = match menu {
                OrderMenu::Active => self.view_menu(id, true),
                OrderMenu::Included => self.view_menu(id, false),
                OrderMenu::InactiveOnly => {
                    let strict = self.view_menu(id, true);
                    self.view_menu(id, false)
                        .into_iter()
                        .filter(|o| !strict.contains(o))
                        .collect()
                }
            };
            for o in entries {
                if !done.contains(&o) {
                    result.push(OrderTypeView::new(
                        &o,
                        id,
                        self.view(),
                        cx.res.style(),
                        &cx.log,
                    ));
                    done.push(o);
                }
            }
        }
        result.sort_by(|a, b| a.index.total_cmp(&b.index));
        result
    }

    pub fn an_order_requiring_a_target_is_selected(&self) -> bool {
        self.order.as_ref().is_some_and(|o| o.nb_args != 0)
    }

    fn an_order_not_requiring_a_target_is_selected(&self) -> bool {
        self.order.as_ref().is_some_and(|o| o.nb_args == 0)
    }

    /// Selects an order and says it, with its comment, its requirements
    /// and, with `help`, what to do next.
    pub(crate) fn select_order(&mut self, cx: &mut Client, order: OrderTypeView, help: bool) {
        let mut m = order.title.clone();
        m.extend(msg(mp::COMMA));
        m.extend(order.full_comment(self.view(), cx.res.style()));
        if help {
            m.extend(msg(mp::COMMA));
            if order.nb_args == 0 {
                m.extend(msg(mp::CONFIRM));
            } else {
                m.extend(msg(mp::SELECT_TARGET_AND_CONFIRM));
            }
        }
        self.order = Some(order);
        cx.voice.item(&m);
    }

    pub(crate) fn cmd_select_order(&mut self, cx: &mut Client, inc: i64, menu: OrderMenu) {
        let orders = self.orders(cx, menu);
        if orders.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            self.order = None;
            return;
        }
        let len = orders.len() as i64;
        let mut index = match &self.order {
            Some(o) => orders.iter().position(|x| x == o).map_or(-1, |i| i as i64),
            None => -1,
        };
        index += inc;
        if index < 0 {
            index = len - 1;
        } else if index >= len {
            index = 0;
        }
        let o = orders[index as usize].clone();
        self.select_order(cx, o, menu != OrderMenu::InactiveOnly);
    }

    pub(crate) fn cmd_order_shortcut(&mut self, cx: &mut Client) {
        if !self.group.is_empty() {
            let mut m = Vec::new();
            for o in self.orders(cx, OrderMenu::Active) {
                if let Some(s) = &o.shortcut {
                    m.push(MsgPart::Text(s.clone()));
                    m.extend(o.title.iter().cloned());
                    m.extend(msg(mp::COMMA));
                }
            }
            if !m.is_empty() {
                self.mode = super::Mode::OrderShortcut;
                cx.voice.item(&m);
                return;
            }
        }
        cx.voice.item(&msg(mp::BEEP));
    }

    /// The key after the order shortcut key: the order with that shortcut.
    pub(crate) fn execute_order_shortcut(&mut self, cx: &mut Client, e: &crate::input::KeyDown) {
        for o in self.orders(cx, OrderMenu::Active) {
            if o.shortcut.as_deref() == Some(e.unicode.as_str()) {
                let no_target = o.nb_args == 0;
                self.select_order(cx, o, true);
                if no_target {
                    self.cmd_validate(cx, OrderFlags::default());
                }
                return;
            }
        }
        cx.voice.item(&msg(mp::BEEP));
    }

    /// Selects the previous order again, and with `now` gives it if it
    /// needs no target. If the unit it was listed for has since gone to a
    /// player the player does not control, the order is the group's own,
    /// as one of its units lists it; if none does, it cannot be given.
    pub(crate) fn cmd_do_again(&mut self, cx: &mut Client, now: bool, flags: OrderFlags) {
        if let Some(mut o) = self.previous_order.clone()
            && !self.group.is_empty()
        {
            o.unit = followed(&self.replaced_in_turns_waiting(), o.unit);
            let owner = self.dobjets.get(&o.unit).and_then(EntityView::player);
            if owner.is_some_and(|p| !self.view().controls(p)) {
                let listed = self.orders(cx, OrderMenu::Included);
                match listed.into_iter().find(|x| *x == o) {
                    Some(x) => o = x,
                    None => {
                        cx.voice.item(&msg(mp::BEEP));
                        return;
                    }
                }
            }
            let no_target = o.nb_args == 0;
            self.select_order(cx, o, true);
            if now && no_target {
                self.cmd_validate(cx, flags);
            }
        }
    }

    /// Sends an order to the units of the group that may receive it, by
    /// the ids the game gives them now (see [`GameInterface::group_now`]).
    pub(crate) fn send_order(&mut self, cx: &Client, order: Order, flags: OrderFlags) {
        let g: Vec<EntityId> = match &order {
            Order::Default(_) | Order::JoinGroup(_) => {
                self.group_now().into_iter().map(|(_, id)| id).collect()
            }
            // In no unit's menu: no unit receives it, as in the original.
            Order::ResetGroup(_) => Vec::new(),
            Order::Keyword(o) => {
                // As the menus list it: a type the rules lack is in none.
                let rules = &self.view().game_start().rules;
                match o.type_name().map(|t| rules.type_id(t)) {
                    Some(None) => Vec::new(),
                    t => self.units_offering(o.keyword(), t.flatten()),
                }
            }
        };
        let mut command = Command::Order {
            order,
            queue: flags.queue(),
            imperative: flags.imperative,
        };
        for (old, new) in self.replaced_in_turns_waiting() {
            follow_in_command(&mut command, old, new);
        }
        if self.previous_group.as_ref() != Some(&g) {
            self.send_command(cx, Command::Control(g.clone()));
            self.previous_group = Some(g);
        }
        self.send_command(cx, command);
    }

    /// The units of the group, each with the id the game gives it now. A
    /// unit replaced in a turn that has come and is not yet taken has the
    /// id of the unit it became: the game and the seat's view know only
    /// that one, while the interface's copies know only the old one until
    /// it takes the turn.
    fn group_now(&self) -> Vec<(EntityId, EntityId)> {
        let replaced = self.replaced_in_turns_waiting();
        let now = |u| followed(&replaced, u);
        self.group.iter().map(|&u| (u, now(u))).collect()
    }

    /// `units` as the game has them now: each one replaced in a turn
    /// waiting to be taken is the unit that took its place.
    pub(super) fn followed_now(&self, units: &[EntityId]) -> Vec<EntityId> {
        let replaced = self.replaced_in_turns_waiting();
        units.iter().map(|&u| followed(&replaced, u)).collect()
    }

    /// The units replaced in the turns that have come and wait to be
    /// taken, old and new, in order.
    fn replaced_in_turns_waiting(&self) -> Vec<(EntityId, EntityId)> {
        let ends = self.srv_queue.iter().filter_map(|e| match e {
            super::SrvEvent::EndOfTurn(m) => Some(m),
            _ => None,
        });
        ends.flat_map(|m| replacements(m)).collect()
    }

    /// Makes the commands not yet sent name the units that took the place
    /// of others in the turn `m`, as soon as it comes: they may be sent
    /// before the interface takes the turn.
    pub(super) fn follow_replaced_in_commands(&mut self, m: &TurnMessage) {
        for (old, new) in replacements(m) {
            for c in &mut self.pending_commands {
                follow_in_command(c, old, new);
            }
        }
    }

    /// The units of the group whose menu has the order of `keyword` and
    /// type `t`, by the ids the game gives them now.
    fn units_offering(&self, keyword: Keyword, t: Option<TypeId>) -> Vec<EntityId> {
        self.group_now()
            .into_iter()
            .map(|(_, id)| id)
            .filter(|&id| {
                self.view_menu(id, false)
                    .iter()
                    .any(|o| o.keyword == keyword && o.type_id == t)
            })
            .collect()
    }

    /// What an order aims at: the target, else the zoom, else the square;
    /// nothing before a square is selected.
    fn ui_target(&self) -> Option<UiTarget<'_>> {
        match (self.target_view(), &self.zoom, self.place) {
            (Some(v), _, _) => Some(UiTarget::Entity(v)),
            (None, Some(z), Some(p)) => Some(UiTarget::Zoom(z, p)),
            (None, None, Some(p)) => Some(UiTarget::Square(p)),
            (None, _, None) => None,
        }
    }

    /// What an order aims at, as the game takes it, and its title.
    fn ui_order_target(&self, cx: &Client) -> Option<(OrderTarget, Msg)> {
        Some(match self.ui_target()? {
            UiTarget::Entity(v) => (OrderTarget::Entity(v.id()), self.title(cx, v)),
            UiTarget::Zoom(z, p) => {
                let square = self.view().square(p);
                let (x, y) = z.point(square);
                (OrderTarget::Point { square: p, x, y }, z.title(square))
            }
            UiTarget::Square(p) => (
                OrderTarget::Square(p),
                self.view().square(p).title().clone(),
            ),
        })
    }

    pub(crate) fn cmd_validate(&mut self, cx: &mut Client, flags: OrderFlags) {
        self.validate(cx, flags);
        self.order = None;
    }

    fn validate(&mut self, cx: &mut Client, flags: OrderFlags) {
        if self.group.is_empty() {
            cx.voice.item(&msg(mp::NO_UNIT_CONTROLLED));
        } else if self.order.is_none() {
            self.cmd_command_unit(cx);
        } else if self.an_order_not_requiring_a_target_is_selected() {
            let o = self.order.clone().expect("checked");
            match o.order(self.view()) {
                Some(order) => {
                    self.send_order(cx, Order::Keyword(order), flags);
                    cx.voice.item(&o.title);
                    self.previous_order = Some(o);
                }
                None => refuse_order(cx),
            }
        } else if self.an_order_requiring_a_target_is_selected() {
            let o = self.order.clone().expect("checked");
            match (o.order(self.view()), self.ui_order_target(cx)) {
                (Some(order), Some((target, title)))
                    if self.orders(cx, OrderMenu::Active).contains(&o) =>
                {
                    self.send_order(cx, Order::Keyword(order.at(target)), flags);
                    cx.voice.item(&[o.title.clone(), title].concat());
                    self.previous_order = Some(o);
                }
                // The units can no longer take the order, or no square is
                // selected yet to aim it at.
                _ => refuse_order(cx),
            }
        }
    }

    /// Says what the default order at `target`, whose title is `title`,
    /// means for each kind of unit of the group: "gather a goldmine, go to
    /// a goldmine".
    fn say_default_confirmation(&mut self, cx: &mut Client, target: OrderTarget, title: &Msg) {
        let target = match target {
            OrderTarget::Entity(t) => {
                OrderTarget::Entity(followed(&self.replaced_in_turns_waiting(), t))
            }
            t @ (OrderTarget::Square(_) | OrderTarget::Point { .. }) => t,
        };
        let mut msgs: Vec<Msg> = Vec::new();
        for (u, id) in self.group_now() {
            if !self.dobjets.contains_key(&u) {
                continue;
            }
            let keyword = self.view().default_order(id, target);
            // Every keyword the default order gives is an order without a
            // type.
            if let Some(entry) = keyword.and_then(MenuEntry::plain) {
                let o = OrderTypeView::new(&entry, id, self.view(), cx.res.style(), &cx.log);
                let m = [o.title, title.clone()].concat();
                if !msgs.contains(&m) {
                    msgs.push(m);
                }
            }
        }
        let mut confirmation = Vec::new();
        for m in msgs {
            confirmation.extend(m);
            confirmation.extend(msg(mp::COMMA));
        }
        if confirmation.is_empty() {
            cx.voice.item(&msg(mp::BEEP));
        } else {
            cx.voice.item(&confirmation);
        }
    }

    pub(crate) fn cmd_default(&mut self, cx: &mut Client, flags: OrderFlags) {
        if self.group.is_empty() {
            cx.voice.item(&msg(mp::NO_UNIT_CONTROLLED));
        } else if let Some((target, title)) = self.ui_order_target(cx) {
            self.send_order(cx, Order::Default(target), flags);
            self.say_default_confirmation(cx, target, &title);
        } else {
            // No square is selected yet to aim at.
            cx.voice.item(&msg(mp::BEEP));
        }
        self.order = None;
    }
}

/// The beep of an order that cannot be given.
fn refuse_order(cx: &Client) {
    cx.audio.play_stereo(
        cx.sounds.get_sound(BEEP_SOUND),
        StereoVolume::Both(1.0),
        0.0,
    );
}

/// The units replaced in the turn `m`, old and new, in order.
fn replacements(m: &TurnMessage) -> impl Iterator<Item = (EntityId, EntityId)> + '_ {
    m.events.iter().filter_map(|e| match *e {
        Event::Replaced { old, new } => Some((old, new)),
        _ => None,
    })
}

/// The unit `u` became through `replaced`, in order: itself if none.
fn followed(replaced: &[(EntityId, EntityId)], mut u: EntityId) -> EntityId {
    for &(old, new) in replaced {
        if u == old {
            u = new;
        }
    }
    u
}

/// Makes a command name `new` where it named `old`: the units it
/// controls, or the unit its order aims at.
fn follow_in_command(c: &mut Command, old: EntityId, new: EntityId) {
    let aimed = OrderTarget::Entity(old);
    match c {
        Command::Control(units) => {
            for u in units.iter_mut().filter(|u| **u == old) {
                *u = new;
            }
        }
        Command::Order { order, .. } => match order {
            Order::Default(t) if *t == aimed => *t = OrderTarget::Entity(new),
            Order::Keyword(o) if o.target() == Some(&aimed) => {
                *o = o.clone().at(OrderTarget::Entity(new));
            }
            Order::Default(_) | Order::Keyword(_) | Order::ResetGroup(_) | Order::JoinGroup(_) => {}
        },
        Command::Speed(_) | Command::Quit | Command::Cheat(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{cheat, enemy, go, run_until, training, unit};
    use super::*;
    use crate::game::order::entry;
    use crate::input::{Key, Mods};

    #[test]
    fn an_order_with_no_square_to_aim_at_beeps_and_is_not_given() {
        let (mut h, mut g) = training();
        h.run_until_voice_idle(&mut g);
        let me = g.game.me;
        let peasant = unit(&g, me, "peasant");
        g.game.group = vec![peasant];
        let go = g
            .game
            .orders(&h.client, OrderMenu::Active)
            .into_iter()
            .find(|o| o.keyword == Keyword::Go)
            .unwrap();
        (g.game.place, g.game.target, g.game.zoom) = (None, None, None);
        let pending = g.game.pending_commands.len();
        let (from, sounds) = (h.utterances().len(), h.audio.played().len());
        g.game.order = Some(go);
        g.game.cmd_validate(&mut h.client, OrderFlags::default());
        let played: Vec<String> = h.audio.played()[sounds..]
            .iter()
            .map(|(_, s)| s.clone())
            .collect();
        assert_eq!(played, [BEEP_SOUND]);
        assert_eq!(h.said_since(from), Vec::<String>::new());
        g.game.cmd_default(&mut h.client, OrderFlags::default());
        assert_eq!(h.said_since(from), ["<1029>"]);
        assert_eq!(g.game.pending_commands.len(), pending, "no order given");
    }

    #[test]
    fn units_upgraded_stay_selected_and_in_their_recallable_group() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let townhall = unit(&g, me, "townhall");
        let w = g.game.host().world();
        let home = w.entity(townhall).unwrap().square().unwrap();
        let home = g.game.view().square(home).name().to_owned();
        cheat(
            &mut g,
            me,
            &format!("add_units {home} lumbermill magestower 2 archer"),
        );
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        let of_type = |g: &crate::testkit::TestGame, t: &str| -> Vec<EntityId> {
            let w = g.game.host().world();
            let units = w.player(me).units.iter().copied();
            units
                .filter(|&u| w.entity(u).is_some_and(|e| &*e.type_name == t))
                .collect()
        };
        let archers = of_type(&g, "archer");
        assert_eq!(archers.len(), 2);
        g.game.group = archers.clone();
        h.key(&mut g, Key::Num6, Mods::CTRL);
        let control = format!("control {} {}", archers[0], archers[1]);
        g.game.host_mut().queue_line(me, &control);
        g.game
            .host_mut()
            .queue_line(me, "order 0 0 upgrade_to darkarcher");
        run_until(&mut h, &mut g, |game| {
            let dark = |v: &EntityView| &*v.model.type_name == "darkarcher";
            game.dobjets.values().filter(|v| dark(v)).count() == 2
        });
        let dark = of_type(&g, "darkarcher");
        // The second archer is upgraded first.
        let (first, second) = (dark[1], dark[0]);
        assert_eq!(g.game.group, [first, second]);
        g.game.group = Vec::new();
        h.run_until_voice_idle(&mut g);
        let said = h.key(&mut g, Key::Num6, Mods::NONE);
        assert_eq!(g.game.group, [first, second]);
        assert_eq!(said, [", You control 2 dark archer , ,"]);
    }

    #[test]
    fn what_names_a_unit_upgraded_names_the_unit_it_became() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let townhall = unit(&g, me, "townhall");
        let w = g.game.host().world();
        let home = w.entity(townhall).unwrap().square().unwrap();
        let home = g.game.view().square(home).name().to_owned();
        cheat(
            &mut g,
            me,
            &format!("add_units {home} lumbermill 2 scouttower"),
        );
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        let w = g.game.host().world();
        let towers = w.player(me).units.iter().copied();
        let towers: Vec<EntityId> = towers
            .filter(|&u| &*w.entity(u).unwrap().type_name == "scouttower")
            .collect();
        let (upgraded, other) = (towers[0], towers[1]);
        // The first tower is upgraded from the order menu.
        g.game.group = vec![upgraded];
        let pick = |g: &crate::testkit::TestGame, cx: &Client, menu, entry: &str| {
            let listed = g.game.orders(cx, menu).into_iter();
            let entry = Some(entry);
            listed
                .filter(|o| o.keyword == Keyword::UpgradeTo)
                .find(|o| {
                    let name = o
                        .type_id
                        .map(|t| g.game.view().unit_type_by_id(t).name.as_str());
                    name == entry
                })
                .unwrap()
        };
        let o = pick(&g, &h.client, OrderMenu::Active, "guardtower");
        g.game.select_order(&mut h.client, o, true);
        g.game.cmd_validate(&mut h.client, OrderFlags::default());
        // An order picked for it waits for its target.
        let o = pick(&g, &h.client, OrderMenu::Included, "cannontower");
        g.game.order = Some(o);
        run_until(&mut h, &mut g, |game| {
            game.dobjets
                .values()
                .any(|v| &*v.model.type_name == "guardtower")
        });
        let guardtower = unit(&g, me, "guardtower");
        assert_eq!(g.game.order.as_ref().map(|o| o.unit), Some(guardtower));
        assert_eq!(
            g.game.previous_order.as_ref().map(|o| o.unit),
            Some(guardtower)
        );
        assert_eq!(g.game.previous_group, Some(vec![guardtower]));
        // "Do again" on the other tower says what the upgrade costs: the
        // player has the lumber mill it requires.
        g.game.order = None;
        g.game.group = vec![other];
        let from = h.utterances().len();
        g.game
            .cmd_do_again(&mut h.client, false, OrderFlags::default());
        assert_eq!(
            h.said_since(from),
            ["upgrade to guard tower , requires 5 gold and 2 wood , press enter to confirm"]
        );
        g.game.cmd_validate(&mut h.client, OrderFlags::default());
        let turn = g.game.view().turn() + 1;
        h.run_until_turn(&mut g, turn);
        let w = g.game.host().world();
        let orders = w.entity(other).unwrap().orders();
        let orders: Vec<_> = orders.iter().map(|o| o.words(w)).collect();
        assert_eq!(orders, [["upgrade_to", "guardtower"]]);
    }

    /// A game on a fake server whose game, played here by `host`, has
    /// just upgraded the player's town hall `old` to the keep `new`; the
    /// turn `upgraded` that tells it is not yet sent. The player's group
    /// is the town hall.
    struct UpgradeOnAServer {
        h: crate::testkit::Harness,
        g: crate::testkit::TestGame,
        server: crate::testkit::FakeServer,
        host: soundrts_world::Host,
        before: std::sync::Arc<TurnMessage>,
        upgraded: std::sync::Arc<TurnMessage>,
        old: EntityId,
        new: EntityId,
    }

    impl UpgradeOnAServer {
        fn new() -> UpgradeOnAServer {
            use crate::clock::Clock;
            use crate::testkit::{FakeStream, Harness, Script, controller, slot, upstream_root};
            use soundrts_protocol::game::SeatFrame;
            use soundrts_protocol::{Login, SlotPlayer, Speed};
            use soundrts_world::{Cheat, GameSetup, Host, KeywordOrder, Queue, TurnChange};
            let read = |f: &str| std::fs::read_to_string(upstream_root().join(f)).unwrap();
            let mut h = Harness::new("en", Script::new().keep_open());
            let server = h.join_fake_server("andrew", &["bob"]);
            let slots = [
                slot(SlotPlayer::Human(Login::new("andrew").unwrap()), 1),
                slot(SlotPlayer::Human(Login::new("bob").unwrap()), 2),
            ];
            let stream = FakeStream::new(&mut h, "jl1", &slots, 0, 1);
            let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
            h.run_until_turn(&mut g, 0);
            let controllers = slots.iter().map(controller).collect();
            let mut setup = GameSetup::multiplayer(
                read("res/rules.txt"),
                read("res/ai.txt"),
                read("res/multi/jl1.txt"),
                1,
                controllers,
            );
            setup.allow_cheatmode = true;
            let mut host = Host::new(&setup).unwrap();
            let me = host.seats()[0];
            let add = Cheat::Trigger("add_units a1 barracks townhall".into());
            let _ = host.queue(me, Command::Cheat(add));
            let _ = host.queue(me, Command::Cheat(Cheat::Resources));
            let before = host.step(me).message;
            let change = TurnChange::between(g.game.view().message(), &before);
            server.frame_at(h.clock.now(), SeatFrame::Change(change));
            h.run_until_turn(&mut g, 1);
            let w = host.world();
            let mut units = w.player(me).units.iter().copied();
            let old = units
                .find(|&u| &*w.entity(u).unwrap().type_name == "townhall")
                .unwrap();
            let _ = host.queue(me, Command::Control(vec![old]));
            let upgrade = KeywordOrder::typed(Keyword::UpgradeTo, "keep").unwrap();
            let _ = host.queue(
                me,
                Command::Order {
                    order: Order::Keyword(upgrade),
                    queue: Queue::Replace,
                    imperative: false,
                },
            );
            let upgraded = loop {
                let m = host.step(me).message;
                if m.events.iter().any(|e| matches!(e, Event::Replaced { .. })) {
                    break m;
                }
                assert!(host.world().time < 240_000, "the upgrade did not end");
            };
            let new = replacements(&upgraded).next().unwrap().1;
            g.game.group = vec![old];
            UpgradeOnAServer {
                h,
                g,
                server,
                host,
                before,
                upgraded,
                old,
                new,
            }
        }

        /// Sends the turn of the upgrade, runs the client until it has taken
        /// it, and plays here what the client sent. The units the player
        /// controls then.
        fn send_the_upgrade_and_play(&mut self) -> Option<Vec<EntityId>> {
            use crate::clock::Clock;
            use soundrts_protocol::game::SeatFrame;
            use soundrts_world::TurnChange;
            let change = TurnChange::between(&self.before, &self.upgraded);
            let now = self.h.clock.now();
            self.server.frame_at(now, SeatFrame::Change(change));
            let turn = self.upgraded.time / 300;
            self.h.run_until_turn(&mut self.g, turn);
            self.h.step(&mut self.g);
            let me = self.g.game.me;
            for (c, _) in self.server.commands() {
                let _ = self.host.queue(me, c);
            }
            self.host.step(me);
            self.host.world().player(me).group.clone()
        }
    }

    #[test]
    fn an_order_given_before_the_interface_takes_the_turn_of_an_upgrade_goes_to_the_new_unit() {
        let mut t = UpgradeOnAServer::new();
        let (old, new) = (t.old, t.new);
        let style = t.h.client.res.style();
        let view = t.g.game.view();
        let train = OrderTypeView::new(
            &entry(view, "train peasant"),
            old,
            view,
            style,
            &t.h.client.log,
        );
        t.g.game.order = Some(train);
        // Enter, pressed when the turn has come and before it is taken.
        t.h.press(Key::Return, Mods::NONE);
        assert_eq!(t.send_the_upgrade_and_play(), Some(vec![new]));
        let w = t.host.world();
        let orders = w.entity(new).unwrap().orders();
        let orders: Vec<_> = orders.iter().map(|o| o.words(w)).collect();
        assert_eq!(orders, [["train", "peasant"]]);
        assert_eq!(t.g.game.group, [new]);
        assert_eq!(t.g.game.previous_group, Some(vec![new]));
    }

    #[test]
    fn an_order_aimed_before_the_interface_takes_the_turn_of_an_upgrade_goes_to_the_new_unit() {
        let mut t = UpgradeOnAServer::new();
        let (old, new) = (t.old, t.new);
        let style = t.h.client.res.style();
        let view = t.g.game.view();
        let rally = entry(view, "rallying_point");
        let rally = Some(OrderTypeView::new(
            &rally,
            old,
            view,
            style,
            &t.h.client.log,
        ));
        let b1 = view.square_at(1, 0).unwrap();
        t.g.game.order = rally;
        (t.g.game.place, t.g.game.target, t.g.game.zoom) = (Some(b1), None, None);
        // Enter, pressed when the turn has come and before it is taken.
        t.h.press(Key::Return, Mods::NONE);
        assert_eq!(t.send_the_upgrade_and_play(), Some(vec![new]));
        let rallying_point = t
            .host
            .world()
            .entity(new)
            .unwrap()
            .creature()
            .unwrap()
            .rallying_point;
        assert_eq!(
            rallying_point,
            Some(soundrts_world::orders::TargetRef::Square(b1))
        );
    }

    #[test]
    fn what_is_said_in_the_turn_of_an_upgrade_before_it_is_taken_asks_about_the_new_unit() {
        let mut t = UpgradeOnAServer::new();
        let (old, new) = (t.old, t.new);
        let w = t.host.world();
        let me = t.g.game.me;
        let of_type = |ty: &str| {
            let mut units = w.player(me).units.iter().copied();
            units.find(|&u| u != old && u != new && &*w.entity(u).unwrap().type_name == ty)
        };
        let (peasant, other) = (of_type("peasant").unwrap(), of_type("townhall").unwrap());
        // "Do again" an upgrade listed for the town hall, on the other one;
        // then a peasant's default order aimed at the town hall.
        let style = t.h.client.res.style();
        let view = t.g.game.view();
        let keep = entry(view, "upgrade_to keep");
        t.g.game.previous_order =
            Some(OrderTypeView::new(&keep, old, view, style, &t.h.client.log));
        t.g.game.group = vec![other];
        let from = t.h.utterances().len();
        t.h.press(Key::R, Mods::ALT);
        t.h.press(Key::Return, Mods::NONE);
        t.send_the_upgrade_and_play();
        let said = t.h.said_since(from);
        let requires = "upgrade to keep , requires 10 gold and 15 wood , press enter to confirm";
        assert_eq!(said.first().map(String::as_str), Some(requires));
        let w = t.host.world();
        let orders: Vec<_> = w
            .entity(other)
            .unwrap()
            .orders()
            .iter()
            .map(|o| o.words(w))
            .collect();
        assert_eq!(orders, [["upgrade_to", "keep"]]);
        let mut t = UpgradeOnAServer::new();
        (t.g.game.group, t.g.game.target, t.g.game.order) = (vec![peasant], Some(t.old), None);
        let from = t.h.utterances().len();
        t.h.press(Key::Backspace, Mods::NONE);
        t.send_the_upgrade_and_play();
        let said = t.h.said_since(from);
        assert_eq!(
            said.first().map(String::as_str),
            Some("go to town hall 1 ,")
        );
        let w = t.host.world();
        let orders: Vec<_> = w
            .entity(peasant)
            .unwrap()
            .orders()
            .iter()
            .map(|o| o.words(w))
            .collect();
        assert_eq!(orders, [["go", &t.new.0.to_string()]]);
    }

    #[test]
    fn a_command_waiting_to_be_sent_when_the_turn_of_an_upgrade_comes_names_the_new_unit() {
        use soundrts_protocol::MAX_TURN_ORDER_BYTES;
        use soundrts_protocol::game::encode_command;
        let mut t = UpgradeOnAServer::new();
        let (old, new) = (t.old, t.new);
        // As many commands as the server takes for a turn, then an order
        // to the town hall, which waits for the next turn.
        let filler = Command::Order {
            order: Order::JoinGroup("x".repeat(1024)),
            queue: soundrts_world::Queue::Replace,
            imperative: false,
        };
        let len = encode_command(&filler).unwrap().len();
        for _ in 0..=MAX_TURN_ORDER_BYTES / 2 / len {
            t.g.game.send_command(&t.h.client, filler.clone());
        }
        t.h.step(&mut t.g);
        let a1 = t.g.game.view().square_at(0, 0).unwrap();
        let order = Order::Default(OrderTarget::Square(a1));
        let flags = OrderFlags::default();
        t.g.game.send_order(&t.h.client, order, flags);
        t.h.step(&mut t.g);
        assert!(
            t.g.game
                .pending_commands
                .contains(&Command::Control(vec![old]))
        );
        assert_eq!(t.send_the_upgrade_and_play(), Some(vec![new]));
    }

    #[test]
    fn do_again_reads_nothing_of_a_player_the_player_does_not_control() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let peasant = unit(&g, me, "peasant");
        let townhall = unit(&g, them, "townhall");
        let theirs = unit(&g, them, "peasant");
        // The computer has a keep, which stables require; the player has
        // none.
        let w = g.game.host().world();
        let square = w.entity(townhall).unwrap().square().unwrap();
        let name = g.game.view().square(square).name().to_owned();
        g.game
            .host_mut()
            .queue_line(them, &format!("cmd add_units {name} keep"));
        let turn = g.game.view().turn() + 2;
        h.run_until_turn(&mut g, turn);
        assert!(g.game.host().world().has(them, "keep"));
        // Orders last listed for units the player now sees as the
        // computer's: one the player's peasant cannot give, one it can.
        for (u, mine) in [(townhall, unit(&g, me, "townhall")), (theirs, peasant)] {
            let mut e = g.game.view().perceived(mine).unwrap();
            (e.id, e.player, e.own) = (u, Some(them), None);
            g.game.dobjets.insert(u, EntityView::new(e));
        }
        let style = h.client.res.style();
        let listed = |order: &str, unit: EntityId| {
            let view = g.game.view();
            OrderTypeView::new(&entry(view, order), unit, view, style, &h.client.log)
        };
        let recruit = listed("train peasant", townhall);
        let stables = listed("build stables", theirs);
        g.game.group = vec![peasant];
        let mut said = |g: &mut crate::testkit::TestGame, o: OrderTypeView| {
            g.game.previous_order = Some(o);
            let from = h.utterances().len();
            g.game
                .cmd_do_again(&mut h.client, false, OrderFlags::default());
            h.said_since(from)
        };
        // The beep.
        assert_eq!(said(&mut g, recruit), ["<1029>"]);
        assert_eq!(
            said(&mut g, stables),
            [
                "build stables on , requires keep , press tab to select the target and then press enter to confirm"
            ]
        );
        assert_eq!(g.game.order.as_ref().map(|o| o.unit), Some(peasant));
    }

    #[test]
    fn shift_selects_the_idle_soldiers_of_a_type_here_and_control_all_over_the_map() {
        use crate::input::{Key, Mods};
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let square_of = |g: &crate::testkit::TestGame, u: EntityId| {
            g.game.host().world().entity(u).unwrap().square().unwrap()
        };
        let home = square_of(&g, unit(&g, me, "townhall"));
        let away = square_of(&g, unit(&g, them, "townhall"));
        let name = |g: &crate::testkit::TestGame, s| g.game.view().square(s).name().to_owned();
        let (home_name, away_name) = (name(&g, home), name(&g, away));
        cheat(
            &mut g,
            me,
            &format!("add_units {home_name} 3 footman 1 archer"),
        );
        cheat(&mut g, me, &format!("add_units {away_name} 1 footman"));
        let turn = g.game.view().turn() + 1;
        h.run_until_turn(&mut g, turn);
        let footmen: Vec<EntityId> = g.game.host().world().player(me).units.clone();
        let footmen: Vec<EntityId> = footmen
            .into_iter()
            .filter(|&u| &*g.game.host().world().entity(u).unwrap().type_name == "footman")
            .collect();
        assert_eq!(footmen.len(), 4);
        // One footman here sets off for the other side of the map.
        go(&mut g, me, footmen[2], away);
        h.run_until_turn(&mut g, turn + 2);
        assert_eq!(square_of(&g, footmen[2]), home);
        g.game.place = Some(home);
        let said = h.key(&mut g, Key::F, Mods::SHIFT);
        let mut group = g.game.group.clone();
        group.sort();
        assert_eq!(group, [footmen[0], footmen[1]]);
        assert_eq!(said[0], ", You control 2 footman , ,");
        let ctrl_shift = Mods {
            shift: true,
            ..Mods::CTRL
        };
        h.key(&mut g, Key::F, ctrl_shift);
        let mut group = g.game.group.clone();
        group.sort();
        assert_eq!(group, [footmen[0], footmen[1], footmen[3]]);
    }
}
