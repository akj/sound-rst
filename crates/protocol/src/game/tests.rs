//! The encoding of single values. Whole games are in
//! `tests/game_stream.rs`.

use std::sync::Arc;

use soundrts_world::orders::Keyword;
use soundrts_world::{
    Cheat, Controller, EntityId, Event, KeywordOrder, KnownEntity, KnownOrder, KnownTarget, Order,
    OrderTarget, OwnPart, PlayerId, Queue, SquareId,
};

use super::*;

fn commands() -> Vec<Command> {
    let order = |order, queue, imperative| Command::Order {
        order,
        queue,
        imperative,
    };
    vec![
        Command::Control(vec![EntityId(4), EntityId(70_000)]),
        Command::Control(Vec::new()),
        order(
            Order::Keyword(
                KeywordOrder::new(Keyword::Go)
                    .unwrap()
                    .at(OrderTarget::Point {
                        square: SquareId(3),
                        x: -5,
                        y: i64::MAX,
                    }),
            ),
            Queue::Append,
            true,
        ),
        order(
            Order::Keyword(KeywordOrder::typed(Keyword::Train, "footman").unwrap()),
            Queue::Replace,
            false,
        ),
        order(
            Order::Default(OrderTarget::Entity(EntityId(9))),
            Queue::Replace,
            false,
        ),
        order(Order::JoinGroup("1".into()), Queue::Replace, false),
        order(Order::ResetGroup("é".into()), Queue::Replace, false),
        Command::Speed(1.5),
        // JSON reads this back one bit off without `float_roundtrip`.
        Command::Speed(1.012_615_265_993_759_7),
        Command::Quit,
        Command::Cheat(Cheat::AddUnits {
            square: Some(SquareId(1)),
            units: "3 footman".into(),
        }),
        Command::Cheat(Cheat::Trigger("victory".into())),
        Command::Cheat(Cheat::PlayAs(PlayerId(2))),
    ]
}

#[test]
fn a_command_comes_back_as_it_was_sent() {
    for c in commands() {
        let bytes = encode_command(&c).unwrap();
        assert_eq!(decode_command(&bytes), Ok(c));
    }
}

#[test]
fn a_command_comes_back_from_json_as_well() {
    for c in commands() {
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), c);
    }
}

#[test]
fn a_speed_that_is_not_one_is_refused_both_ways() {
    for s in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
        assert!(encode_command(&Command::Speed(s)).is_err(), "{s}");
        let bytes = postcard::to_allocvec(&Command::Speed(s)).unwrap();
        assert!(decode_command(&bytes).is_err(), "{s}");
    }
}

