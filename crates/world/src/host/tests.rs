use super::*;
use crate::command::{KeywordOrder, Order, OrderTarget, Queue};
use crate::entity::EntityId;
use crate::orders::Keyword;
use crate::player::Controller;
use crate::world::multiplayer_default_triggers;
use crate::{AiScripts, Rules};

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/soundrts-upstream")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A training game on a map: the local human against an easy computer.
fn training(map: &str, seed: i64) -> GameSetup {
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    let mut computer = Controller::computer("easy");
    computer.alliance = Some("2".into());
    GameSetup {
        rules: read("res/rules.txt"),
        ai: read("res/ai.txt"),
        map: read(map),
        seed,
        controllers: vec![me, computer],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: true,
        random_starts: true,
        equivalents: true,
    }
}

/// The interface's `build_world`, before the set-up moved into the
/// simulation.
#[allow(clippy::too_many_arguments)]
fn old_build_world(
    rules: &str,
    ai: &str,
    default_triggers: Vec<crate::DefaultTrigger>,
    seed: i64,
    allow_cheatmode: bool,
    map_definition: &str,
    controllers: &[Controller],
    random_starts: bool,
    equivalents: bool,
) -> Result<(World, Vec<PlayerId>), String> {
    let rules = Rules::load(&[rules]);
    let ai = AiScripts::load(&[ai]);
    let mut world = World::new(rules, ai, default_triggers, seed);
    world.allow_cheatmode = allow_cheatmode;
    world.load_and_build_map(map_definition).map_err(|e| e.0)?;
    let players = world
        .populate_map(controllers, random_starts, equivalents)
        .map_err(|e| e.0)?;
    Ok((world, players))
}

fn old_build(s: &GameSetup) -> Result<(World, Vec<PlayerId>), String> {
    old_build_world(
        &s.rules,
        &s.ai,
        s.default_triggers.clone(),
        s.seed,
        s.allow_cheatmode,
        &s.map,
        &s.controllers,
        s.random_starts,
        s.equivalents,
    )
}

#[test]
fn a_setup_builds_the_world_the_interface_built() {
    let mut setups = vec![
        training("res/multi/jl1.txt", 1),
        training("res/multi/js10.txt", 77),
    ];
    // A mission: no default triggers, start units kept, no random start.
    let mut mission = training("res/multi/jl1.txt", 3);
    mission.controllers.truncate(1);
    mission.default_triggers = Vec::new();
    mission.equivalents = false;
    mission.random_starts = false;
    setups.push(mission);
    // A game on a server: two humans, one of them remote, no cheats.
    let mut server = training("res/multi/jl1.txt", 9);
    server.controllers[1] = Controller::human("you", false);
    server.allow_cheatmode = false;
    setups.push(server);
    for s in &setups {
        let (mut old, players) = old_build(s).unwrap();
        let mut host = Host::new(s).unwrap();
        assert_eq!(host.seats(), players);
        assert_eq!(host.world.allow_cheatmode, s.allow_cheatmode);
        assert_eq!(host.world.digest_u64(), old.digest_u64());
        assert_eq!(host.drain_warnings(), old.drain_warnings());
        for _ in 0..50 {
            host.step(players[0]);
            old.update();
        }
        assert_eq!(host.world.digest_u64(), old.digest_u64());
    }
}

#[test]
fn a_map_error_is_the_old_one() {
    let mut s = training("res/multi/jl1.txt", 1);
    s.map = "nb_columns x\n".into();
    let old = old_build(&s).map(|_| ()).unwrap_err();
    assert_eq!(Host::new(&s).map(|_| ()).unwrap_err(), old);
}

/// A training game after a few turns, with a priest so that mana matters.
fn game() -> (Host, PlayerId) {
    let mut host = Host::new(&training("res/multi/jl1.txt", 5)).unwrap();
    let me = host.seats()[0];
    let told = host.queue(
        me,
        Command::Cheat(Cheat::Trigger("add_units a1 2 priest".into())),
    );
    assert!(told.is_none(), "a trigger waits for the step");
    for _ in 0..5 {
        host.step(me);
    }
    (host, me)
}

