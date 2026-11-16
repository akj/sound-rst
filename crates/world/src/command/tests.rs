use super::*;
use crate::{AiScripts, Rules};

fn jl1() -> World {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
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

fn order(order: Order, queue: bool, imperative: bool) -> Command {
    Command::Order {
        order,
        queue: if queue { Queue::Append } else { Queue::Replace },
        imperative,
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

#[test]
fn a_line_is_read_as_its_command_or_refused() {
    let w = jl1();
    let a1 = w.square_by_name("a1").unwrap();
    let id = w.square(a1).id;
    let parse = |line: &str| Command::parse(line, &w);
    let go = |t| {
        let go = KeywordOrder::new(Keyword::Go).unwrap().at(t);
        Ok((order(Order::Keyword(go), false, false), None))
    };
    assert_eq!(
        parse(&format!("order 0 0 go {id}")),
        go(OrderTarget::Square(a1))
    );
    // A square may be named: it is the same square.
    assert_eq!(parse("order 0 0 go a1"), go(OrderTarget::Square(a1)));
    assert_eq!(
        parse("order 0 0 go 42"),
        go(OrderTarget::Entity(EntityId(42)))
    );
    // A control that cannot be read chooses no unit.
    let (c, warning) = parse("control 1-9223372036854775807").unwrap();
    assert_eq!(c, Command::Control(Vec::new()));
    assert!(warning.unwrap().contains("too many ids"));
    assert_eq!(
        parse("control 3 4294967296 5").unwrap().0,
        Command::Control(vec![EntityId(3), EntityId(5)])
    );
    assert_eq!(
        parse("cmd add_units a1  footman").unwrap().0,
        Command::Cheat(Cheat::Trigger("add_units a1 footman".into()))
    );
    let refused = [
        "".to_owned(),
        "dance".to_owned(),
        "neutral_quit".to_owned(),
        "speed fast".to_owned(),
        "order 0".to_owned(),
        "order 0 0".to_owned(),
        "order 0 0 dance".to_owned(),
        "order 0 0 train".to_owned(),
        "order 0 0 go None".to_owned(),
        "order 0 0 go 1 2".to_owned(),
        "order 0 0 default".to_owned(),
        format!("order 0 0 go zoom-{id}-1"),
        format!("order 0 0 go zoom-{id}-x-1"),
    ];
    for line in refused {
        assert!(parse(&line).is_err(), "{line:?}");
    }
    assert_eq!(
        parse("dance"),
        Err("unknown command: 'cmd_dance' (dance)".to_owned())
    );
}
