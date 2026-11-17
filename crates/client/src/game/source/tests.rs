//! A game from its start to its end against each source: what is said
//! when it starts, the keys and the turns between, and what is said when
//! it ends.

use std::sync::Arc;

use soundrts_protocol::{Computer, Login, Skill, SlotPlayer, Speed};
use soundrts_world::Completeness;

use crate::clock::Clock;
use crate::game::GameInterface;
use crate::game::replay::{Keep, Replay};
use crate::input::{Key, Mods};
use crate::testkit::{FakeStream, Harness, Script, TestGame, render, slot};

/// Every utterance said since `from`, as lines.
fn said_since(h: &Harness, from: usize) -> Vec<String> {
    h.utterances()[from..].iter().map(render).collect()
}

/// Opens the game menu and chooses "cancel game", one key a pass: what
/// was said meanwhile.
fn cancel_game(h: &mut Harness, g: &mut TestGame) -> Vec<String> {
    let from = h.utterances().len();
    for key in [Key::F10, Key::Down, Key::Return] {
        h.press(key, Mods::NONE);
        h.step(g);
    }
    said_since(h, from)
}

#[test]
fn a_game_run_here_starts_with_its_objective_and_ends_with_the_score_its_record_kept() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training("jl1", &["quiet"], 1);
    h.run_until_voice_idle(&mut g);
    assert_eq!(said_since(&h, 0), ["objective: neutralized enemy"]);
    h.run_until_turn(&mut g, 10);
    assert_eq!(
        cancel_game(&mut h, &mut g),
        ["Menu", "cancel game ,", "cancel game"]
    );
    let from = h.utterances().len();
    h.finish(&mut g);
    let said = said_since(&h, from);
    assert_eq!(said[0], "Defeat at 0 minutes 3 seconds", "{said:#?}");
    assert!(said.last().unwrap().starts_with("score "), "{said:#?}");
    assert_eq!(g.session.as_ref().map(|s| s.has_victory), Some(false));
    let kept = crate::saves::list_saves(&h.client.paths.records());
    assert_eq!(kept.len(), 1, "the finished game is kept");
    assert!(kept[0].header.last_game);
}

#[test]
fn a_game_on_a_server_starts_with_its_objective_and_ends_with_the_score_then_the_menus() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let server = h.join_fake_server("andrew", &["bob"]);
    let andrew = SlotPlayer::Human(Login::new("andrew").unwrap());
    let quiet = SlotPlayer::Computer(Computer::Quiet(Skill::NORMAL));
    let slots = [slot(andrew, 1), slot(quiet, 2)];
    let mut stream = FakeStream::new(&mut h, "jl1", &slots, 0, 1);
    let mut g = h.join_fake_game(&server, &stream, "jl1", Speed::Normal);
    h.run_until_voice_idle(&mut g);
    assert_eq!(said_since(&h, 0), ["objective: neutralized enemy"]);
    for turn in 1..=10 {
        for frame in stream.turn(Vec::new()) {
            server.frame_at(h.clock.now(), frame);
        }
        h.run_until_turn(&mut g, turn);
    }
    assert_eq!(
        cancel_game(&mut h, &mut g),
        ["Menu", "cancel game ,", "cancel game"]
    );
    // The quit goes to the server, which plays it at its next turn, tells
    // the player the game is over for them, then sends the end.
    h.step(&mut g);
    let sent: Vec<_> = server.commands().into_iter().map(|(c, _)| c).collect();
    assert_eq!(sent, [soundrts_world::Command::Quit]);
    let now = h.clock.now();
    for frame in stream.turn(sent) {
        server.frame_at(now, frame);
    }
    server.frame_at(now + 1.0, stream.end());
    let from = h.utterances().len();
    h.finish(&mut g);
    let said = said_since(&h, from);
    assert_eq!(said[0], "Defeat at 0 minutes 3 seconds", "{said:#?}");
    let menus = said.len() - 1;
    assert!(said[menus - 1].starts_with("score "), "{said:#?}");
    assert_eq!(said[menus], "Menu please make a selection");
    assert!(h.client.net.as_ref().unwrap().game.is_none());
}

#[test]
fn a_record_listened_to_starts_with_whose_side_it_is_and_ends_with_the_score() {
    let record = {
        let mut h = Harness::new("en", Script::new().keep_open());
        let mut g = h.start_training("jl1", &["quiet"], 1);
        for _ in 0..20 {
            crate::game::testing::step(&mut g);
        }
        g.game.host().record().clone()
    };
    let mut h = Harness::new("en", Script::new().keep_open());
    let map = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&map));
    let replay = Replay::new(Arc::new(record), Completeness::Whole, Keep::AlreadyKept).unwrap();
    let me = replay.first_player().unwrap();
    let mut game = GameInterface::listening(&mut h.client, replay, me).unwrap();
    game.begin(&mut h.client, Vec::new());
    h.client.input.clear();
    let mut g = TestGame {
        session: None,
        game,
    };
    h.run_until_voice_idle(&mut g);
    // Then the news of the first turns.
    assert_eq!(
        said_since(&h, 0)[..2],
        ["watching player", "objective: neutralized enemy"]
    );
    h.run_until_turn(&mut g, 5);
    assert_eq!(h.key(&mut g, Key::Equals, Mods::NONE), ["speed fast 2"]);
    let from = h.utterances().len();
    h.finish(&mut g);
    let said = said_since(&h, from);
    // The news still waiting, then the score: its outcome, units,
    // buildings, two resources, and the score itself.
    assert_eq!(said[0], "end of the recording", "{said:#?}");
    let score = &said[said.len() - 6..];
    assert_eq!(score[0], "Not over at 0 minutes 6 seconds", "{said:#?}");
    assert!(score[5].starts_with("score "), "{said:#?}");
}
