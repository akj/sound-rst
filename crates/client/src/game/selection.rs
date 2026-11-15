//! Choosing what to act on and acting: the target, the units controlled,
//! the order, and sending the order to the game.

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;
use soundrts_world::{Command, EntityId, Event, Order, OrderTarget, TurnMessage};

use crate::audio::stereo;
use crate::client::Client;
use crate::net::words::{self, text};
use crate::sound::StereoVolume;

use super::command::{OrderFlags, OrderMenu, UnitFilter};
use super::entity::EntityView;
use super::order::{OrderTypeView, PRECISION};
use super::squares::{remove_duplicates, summary};
use super::{BEEP_SOUND, GameInterface, Oops, R};

/// What an order with a target aims at when no object is selected.
#[derive(Clone, Debug, PartialEq)]
pub enum UiTarget {
    Entity(EntityId),
    Zoom,
    Square(soundrts_world::SquareId),
    /// No square selected yet.
    Nothing,
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

/// `x[:end]` with Python's slice end.
fn py_slice_end(len: usize, end: i64) -> usize {
    if end < 0 {
        (len as i64 + end).max(0) as usize
    } else {
        (end as usize).min(len)
    }
}

impl GameInterface {
    // target

    /// The selection order of a target: by kind, then title length, then
    /// distance.
    fn priority(&self, cx: &Client, v: &EntityView) -> R<(f64, usize, f64)> {
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
        } else if v.qty() > 0 {
            let rt = self.resource_type(&v.model).ok_or_else(|| {
                Oops::new("unsupported operand type(s) for /: 'NoneType' and 'float'")
            })?;
            p = 1.0 + rt as f64 / 100.0;
        } else if v.model.is_repairable && v.hp() < v.hp_max() {
            p = 2.0;
        } else if v.model.is_a_building_land {
            p = 3.0;
        } else if v.model.is_exit() {
            p = 4.0;
        }
        Ok((p, self.title(cx, v)?.len(), self.distance_to(v)))
    }

