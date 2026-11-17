//! Choosing what to act on and acting: the target, the units controlled,
//! the order, and sending the order to the game.

use std::num::NonZeroU32;

use soundrts_world::msg::{Cue, Msg, MsgPart, Phrase, comma, cue, msg, nb2msg};
use soundrts_world::orders::{Keyword, MenuEntry};
use soundrts_world::rules::TypeId;
use soundrts_world::{Command, EntityId, Order, OrderTarget};

use crate::audio::stereo;
use crate::client::Client;
use crate::message::summary;
use crate::sound::StereoVolume;

use super::GameInterface;
use super::command::{OrderFlags, OrderMenu, UnitFilter};
use super::entity::EntityView;
use super::focus::Zoom;
use super::order::{OrderTypeView, PRECISION};
use super::squares::remove_duplicates;

/// What an order with a target aims at: the selected object, else the
/// point of the zoom on its square, else the selected square.
enum UiTarget<'a> {
    /// The unit by the id the game gives it now, as the interface shows it.
    Entity(EntityId, &'a EntityView),
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
        for (id, v) in self.known.entities() {
            if self.is_visible(cx, v)
                && (types.is_empty()
                    || types.iter().any(|t| t == v.type_name())
                    || types.iter().any(|t| t == "useful") && self.is_a_useful_target(cx, v))
            {
                choices.push((self.priority(cx, v), id));
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
        self.known.unit(self.target?)
    }

    /// Says the target with a beep from its direction.
    pub(crate) fn say_target(&self, cx: &mut Client) {
        let Some(v) = self.target_view() else { return };
        let mut d = self.positional_description(cx, v);
        if self.an_order_requiring_a_target_is_selected()
            && let Some(o) = &self.order
        {
            d.extend(comma());
            d.extend(o.title.iter().cloned());
        }
        let (mut l, mut r) = stereo(self.x, self.y, v.x(), v.y(), self.o, 1.0, true);
        if l.max(r) < 0.2 {
            (l, r) = (0.2, 0.2);
        }
        let m = [cue(Cue::PositionalBeep), d].concat();
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
            cx.voice.item(&msg(Phrase::Nothing));
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
            if e.is_inside() || self.known.unit(u).is_none() {
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
                let v = &self.known[*id];
                let t: Vec<usize> = self
                    .short_title(cx, v)
                    .iter()
                    .map(|p| match p {
                        // A title is the names of its style: the units
                        // come in the order the language files list them.
                        MsgPart::Name(name) => cx.sounds.rank(name),
                        _ => 0,
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
        let view = self.known.seat();
        let group = std::mem::take(&mut self.group);
        self.group = group
            .into_iter()
            .filter(|&u| {
                self.known.unit(u).is_some_and(|v| {
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
                .filter(|(_, x)| {
                    let key = x.keyboard.as_ref().and_then(|k| k.first());
                    key.is_some_and(|k| filter.keyboard_types.contains(k))
                })
                .map(|(name, _)| name.to_owned())
                .collect(),
        )
    }

    fn regroup(&mut self, cx: &mut Client, portion: NonZeroU32, filter: &UnitFilter) {
        self.update_group();
        if let Some(&first) = self.group.first() {
            let initial = &self.known[first];
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
                        .map(|&id| self.known[id].type_name().to_owned())
                        .collect::<Vec<_>>(),
                ),
            };
            let units = self.units(cx, true, false);
            let mut group = Vec::new();
            for t in &types {
                let m: Vec<EntityId> = units
                    .iter()
                    .copied()
                    .filter(|&x| {
                        let v = &self.known[x];
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
            let v = &self.known[unit];
            let m = [
                self.ext_title(cx, v),
                self.orders_txt(cx, v),
                msg(Phrase::AwaitingYourOrders),
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
            .filter(|&x| {
                let v = &self.known[x];
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
            let v = &self.known[self.group[0]];
            let m = [
                prefix.clone(),
                msg(Phrase::YouControl),
                self.ext_title(cx, v),
                self.orders_txt(cx, v),
            ]
            .concat();
            cx.voice.item(&m);
        } else if self.group.len() > 1 {
            let mut orders = Vec::new();
            let mut titles = Vec::new();
            for x in &self.group {
                if let Some(v) = self.known.unit(*x) {
                    orders.push(self.orders_txt(cx, v));
                    titles.push(self.short_title(cx, v));
                }
            }
            let mut m = [
                prefix.clone(),
                comma(),
                msg(Phrase::YouControl),
                summary(&titles, false),
            ]
            .concat();
            if remove_duplicates(&orders).len() == 1 {
                m.extend(comma());
                m.extend(orders[0].iter().cloned());
            } else if remove_duplicates(&titles).len() == 1 {
                m.extend(comma());
                m.extend(summary(&orders, false));
            }
            cx.voice.item(&m);
        } else {
            let m = [prefix.clone(), comma(), msg(Phrase::NoUnitControlled)].concat();
            cx.voice.item(&m);
        }
    }

    pub(crate) fn cmd_unit_status(&mut self, cx: &mut Client) {
        let place_title = match self.group.first().and_then(|&g| self.known.unit(g)) {
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
            && let Some(v) = self.known.unit(self.group[0])
        {
            let d = self.description(cx, v);
            cx.voice.item(&d);
        } else if self.group.len() > 1 {
            let (mut hp, mut hp_max, mut hurt) = (0, 0, 0);
            let mut titles = Vec::new();
            for v in self.group.iter().filter_map(|&u| self.known.unit(u)) {
                hp += v.model.hp;
                hp_max += v.model.hp_max;
                if v.model.hp * 2 < v.model.hp_max {
                    hurt += 1;
                }
                titles.push(self.short_title(cx, v));
            }
            let mut m = [
                nb2msg(hp / PRECISION),
                msg(Phrase::HitPointsOn),
                nb2msg(hp_max / PRECISION),
            ]
            .concat();
            if hurt > 0 {
                m.extend(comma());
                m.extend(nb2msg(hurt));
                m.extend(msg(Phrase::BadlyHurt));
            }
            m.extend(comma());
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
        for &id in &self.group {
            if self.known.unit(id).is_none() {
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
        m.extend(comma());
        m.extend(order.full_comment(self.view(), cx.res.style()));
        if help {
            m.extend(comma());
            if order.nb_args == 0 {
                m.extend(msg(Phrase::PressEnterToConfirm));
            } else {
                m.extend(msg(Phrase::SelectTargetAndConfirm));
            }
        }
        self.order = Some(order);
        cx.voice.item(&m);
    }

    pub(crate) fn cmd_select_order(&mut self, cx: &mut Client, inc: i64, menu: OrderMenu) {
        let orders = self.orders(cx, menu);
        if orders.is_empty() {
            cx.voice.item(&msg(Phrase::Nothing));
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
                    m.push(MsgPart::literal(s.as_str()));
                    m.extend(o.title.iter().cloned());
                    m.extend(comma());
                }
            }
            if !m.is_empty() {
                self.mode = super::Mode::OrderShortcut;
                cx.voice.item(&m);
                return;
            }
        }
        cx.voice.item(&cue(Cue::Beep));
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
        cx.voice.item(&cue(Cue::Beep));
    }

    /// Selects the previous order again, and with `now` gives it if it
    /// needs no target. If the unit it was listed for has since gone to a
    /// player the player does not control, the order is the group's own,
    /// as one of its units lists it; if none does, it cannot be given.
    pub(crate) fn cmd_do_again(&mut self, cx: &mut Client, now: bool, flags: OrderFlags) {
        if let Some(mut o) = self.previous_order.clone()
            && !self.group.is_empty()
        {
            let owner = self.known.unit(o.unit).and_then(EntityView::player);
            if owner.is_some_and(|p| !self.view().controls(p)) {
                let listed = self.orders(cx, OrderMenu::Included);
                match listed.into_iter().find(|x| *x == o) {
                    Some(x) => o = x,
                    None => {
                        cx.voice.item(&cue(Cue::Beep));
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

    /// Sends an order to the units of the group that may receive it.
    pub(crate) fn send_order(&mut self, cx: &Client, order: Order, flags: OrderFlags) {
        let g: Vec<EntityId> = match &order {
            Order::Default(_) | Order::JoinGroup(_) => self.group.clone(),
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
        let command = Command::Order {
            order,
            queue: flags.queue(),
            imperative: flags.imperative,
        };
        if self.previous_group.as_ref() != Some(&g) {
            self.send_command(cx, Command::Control(g.clone()));
            self.previous_group = Some(g);
        }
        self.send_command(cx, command);
    }

    /// The units of the group whose menu has the order of `keyword` and
    /// type `t`.
    fn units_offering(&self, keyword: Keyword, t: Option<TypeId>) -> Vec<EntityId> {
        self.group
            .iter()
            .copied()
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
            (Some(v), _, _) => Some(UiTarget::Entity(self.target?, v)),
            (None, Some(z), Some(p)) => Some(UiTarget::Zoom(z, p)),
            (None, None, Some(p)) => Some(UiTarget::Square(p)),
            (None, _, None) => None,
        }
    }

    /// What an order aims at, as the game takes it, and its title.
    fn ui_order_target(&self, cx: &Client) -> Option<(OrderTarget, Msg)> {
        Some(match self.ui_target()? {
            UiTarget::Entity(id, v) => (OrderTarget::Entity(id), self.title(cx, v)),
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
            cx.voice.item(&msg(Phrase::NoUnitControlled));
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
        let mut msgs: Vec<Msg> = Vec::new();
        for &id in &self.group {
            if self.known.unit(id).is_none() {
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
            confirmation.extend(comma());
        }
        if confirmation.is_empty() {
            cx.voice.item(&cue(Cue::Beep));
        } else {
            cx.voice.item(&confirmation);
        }
    }

    pub(crate) fn cmd_default(&mut self, cx: &mut Client, flags: OrderFlags) {
        if self.group.is_empty() {
            cx.voice.item(&msg(Phrase::NoUnitControlled));
        } else if let Some((target, title)) = self.ui_order_target(cx) {
            self.send_order(cx, Order::Default(target), flags);
            self.say_default_confirmation(cx, target, &title);
        } else {
            // No square is selected yet to aim at.
            cx.voice.item(&cue(Cue::Beep));
        }
        self.order = None;
    }
}

/// The beep of an order that cannot be given.
fn refuse_order(cx: &Client) {
    cx.audio
        .play_stereo(cx.sounds.cue(Cue::Beep), StereoVolume::Both(1.0), 0.0);
}

#[cfg(test)]
mod tests {
    use super::super::testing::{cheat, enemy, go, run_until, training, unit};
    use super::*;
    use crate::game::order::entry;
    use crate::input::{Key, Mods};
    use soundrts_protocol::game::SeatFrame;
    use soundrts_world::Event;

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
        assert_eq!(played, ["ui/beep"]);
        assert_eq!(h.said_since(from), Vec::<String>::new());
        g.game.cmd_default(&mut h.client, OrderFlags::default());
        assert_eq!(h.said_since(from), ["<ui/beep>"]);
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
            game.known().entities().filter(|(_, v)| dark(v)).count() == 2
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
            game.known()
                .entities()
                .any(|(_, v)| &*v.model.type_name == "guardtower")
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

    /// A game on a fake server that takes cheats, played with the keys:
    /// the player, against bob, has a second town hall on a1, a barracks
    /// and plenty of resources, and the cursor on its first town hall's
    /// square. The server plays each turn with the commands the client
    /// sent since the turn before.
    struct OnAServer {
        h: crate::testkit::Harness,
        g: crate::testkit::TestGame,
        server: crate::testkit::FakeServer,
        stream: crate::testkit::FakeStream,
        /// The commands sent that the server has played.
        played: usize,
    }

    impl OnAServer {
        fn new() -> OnAServer {
            use crate::testkit::{FakeStream, Harness, Script, slot};
            use soundrts_protocol::{Login, SlotPlayer, Speed};
            use soundrts_world::Cheat;
            let mut h = Harness::new("en", Script::new().keep_open());
            let server = h.join_fake_server("andrew", &["bob"]);
            let slots = [
                slot(SlotPlayer::Human(Login::new("andrew").unwrap()), 1),
                slot(SlotPlayer::Human(Login::new("bob").unwrap()), 2),
            ];
            let stream = FakeStream::taking_cheats(&mut h, "jl1", &slots, 0, 1);
            let g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
            let mut s = OnAServer {
                h,
                g,
                server,
                stream,
                played: 0,
            };
            s.h.run_until_turn(&mut s.g, 0);
            let add = Cheat::Trigger("add_units a1 barracks townhall".into());
            s.serve(vec![Command::Cheat(add), Command::Cheat(Cheat::Resources)]);
            s.take();
            s.h.run_until_voice_idle(&mut s.g);
            s
        }

        /// The server plays a turn with the commands the client sent
        /// since the turn before, then `more`, and sends its frames now.
        fn serve(&mut self, more: Vec<Command>) {
            let frames = self.play(more);
            self.deliver(frames);
        }

        /// The server plays a turn with the commands the client sent
        /// since the turn before, then `more`: the frames it sends for it.
        fn play(&mut self, more: Vec<Command>) -> Vec<SeatFrame> {
            let mut commands = self.sent();
            self.played = self.server.commands().len();
            commands.extend(more);
            self.stream.turn(commands)
        }

        /// The server sends `frames` now.
        fn deliver(&self, frames: Vec<SeatFrame>) {
            use crate::clock::Clock;
            let now = self.h.clock.now();
            for frame in frames {
                self.server.frame_at(now, frame);
            }
        }

        /// The client takes the turns served, and handles them.
        fn take(&mut self) {
            let turn = self.stream.message().time / 300;
            self.h.run_until_turn(&mut self.g, turn);
        }

        /// Serves turns, each taken and handled, until the server plays
        /// the turn that replaces a unit: its frames, not yet sent, and
        /// the unit replaced, old and new.
        fn play_until_replaced(&mut self) -> (Vec<SeatFrame>, EntityId, EntityId) {
            for _ in 0..1000 {
                let frames = self.play(Vec::new());
                let events = &self.stream.message().events;
                let replaced = events.iter().find_map(|e| match *e {
                    Event::Replaced { old, new } => Some((old, new)),
                    _ => None,
                });
                if let Some((old, new)) = replaced {
                    return (frames, old, new);
                }
                self.deliver(frames);
                self.take();
            }
            panic!("no unit was replaced");
        }

        /// The server sends `frames`, and `keys` are pressed: the next
        /// pass of the loop takes the turn, then handles the keys, before
        /// the end of the turn. What is said until the voice is idle.
        fn keys_when_the_turn_comes(
            &mut self,
            frames: Vec<SeatFrame>,
            keys: &[(Key, Mods)],
        ) -> Vec<String> {
            self.deliver(frames);
            let from = self.h.utterances().len();
            for &(key, mods) in keys {
                self.h.press(key, mods);
            }
            self.h.step(&mut self.g);
            self.h.run_until_voice_idle(&mut self.g);
            self.h.said_since(from)
        }

        /// Presses `key` until what it says first has `words`.
        fn key_until(&mut self, key: Key, mods: Mods, words: &str) {
            for _ in 0..30 {
                let said = self.h.key(&mut self.g, key, mods);
                if said.first().is_some_and(|s| s.contains(words)) {
                    return;
                }
            }
            panic!("no {words}");
        }

        /// Selects the town hall under the cursor, and upgrades it to a
        /// keep.
        fn upgrade_the_town_hall(&mut self) {
            self.key_until(Key::Tab, Mods::NONE, "town hall");
            self.h.key(&mut self.g, Key::Backslash, Mods::NONE);
            self.key_until(Key::A, Mods::NONE, "upgrade to keep");
            self.h.key(&mut self.g, Key::Return, Mods::NONE);
        }

        /// The commands the client sent that the server has not played.
        fn sent(&self) -> Vec<Command> {
            let sent = self.server.commands();
            sent[self.played..].iter().map(|(c, _)| c.clone()).collect()
        }

        /// The orders of the unit `u` in the server's game.
        fn orders(&self, u: EntityId) -> Vec<Vec<String>> {
            let w = self.stream.host().world();
            let orders = w.entity(u).unwrap().orders();
            orders.iter().map(|o| o.words(w)).collect()
        }

        /// The player's units of type `t` in the server's game.
        fn units_of_type(&self, t: &str) -> Vec<EntityId> {
            let w = self.stream.host().world();
            let me = self.g.game.me;
            let units = w.player(me).units.iter().copied();
            units
                .filter(|&u| &*w.entity(u).unwrap().type_name == t)
                .collect()
        }
    }

    /// Train peasant, for the town hall the player controls.
    fn train_peasant() -> Command {
        let train = soundrts_world::KeywordOrder::typed(Keyword::Train, "peasant").unwrap();
        Command::Order {
            order: Order::Keyword(train),
            queue: soundrts_world::Queue::Replace,
            imperative: false,
        }
    }

    #[test]
    fn an_order_given_when_the_turn_of_an_upgrade_comes_goes_to_the_new_unit() {
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        s.key_until(Key::A, Mods::NONE, "recruit peasant");
        let (frames, old, new) = s.play_until_replaced();
        assert_eq!(s.g.game.group, [old]);
        let said = s.keys_when_the_turn_comes(frames, &[(Key::Return, Mods::NONE)]);
        assert_eq!(said.first().map(String::as_str), Some("recruit peasant"));
        assert_eq!(s.sent(), [train_peasant()]);
        s.serve(Vec::new());
        assert_eq!(s.orders(new), [["train", "peasant"]]);
    }

    #[test]
    fn an_order_aimed_when_the_turn_of_an_upgrade_comes_goes_to_the_new_unit() {
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        let b1 = s.g.game.place.unwrap();
        s.h.key(&mut s.g, Key::Escape, Mods::NONE);
        s.key_until(Key::A, Mods::NONE, "rally to");
        let (frames, _, new) = s.play_until_replaced();
        s.keys_when_the_turn_comes(frames, &[(Key::Return, Mods::NONE)]);
        s.serve(Vec::new());
        let w = s.stream.host().world();
        let rallying_point = w.entity(new).unwrap().creature().unwrap().rallying_point;
        assert_eq!(
            rallying_point,
            Some(soundrts_world::orders::TargetRef::Square(b1))
        );
    }

    #[test]
    fn do_again_when_the_turn_of_an_upgrade_comes_says_what_the_upgrade_costs() {
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        let (frames, old, _) = s.play_until_replaced();
        let other = s.units_of_type("townhall");
        assert_eq!(other.len(), 1);
        assert_ne!(other[0], old);
        // The other town hall, by the keys.
        for _ in 0..10 {
            if s.g.game.group == other {
                break;
            }
            s.h.key(&mut s.g, Key::W, Mods::NONE);
        }
        assert_eq!(s.g.game.group, other);
        let keys = [(Key::R, Mods::ALT), (Key::Return, Mods::NONE)];
        let said = s.keys_when_the_turn_comes(frames, &keys);
        assert_eq!(
            said[..2],
            [
                "upgrade to keep , requires 10 gold and 15 wood , press enter to confirm",
                "upgrade to keep"
            ]
        );
        s.serve(Vec::new());
        assert_eq!(s.orders(other[0]), [["upgrade_to", "keep"]]);
    }

    #[test]
    fn a_default_order_at_a_unit_upgraded_when_its_turn_comes_names_the_new_unit() {
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        // A peasant, by the keys; the town hall stays the target.
        s.key_until(Key::E, Mods::NONE, "peasant");
        let peasant = s.g.game.group[0];
        let (frames, old, new) = s.play_until_replaced();
        assert_eq!(s.g.game.target, Some(old));
        let said = s.keys_when_the_turn_comes(frames, &[(Key::Backspace, Mods::NONE)]);
        assert_eq!(
            said.first().map(String::as_str),
            Some("go to town hall 1 ,")
        );
        s.serve(Vec::new());
        assert_eq!(s.orders(peasant), [["go", &new.0.to_string()]]);
    }

    #[test]
    fn a_command_waiting_to_be_sent_when_the_turn_of_an_upgrade_comes_names_the_new_unit() {
        use soundrts_protocol::MAX_TURN_ORDER_BYTES;
        use soundrts_protocol::game::encode_command;
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        // A peasant takes the place of the town hall in the group last
        // sent, so that an order to the town hall controls it again.
        s.key_until(Key::E, Mods::NONE, "peasant");
        s.h.key(&mut s.g, Key::Backspace, Mods::NONE);
        let (frames, old, new) = s.play_until_replaced();
        // As many commands as the server takes for a turn are sent before
        // the turn comes; the order to the town hall waits for the next.
        let filler = Command::Order {
            order: Order::JoinGroup("x".repeat(1024)),
            queue: soundrts_world::Queue::Replace,
            imperative: false,
        };
        let len = encode_command(&filler).unwrap().len();
        for _ in 0..=MAX_TURN_ORDER_BYTES / 2 / len {
            s.g.game.send_command(&s.h.client, filler.clone());
        }
        s.h.step(&mut s.g);
        s.h.key(&mut s.g, Key::Backslash, Mods::NONE);
        s.key_until(Key::A, Mods::NONE, "stop");
        s.h.key(&mut s.g, Key::Return, Mods::NONE);
        let stop = Command::Order {
            order: Order::Keyword(soundrts_world::KeywordOrder::new(Keyword::Stop).unwrap()),
            queue: soundrts_world::Queue::Replace,
            imperative: false,
        };
        let waiting = |s: &OnAServer| s.g.game.pending_commands()[1..].to_vec();
        assert_eq!(waiting(&s), [Command::Control(vec![old]), stop.clone()]);
        // The turn of the upgrade comes; the order goes after it.
        s.deliver(frames);
        s.h.step(&mut s.g);
        assert_eq!(waiting(&s), [Command::Control(vec![new]), stop.clone()]);
        s.take();
        let sent = s.sent();
        assert_eq!(sent[sent.len() - 2..], [Command::Control(vec![new]), stop]);
        s.serve(Vec::new());
        let w = s.stream.host().world();
        assert_eq!(w.player(s.g.game.me).group, Some(vec![new]));
    }

    #[test]
    fn a_command_naming_a_unit_as_it_is_shown_names_the_unit_it_became() {
        use std::cell::Cell;
        use std::rc::Rc;
        let mut s = OnAServer::new();
        s.upgrade_the_town_hall();
        let (frames, old, new) = s.play_until_replaced();
        // Once the turn is taken, before its end: the town hall is still
        // shown, by its old id, and a command names it so.
        let shown = Rc::new(Cell::new(None));
        let seen = Rc::clone(&shown);
        s.h.client.game.hook = Some(Rc::new(move |g, cx| {
            if seen.get().is_none() && g.view().known(new).is_some() {
                seen.set(g.known().unit(new).map(EntityView::id));
                g.send_command(cx, Command::Control(vec![old]));
            }
        }));
        s.deliver(frames);
        s.h.step(&mut s.g);
        s.h.client.game.hook = None;
        assert_eq!(shown.get(), Some(old));
        assert_eq!(s.g.game.pending_commands(), [Command::Control(vec![new])]);
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
        let world = g.game.host().world();
        assert!(world.has(them, world.rules.class_id("keep").unwrap()));
        // Orders last listed for units the player now sees as the
        // computer's: one the player's peasant cannot give, one it can.
        for (u, mine) in [(townhall, unit(&g, me, "townhall")), (theirs, peasant)] {
            let mut e = g.game.view().perceived(mine).unwrap();
            (e.id, e.player, e.own) = (u, Some(them), None);
            g.game.known.copy_mut().insert(u, EntityView::new(e));
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
        assert_eq!(said(&mut g, recruit), ["<ui/beep>"]);
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