/// Checks that the cheat leaves the game as the old direct write did, now
/// and turns later.
fn same_as_old_write(cheat: Cheat, old_write: impl FnOnce(&mut World, PlayerId)) {
    let (mut old, me) = game();
    let (mut host, _) = game();
    assert_eq!(host.world.digest_u64(), old.world.digest_u64());
    let before = host.world.digest_u64();
    old_write(&mut old.world, me);
    let told = host.queue(me, Command::Cheat(cheat.clone()));
    let told = told.unwrap_or_else(|| panic!("{cheat:?} is told at once"));
    let seat = told.seat;
    assert_eq!(told.state, host.message(seat).state, "{cheat:?}");
    assert_ne!(host.world.digest_u64(), before, "{cheat:?} changed nothing");
    assert_eq!(host.world.digest_u64(), old.world.digest_u64(), "{cheat:?}");
    for _ in 0..20 {
        host.step(me);
        old.step(me);
    }
    assert_eq!(host.world.digest_u64(), old.world.digest_u64(), "{cheat:?}");
}

// The old writes are the interface's code before cheats were commands.

#[test]
fn the_resources_cheat_does_what_the_console_did() {
    same_as_old_write(Cheat::Resources, |w, me| {
        let p = w.player_mut(me);
        for r in &mut p.resources {
            *r += 1000 * 1000;
        }
    });
    let (mut host, me) = game();
    let before = host.world.player(me).resources.clone();
    let told = host.queue(me, Command::Cheat(Cheat::Resources)).unwrap();
    let after: Vec<i64> = before.iter().map(|r| r + 1_000_000).collect();
    assert_eq!(host.world.player(me).resources, after);
    // The seat is told at once, between turns.
    assert_eq!((told.seat, told.state.resources), (me, after));
}

#[test]
fn the_techs_cheat_does_what_the_console_did() {
    same_as_old_write(Cheat::AllTechs, |w, me| {
        w.player_mut(me).cheat_has_all = true;
    });
}

#[test]
fn the_mana_cheat_does_what_the_console_did() {
    same_as_old_write(Cheat::InfiniteMana, |w, me| {
        for u in w.player(me).units.clone() {
            if let Some(e) = w.entity_mut(u) {
                e.stats.mana_regen *= 1000;
            }
        }
    });
    let (mut host, me) = game();
    let told = host.queue(me, Command::Cheat(Cheat::InfiniteMana));
    assert_eq!(told.map(|m| m.seat), Some(me));
    let regen: Vec<i64> = host
        .world
        .player(me)
        .units
        .iter()
        .map(|&u| host.world.entity(u).unwrap().stats.mana_regen)
        .filter(|&r| r != 0)
        .collect();
    // The priests' 1 mana a second, in thousandths, times 1000.
    assert_eq!(regen, [1_000_000, 1_000_000]);
}

#[test]
fn playing_as_another_player_does_what_the_interface_did() {
    let (host, me) = game();
    let other = host.seats()[1];
    same_as_old_write(Cheat::PlayAs(other), |w, me| {
        let new = other;
        let login = w.player(new).controller.login();
        w.player_mut(new).controller.kind = ControllerKind::Human { login, local: true };
        if let ControllerKind::Human { local, .. } = &mut w.player_mut(me).controller.kind {
            *local = false;
        }
    });
    let (mut host, _) = game();
    let told = host
        .queue(me, Command::Cheat(Cheat::PlayAs(other)))
        .unwrap();
    // The seat is now the other player's.
    assert_eq!(told.seat, other);
    assert_eq!(
        host.world.player(other).controller.kind,
        ControllerKind::Human {
            login: "ai_easy".into(),
            local: true
        }
    );
    assert_eq!(
        host.world.player(me).controller.kind,
        ControllerKind::Human {
            login: "me".into(),
            local: false
        }
    );
    // The events and the end of the turn now go to the other player.
    let report = host.step(other);
    assert_eq!(report.end_of_turn.map(|m| m.seat), Some(other));
    let report = host.step(me);
    assert!(report.end_of_turn.is_none() && report.then.is_empty());
}

