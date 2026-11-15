#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::PathBuf;

use soundrts_world::msg::MsgPart;
use soundrts_world::{Cheat, Command, EntityId, SIM_VERSION};

use super::*;
use crate::campaign::Chapter;
use crate::game::testing::{said, training};
use crate::input::Mods;
use crate::saves::{Slot, read_save_bytes, write_save};
use crate::testkit::{Harness, Script, TestGame};

/// Saves the game to a new file and returns it.
fn save_new(h: &mut Harness, g: &mut TestGame) -> PathBuf {
    g.game.save_game(&mut h.client, Slot::New).unwrap()
}

#[test]
fn a_game_saved_and_loaded_tells_what_it_told_and_plays_on_the_same() {
    let (mut h, mut g) = training();
    let me = g.game.me;
    // A cursor on a square and a selection, then commands waiting.
    h.run_until_turn(&mut g, 12);
    let square = g.game.view().square_at(1, 1).unwrap();
    g.game.place = Some(square);
    let peasants: Vec<EntityId> = g
        .game
        .view()
        .own_units()
        .filter(|u| &*u.type_name == "peasant")
        .map(|u| u.id)
        .collect();
    g.game.group = peasants.clone();
    g.game
        .send_command(&h.client, Command::Control(peasants.clone()));
    g.game
        .send_command(&h.client, Command::Cheat(Cheat::Resources));
    let path = save_new(&mut h, &mut g);
    let saved = read_save(&path).unwrap();
    let map = h.multiplayer_map("jl1");
    let told = said(&mut h, &g.game.view().square(square).title().clone());
    let from = h.utterances().len();
    let mut session = replayed(&mut h.client, SessionKind::Training, map, saved)
        .unwrap()
        .expect("replayed");
    let mut loaded = TestGame {
        game: session.start(&mut h.client).expect("started"),
        session: Some(session),
    };
    assert_eq!(loaded.game.me, me);
    assert_eq!(loaded.game.place, Some(square));
    assert_eq!(loaded.game.group, peasants);
    assert_eq!(
        loaded.game.host().message(me),
        g.game.host().message(me),
        "the loaded game tells what the saved one told"
    );
    h.run_until_voice_idle(&mut loaded);
    // As at the start of a game, the objective follows.
    assert_eq!(
        h.said_since(from),
        [
            "Loading".to_owned(),
            format!("loaded , {told}"),
            "objective: neutralized enemy".to_owned()
        ]
    );
    // Played on with the same commands, both stay the same.
    for turn in 0..40 {
        if turn == 5 {
            for game in [&mut g.game, &mut loaded.game] {
                game.host_mut()
                    .queue_line(me, &format!("control {}", peasants[0].0));
                game.host_mut().queue_line(me, "order 0 0 go 0");
            }
        }
        let a = g.game.host_mut().step(me);
        let b = loaded.game.host_mut().step(me);
        assert_eq!(a.message, b.message, "turn {turn}");
        assert_eq!(
            g.game.host().world().digest_u64(),
            loaded.game.host().world().digest_u64()
        );
    }
    assert_eq!(loaded.game.host().record(), g.game.host().record());
    // Saved again, the loaded game's record goes on from the first.
    let again = read_save(&save_new(&mut h, &mut loaded)).unwrap();
    assert_eq!(&again.record, g.game.host().record());
}

