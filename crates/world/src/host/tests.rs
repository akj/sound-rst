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