#[test]
fn a_game_without_cheats_refuses_them() {
    let mut s = training("res/multi/jl1.txt", 5);
    s.allow_cheatmode = false;
    let mut host = Host::new(&s).unwrap();
    let me = host.seats()[0];
    host.drain_warnings();
    let before = host.world.digest_u64();
    for c in [
        Cheat::Resources,
        Cheat::AllTechs,
        Cheat::InfiniteMana,
        Cheat::PlayAs(host.seats()[1]),
    ] {
        assert!(host.queue(me, Command::Cheat(c)).is_none());
    }
    assert_eq!(host.world.digest_u64(), before);
    let warnings = host.drain_warnings();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert_eq!(warnings[0], "cheat mode is not allowed: Resources");
}

#[test]
fn commands_run_at_the_next_step_in_the_order_given() {
    let mut host = Host::new(&training("res/multi/jl1.txt", 5)).unwrap();
    let me = host.seats()[0];
    host.step(me);
    for s in [2.0, 0.5, 4.0] {
        assert!(host.queue(me, Command::Speed(s)).is_none());
    }
    // Nothing runs before the step.
    assert_eq!(host.world.turn(), 1);
    let end = host.step(me).end_of_turn.unwrap();
    let speeds: Vec<f64> = end
        .events
        .iter()
        .filter_map(|e| match e {
            &Event::Speed(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(speeds, [2.0, 0.5, 4.0]);
}

#[test]
fn a_typed_command_and_its_old_line_play_the_same_game() {
    let (mut typed, me) = game();
    let (mut text, _) = game();
    let peasants: Vec<EntityId> = typed
        .world
        .player(me)
        .units
        .iter()
        .copied()
        .filter(|&u| &*typed.world.entity(u).unwrap().type_name == "peasant")
        .collect();
    let b2 = typed.world.square_by_name("b2").unwrap();
    let ids: Vec<String> = peasants.iter().map(|u| u.0.to_string()).collect();
    let _ = typed.queue(me, Command::Control(peasants));
    let _ = typed.queue(
        me,
        Command::Order {
            order: Order::Keyword(
                KeywordOrder::new(Keyword::Go)
                    .unwrap()
                    .at(OrderTarget::Square(b2)),
            ),
            queue: Queue::Replace,
            imperative: false,
        },
    );
    text.queue_line(me, &format!("control {}", ids.join(" ")));
    text.queue_line(me, &format!("order 0 0 go {}", text.world.square(b2).id));
    let before = typed.world.digest_u64();
    for _ in 0..30 {
        typed.step(me);
        text.step(me);
        assert_eq!(typed.world.digest_u64(), text.world.digest_u64());
    }
    assert_ne!(typed.world.digest_u64(), before);
}

#[test]
fn a_turn_ends_with_what_the_seat_knows_then_and_the_game_may_end_after() {
    let (mut host, me) = game();
    let report = host.step(me);
    let end = report.end_of_turn.unwrap();
    // On an ordinary turn the end is what the seat knows after the step.
    assert!(Arc::ptr_eq(&end, &report.message));
    assert!(report.then.is_empty());
    assert_eq!(end.time, host.world.time);

    // A victory ends the game at the end of the turn: every player leaves
    // it after, and the seat's units with it.
    let _ = host.queue(me, Command::Cheat(Cheat::Victory));
    let report = host.step(me);
    let end = report.end_of_turn.unwrap();
    assert!(end.state.players.contains(&me));
    assert!(end.state.has_victory);
    let own = |m: &TurnMessage| {
        m.perceived
            .iter()
            .filter(|e| e.player == Some(me) && e.place.is_some())
            .count()
    };
    assert!(own(&end) > 0, "the units are there at the end of the turn");
    assert!(
        matches!(report.then[..], [Event::Quit]),
        "{:?}",
        report.then
    );
    let after = &report.message;
    assert!(after.state.players.is_empty());
    assert_eq!(own(after), 0, "the units left with the player");
    assert_eq!(after.time, end.time);
    // The game is over: no more ends of turn.
    let report = host.step(me);
    assert!(report.end_of_turn.is_none());
}

/// Two humans on jl1, each told the events, as a host on a server plays it.
fn two_humans() -> Host {
    let mut a = Controller::human("a", true);
    a.alliance = Some("1".into());
    let mut b = Controller::human("b", true);
    b.alliance = Some("2".into());
    let setup = GameSetup::multiplayer(
        read("res/rules.txt"),
        read("res/ai.txt"),
        read("res/multi/jl1.txt"),
        4,
        vec![a, b],
    );
    Host::new(&setup).unwrap()
}

/// The entities of the order-ok events among `events`.
fn ordered(events: &[Event]) -> Vec<EntityId> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Entity {
                entity,
                event: crate::EntityEvent::OrderOk,
            } => Some(entity.id),
            _ => None,
        })
        .collect()
}