#[test]
fn the_load_menu_lists_the_saves_newest_first_and_delete_asks_first() {
    let script = Script::new()
        .keep_open()
        .at(1.0)
        .tap(Key::Down)
        .at(2.0)
        .tap(Key::Down)
        .at(3.0)
        .tap(Key::Delete)
        // Cancelled: the save stays.
        .at(4.0)
        .tap(Key::Escape)
        .at(5.0)
        .tap(Key::Down)
        .at(6.0)
        .tap(Key::Delete)
        .at(7.0)
        .tap(Key::Down)
        .at(8.0)
        .tap(Key::Return)
        .at(9.0)
        .tap(Key::Down)
        .at(10.0)
        .tap(Key::Down)
        .at(11.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    // A game of a few turns, played without an interface.
    let m = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&m));
    let setup = soundrts_world::GameSetup {
        rules: h.client.res.rules_text(),
        ai: h.client.res.ai_text(),
        map: m.definition.clone(),
        seed: 1,
        controllers: vec![
            soundrts_world::Controller::human("player", true),
            soundrts_world::Controller::computer("quiet"),
        ],
        default_triggers: soundrts_world::multiplayer_default_triggers(),
        allow_cheatmode: true,
        random_starts: true,
        equivalents: true,
    };
    h.client.res.set_map(None);
    let mut host = Host::new(&setup).unwrap();
    for _ in 0..3 {
        host.step(host.seats()[0]);
    }
    let record = host.record().clone();
    let dir = h.client.paths.saves();
    let info = SaveInfo {
        game: SavedGame::Training {
            map: "jl1".to_owned(),
        },
        title: vec![MsgPart::Text("jl1".to_owned())],
    };
    let now = wall_now();
    let ui = InterfaceState::default();
    write_save(&dir, &info, &record, &ui, now - 7200, Slot::New).unwrap();
    write_save(&dir, &info, &record, &ui, now - 10, Slot::LastGame).unwrap();
    let from = h.utterances().len();
    load_menu(&mut h.client).unwrap();
    assert_eq!(
        h.said_since(from),
        [
            "load a game",
            "last game , jl1 , 0 minutes , saved just now",
            "jl1 , 0 minutes , saved 2 hours ago",
            "delete this saved game?",
            "cancel",
            "load a game",
            "last game , jl1 , 0 minutes , saved just now",
            "delete this saved game?",
            "delete ,",
            "delete",
            "deleted",
            "load a game",
            "jl1 , 0 minutes , saved 2 hours ago",
            "go back to the previous menu ,",
            "go back to the previous menu",
        ]
    );
    let left = list_saves(&dir);
    assert_eq!(left.len(), 1);
    assert!(!left[0].header.last_game);
}

#[test]
fn escape_while_loading_gives_up_and_another_version_is_refused() {
    let (mut h, mut g) = training();
    h.run_until_turn(&mut g, 120);
    let path = save_new(&mut h, &mut g);
    h.press(Key::Escape, Mods::NONE);
    let from = h.utterances().len();
    load_game(&mut h.client, &path).unwrap();
    assert_eq!(h.said_since(from), ["Loading", "loading stopped"]);
    // A save of another version.
    let text = std::fs::read_to_string(&path).unwrap();
    let other = text.replacen(
        &format!("\"sim_version\":{SIM_VERSION}}}"),
        &format!("\"sim_version\":{}}}", SIM_VERSION + 1),
        1,
    );
    assert_ne!(other, text);
    std::fs::write(&path, other).unwrap();
    let from = h.utterances().len();
    load_game(&mut h.client, &path).unwrap();
    assert_eq!(
        h.said_since(from),
        ["cannot load , made by another version of the game"]
    );
}

/// A save cut at every byte of its last lines loads to a whole earlier
/// turn, or is refused: it never panics.
#[test]
fn a_save_cut_short_loads_to_a_whole_earlier_turn_or_is_refused() {
    let (mut h, mut g) = training();
    for turn in [5, 9, 14, 20] {
        h.run_until_turn(&mut g, turn);
        g.game
            .send_command(&h.client, Command::Cheat(Cheat::ToggleMode));
        g.game.save_game(&mut h.client, Slot::LastGame).unwrap();
    }
    let bytes = std::fs::read(h.client.paths.saves().join("last-training.sav")).unwrap();
    let whole = read_save_bytes(&bytes).unwrap();
    let ends: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'\n')
        .map(|(i, _)| i + 1)
        .collect();
    let from = ends[ends.len() - 5];
    let mut replays = std::collections::BTreeSet::new();
    for cut in from..=bytes.len() {
        let s = read_save_bytes(&bytes[..cut]).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        assert!(s.record.turns_run() <= whole.record.turns_run());
        if replays.insert((s.record.turns_run(), s.record.turns().len())) {
            let host = Host::replay(&s.record, |_| ControlFlow::Continue(()));
            assert!(host.is_ok(), "cut {cut}");
        }
    }
    assert!(replays.len() >= 3, "{replays:?}");
    // Cut in the save's own line or the record's set-up, it is refused.
    for cut in [1, ends[0] - 1, ends[0] + 3, ends[2] - 1] {
        assert!(read_save_bytes(&bytes[..cut]).is_err(), "cut {cut}");
    }
}

