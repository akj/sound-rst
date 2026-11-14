use super::*;
use crate::entity::Place;
use crate::player::Controller;
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

/// jl1 with a human, `me`, against an easy computer, cheats allowed.
fn game() -> (World, PlayerId, PlayerId) {
    let mut w = jl1();
    w.allow_cheatmode = true;
    let players = w
        .populate_map(
            &[Controller::human("me", true), Controller::computer("easy")],
            false,
            true,
        )
        .unwrap();
    (w, players[0], players[1])
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

// A typed command and the line of its text play the same game.

/// The square of a player's first unit.
fn home(w: &World, p: PlayerId) -> SquareId {
    match w.e(w.player(p).units[0]).place {
        Some(Place::Square(s)) => s,
        _ => panic!("the first unit is on a square"),
    }
}

/// What `me` may aim at: squares, a point, a deposit, an enemy, an id
/// that names nothing, and the player's own units.
fn aims(w: &World, me: PlayerId, you: PlayerId) -> Vec<OrderTarget> {
    let home = home(w, me);
    let h = w.square(home);
    let deposit = w
        .squares
        .iter()
        .flat_map(|s| s.objects.iter().copied())
        .find(|&o| w.e(o).is_deposit())
        .unwrap();
    let mut aims = vec![
        OrderTarget::Square(home),
        OrderTarget::Square(w.square_by_name("b2").unwrap()),
        OrderTarget::Point {
            square: home,
            x: h.x + (h.xmax - h.xmin) / 3,
            y: h.y,
        },
        OrderTarget::Entity(deposit),
        OrderTarget::Entity(w.player(you).units[0]),
        OrderTarget::Entity(EntityId(999_999)),
    ];
    aims.extend(w.player(me).units.iter().map(|&u| OrderTarget::Entity(u)));
    aims
}

/// A type for each keyword that takes one.
fn type_for(k: Keyword) -> &'static str {
    match k {
        Keyword::Train => "peasant",
        Keyword::Research => "melee_weapon",
        Keyword::UpgradeTo => "knight",
        Keyword::Build => "farm",
        _ => "a_meteors",
    }
}

/// Gives `commands` to `a` as values and to `b` as the lines of their
/// text, then plays `turns` turns, checking that both play the same game.
fn play(a: &mut World, b: &mut World, me: PlayerId, commands: &[Command], turns: usize) {
    let mut lines = Vec::new();
    for c in commands {
        let line = c.line(a).unwrap();
        let parsed = Command::parse(&line, b).unwrap();
        match c {
            Command::Cheat(Cheat::Victory | Cheat::AddUnits { .. } | Cheat::Trigger(_)) => {
                // The words after `cmd` are the map language, read as such.
                let words = line.strip_prefix("cmd ").unwrap();
                let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
                assert_eq!(parsed, (Command::Cheat(Cheat::Trigger(words)), None));
            }
            // A `join_group` with a target writes the target as the name of
            // the group, as `Order::JoinGroup` does.
            Command::Order {
                order: Order::Keyword(o),
                queue,
                imperative,
            } if o.keyword() == Keyword::JoinGroup && o.target().is_some() => {
                let name = o.target().unwrap().text(a);
                let joined = order(Order::JoinGroup(name), *queue == Queue::Append, *imperative);
                assert_eq!(parsed, (joined, None), "{line}");
            }
            _ => assert_eq!(parsed, (c.clone(), None), "{line}"),
        }
        a.queue(me, c.clone());
        b.queue_command(me, &line);
        lines.push(line);
    }
    for _ in 0..turns {
        a.update();
        b.update();
        assert_eq!(a.digest_u64(), b.digest_u64(), "{lines:?}");
    }
}

