//! Choosing what to act on and acting: the target, the units controlled,
//! the order, and sending the order to the world.

use soundrts_world::EntityId;
use soundrts_world::msg::{Msg, MsgPart, msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;

use crate::audio::stereo;
use crate::client::Client;
use crate::sound::StereoVolume;

use super::command::{OrderFlags, OrderMenu, UnitFilter};
use super::entity::EntityView;
use super::order::OrderTypeView;
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
    fn accepts(&self, type_name: &str) -> bool {
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
                && v.model.is_a_building_land()
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
        } else if v.model.stats.is_repairable != 0 && v.hp() < v.hp_max() {
            p = 2.0;
        } else if v.model.is_a_building_land() {
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
        let p = self.world.player(self.me);
        let mut result: Vec<EntityId> = Vec::new();
        for &ap in &p.allied_control {
            for &u in &self.world.player(ap).units {
                let Some(e) = self.world.entity(u) else {
                    continue;
                };
                if e.is_inside() || !self.dobjets.contains_key(&u) {
                    continue;
                }
                let has_menu = even_if_no_menu || {
                    let t = self.world.rules.unit_type(&e.type_name);
                    t.is_some_and(|t| !t.can_train.is_empty() || !t.can_upgrade_to.is_empty())
                        || !e.orders.is_empty()
                        || !e.basic_abilities().is_empty()
                        || !self.view_menu(u, false).is_empty()
                };
                if has_menu {
                    result.push(u);
                }
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

    /// Keeps in the group the units still known and controlled.
    pub fn update_group(&mut self) {
        let p = self.world.player(self.me);
        let group = std::mem::take(&mut self.group);
        self.group = group
            .into_iter()
            .filter(|u| {
                self.dobjets.get(u).is_some_and(|v| {
                    v.player()
                        .is_some_and(|vp| vp == self.me || p.allied_control.contains(&vp))
                })
            })
            .collect();
    }

    /// The types named by `keyboard` in style, from the keys' names.
    fn arrange(&self, cx: &Client, filter: &UnitFilter) -> TypeFilter {
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
                            && (!filter.idle || v.model.orders.is_empty())
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
                    && (!filter.idle || v.model.orders.is_empty())
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
                .map(|p| self.world.square(p).title.clone())
                .unwrap_or_default(),
        };
        self.say_group(cx, &place_title)?;
        if !self.group.is_empty() {
            self.follow_mode = true;
            self.follow_if_needed(cx)?;
        }
        Ok(())
    }

    pub(crate) fn cmd_unit_hp_status(&self, cx: &mut Client) {
        if self.group.len() == 1
            && let Some(v) = self.dobjets.get(&self.group[0])
        {
            let d = self.description(cx, v);
            cx.voice.item(&d);
        }
    }

    // recallable groups

    pub(crate) fn cmd_set_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) -> R {
        self.send_order(cx, "reset_group", Some(name), flags)?;
        self.send_order(cx, "join_group", Some(name), flags)
    }

    pub(crate) fn cmd_append_group(&mut self, cx: &Client, name: &str, flags: OrderFlags) -> R {
        self.send_order(cx, "join_group", Some(name), flags)
    }

    pub(crate) fn cmd_recall_group(&mut self, cx: &mut Client, name: &str) -> R {
        self.group = self
            .world
            .groups
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, units)| units.clone())
            .unwrap_or_default();
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
                        OrderTypeView::new(&o, *u, &self.world, cx.res.style(), &cx.log)
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
        m.extend(order.full_comment(&self.world, cx.res.style()));
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
                self.shortcut_mode = true;
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

    pub(crate) fn cmd_do_again(&mut self, cx: &mut Client, now: bool, flags: OrderFlags) -> R {
        if let Some(o) = self.previous_order.clone()
            && !self.group.is_empty()
        {
            let no_target = o.nb_args == 0;
            self.select_order(cx, o, true);
            if now && no_target {
                self.cmd_validate(cx, flags)?;
            }
        }
        Ok(())
    }

    /// Sends an order to the units of the group that may receive it.
    pub(crate) fn send_order(
        &mut self,
        cx: &Client,
        order: &str,
        target: Option<&str>,
        flags: OrderFlags,
    ) -> R {
        let g: Vec<EntityId> = if order == "default" || order == "join_group" {
            self.group.clone()
        } else {
            let mut g = Vec::new();
            for uid in &self.group {
                if !self.dobjets.contains_key(uid) {
                    return Err(Oops::missing(uid.0));
                }
                if self.view_menu(*uid, false).iter().any(|o| o == order) {
                    g.push(*uid);
                }
            }
            g
        };
        if self.previous_group.as_ref() != Some(&g) {
            let ids: Vec<i64> = g.iter().map(|u| i64::from(u.0)).collect();
            let line = format!("control {}", soundrts_world::group::encode(&ids));
            self.write_line(cx, &line);
            self.previous_group = Some(g);
        }
        let order = match target {
            Some(t) => format!("{order} {t}"),
            None => order.to_owned(),
        };
        let line = format!(
            "order {} {} {order}",
            i32::from(flags.queue),
            i32::from(flags.imperative)
        );
        self.write_line(cx, &line);
        Ok(())
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

    /// The id the `order` command accepts for the target.
    fn ui_target_id(&self) -> R<String> {
        Ok(match self.ui_target() {
            UiTarget::Entity(t) => t.0.to_string(),
            UiTarget::Zoom => {
                let (Some(z), Some(p)) = (&self.zoom, self.place) else {
                    unreachable!("zoom mode needs a square")
                };
                z.id(self.world.square(p))
            }
            UiTarget::Square(p) => self.world.square(p).id.to_string(),
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
                Ok(z.title(self.world.square(p)))
            }
            UiTarget::Square(p) => Ok(self.world.square(p).title.clone()),
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
            self.send_order(cx, &o.encode(), None, flags)?;
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
                let id = self.ui_target_id()?;
                self.send_order(cx, &o.encode(), Some(&id), flags)?;
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
        let id = self.ui_target_id()?;
        let mut msgs: Vec<Msg> = Vec::new();
        for u in self.group.clone() {
            if !self.dobjets.contains_key(&u) {
                continue;
            }
            let order = self
                .world
                .get_default_order(u, &id)
                .map_err(|e| Oops::new(e.0))?;
            if let Some(order) = order {
                let o = OrderTypeView::new(order, u, &self.world, cx.res.style(), &cx.log)
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
            let id = self.ui_target_id()?;
            self.send_order(cx, "default", Some(&id), flags)?;
            self.say_default_confirmation(cx)?;
        }
        self.order = None;
        Ok(())
    }
}