#[test]
fn the_single_player_menu_offers_loading_only_when_there_is_a_save() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let labels = |h: &mut Harness| -> Vec<String> {
        let menu = crate::singleplayer::build_single_player_menu(&h.client);
        menu.choices.iter().map(|c| said(h, &c.label)).collect()
    };
    let before = labels(&mut h);
    assert!(!before.contains(&"load a game".to_owned()), "{before:?}");
    let saves = h.client.paths.saves();
    std::fs::write(saves.join("x.sav"), b"").unwrap();
    let after = labels(&mut h);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after[after.len() - 2], "load a game");
}

#[test]
fn a_mission_saved_loads_in_its_campaign_with_its_resources() {
    // The keys run out a few seconds into the loaded game, and the window
    // closes.
    let mut h = Harness::new("en", Script::new().at(40.0).tap(Key::F1));
    h.client.game.seed = Some(3);
    let campaign = h
        .client
        .res
        .campaigns()
        .into_iter()
        .find(|c| c.name == "campaign")
        .expect("the first campaign");
    let (number, map) = campaign
        .chapters
        .iter()
        .find_map(|c| match c {
            Chapter::Mission { number, map } => Some((*number, map.clone())),
            Chapter::CutScene { .. } => None,
        })
        .expect("a mission");
    h.client.res.set_campaign(Some(&campaign));
    let mut session = Session::mission(&h.client, Arc::new(map), &campaign, number);
    let mut g = TestGame {
        game: session.start(&mut h.client).expect("started"),
        session: Some(session),
    };
    h.client.input.clear();
    g.game.set_turn_due(h.client.clock.now());
    h.run_until_turn(&mut g, 4);
    let path = save_new(&mut h, &mut g);
    let me = g.game.me;
    let expected = g.game.host().message(me);
    drop(g);
    h.client.res.set_map(None);
    h.client.res.set_campaign(None);
    let header = read_save(&path).unwrap().header;
    assert_eq!(
        header.game,
        SavedGame::Mission {
            campaign: "campaign".to_owned(),
            chapter: number
        }
    );
    let from = h.utterances().len();
    let result = load_game(&mut h.client, &path);
    assert!(result.is_err(), "the window closed during the loaded game");
    let heard = h.said_since(from);
    // The cursor was on a1; the mission's objectives come with its first
    // turns, after the save.
    assert_eq!(
        heard[..3],
        ["Loading", "loaded , a 1", "new objective build 1 farm"]
    );
    // Closing the window saved the mission again, as the last game, from
    // where it was loaded.
    let last = read_save(&h.client.paths.saves().join("last-mission.sav")).unwrap();
    assert_eq!(last.header.game, header.game);
    let loaded = Host::replay(&last.record, |_| ControlFlow::Continue(())).unwrap();
    assert!(loaded.record().turns_run() >= 4);
    let first = read_save(&path).unwrap();
    assert_eq!(
        &last.record.turns()[..first.record.turns().len()],
        first.record.turns()
    );
    let at_save = Host::replay(&first.record, |_| ControlFlow::Continue(())).unwrap();
    assert_eq!(at_save.message(me), expected);
}

#[test]
fn a_save_without_a_local_player_first_is_refused() {
    let (mut h, g) = training();
    let mut setup = g.game.host().record().setup().clone();
    setup.controllers[0] = soundrts_world::Controller::human("player", false);
    let mut host = Host::new(&setup).unwrap();
    host.step_seats(&[]);
    let info = SaveInfo {
        game: SavedGame::Training {
            map: "jl1".to_owned(),
        },
        title: vec![MsgPart::Text("jl1".to_owned())],
    };
    let dir = h.client.paths.saves();
    let ui = InterfaceState::default();
    let path = write_save(&dir, &info, host.record(), &ui, wall_now(), Slot::New).unwrap();
    let from = h.utterances().len();
    load_game(&mut h.client, &path).unwrap();
    assert_eq!(
        h.said_since(from),
        ["Loading", "cannot load , the file is damaged"]
    );
}