#[test]
fn every_command_and_the_line_of_its_text_play_the_same_game() {
    let (mut a, me, you) = game();
    let (mut b, _, _) = game();
    assert_eq!(a.digest_u64(), b.digest_u64());
    let before = a.player(me).units.len();
    let home = home(&a, me);
    play(
        &mut a,
        &mut b,
        me,
        &[
            Command::Cheat(Cheat::ToggleMode),
            Command::Cheat(Cheat::AddUnits {
                square: Some(home),
                units: "footman knight mage necromancer priest boat blacksmith".into(),
            }),
            Command::Cheat(Cheat::AddUnits {
                square: None,
                units: "archer".into(),
            }),
            Command::Cheat(Cheat::Trigger("add_units b2 2 footman".into())),
            Command::Speed(2.0),
        ],
        2,
    );
    assert_eq!(a.player(me).units.len(), before + 10);
    let flags = [(Queue::Replace, false), (Queue::Append, true)];
    for &keyword in Keyword::ALL {
        let o = if keyword.is_complex() {
            KeywordOrder::typed(keyword, type_for(keyword)).unwrap()
        } else {
            KeywordOrder::new(keyword).unwrap()
        };
        for (queue, imperative) in flags {
            let mut orders = vec![Order::Keyword(o.clone())];
            orders.extend(
                aims(&a, me, you)
                    .into_iter()
                    .map(|t| Order::Keyword(o.clone().at(t))),
            );
            for order in orders {
                let control = Command::Control(a.player(me).units.clone());
                let order = Command::Order {
                    order,
                    queue,
                    imperative,
                };
                play(&mut a, &mut b, me, &[control, order], 2);
            }
        }
    }
    for (queue, imperative) in flags {
        let mut orders: Vec<Order> = aims(&a, me, you).into_iter().map(Order::Default).collect();
        orders.extend([
            Order::ResetGroup("1".into()),
            Order::JoinGroup("1".into()),
            Order::JoinGroup("2".into()),
            Order::ResetGroup("2".into()),
        ]);
        for order in orders {
            let control = Command::Control(a.player(me).units.clone());
            let order = Command::Order {
                order,
                queue,
                imperative,
            };
            play(&mut a, &mut b, me, &[control, order], 2);
        }
    }
    play(&mut a, &mut b, me, &[Command::Cheat(Cheat::Victory)], 2);
    play(&mut a, &mut b, me, &[Command::Quit], 3);
    assert!(!a.players.contains(&me));
}

#[test]
fn a_typed_order_and_its_line_store_the_same_order() {
    let (mut a, me, you) = game();
    let (mut b, _, _) = game();
    let peasant = a
        .player(me)
        .units
        .iter()
        .copied()
        .find(|&u| &*a.e(u).type_name == "peasant")
        .unwrap();
    let home = home(&a, me);
    // An enemy the player sees.
    let mut enemies = Vec::new();
    for w in [&mut a, &mut b] {
        let footman = w.rules.type_id("footman").unwrap();
        let (x, y) = (w.square(home).x, w.square(home).y);
        let e = w.create_creature(footman, Some(you), Place::Square(home), x, y);
        enemies.push(e.unwrap());
        w.update();
    }
    assert_eq!(enemies[0], enemies[1]);
    let enemy = enemies[0];
    let h = a.square(home);
    let (x, y, id) = (h.x + (h.xmax - h.xmin) / 3, h.y, h.id);
    let b2 = a.square_by_name("b2").unwrap();
    let b2_id = a.square(b2).id;
    let go = || KeywordOrder::new(Keyword::Go).unwrap();
    let farm = KeywordOrder::typed(Keyword::Build, "farm").unwrap();
    let cases = [
        (
            go().at(OrderTarget::Square(b2)),
            false,
            (Keyword::Go, None, vec![b2_id.to_string()]),
        ),
        // An imperative `go` toward an enemy becomes `attack`.
        (
            go().at(OrderTarget::Entity(enemy)),
            true,
            (Keyword::Attack, None, vec![enemy.to_string()]),
        ),
        (
            farm.at(OrderTarget::Point { square: home, x, y }),
            false,
            (
                Keyword::Build,
                Some("farm".to_owned()),
                vec![format!("zoom-{id}-{x}-{y}")],
            ),
        ),
    ];
    let stored = |w: &World| {
        let o = &w.e(peasant).orders[0];
        (o.keyword, o.type_name.clone(), o.args.clone())
    };
    // Read before the turn moves the peasant, which flees the enemy.
    for (o, imperative, expected) in cases {
        let commands = [
            Command::Control(vec![peasant]),
            order(Order::Keyword(o), false, imperative),
        ];
        for c in commands {
            let (parsed, _) = Command::parse(&c.line(&a).unwrap(), &b).unwrap();
            b.execute_command(me, &parsed).unwrap();
            a.execute_command(me, &c).unwrap();
        }
        assert_eq!(stored(&a), expected);
        assert_eq!(stored(&b), expected);
        assert_eq!(a.digest_u64(), b.digest_u64());
    }
}

#[test]
fn a_line_is_read_as_the_command_it_writes_or_refused() {
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
