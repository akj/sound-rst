#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use soundrts_world::{Controller, GameSetup, Host, SIM_VERSION, multiplayer_default_triggers};

use super::*;
use crate::clock::Clock;
use crate::input::{Key, Mods};
use crate::saves::{InterfaceState, Slot, write_save};
use crate::session::Session;
use crate::testkit::{Harness, LiveKeys, Script};

/// The record of a training game on jl1 against a quiet computer, of
/// `turns` turns, played without an interface.
fn record(h: &mut Harness, turns: u64) -> Record {
    let m = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&m));
    let setup = GameSetup {
        rules: h.client.res.rules_text(),
        ai: h.client.res.ai_text(),
        map: m.definition.clone(),
        seed: 1,
        controllers: vec![
            Controller::human("player", true),
            Controller::computer("quiet"),
        ],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: true,
        computer_skill: soundrts_world::Skill::NORMAL,
        random_starts: true,
        equivalents: true,
    };
    h.client.res.set_map(None);
    let mut host = Host::new(&setup).unwrap();
    for _ in 0..turns {
        host.step(host.seats()[0]);
    }
    host.record().clone()
}

fn jl1() -> SaveInfo {
    SaveInfo {
        game: SavedGame::Training {
            map: "jl1".to_owned(),
        },
        title: vec![MsgPart::Text("jl1".to_owned())],
    }
}

#[test]
fn the_list_has_the_saves_and_the_records_newest_first() {
    let mut h = Harness::new("en", Script::new());
    let record = record(&mut h, 3);
    let now = wall_now();
    let ui = InterfaceState::default();
    let (saves, records) = (h.client.paths.saves(), h.client.paths.records());
    write_save(&saves, &jl1(), &record, &ui, now - 7200, Slot::New).unwrap();
    write_save(&saves, &jl1(), &record, &ui, now - 60, Slot::LastGame).unwrap();
    write_save(&records, &jl1(), &record, &ui, now - 10, Slot::LastGame).unwrap();
    write_save(&records, &jl1(), &record, &ui, now - 86_400 * 3, Slot::New).unwrap();
    let server = records.join("game-000001700000000-room3.jsonl");
    std::fs::write(&server, record.to_bytes()).unwrap();
    // Not a record: a file of another kind is not listed.
    std::fs::write(records.join("notes.txt"), b"hello").unwrap();
    let list = list_listenable(&saves, &records);
    let said: Vec<String> = list
        .iter()
        .map(|e| crate::game::testing::said(&mut h, &spoken(e, now)))
        .collect();
    assert_eq!(
        said,
        [
            "game 000001700000000 room3 , saved just now",
            "last finished game , jl1 , 0 minutes , saved just now",
            "last game , jl1 , 0 minutes , saved 1 minutes ago",
            "jl1 , 0 minutes , saved 2 hours ago",
            "jl1 , 0 minutes , saved 3 days ago",
        ]
    );
    assert!(any_to_listen(&h.client));
}

#[test]
fn a_record_is_listened_to_its_end_then_the_score_and_the_menu() {
    let script = Script::new()
        .keep_open()
        .at(1.0)
        .tap(Key::Down)
        .at(2.0)
        .tap(Key::Return);
    let mut h = Harness::new("en", script);
    let record = record(&mut h, 20);
    let records = h.client.paths.records();
    std::fs::create_dir_all(&records).unwrap();
    std::fs::write(records.join("friend.jsonl"), record.to_bytes()).unwrap();
    let from = h.utterances().len();
    listen_menu(&mut h.client).unwrap();
    let said = h.said_since(from);
    assert_eq!(said[..2], ["listen to a game", "friend , saved just now ,"]);
    let watching = said.iter().position(|s| s == "watching player").unwrap();
    assert_eq!(said[watching + 1], "objective: neutralized enemy");
    let end = said
        .iter()
        .position(|s| s == "end of the recording")
        .unwrap();
    assert_eq!(said[end + 1], "Not over at 0 minutes 6 seconds");
    assert!(said.last().unwrap().starts_with("score "), "{said:?}");
}