/// A keyword that takes no type, sent with one, is refused: the world
/// takes a `KeywordOrder` as valid.
#[test]
fn a_keyword_order_with_a_type_it_cannot_have_is_refused() {
    let typed = Command::Order {
        order: Order::Keyword(KeywordOrder::typed(Keyword::Train, "footman").unwrap()),
        queue: Queue::Replace,
        imperative: false,
    };
    let plain = Command::Order {
        order: Order::Keyword(KeywordOrder::new(Keyword::Go).unwrap()),
        queue: Queue::Replace,
        imperative: false,
    };
    let mut bytes = encode_command(&typed).unwrap();
    let go = encode_command(&plain).unwrap();
    // The commands differ first at the keyword.
    let at = bytes.iter().zip(&go).position(|(a, b)| a != b).unwrap();
    bytes[at] = go[at];
    assert!(decode_command(&bytes).is_err());
    // Postcard keeps no reason; JSON shows it is this one.
    let text = serde_json::to_string(&typed)
        .unwrap()
        .replace(r#""Train""#, r#""Go""#);
    let refused = serde_json::from_str::<Command>(&text).unwrap_err();
    assert!(
        refused.to_string().contains("a type it cannot have"),
        "{refused}"
    );
}

#[test]
fn a_setup_comes_back_as_it_was_sent() {
    let setup = GameSetup {
        rules: "def footman\nclass soldier\n".into(),
        ai: String::new(),
        map: "square_width 12\n".into(),
        seed: -3,
        controllers: vec![Controller::human("me", true), Controller::computer("easy")],
        default_triggers: soundrts_world::multiplayer_default_triggers(),
        allow_cheatmode: false,
        computer_skill: soundrts_world::Skill::NORMAL,
        random_starts: true,
        equivalents: false,
    };
    let bytes = encode_setup(&setup).unwrap();
    assert_eq!(decode_setup(&bytes), Ok(setup));
}

#[test]
fn bytes_left_over_and_too_many_bytes_are_refused() {
    let mut bytes = encode_command(&Command::Quit).unwrap();
    bytes.push(0);
    assert!(decode_command(&bytes).unwrap_err().0.contains("left over"));
    let long = vec![0; MAX_GAME_MESSAGE_BYTES + 1];
    assert!(decode_end(&long).unwrap_err().0.contains("too long"));
    assert!(decode_frame(&long).unwrap_err().0.contains("too long"));
}

#[test]
fn a_command_is_no_longer_than_an_order() {
    let most = crate::MAX_COMMAND_BYTES;
    assert!(
        decode_command(&vec![0; most + 1])
            .unwrap_err()
            .0
            .contains("too long")
    );
    // A list of ids: one byte of length, then three bytes for each id.
    let ids = |n: u32| Command::Control((0..n).map(|i| EntityId(20_000 + i)).collect());
    let fits = encode_command(&ids(1300)).unwrap();
    assert_eq!(decode_command(&fits), Ok(ids(1300)));
    assert!(fits.len() <= most);
    assert!(encode_command(&ids(1400)).is_err());
}

#[test]
fn a_seat_frame_comes_back_as_it_was_sent() {
    let end = GameEnd {
        stats: Default::default(),
    };
    // The kind of frame comes first: the end is the fourth.
    let frame = SeatFrame::End(end);
    let bytes = encode_frame(&frame).unwrap();
    assert_eq!(bytes[0], 3);
    assert_eq!(decode_frame(&bytes), Ok(frame));
    let a = || crate::Login::new("a").unwrap();
    for frame in [
        SeatFrame::Seat {
            slot: 1,
            change: crate::SeatChange::Removed { by: a() },
        },
        SeatFrame::Seat {
            slot: 2,
            change: crate::SeatChange::Dropped,
        },
        SeatFrame::Host { slot: 3 },
        SeatFrame::Flare {
            from: soundrts_world::PlayerId(2),
            square: soundrts_world::SquareId(14),
        },
        SeatFrame::Clock {
            by: a(),
            change: crate::ClockChange::Speed {
                speed: crate::Speed::Fast2,
            },
        },
        SeatFrame::NotStarted(crate::StartRefusal::CannotBuild {
            error: crate::ChatText::clean("no square a1").unwrap(),
        }),
        SeatFrame::Stopped(crate::StopReason::Crashed),
    ] {
        let bytes = encode_frame(&frame).unwrap();
        assert_eq!(decode_frame(&bytes), Ok(frame));
    }
    // A token that is not one is refused; a seat is a player's or a
    // watcher's.
    let seated_as = |you: Role, token: &str| {
        let mut bytes = encode_frame(&SeatFrame::Seated {
            you,
            map: crate::MapInfo {
                file_name: crate::MapFileName::new("jl1.txt").unwrap(),
                hash: crate::MapHash::of(b""),
                title_sounds: Vec::new(),
                nb_players_min: 2,
                nb_players_max: 2,
            },
            speed: crate::Speed::Normal,
            paused: false,
            hold: std::time::Duration::from_secs(90),
            token: crate::SeatToken::from_bits(7),
        })
        .unwrap();
        let at = bytes.len() - 32;
        bytes[at..].copy_from_slice(token.as_bytes());
        decode_frame(&bytes)
    };
    let seated = |token: &str| seated_as(Role::Player(0), token);
    assert!(matches!(
        seated_as(Role::Watcher, &"b".repeat(32)),
        Ok(SeatFrame::Seated {
            you: Role::Watcher,
            ..
        })
    ));
    assert!(seated(&"0".repeat(32)).is_ok());
    assert!(seated(&"G".repeat(32)).is_err());
}

fn entity(id: u32, type_name: &str) -> KnownEntity {
    KnownEntity {
        id: EntityId(id),
        class: soundrts_world::rules::EntityClass::Soldier,
        type_name: Arc::from(type_name),
        player: Some(PlayerId(0)),
        place: Some(soundrts_world::Place::Square(SquareId(0))),
        x: 1,
        y: 2,
        o: 90,
        hp: 1000,
        hp_max: 1000,
        mana: 0,
        mana_max: 0,
        qty: 0,
        site_type: None,
        number: Some(1),
        blocked_exit: None,
        airground: Default::default(),
        actual_speed: 1000,
        is_moving: false,
        is_mobile: true,
        is_invisible: false,
        is_cloaked: false,
        is_a_detector: false,
        is_a_cloaker: false,
        is_a_building_land: false,
        is_repairable: false,
        is_healable: true,
        is_a_gate: false,
        is_a_threat: true,
        exit: None,
        activity: None,
        upgrades: Vec::new(),
        last_seen: Some(0),
        own: None,
    }
}

/// `inner`, remembered by an order of an entity of id `id`.
fn aiming_at(id: u32, inner: KnownEntity) -> KnownEntity {
    let mut e = entity(id, "footman");
    e.own = Some(OwnPart {
        orders: vec![KnownOrder {
            keyword: Keyword::Attack,
            type_id: None,
            is_deferred: false,
            is_imperative: false,
            time_left: None,
            target: Some(KnownTarget::Remembered(Box::new(inner))),
            patrol: None,
        }],
        has_basic_abilities: true,
        distance_to_goal: 0,
        cargo: None,
        inside: None,
        stance: None,

        group: None,
        is_idle: false,
        auto_gather: None,
        auto_repair: None,
        transport_capacity: 0,
        transport_volume: 0,
        food_cost: 1,
    });
    e
}

fn message(perceived: Vec<KnownEntity>, events: Vec<Event>) -> TurnMessage {
    TurnMessage {
        seat: PlayerId(0),
        time: 300,
        events,
        state: soundrts_world::SeatState {
            resources: vec![1000],
            available_food: 0,
            used_food: 0,
            cheat_mode: false,
            has_victory: false,
            objectives: Vec::new(),
            players: vec![PlayerId(0)],
            defeated: Vec::new(),
            allies: vec![vec![PlayerId(0)]],
            controlled: Vec::new(),
            groups: Vec::new(),
        },
        squares: soundrts_world::KnownSquares {
            observed: Default::default(),
            observed_before: Default::default(),
            partly_observed: Default::default(),
            terrain: Default::default(),
        },
        perceived: perceived
            .into_iter()
            .map(|mut e| {
                e.last_seen = None;
                e
            })
            .collect(),
        remembered: Vec::new(),
        unperceived_units: Vec::new(),
    }
}

/// Remembered entities nest in order targets as deep as the limit, and
/// no deeper, both ways; a message nested a hundred thousand deep is
/// refused before the stack runs out.
#[test]
fn remembered_entities_nest_only_so_deep() {
    let marker = || entity(7, "a_marker_of_its_own");
    let nested = |depth: u32| (0..depth).fold(marker(), |inner, i| aiming_at(100 + i, inner));
    let at_limit = message(vec![nested(8)], Vec::new());
    let bytes = encode_turn(&at_limit).unwrap();
    let back: TurnMessage = decode(&bytes).unwrap();
    assert_eq!(back, at_limit);
    assert!(encode_turn(&message(vec![nested(9)], Vec::new())).is_err());

    // The bytes of one level around the marker, to nest by hand.
    let marker_bytes = postcard::to_allocvec(&marker()).unwrap();
    let one = postcard::to_allocvec(&message(vec![nested(1)], Vec::new())).unwrap();
    let find =
        |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle).unwrap();
    let mut level = aiming_at(100, marker());
    level.last_seen = None;
    let level = postcard::to_allocvec(&level).unwrap();
    let m = find(&level, &marker_bytes);
    let (before, after) = (&level[..m], &level[m + marker_bytes.len()..]);
    let at = find(&one, &level);
    for depth in [8, 9, 100_000] {
        let mut bytes = one[..at].to_vec();
        for _ in 0..depth {
            bytes.extend_from_slice(before);
        }
        bytes.extend_from_slice(&marker_bytes);
        for _ in 0..depth {
            bytes.extend_from_slice(after);
        }
        bytes.extend_from_slice(&one[at + level.len()..]);
        let decoded = decode::<TurnMessage>(&bytes);
        assert_eq!(decoded.is_ok(), depth <= 8, "{depth} levels");
    }
}

