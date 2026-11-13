//! The orders a group may receive, as the order menu says them: title,
//! comment, requirements and costs, shortcut and place in the menu.

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;
use soundrts_world::{EntityId, World};

use crate::style::Style;

use super::{style_msg, style_msg_quiet};

/// The precision of the simulation's decimal values.
pub const PRECISION: i64 = 1000;

/// A count said in the feminine when it is 1 ("une ration").
pub fn nb2msg_f(n: i64) -> Msg {
    if n == 1 {
        return msg(mp::ONE_F);
    }
    nb2msg(n)
}

/// Replaces each `$1` of a title with `arg`.
pub fn substitute_args(t: Msg, arg: &Msg) -> Msg {
    let mut out = Vec::with_capacity(t.len() + arg.len());
    for part in t {
        if part == MsgPart::Text("$1".into()) {
            out.extend(arg.iter().cloned());
        } else {
            out.push(part);
        }
    }
    out
}

/// The place of an order kind in the menu: its style `index`, or the end.
pub fn ord_index(style: &Style, log: &crate::log::Log, keyword: &str) -> f64 {
    let words = style
        .get_or_warn(keyword, "index", false)
        .map(|v| match v {
            crate::style::StyleValue::Words(w) | crate::style::StyleValue::IfMe(w, _) => w.clone(),
            crate::style::StyleValue::Noise(_) => Vec::new(),
        })
        .unwrap_or_default();
    match words.first() {
        Some(w) => match soundrts_world::nofloat::py_float(w) {
            Ok(f) => f,
            Err(_) => {
                log.warning(format!(
                    "{keyword}.index should be a number or nothing (check style.txt)"
                ));
                9999.0
            }
        },
        None => 9999.0,
    }
}

/// The order kinds of the menus: those with an `index` in style, in index
/// order.
pub fn orders_list(style: &Style, log: &crate::log::Log) -> Vec<Keyword> {
    let mut list: Vec<Keyword> = Keyword::ALL
        .iter()
        .copied()
        .filter(|k| style.has(k.as_str(), "index"))
        .collect();
    list.sort_by(|a, b| {
        ord_index(style, log, a.as_str()).total_cmp(&ord_index(style, log, b.as_str()))
    });
    list
}

/// An order a unit could receive: `go`, `train footman`...
#[derive(Clone, Debug)]
pub struct OrderTypeView {
    pub keyword: Keyword,
    pub type_name: Option<String>,
    /// The unit the order was listed for.
    pub unit: EntityId,
    pub requirements: Vec<String>,
    pub title: Msg,
    pub shortcut: Option<String>,
    pub index: f64,
    pub comment: Msg,
    pub cost: Vec<i64>,
    pub food_cost: i64,
    pub nb_args: usize,
}

impl PartialEq for OrderTypeView {
    /// Two entries are the same order if they have the same keyword and
    /// type, whatever unit they were listed for.
    fn eq(&self, other: &Self) -> bool {
        self.keyword == other.keyword && self.type_name == other.type_name
    }
}

impl OrderTypeView {
    /// The menu entry `order` ("train footman") of `unit`.
    pub fn new(
        order: &str,
        unit: EntityId,
        world: &World,
        style: &Style,
        log: &crate::log::Log,
    ) -> Option<OrderTypeView> {
        let mut words = order.split_whitespace();
        let keyword = Keyword::parse(words.next()?)?;
        let type_name = words.next().map(str::to_owned);
        let requirements = type_name
            .as_deref()
            .and_then(|t| world.rules.unit_type(t))
            .map(|t| t.requirements.clone())
            .unwrap_or_default();
        let mut title = style_msg(style, keyword.as_str(), "title");
        if let Some(t) = &type_name {
            let t2 = style_msg(style, t, "title");
            title = substitute_args(title, &t2);
        }
        let shortcut = style_msg_quiet(style, keyword.as_str(), "shortcut")
            .first()
            .or(type_name
                .as_deref()
                .map(|t| style_msg_quiet(style, t, "shortcut"))
                .unwrap_or_default()
                .first())
            .map(|p| p.to_string());
        let comment = style_msg_quiet(style, keyword.as_str(), "comment");
        let info = world.order_info(unit, keyword, type_name.as_deref());
        if info.is_none() {
            log.error(format!("problem with the order {order} of {}", unit.0));
        }
        let info = info.unwrap_or(soundrts_world::orders::OrderInfo {
            cost: Vec::new(),
            food_cost: 0,
            nb_args: 0,
        });
        Some(OrderTypeView {
            keyword,
            type_name,
            unit,
            requirements,
            title,
            shortcut,
            index: ord_index(style, log, keyword.as_str()),
            comment,
            cost: info.cost,
            food_cost: info.food_cost,
            nb_args: info.nb_args,
        })
    }