#[test]
fn stepping_every_seat_tells_each_its_own_events() {
    let mut host = two_humans();
    let seats = host.seats().to_vec();
    let (a, b) = (seats[0], seats[1]);
    let peasants = |host: &Host, p: PlayerId| -> Vec<EntityId> {
        let w = &host.world;
        let mut ids: Vec<EntityId> = w
            .player(p)
            .units
            .iter()
            .copied()
            .filter(|&u| &*w.entity(u).unwrap().type_name == "peasant")
            .collect();
        ids.sort();
        ids
    };
    let (mine, theirs) = (peasants(&host, a), peasants(&host, b));
    assert!(!mine.is_empty() && !theirs.is_empty());
    let b2 = host.world.square_by_name("b2").unwrap();
    let go = |units: Vec<EntityId>| {
        [
            Command::Control(units),
            Command::Order {
                order: Order::Keyword(
                    KeywordOrder::new(Keyword::Go)
                        .unwrap()
                        .at(OrderTarget::Square(b2)),
                ),
                queue: Queue::Replace,
                imperative: false,
            },
        ]
    };
    for c in go(mine.clone()) {
        assert!(host.queue(a, c).is_none());
    }
    let turns = host.step_seats(&seats);
    assert_eq!(turns.len(), 2);
    assert_eq!((turns[0].message.seat, turns[1].message.seat), (a, b));
    let ends: Vec<&TurnMessage> = turns
        .iter()
        .map(|t| &**t.end_of_turn.as_ref().unwrap())
        .collect();
    let mut told = ordered(&ends[0].events);
    told.sort();
    assert_eq!(told, mine, "a hears its own peasants obey");
    assert_eq!(ordered(&ends[1].events), [], "b hears nothing of it");
    for t in &turns {
        assert!(t.then.is_empty());
        assert!(Arc::ptr_eq(t.end_of_turn.as_ref().unwrap(), &t.message));
    }

    // Now b orders, and only a is asked for: b's events are drained all
    // the same, and are not told to a.
    for c in go(theirs.clone()) {
        assert!(host.queue(b, c).is_none());
    }
    let turns = host.step_seats(&[a]);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].message.seat, a);
    assert_eq!(ordered(&turns[0].message.events), []);
    let nothing_left = |host: &Host| {
        for &p in &seats {
            assert_eq!(host.world.player(p).events, [], "player {}", p.0);
            assert!(host.world.player(p).end_of_turn.is_none(), "player {}", p.0);
        }
    };
    for _ in 0..20 {
        nothing_left(&host);
        host.step_seats(&[b]);
    }
    nothing_left(&host);
}