/// Each player's faction is sent with the start, by player id.
#[test]
fn the_start_carries_each_players_faction() {
    let start = StartMessage {
        map: soundrts_world::GameMap {
            square_width: 12_000,
            squares: Vec::new(),
            exits: Vec::new(),
            objective: Vec::new(),
            intro: Vec::new(),
        },
        player_names: vec![Vec::new(), Vec::new()],
        player_factions: vec!["orc_faction".into(), "human_faction".into()],
        cheats_allowed: false,
    };
    let sent = decode_start(&encode_start(&start).unwrap()).unwrap();
    assert_eq!(sent.player_factions, ["orc_faction", "human_faction"]);
    let start = GameStart::from_message(sent, soundrts_world::Rules::default());
    assert_eq!(
        start.message().player_factions,
        ["orc_faction", "human_faction"]
    );
}

/// A cost the order menus subtract from: one out of range would overflow
/// there, so it is refused before a seat's view sees it.
#[test]
fn an_own_number_out_of_range_is_refused() {
    let start = GameStart::from_message(
        StartMessage {
            map: soundrts_world::GameMap {
                square_width: 12_000,
                squares: Vec::new(),
                exits: Vec::new(),
                objective: Vec::new(),
                intro: Vec::new(),
            },
            player_names: Vec::new(),
            player_factions: Vec::new(),
            cheats_allowed: false,
        },
        soundrts_world::Rules::default(),
    );
    let mut unit = aiming_at(1, entity(2, "footman"));
    unit.place = None;
    if let Some(KnownTarget::Remembered(inner)) =
        unit.own.as_mut().unwrap().orders[0].target.as_mut()
    {
        inner.place = None;
    }
    let fine = message(vec![unit.clone()], Vec::new());
    assert_eq!(
        decode_turn(&encode_turn(&fine).unwrap(), &start),
        Ok(fine.clone())
    );
    unit.own.as_mut().unwrap().food_cost = i64::MIN;
    let hostile = message(vec![unit], Vec::new());
    assert!(decode_turn(&encode_turn(&hostile).unwrap(), &start).is_err());
    let change = TurnChange::between(&fine, &hostile);
    assert!(apply_change(&encode_change(&change).unwrap(), &fine, &start).is_err());
}
