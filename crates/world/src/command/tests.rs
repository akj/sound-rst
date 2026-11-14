use super::*;
use crate::{AiScripts, Rules};

fn jl1() -> World {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    let read = |f: &str| {
        let bytes = std::fs::read(dir.join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let rules = Rules::load(&[read("res/rules.txt")]);
    let ai = AiScripts::load(&[read("res/ai.txt")]);
    let mut w = World::new(rules, ai, Vec::new(), 0);
    w.load_and_build_map(&read("res/multi/jl1.txt")).unwrap();
    w
}

// The interface's code that wrote each line before commands were typed:
// `send_order`, `ui_target_id` with `Zoom::id`, and `OrderTypeView::encode`.

fn old_target(world: &World, t: OrderTarget) -> String {
    match t {
        OrderTarget::Entity(t) => t.0.to_string(),
        OrderTarget::Point { square, x, y } => {
            format!("zoom-{}-{}-{}", world.square(square).id, x, y)
        }
        OrderTarget::Square(p) => world.square(p).id.to_string(),
    }
}

fn old_order(order: &str, target: Option<&str>, queue: bool, imperative: bool) -> String {
    let order = match target {
        Some(t) => format!("{order} {t}"),
        None => order.to_owned(),
    };
    format!(
        "order {} {} {order}",
        i32::from(queue),
        i32::from(imperative)
    )
}

fn order(order: Order, queue: bool, imperative: bool) -> Command {
    Command::Order {
        order,
        queue: if queue { Queue::Append } else { Queue::Replace },
        imperative,
    }
}

fn targets(w: &World) -> Vec<OrderTarget> {
    let a1 = w.square_by_name("a1").unwrap();
    let c4 = w.square_by_name("c4").unwrap();
    vec![
        OrderTarget::Square(a1),
        OrderTarget::Square(c4),
        OrderTarget::Point {
            square: a1,
            x: 2000,
            y: 10000,
        },
        OrderTarget::Point {
            square: c4,
            x: -1,
            y: 0,
        },
        OrderTarget::Entity(EntityId(7)),
        OrderTarget::Entity(EntityId(123456)),
    ]
}

const FLAGS: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

#[test]
fn every_keyword_order_has_the_line_the_interface_wrote() {
    let w = jl1();
    let mut aims: Vec<Option<OrderTarget>> = vec![None];
    aims.extend(targets(&w).into_iter().map(Some));
    for &keyword in Keyword::ALL {
        let (o, encoded) = if keyword.is_complex() {
            (
                KeywordOrder::typed(keyword, "footman").unwrap(),
                format!("{} footman", keyword.as_str()),
            )
        } else {
            (
                KeywordOrder::new(keyword).unwrap(),
                keyword.as_str().to_owned(),
            )
        };
        for (queue, imperative) in FLAGS {
            for aim in &aims {
                let id = aim.map(|t| old_target(&w, t));
                let o = match aim {
                    Some(t) => o.clone().at(*t),
                    None => o.clone(),
                };
                assert_eq!(
                    order(Order::Keyword(o), queue, imperative).line(&w),
                    Some(old_order(&encoded, id.as_deref(), queue, imperative))
                );
            }
        }
    }
}

#[test]
fn order_lines_spelled_out() {
    let w = jl1();
    let a1 = w.square_by_name("a1").unwrap();
    let id = w.square(a1).id;
    let line = |c: Command| c.line(&w).unwrap();
    let go = KeywordOrder::new(Keyword::Go).unwrap();
    let to_a1 = Order::Keyword(go.clone().at(OrderTarget::Square(a1)));
    assert_eq!(
        line(order(to_a1, false, false)),
        format!("order 0 0 go {id}")
    );
    let to_42 = Order::Keyword(go.at(OrderTarget::Entity(EntityId(42))));
    assert_eq!(line(order(to_42, true, true)), "order 1 1 go 42");
    let farm = KeywordOrder::typed(Keyword::Build, "farm").unwrap();
    let point = OrderTarget::Point {
        square: a1,
        x: 2000,
        y: 10000,
    };
    assert_eq!(
        line(order(Order::Keyword(farm.at(point)), true, false)),
        format!("order 1 0 build farm zoom-{id}-2000-10000")
    );
    let train = KeywordOrder::typed(Keyword::Train, "peasant").unwrap();
    assert_eq!(
        line(order(Order::Keyword(train), false, false)),
        "order 0 0 train peasant"
    );
    let stop = KeywordOrder::new(Keyword::Stop).unwrap();
    assert_eq!(
        line(order(Order::Keyword(stop), false, true)),
        "order 0 1 stop"
    );
}

#[test]
fn default_and_group_orders_have_the_lines_the_interface_wrote() {
    let w = jl1();
    for (queue, imperative) in FLAGS {
        for t in targets(&w) {
            let old = old_order("default", Some(&old_target(&w, t)), queue, imperative);
            assert_eq!(
                order(Order::Default(t), queue, imperative).line(&w),
                Some(old)
            );
        }
        for name in ["1", "9", "army"] {
            assert_eq!(
                order(Order::ResetGroup(name.into()), queue, imperative).line(&w),
                Some(old_order("reset_group", Some(name), queue, imperative))
            );
            assert_eq!(
                order(Order::JoinGroup(name.into()), queue, imperative).line(&w),
                Some(old_order("join_group", Some(name), queue, imperative))
            );
        }
    }
    let a1 = w.square_by_name("a1").unwrap();
    assert_eq!(
        order(Order::Default(OrderTarget::Square(a1)), false, false).line(&w),
        Some(format!("order 0 0 default {}", w.square(a1).id))
    );
    assert_eq!(
        order(Order::ResetGroup("3".into()), true, false).line(&w),
        Some("order 1 0 reset_group 3".into())
    );
    assert_eq!(
        order(Order::JoinGroup("3".into()), false, false).line(&w),
        Some("order 0 0 join_group 3".into())
    );
}

#[test]
fn control_lines_join_runs_of_ids() {
    let w = jl1();
    let control = |ids: &[u32]| Command::Control(ids.iter().map(|&i| EntityId(i)).collect());
    assert_eq!(control(&[]).line(&w), Some("control ".into()));
    assert_eq!(control(&[5]).line(&w), Some("control 5".into()));
    assert_eq!(
        control(&[1, 2, 3, 7, 9, 10]).line(&w),
        Some("control 1-3 7 9-10".into())
    );
    assert_eq!(control(&[8, 2, 3]).line(&w), Some("control 8 2-3".into()));
}

#[test]
fn the_game_menu_and_console_lines() {
    let w = jl1();
    let line = |c: Command| c.line(&w);
    // The game menu wrote its speeds as these texts.
    for (s, text) in [(0.5, "0.5"), (1.0, "1.0"), (2.0, "2.0"), (4.0, "4.0")] {
        assert_eq!(line(Command::Speed(s)), Some(format!("speed {text}")));
    }
    assert_eq!(line(Command::Quit), Some("quit".into()));
    assert_eq!(
        line(Command::Cheat(Cheat::ToggleMode)),
        Some("toggle_cheatmode".into())
    );
    assert_eq!(
        line(Command::Cheat(Cheat::Victory)),
        Some("cmd victory".into())
    );
    let a1 = w.square_by_name("a1").unwrap();
    let add = |square, units: &str| {
        line(Command::Cheat(Cheat::AddUnits {
            square,
            units: units.into(),
        }))
    };
    assert_eq!(
        add(Some(a1), "8 peasant"),
        Some("cmd add_units a1 8 peasant".into())
    );
    // No square selected: the console wrote an empty name.
    assert_eq!(add(None, "priest"), Some("cmd add_units  priest".into()));
    assert_eq!(
        line(Command::Cheat(Cheat::Trigger(
            "add_units b2 3 footman".into()
        ))),
        Some("cmd add_units b2 3 footman".into())
    );
    for c in [
        Cheat::Resources,
        Cheat::AllTechs,
        Cheat::InfiniteMana,
        Cheat::PlayAs(PlayerId(1)),
    ] {
        assert_eq!(line(Command::Cheat(c)), None);
    }
}

#[test]
fn a_keyword_order_has_a_type_exactly_when_its_keyword_needs_one() {
    for &k in Keyword::ALL {
        assert_eq!(KeywordOrder::new(k).is_some(), !k.is_complex(), "{k:?}");
        assert_eq!(
            KeywordOrder::typed(k, "footman").is_some(),
            k.is_complex(),
            "{k:?}"
        );
    }
    assert_eq!(KeywordOrder::typed(Keyword::Train, ""), None);
    assert_eq!(KeywordOrder::typed(Keyword::Train, "foot man"), None);
    let o = KeywordOrder::typed(Keyword::Use, "a_holy_vision")
        .unwrap()
        .at(OrderTarget::Entity(EntityId(3)));
    assert_eq!(o.keyword(), Keyword::Use);
    assert_eq!(o.type_name(), Some("a_holy_vision"));
    assert_eq!(o.target(), Some(&OrderTarget::Entity(EntityId(3))));
}