    /// The order as the `order` command writes it.
    pub fn encode(&self) -> String {
        match &self.type_name {
            Some(t) => format!("{} {t}", self.keyword.as_str()),
            None => self.keyword.as_str().to_owned(),
        }
    }

    /// An ability with a long range: the target may be any square.
    pub fn target_shouldnt_collide(&self, world: &World) -> bool {
        self.keyword == Keyword::Use
            && self
                .type_name
                .as_deref()
                .and_then(|t| world.rules.unit_type(t))
                .is_some_and(|t| t.stats.effect_range > 12 * PRECISION)
    }

    /// The missing requirements, else the costs, after "requires".
    fn requirements_msg(&self, world: &World, style: &Style) -> Msg {
        let player = world.entity(self.unit).and_then(|e| e.player);
        let missing: Vec<&String> = self
            .requirements
            .iter()
            .filter(|r| !player.is_some_and(|p| world.has(p, r)))
            .collect();
        let mut and_index = 0;
        let mut m: Msg = Vec::new();
        for t in &missing {
            and_index = m.len();
            m.extend(style_msg(style, t, "title"));
        }
        if missing.is_empty() {
            for (i, &c) in self.cost.iter().enumerate() {
                if c != 0 {
                    and_index = m.len();
                    m.extend(nb2msg(c / PRECISION));
                    m.extend(style_msg(
                        style,
                        "parameters",
                        &format!("resource_{i}_title"),
                    ));
                }
            }
            if self.food_cost != 0 {
                and_index = m.len();
                m.extend(nb2msg_f(self.food_cost));
                m.extend(style_msg(style, "parameters", "food_title"));
            }
        }
        if and_index > 0 {
            let and = style_msg(style, "parameters", "and");
            m.splice(and_index..and_index, and);
        }
        if !m.is_empty() {
            let requires = style_msg(style, "parameters", "requires");
            m.splice(0..0, requires);
        }
        m
    }

    /// The comment, then the requirements or costs.
    pub fn full_comment(&self, world: &World, style: &Style) -> Msg {
        let mut m = self.comment.clone();
        m.extend(self.requirements_msg(world, style));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> MsgPart {
        MsgPart::from(s)
    }

    #[test]
    fn each_dollar_one_becomes_the_argument() {
        let title = vec![t("101"), t("$1"), t("119")];
        assert_eq!(
            substitute_args(title, &vec![MsgPart::Num(5), MsgPart::Num(6)]),
            vec![t("101"), MsgPart::Num(5), MsgPart::Num(6), t("119")]
        );
        assert_eq!(
            substitute_args(vec![t("$1"), t("$1")], &vec![t("x")]),
            vec![t("x"), t("x")]
        );
        assert_eq!(substitute_args(vec![t("$1")], &Vec::new()), Vec::new());
        assert_eq!(nb2msg_f(1), msg(mp::ONE_F));
        assert_eq!(nb2msg_f(2), nb2msg(2));
    }

    #[test]
    fn the_menu_lists_the_order_kinds_with_an_index_in_index_order() {
        let mut style = Style::new(crate::log::Log::silent());
        style.load(&["def go\nindex 1\ndef train\nindex 7\ndef stop\nindex 0.5\ndef gather\nindex 3\ndef build\nindex x\n"]);
        let list = orders_list(&style, &crate::log::Log::silent());
        assert_eq!(
            list,
            [
                Keyword::Stop,
                Keyword::Go,
                Keyword::Gather,
                Keyword::Train,
                Keyword::Build
            ]
        );
    }
}