#[test]
fn stepping_every_seat_tells_each_how_the_game_ended() {
    let mut a = Controller::human("a", true);
    a.alliance = Some("1".into());
    let mut b = Controller::human("b", true);
    b.alliance = Some("2".into());
    let mut setup = GameSetup::multiplayer(
        read("res/rules.txt"),
        read("res/ai.txt"),
        read("res/multi/jl1.txt"),
        4,
        vec![a, b],
    );
    setup.allow_cheatmode = true;
    let mut host = Host::new(&setup).unwrap();
    let seats = host.seats().to_vec();
    host.step_seats(&seats);
    // A victory ends the game at the end of the turn: the winner is told
    // the turn, then that everyone left.
    let _ = host.queue(seats[0], Command::Cheat(Cheat::Victory));
    let turns = host.step_seats(&seats);
    let (won, lost) = (&turns[0], &turns[1]);
    let end = won.end_of_turn.as_ref().unwrap();
    assert!(end.state.players.contains(&seats[0]));
    assert!(end.state.has_victory);
    assert!(matches!(won.then[..], [Event::Quit]), "{:?}", won.then);
    assert!(won.message.state.players.is_empty());
    assert_eq!(won.message.time, end.time);
    // The loser left during the turn: no end of turn, and every event of
    // the step after.
    assert!(lost.end_of_turn.is_none());
    assert!(matches!(lost.then[..], [Event::Quit]), "{:?}", lost.then);
    assert_eq!(lost.message.state.defeated, [seats[1]]);
}

// Commands naming nothing, as a broken or hostile client may send them.

/// An order with this keyword, type and target.
fn order(kw: Keyword, type_name: Option<&str>, target: Option<OrderTarget>) -> Command {
    let o = match type_name {
        Some(t) => KeywordOrder::typed(kw, t).unwrap(),
        None => KeywordOrder::new(kw).unwrap(),
    };
    let o = match target {
        Some(t) => o.at(t),
        None => o,
    };
    Command::Order {
        order: Order::Keyword(o),
        queue: Queue::Replace,
        imperative: false,
    }
}

fn default_order(target: OrderTarget) -> Command {
    Command::Order {
        order: Order::Default(target),
        queue: Queue::Replace,
        imperative: true,
    }
}

/// A training game whose human controls all its units.
fn controlling_all() -> (Host, PlayerId) {
    let (mut host, me) = game();
    let own = host.world.player(me).units.clone();
    assert!(host.queue(me, Command::Control(own)).is_none());
    host.step(me);
    host.drain_warnings();
    (host, me)
}

#[test]
fn a_command_naming_a_square_off_the_map_is_refused_when_it_is_queued() {
    let (mut host, me) = controlling_all();
    let off = crate::entity::SquareId(u32::MAX);
    let point = OrderTarget::Point {
        square: off,
        x: 0,
        y: 0,
    };
    let mut commands = Vec::new();
    for t in [OrderTarget::Square(off), point] {
        for kw in [
            Keyword::Go,
            Keyword::Attack,
            Keyword::Patrol,
            Keyword::Gather,
            Keyword::Block,
            Keyword::Repair,
            Keyword::RallyingPoint,
            Keyword::Enter,
        ] {
            commands.push(order(kw, None, Some(t)));
        }
        commands.push(order(Keyword::Build, Some("farm"), Some(t)));
        commands.push(order(Keyword::Use, Some("a_holy_vision"), Some(t)));
        commands.push(default_order(t));
    }
    commands.push(Command::Cheat(Cheat::AddUnits {
        square: Some(off),
        units: "footman".into(),
    }));
    let before = host.world.digest_u64();
    for c in &commands {
        assert_eq!(host.check(me, c), Err(Refused::NoSuchSquare(off)));
        assert!(host.queue(me, c.clone()).is_none());
    }
    assert_eq!(host.world.digest_u64(), before);
    let warnings = host.drain_warnings();
    let expected: Vec<String> = commands
        .iter()
        .map(|c| format!("command {c:?} refused: no square 4294967295"))
        .collect();
    assert_eq!(warnings, expected);
    for _ in 0..3 {
        host.step(me);
    }
    // The world refuses it too, for a caller without a host.
    host.drain_warnings();
    let c = default_order(OrderTarget::Square(off));
    host.world.queue(me, c.clone());
    let refused = format!("command {c:?} refused: no square 4294967295");
    assert_eq!(host.step(me).warnings, [refused]);
}