    fn object_choices(&self, cx: &Client, inc: i64, types: &[String]) -> R<Vec<EntityId>> {
        let mut choices = Vec::new();
        for v in self.dobjets.values() {
            if self.is_visible(cx, v)?
                && (types.is_empty()
                    || types.iter().any(|t| t == v.type_name())
                    || types.iter().any(|t| t == "useful") && self.is_a_useful_target(cx, v))
            {
                choices.push((self.priority(cx, v)?, v.id()));
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
        Ok(ids)
    }

    /// Says the target with a beep from its direction.
    pub(crate) fn say_target(&self, cx: &mut Client) -> R {
        let Some(t) = self.target else { return Ok(()) };
        let v = self
            .dobjets
            .get(&t)
            .ok_or_else(|| Oops::missing("target"))?;
        let mut d = self.positional_description(cx, v)?;
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
        Ok(())
    }

    pub(crate) fn cmd_examine(&self, cx: &mut Client) -> R {
        if self.target.is_some() {
            self.say_target(cx)
        } else if self.zoom.is_some() {
            self.say_zoom(cx, &Vec::new());
            Ok(())
        } else {
            self.say_square(cx, self.place, &Vec::new());
            Ok(())
        }
    }

    pub(crate) fn cmd_select_target(&mut self, cx: &mut Client, inc: i64, types: &[String]) -> R {
        let choices = self.object_choices(cx, inc, types)?;
        if choices.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            self.target = None;
        } else {
            let sel = self
                .target
                .and_then(|t| choices.iter().position(|&c| c == t))
                .map_or(0, |i| i + 1);
            self.target = Some(choices[if sel >= choices.len() { 0 } else { sel }]);
            self.say_target(cx)?;
        }
        Ok(())
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
    /// its place in the group, and as the selected object if the turn's
    /// message `m` knows it. Done before the turn's entities are taken,
    /// which keep the selected object only if it is still known.
    pub(super) fn follow_replaced_units(&mut self, m: &TurnMessage) {
        for e in &m.events {
            let &Event::Replaced { old, new } = e else {
                continue;
            };
            for u in self.group.iter_mut().filter(|u| **u == old) {
                *u = new;
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

    fn regroup(&mut self, cx: &mut Client, portion: i64, filter: &UnitFilter) -> R {
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
            if portion == 0 {
                return Err(Oops::new("integer division or modulo by zero"));
            }
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
                let end = soundrts_world::nofloat::floordiv(m.len() as i64, portion);
                group.extend_from_slice(&m[..py_slice_end(m.len(), end)]);
            }
            if !group.contains(&first) && types.contains(&initial_type) {
                group.pop();
                group.push(first);
            }
            self.group = group;
        }
        self.say_group(cx, &Vec::new())
    }

    pub(crate) fn cmd_group(&mut self, cx: &mut Client, portion: i64, filter: &UnitFilter) -> R {
        self.regroup(cx, portion, filter)
    }

    pub(crate) fn cmd_ungroup(&mut self, cx: &mut Client) -> R {
        if self.group.len() > 1 {
            self.group.truncate(1);
        }
        self.say_group(cx, &Vec::new())
    }

    fn command_unit(&mut self, cx: &mut Client, unit: EntityId, silent: bool) -> R {
        if !silent {
            let v = &self.dobjets[&unit];
            let m = [
                self.ext_title(cx, v)?,
                self.orders_txt(cx, v)?,
                msg(mp::AWAITING_YOUR_ORDERS),
            ]
            .concat();
            cx.voice.item(&m);
        }
        self.group = vec![unit];
        Ok(())
    }

    pub(crate) fn cmd_command_unit(&mut self, cx: &mut Client) -> R {
        if let Some(t) = self.target
            && self.units(cx, true, false).contains(&t)
        {
            self.command_unit(cx, t, false)?;
        }
        Ok(())
    }

    fn select_unit(&mut self, cx: &mut Client, inc: i64, filter: &UnitFilter, silent: bool) -> R {
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
            return Ok(());
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
        self.command_unit(cx, units[sel as usize], silent)?;
        self.order = None;
        Ok(())
    }

    pub(crate) fn cmd_select_unit(&mut self, cx: &mut Client, inc: i64, filter: &UnitFilter) -> R {
        self.select_unit(cx, inc, filter, false)
    }

    pub(crate) fn cmd_select_units(&mut self, cx: &mut Client, filter: &UnitFilter) -> R {
        self.select_unit(cx, 1, filter, true)?;
        self.regroup(cx, 1, filter)
    }

    /// Says the units controlled and their orders, after `prefix`.
    pub(crate) fn say_group(&mut self, cx: &mut Client, prefix: &Msg) -> R {
        self.update_group();
        if self.group.len() == 1 {
            let v = &self.dobjets[&self.group[0]];
            let m = [
                prefix.clone(),
                msg(mp::YOU_CONTROL),
                self.ext_title(cx, v)?,
                self.orders_txt(cx, v)?,
            ]
            .concat();
            cx.voice.item(&m);
        } else if self.group.len() > 1 {
            let mut orders = Vec::new();
            let mut titles = Vec::new();
            for x in &self.group {
                if let Some(v) = self.dobjets.get(x) {
                    orders.push(self.orders_txt(cx, v)?);
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
        Ok(())
    }

    pub(crate) fn cmd_unit_status(&mut self, cx: &mut Client) -> R {
        let place_title = match self.group.first() {
            Some(g) => {
                let v = self.dobjets.get(g).ok_or_else(|| Oops::missing("unit"))?;
                let p = v
                    .place()
                    .ok_or_else(|| Oops::new("'NoneType' object has no attribute 'title'"))?;
                self.place_title(cx, p)
            }
            None => self
                .place
                .map(|p| self.view().square(p).title().clone())
                .unwrap_or_default(),
        };
        self.say_group(cx, &place_title)?;
        if !self.group.is_empty() {
            self.follow_mode = true;
            self.follow_if_needed(cx)?;
        }
        Ok(())
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

    pub(crate) fn cmd_set_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) -> R {
        self.send_order(cx, Order::ResetGroup(name.to_owned()), flags)?;
        self.send_order(cx, Order::JoinGroup(name.to_owned()), flags)
    }

    pub(crate) fn cmd_append_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) -> R {
        self.send_order(cx, Order::JoinGroup(name.to_owned()), flags)
    }

    pub(crate) fn cmd_recall_group(&mut self, cx: &mut Client, name: &str) -> R {
        self.group = self.view().recallable_group(name);
        self.say_group(cx, &Vec::new())
    }

    /// The orders of the group's units, in menu order.
    pub fn orders(&self, cx: &Client, menu: OrderMenu) -> Vec<OrderTypeView> {
        let mut result: Vec<OrderTypeView> = Vec::new();
        let mut done: Vec<String> = Vec::new();
        for u in &self.group {
            if !self.dobjets.contains_key(u) {
                continue;
            }
            let entries = match menu {
                OrderMenu::Active => self.view_menu(*u, true),
                OrderMenu::Included => self.view_menu(*u, false),
                OrderMenu::InactiveOnly => {
                    let strict = self.view_menu(*u, true);
                    self.view_menu(*u, false)
                        .into_iter()
                        .filter(|o| !strict.contains(o))
                        .collect()
                }
            };
            for o in entries {
                if !done.contains(&o) {
                    if let Some(v) =
                        OrderTypeView::new(&o, *u, self.view(), cx.res.style(), &cx.log)
                    {
                        result.push(v);
                    }
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
    pub(crate) fn execute_order_shortcut(
        &mut self,
        cx: &mut Client,
        e: &crate::input::KeyDown,
    ) -> R {
        for o in self.orders(cx, OrderMenu::Active) {
            if o.shortcut.as_deref() == Some(e.unicode.as_str()) {
                let no_target = o.nb_args == 0;
                self.select_order(cx, o, true);
                if no_target {
                    self.cmd_validate(cx, OrderFlags::default())?;
                }
                return Ok(());
            }
        }
        cx.voice.item(&msg(mp::BEEP));
        Ok(())
    }

    /// Selects the previous order again, and with `now` gives it if it
    /// needs no target. If the unit it was listed for has since gone to a
    /// player the player does not control, the order is the group's own,
    /// as one of its units lists it; if none does, it cannot be given.
    pub(crate) fn cmd_do_again(&mut self, cx: &mut Client, now: bool, flags: OrderFlags) -> R {
        if let Some(mut o) = self.previous_order.clone()
            && !self.group.is_empty()
        {
            let owner = self.dobjets.get(&o.unit).and_then(EntityView::player);
            if owner.is_some_and(|p| !self.view().controls(p)) {
                let listed = self.orders(cx, OrderMenu::Included);
                match listed.into_iter().find(|x| *x == o) {
                    Some(x) => o = x,
                    None => {
                        cx.voice.item(&msg(mp::BEEP));
                        return Ok(());
                    }
                }
            }
            let no_target = o.nb_args == 0;
            self.select_order(cx, o, true);
            if now && no_target {
                self.cmd_validate(cx, flags)?;
            }
        }
        Ok(())
    }

    /// Sends an order to the units of the group that may receive it.
    pub(crate) fn send_order(&mut self, cx: &Client, order: Order, flags: OrderFlags) -> R {
        let g: Vec<EntityId> = match &order {
            Order::Default(_) | Order::JoinGroup(_) => self.group.clone(),
            // In no unit's menu: no unit receives it, as in the original.
            Order::ResetGroup(_) => self.units_offering(None)?,
            Order::Keyword(o) => {
                // As the world's menus list it: `train footman`.
                let entry = match o.type_name() {
                    Some(t) => format!("{} {t}", o.keyword().as_str()),
                    None => o.keyword().as_str().to_owned(),
                };
                self.units_offering(Some(&entry))?
            }
        };
        if self.previous_group.as_ref() != Some(&g) {
            self.send_command(cx, Command::Control(g.clone()));
            self.previous_group = Some(g);
        }
        self.send_command(
            cx,
            Command::Order {
                order,
                queue: flags.queue(),
                imperative: flags.imperative,
            },
        );
        Ok(())
    }

    /// The units of the group whose menu has the order `entry`; none for no
    /// entry. Every unit of the group must be known.
    fn units_offering(&self, entry: Option<&str>) -> R<Vec<EntityId>> {
        let mut g = Vec::new();
        for uid in &self.group {
            if !self.dobjets.contains_key(uid) {
                return Err(Oops::missing(uid.0));
            }
            if entry.is_some_and(|e| self.view_menu(*uid, false).iter().any(|o| o == e)) {
                g.push(*uid);
            }
        }
        Ok(g)
    }

    /// What an order aims at: the target, else the zoom, else the square.
    pub fn ui_target(&self) -> UiTarget {
        match (self.target, &self.zoom, self.place) {
            (Some(t), _, _) => UiTarget::Entity(t),
            (None, Some(_), _) => UiTarget::Zoom,
            (None, None, Some(p)) => UiTarget::Square(p),
            (None, None, None) => UiTarget::Nothing,
        }
    }

    /// What an order aims at, as the game takes it.
    fn ui_order_target(&self) -> R<OrderTarget> {
        Ok(match self.ui_target() {
            UiTarget::Entity(t) => OrderTarget::Entity(t),
            UiTarget::Zoom => {
                let (Some(z), Some(p)) = (&self.zoom, self.place) else {
                    unreachable!("zoom mode needs a square")
                };
                let (x, y) = z.point(self.view().square(p));
                OrderTarget::Point { square: p, x, y }
            }
            UiTarget::Square(p) => OrderTarget::Square(p),
            UiTarget::Nothing => {
                return Err(Oops::new("'NoneType' object has no attribute 'id'"));
            }
        })
    }

    fn ui_target_title(&self, cx: &Client) -> R<Msg> {
        match self.ui_target() {
            UiTarget::Entity(t) => {
                let v = self
                    .dobjets
                    .get(&t)
                    .ok_or_else(|| Oops::missing("target"))?;
                self.title(cx, v)
            }
            UiTarget::Zoom => {
                let (Some(z), Some(p)) = (&self.zoom, self.place) else {
                    unreachable!("zoom mode needs a square")
                };
                Ok(z.title(self.view().square(p)))
            }
            UiTarget::Square(p) => Ok(self.view().square(p).title().clone()),
            UiTarget::Nothing => Err(Oops::new("'NoneType' object has no attribute 'title'")),
        }
    }

    pub(crate) fn cmd_validate(&mut self, cx: &mut Client, flags: OrderFlags) -> R {
        self.validate(cx, flags)?;
        self.order = None;
        Ok(())
    }

    fn validate(&mut self, cx: &mut Client, flags: OrderFlags) -> R {
        if self.group.is_empty() {
            cx.voice.item(&msg(mp::NO_UNIT_CONTROLLED));
        } else if self.order.is_none() {
            self.cmd_command_unit(cx)?;
        } else if self.an_order_not_requiring_a_target_is_selected() {
            let o = self.order.clone().expect("checked");
            self.send_order(cx, Order::Keyword(o.order()?), flags)?;
            cx.voice.item(&o.title);
            self.previous_order = Some(o);
        } else if self.an_order_requiring_a_target_is_selected() {
            let o = self.order.clone().expect("checked");
            if !self.orders(cx, OrderMenu::Active).contains(&o) {
                cx.audio.play_stereo(
                    cx.sounds.get_sound(BEEP_SOUND),
                    StereoVolume::Both(1.0),
                    0.0,
                );
            } else {
                let target = self.ui_order_target()?;
                self.send_order(cx, Order::Keyword(o.order()?.at(target)), flags)?;
                let m = [o.title.clone(), self.ui_target_title(cx)?].concat();
                cx.voice.item(&m);
                self.previous_order = Some(o);
            }
        }
        Ok(())
    }

    /// Says what the default order means for each kind of unit of the
    /// group: "gather a goldmine, go to a goldmine".
    fn say_default_confirmation(&mut self, cx: &mut Client) -> R {
        let target = self.ui_order_target()?;
        let mut msgs: Vec<Msg> = Vec::new();
        for u in self.group.clone() {
            if !self.dobjets.contains_key(&u) {
                continue;
            }
            let keyword = self
                .view()
                .default_order(u, target)
                .map_err(|e| Oops::new(e.0))?;
            if let Some(keyword) = keyword {
                let order = keyword.as_str();
                let o = OrderTypeView::new(order, u, self.view(), cx.res.style(), &cx.log)
                    .ok_or_else(|| Oops::missing(order))?;
                let m = [o.title, self.ui_target_title(cx)?].concat();
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
        Ok(())
    }

    pub(crate) fn cmd_default(&mut self, cx: &mut Client, flags: OrderFlags) -> R {
        if self.group.is_empty() {
            cx.voice.item(&msg(mp::NO_UNIT_CONTROLLED));
        } else {
            let target = self.ui_order_target()?;
            self.send_order(cx, Order::Default(target), flags)?;
            self.say_default_confirmation(cx)?;
        }
        self.order = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{cheat, enemy, go, run_until, training, unit};
    use super::*;
    use crate::input::{Key, Mods};

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
            g.game.dobjets.insert(u, EntityView::new(e, &h.client.rng));
        }
        let style = h.client.res.style();
        let listed = |order: &str, unit: EntityId| {
            OrderTypeView::new(order, unit, g.game.view(), style, &h.client.log).unwrap()
        };
        let recruit = listed("train peasant", townhall);
        let stables = listed("build stables", theirs);
        g.game.group = vec![peasant];
        let mut said = |g: &mut crate::testkit::TestGame, o: OrderTypeView| {
            g.game.previous_order = Some(o);
            let from = h.utterances().len();
            g.game
                .cmd_do_again(&mut h.client, false, OrderFlags::default())
                .unwrap();
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
