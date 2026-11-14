//! A running game for the tests of the interface: a training game on jl1
//! against a quiet computer, on recording devices.

use soundrts_world::msg::Msg;
use soundrts_world::{EntityId, PlayerId};

use crate::testkit::{Harness, Script, TestGame};

/// A training game on jl1 against a quiet computer, after three turns.
pub(crate) fn training() -> (Harness, TestGame) {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training("jl1", &["quiet"], 1);
    h.run_until_turn(&mut g, 3);
    (h, g)
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
    let w = g.game.host.world();
    w.player(p)
        .units
        .iter()
        .copied()
        .find(|&u| w.entity(u).is_some_and(|e| &*e.type_name == type_name))
        .unwrap_or_else(|| panic!("no {type_name}"))
}
