//! A running game for the tests of the interface: a training game
//! against a quiet computer, on recording devices.

use soundrts_world::msg::Msg;
use soundrts_world::{EntityId, PlayerId, SquareId};

use super::GameInterface;
use crate::session::Session;
use crate::testkit::{Harness, Script, TestGame};

/// A training game on jl1 against a quiet computer, after three turns.
pub(crate) fn training() -> (Harness, TestGame) {
    training_on("jl1")
}

/// A training game on `map` against a quiet computer, after three turns.
pub(crate) fn training_on(map: &str) -> (Harness, TestGame) {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training(map, &["quiet"], 1);
    h.run_until_turn(&mut g, 3);
    (h, g)
}

/// A training game on `map` against computers of these types, in these
/// alliances (the player's first), after three turns.
pub(crate) fn training_allied(
    map: &str,
    computers: &[&str],
    alliances: &[&str],
) -> (Harness, TestGame) {
    let mut h = Harness::new("en", Script::new().keep_open());
    h.client.game.seed = Some(1);
    let m = h
        .client
        .res
        .multiplayer_maps()
        .into_iter()
        .find(|m| m.name == map)
        .expect("a multiplayer map");
    h.client.res.set_map(Some(&m));
    let mut players = vec![soundrts_world::Controller::human(
        h.client.config.login.as_str(),
        true,
    )];
    players.extend(
        computers
            .iter()
            .map(|c| soundrts_world::Controller::computer(c)),
    );
    for (p, a) in players.iter_mut().zip(alliances) {
        p.alliance = Some((*a).to_owned());
    }
    let mut session = Session::training(&h.client, m, players);
    let mut game = session.start(&mut h.client).expect("a valid map");
    h.client.input.clear();
    game.set_turn_due(h.client.clock.now());
    let mut g = TestGame {
        session: Some(session),
        game,
    };
    h.run_until_turn(&mut g, 3);
    (h, g)
}

/// Gives `p` a command in the map language, as the console does.
pub(crate) fn cheat(g: &mut TestGame, p: PlayerId, line: &str) {
    g.game.host_mut().queue_line(p, &format!("cmd {line}"));
}

/// Runs a turn of the game and tells the seat, without the interface
/// handling what happened.
pub(crate) fn step(g: &mut TestGame) {
    let me = g.game.me;
    let report = g.game.host_mut().step(me);
    g.game.known.seat_mut().install(report.message);
}

/// Orders `unit` of `p` to go to the center of `square`.
pub(crate) fn go(g: &mut TestGame, p: PlayerId, unit: EntityId, square: SquareId) {
    let to = g.game.view().square(square).protocol_id();
    g.game
        .host_mut()
        .queue_line(p, &format!("control {}", unit.0));
    g.game
        .host_mut()
        .queue_line(p, &format!("order 0 0 go {to}"));
}

/// Runs the game a turn at a time until `done`, at most 400 turns.
pub(crate) fn run_until(h: &mut Harness, g: &mut TestGame, done: impl Fn(&GameInterface) -> bool) {
    for _ in 0..400 {
        if done(&g.game) {
            return;
        }
        let next = g.game.view().turn() + 1;
        h.run_until_turn(g, next);
    }
    panic!("not done after 400 turns");
}

/// What the voice says of a message.
pub(crate) fn said(h: &mut Harness, m: &Msg) -> String {
    let from = h.utterances().len();
    h.client.voice.item(m);
    h.said_since(from).join(" | ")
}

/// The computer.
pub(crate) fn enemy(g: &TestGame) -> PlayerId {
    let me = g.game.me;
    *g.game
        .view()
        .players()
        .iter()
        .find(|&&p| p != me)
        .expect("a computer")
}

/// The first unit of `p` of this type, in the whole world.
pub(crate) fn unit(g: &TestGame, p: PlayerId, type_name: &str) -> EntityId {
    let w = g.game.host().world();
    w.player(p)
        .units
        .iter()
        .copied()
        .find(|&u| w.entity(u).is_some_and(|e| &*e.type_name == type_name))
        .unwrap_or_else(|| panic!("no {type_name}"))
}