#[test]
fn a_record_of_another_version_or_damaged_is_refused_with_the_reason() {
    let mut h = Harness::new("en", Script::new());
    let record = record(&mut h, 3);
    let records = h.client.paths.records();
    std::fs::create_dir_all(&records).unwrap();
    let text = String::from_utf8(record.to_bytes()).unwrap();
    let other = text.replacen(
        &format!("\"sim_version\":{SIM_VERSION}}}"),
        &format!("\"sim_version\":{}}}", SIM_VERSION + 1),
        1,
    );
    assert_ne!(other, text);
    let cases = [
        (
            other.as_bytes(),
            "cannot load , made by another version of the game",
        ),
        (
            &b"{\"format\":\"soundrts-record-1\"\nnonsense\n"[..],
            "cannot load , the file is damaged",
        ),
        (
            &b"not a game at all\n"[..],
            "cannot load , the file is damaged",
        ),
    ];
    for (bytes, words) in cases {
        let path = records.join("x.jsonl");
        std::fs::write(&path, bytes).unwrap();
        let entry = Listenable {
            path,
            what: What::Record { modified: 0 },
        };
        let from = h.utterances().len();
        listen(&mut h.client, &entry).unwrap();
        assert_eq!(h.said_since(from), [words]);
    }
}

#[test]
fn a_game_that_ends_is_kept_and_listening_to_it_keeps_it_for_good() {
    let live = LiveKeys::default();
    let mut h = Harness::new("en", Script::new().keep_open().with_live(live.clone()));
    // The player quits from the game menu once the objective is told.
    live.tap_at(5.0, Key::F10, Mods::NONE);
    live.tap_at(5.5, Key::Down, Mods::NONE);
    live.tap_at(6.0, Key::Return, Mods::NONE);
    h.client.game.seed = Some(1);
    let map = h.multiplayer_map("jl1");
    let players = crate::testkit::training_players(&h.client, &["easy"]);
    Session::training(&h.client, map, players)
        .run(&mut h.client)
        .unwrap();
    let records = h.client.paths.records();
    let list = list_listenable(&h.client.paths.saves(), &records);
    let finished = list
        .iter()
        .find(|e| matches!(&e.what, What::Kept(h) if h.last_game))
        .expect("the last finished game")
        .clone();
    assert_eq!(finished.path, records.join("last-training.sav"));
    let now = wall_now();
    let said = crate::game::testing::said(&mut h, &spoken(&finished, now));
    assert!(
        said.starts_with("last finished game , jl1 1 , 0 minutes"),
        "{said}"
    );
    // The game left is also the last game to load.
    assert!(
        list.iter()
            .any(|e| matches!(&e.what, What::Save(h) if h.last_game))
    );

    // Listening to it, the game menu keeps it in the records folder.
    let at = h.clock.now();
    live.tap_at(at + 0.5, Key::F10, Mods::NONE);
    live.tap_at(at + 1.0, Key::K, Mods::NONE);
    live.tap_at(at + 1.5, Key::Return, Mods::NONE);
    let from = h.utterances().len();
    listen(&mut h.client, &finished).unwrap();
    let said = h.said_since(from);
    assert!(
        said.contains(&"keep this game's record".to_owned()),
        "{said:?}"
    );
    assert!(
        said.contains(&"kept in the records folder".to_owned()),
        "{said:?}"
    );
    let kept: Vec<Listenable> = list_listenable(&h.client.paths.saves(), &records)
        .into_iter()
        .filter(|e| matches!(&e.what, What::Kept(h) if !h.last_game))
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(
        crate::game::testing::said(&mut h, &spoken(&kept[0], wall_now()))
            .starts_with("jl1 1 , 0 minutes , saved")
    );
}