#[test]
fn a_command_of_a_player_the_game_never_had_is_refused_when_it_is_queued() {
    let (mut host, me) = controlling_all();
    let nobody = PlayerId(99);
    let commands = [
        Command::Control(Vec::new()),
        order(Keyword::Stop, None, None),
        Command::Speed(1.0),
        Command::Quit,
        Command::Cheat(Cheat::Resources),
        Command::Cheat(Cheat::AllTechs),
        Command::Cheat(Cheat::InfiniteMana),
        Command::Cheat(Cheat::PlayAs(me)),
        Command::Cheat(Cheat::Victory),
    ];
    let before = host.world.digest_u64();
    for c in &commands {
        assert_eq!(host.check(nobody, c), Err(Refused::NoSuchPlayer(nobody)));
        assert!(host.queue(nobody, c.clone()).is_none());
    }
    assert_eq!(host.world.digest_u64(), before);
    let warnings = host.drain_warnings();
    let expected: Vec<String> = commands
        .iter()
        .map(|c| format!("command {c:?} refused: no player 99"))
        .collect();
    assert_eq!(warnings, expected);
    host.step(me);
    assert_eq!(host.world.players.len(), 2);
}

#[test]
fn a_command_naming_no_entity_no_type_or_a_point_far_off_its_square_comes_to_nothing() {
    let (mut host, me) = controlling_all();
    let a1 = host.world.square_by_name("a1").unwrap();
    let nothing = EntityId(u32::MAX);
    let own = host.world.player(me).units.clone();
    // Taken, as an entity may have died since the player named it, and it
    // chooses no unit.
    let control = Command::Control(vec![nothing]);
    assert_eq!(host.check(me, &control), Ok(()));
    assert!(host.queue(me, control).is_none());
    host.step(me);
    assert_eq!(host.world.player(me).group, Some(Vec::new()));
    assert!(host.queue(me, Command::Control(own.clone())).is_none());
    let far = |x, y| OrderTarget::Point { square: a1, x, y };
    let group = |order| Command::Order {
        order,
        queue: Queue::Replace,
        imperative: false,
    };
    // Any name is a group's: joining one makes it.
    let long = "é".repeat(1000);
    let mut warnings = Vec::new();
    for c in [
        order(Keyword::Go, None, Some(OrderTarget::Entity(nothing))),
        default_order(OrderTarget::Entity(nothing)),
        order(Keyword::Train, Some("nothing"), None),
        order(Keyword::Use, Some("footman"), None),
        order(Keyword::Go, None, Some(far(i64::MAX, i64::MIN))),
        order(Keyword::Go, None, Some(far(i64::MIN, i64::MAX))),
        group(Order::ResetGroup("never".into())),
        group(Order::JoinGroup(long.clone())),
    ] {
        assert_eq!(host.check(me, &c), Ok(()));
        assert!(host.queue(me, c).is_none());
        warnings.extend(host.step(me).warnings);
    }
    // A point off its square leaves the order without a target: it leaves
    // the queue at the next turn.
    warnings.extend(host.step(me).warnings);
    assert_eq!(host.world.player(me).group, Some(own.clone()));
    assert!(own.iter().all(|&u| host.world.e(u).orders().is_empty()));
    let names: Vec<&str> = host.world.groups.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, [long.as_str()]);
    let zoom = |at: &str| {
        format!("problem with order: [\"go\", \"zoom-2-{at}\"]: the zoom is outside its square")
    };
    let (a, b) = (
        zoom("9223372036854775807--9223372036854775808"),
        zoom("-9223372036854775808-9223372036854775807"),
    );
    // Once for each of the three units that can go.
    assert_eq!(warnings, [&a, &a, &a, &b, &b, &b].map(String::as_str));
}
