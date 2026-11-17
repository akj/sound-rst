#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use crate::game::testing::{said, training};
use crate::input::{Key, Mods};
use crate::testkit::Harness;

/// The saves in the harness's folder.
fn saves(h: &Harness) -> Vec<SaveEntry> {
    list_saves(&h.client.paths.saves())
}

#[test]
fn save_game_in_the_game_menu_writes_a_save_and_the_game_goes_on() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    // A command given since the last turn is in the save.
    g.game
        .send_command(&h.client, Command::Cheat(Cheat::ToggleMode));
    let turn = g.game.view().turn();
    let from = h.utterances().len();
    h.press(Key::F10, Mods::NONE);
    h.step(&mut g);
    // From the top, up goes to the last item, "continue game".
    h.press(Key::Up, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Up, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Return, Mods::NONE);
    h.step(&mut g);
    h.run_until_voice_idle(&mut g);
    // The game goes on after "saved": the news of what the cheat shows
    // follows it.
    assert_eq!(
        h.said_since(from)[..5],
        [
            "Menu",
            "continue game ,",
            "save game ,",
            "save game",
            "saved"
        ]
    );
    let list = saves(&h);
    assert_eq!(list.len(), 1);
    let header = &list[0].header;
    assert_eq!(
        header.game,
        SavedGame::Training {
            map: "jl1".to_owned()
        }
    );
    assert_eq!(said(&mut h, &header.title), "jl1 1");
    assert!(!header.last_game);
    assert_eq!(header.game_time_ms, turn as u64 * 300);
    assert_eq!(header.cursor, g.game.place);
    assert!(wall_now() - header.saved_at < 60);
    let name = list[0]
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("jl1-0min-"), "{name}");
    assert!(name.ends_with(".sav"), "{name}");
    let saved = read_save(&list[0].path).unwrap();
    assert_eq!(saved.completeness, Completeness::Whole);
    // The game went on after the save.
    let now = g.game.host().record();
    assert_eq!(saved.record.setup(), now.setup());
    assert_eq!(saved.record.turns_run(), turn as u64);
    assert_eq!(
        saved.record.turns(),
        &now.turns()[..saved.record.turns().len()]
    );
    let last = saved.record.turns().last().unwrap();
    assert_eq!(last.number(), turn as u64);
    assert_eq!(
        last.commands(),
        [(g.game.me, Command::Cheat(Cheat::ToggleMode))]
    );
    // The game goes on.
    h.run_until_turn(&mut g, turn + 2);
    // Saved again, it is a second save.
    g.game.cmd_save_game(&mut h.client);
    assert_eq!(saves(&h).len(), 2);
}

#[test]
fn quitting_a_game_not_over_saves_it_as_the_last_game_and_a_game_over_is_not_saved() {
    let (mut h, mut g) = training();
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    h.press(Key::F10, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Down, Mods::NONE);
    h.step(&mut g);
    h.press(Key::Return, Mods::NONE);
    h.step(&mut g);
    assert_eq!(
        h.said_since(from),
        ["Menu", "cancel game ,", "cancel game"],
        "nothing is said of the save"
    );
    let list = saves(&h);
    assert_eq!(list.len(), 1);
    assert!(list[0].header.last_game);
    assert_eq!(list[0].path.file_name().unwrap(), "last-training.sav");
    let saved = read_save(&list[0].path).unwrap();
    assert!(
        saved
            .record
            .turns()
            .iter()
            .all(|t| t.commands().iter().all(|(_, c)| *c != Command::Quit)),
        "the save is from before the player quit"
    );
    // Once the player has left, the game is over: leaving again saves
    // nothing more.
    h.run_for(&mut g, 2.0);
    assert!(!g.game.goes_on());
    std::fs::remove_file(&list[0].path).unwrap();
    g.game.save_on_leaving(&mut h.client);
    assert!(saves(&h).is_empty());
}

#[test]
fn a_save_that_cannot_be_written_whole_leaves_the_good_one() {
    let (mut h, mut g) = training();
    g.game.save_on_leaving(&mut h.client);
    let path = h.client.paths.saves().join("last-training.sav");
    let good = std::fs::read(&path).unwrap();
    // The file written beside cannot be made.
    std::fs::create_dir(h.client.paths.saves().join("last-training.sav.tmp")).unwrap();
    h.run_until_turn(&mut g, 6);
    let e = g.game.save_game(&mut h.client, Slot::LastGame);
    assert!(e.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), good);
    // In the game menu, the player hears it.
    let from = h.utterances().len();
    std::fs::remove_dir_all(h.client.paths.saves()).unwrap();
    std::fs::write(h.client.paths.saves(), b"a file where the folder goes").unwrap();
    g.game.cmd_save_game(&mut h.client);
    assert_eq!(h.said_since(from), ["cannot save"]);
}

fn header(title: Msg) -> SaveHeader {
    SaveHeader {
        format: SAVE_FORMAT.to_owned(),
        sim_version: SIM_VERSION,
        game: SavedGame::Training {
            map: "jl1".to_owned(),
        },
        title,
        game_time_ms: 754_000,
        saved_at: 1_000_000,
        last_game: false,
        cursor: None,
        selection: Vec::new(),
    }
}

#[test]
fn the_list_says_the_title_the_game_time_and_the_age() {
    let (mut h, _g) = training();
    let title = vec![MsgPart::literal("jl1"), MsgPart::Name("numeral_1".into())];
    let mut e = header(title);
    let at = |h: &mut Harness, e: &SaveHeader, now| said(h, &spoken_entry(e, now));
    assert_eq!(
        at(&mut h, &e, 1_000_030),
        "jl1 1 , 12 minutes , saved just now"
    );
    assert_eq!(
        at(&mut h, &e, 1_000_000 + 3 * 3600 + 5),
        "jl1 1 , 12 minutes , saved 3 hours ago"
    );
    e.last_game = true;
    e.sim_version = SIM_VERSION + 1;
    assert_eq!(
        at(&mut h, &e, 1_000_000 + 2 * 86_400),
        "last game , jl1 1 , 12 minutes , saved 2 days ago , made by another version of the game"
    );
}

#[test]
fn a_file_name_says_the_map_the_game_time_and_when() {
    assert_eq!(utc_stamp(0), "1970-01-01-000000");
    assert_eq!(utc_stamp(1_763_200_000), "2025-11-15-094640");
    assert_eq!(utc_stamp(951_782_400 + 86_399), "2000-02-29-235959");
    let dir = tempfile::tempdir().unwrap();
    let game = SavedGame::Mission {
        campaign: "my campaign/../x".to_owned(),
        chapter: 2,
    };
    let path = new_file(dir.path(), &game, 125_000, 0);
    assert_eq!(
        path.file_name().unwrap(),
        "mycampaignx-2-2min-1970-01-01-000000.sav"
    );
    std::fs::write(&path, b"").unwrap();
    assert_eq!(
        new_file(dir.path(), &game, 125_000, 0).file_name().unwrap(),
        "mycampaignx-2-2min-1970-01-01-000000-2.sav"
    );
}

#[test]
fn a_damaged_or_hostile_save_is_refused_and_its_title_kept_to_words() {
    let (mut h, _g) = training();
    assert!(matches!(read_save_bytes(b""), Err(LoadError::NotASave)));
    assert!(matches!(
        read_save_bytes(b"{\"format\":\"x\"}\n"),
        Err(LoadError::NotASave)
    ));
    let mut e = header(vec![
        MsgPart::literal("<b>jl1\u{202e}</b>"),
        MsgPart::Literal("x".repeat(500)),
        MsgPart::literal("!!!"),
        MsgPart::Name("../Press Alt F4".to_owned()),
        MsgPart::Name("numeral_1".to_owned()),
    ]);
    let line = serde_json::to_string(&e).unwrap();
    let read = parse_header(line.as_bytes()).unwrap();
    assert_eq!(
        read.title,
        [
            MsgPart::literal("bjl1b"),
            MsgPart::Literal("x".repeat(MAX_TITLE_WORD_CHARS)),
            MsgPart::Name("numeral_1".to_owned())
        ]
    );
    e.title = vec![MsgPart::Name("you_are_capitalized".into()); MAX_TITLE_PARTS + 5];
    let line = serde_json::to_string(&e).unwrap();
    assert_eq!(
        parse_header(line.as_bytes()).unwrap().title.len(),
        MAX_TITLE_PARTS
    );
    e.selection = vec![EntityId(1); MAX_SELECTION + 1];
    let line = serde_json::to_string(&e).unwrap();
    assert!(parse_header(line.as_bytes()).is_none());
    // A header, then a record of another version.
    e.selection.clear();
    let other = format!(
        "{}\n{{\"format\":\"soundrts-record-1\",\"sim_version\":{}}}\n",
        serde_json::to_string(&e).unwrap(),
        SIM_VERSION + 1
    );
    let refused = read_save_bytes(other.as_bytes()).unwrap_err();
    assert_eq!(
        said(&mut h, &refused.spoken()),
        "cannot load , made by another version of the game"
    );
    let damaged = format!("{}\nhello\n", serde_json::to_string(&e).unwrap());
    let refused = read_save_bytes(damaged.as_bytes()).unwrap_err();
    assert_eq!(
        said(&mut h, &refused.spoken()),
        "cannot load , the file is damaged"
    );
}

#[test]
fn closing_the_window_during_a_game_saves_it_as_the_last_game() {
    // The script's keys run out after a few seconds of play, and the
    // window closes.
    let mut h = Harness::new("en", crate::testkit::Script::new().at(20.0).tap(Key::F1));
    h.client.game.seed = Some(1);
    let m = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&m));
    let players = crate::testkit::training_players(&h.client, &["quiet"]);
    let mut session = crate::session::Session::training(&h.client, m, players);
    assert!(session.run(&mut h.client).is_err(), "the window closed");
    let list = saves(&h);
    assert_eq!(list.len(), 1);
    assert!(list[0].header.last_game);
    assert!(list[0].header.game_time_ms > 0);
}

#[test]
fn the_command_line_check_replays_a_record_or_a_save_and_says_how_it_ended() {
    let (mut h, mut g) = training();
    h.run_until_turn(&mut g, 20);
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("game.jsonl");
    let mut bytes = Vec::new();
    g.game.host().record().write(&mut bytes).unwrap();
    std::fs::write(&record, &bytes).unwrap();
    let report = check_record(&record).unwrap();
    let (first, second) = report.split_once(".\n").unwrap();
    assert!(
        first.starts_with(&format!(
            "{}: 20 turns replayed, 0 min 6 s of game time, in ",
            record.display()
        )),
        "{report}"
    );
    assert_eq!(second, "In the game at the end: player, ai_quiet.\n");
    // A save reads as its record.
    let save = g.game.save_game(&mut h.client, Slot::New).unwrap();
    assert!(check_record(&save).unwrap().contains(": 20 turns replayed"));
    // Cut short, then of another version.
    std::fs::write(&record, &bytes[..bytes.len() - 3]).unwrap();
    assert!(
        check_record(&record)
            .unwrap()
            .contains("; the record was cut short and replayed to its last whole turn.\n")
    );
    let other = String::from_utf8(bytes).unwrap().replacen(
        &format!("\"sim_version\":{SIM_VERSION}"),
        "\"sim_version\":1",
        1,
    );
    std::fs::write(&record, other).unwrap();
    assert_eq!(
        check_record(&record),
        Err(format!(
            "{}: made by version 1 of the simulation, this is version {SIM_VERSION}",
            record.display()
        ))
    );
}

#[test]
fn a_unit_upgraded_in_a_turn_not_yet_taken_is_saved_as_the_unit_it_became() {
    upgraded_while_selected(false);
}

/// As a game just loaded, whose selection waits for its first end of turn.
#[test]
fn a_unit_upgraded_before_a_loaded_game_takes_its_selection_back_is_saved_as_it_became() {
    upgraded_while_selected(true);
}

/// Saves a game whose selected archer became a dark archer in a turn the
/// interface has not taken; `loaded`: the selection is a loaded game's,
/// not yet taken back.
fn upgraded_while_selected(loaded: bool) {
    use crate::game::testing::{cheat, unit};
    let (mut h, mut g) = training();
    let me = g.game.me;
    let townhall = unit(&g, me, "townhall");
    let home = g
        .game
        .host()
        .world()
        .entity(townhall)
        .unwrap()
        .square()
        .unwrap();
    let home = g.game.view().square(home).name().to_owned();
    cheat(
        &mut g,
        me,
        &format!("add_units {home} lumbermill magestower archer"),
    );
    let turn = g.game.view().turn() + 2;
    h.run_until_turn(&mut g, turn);
    let archer = unit(&g, me, "archer");
    if loaded {
        let _ = g.game.resume(InterfaceState {
            cursor: None,
            selection: vec![archer],
        });
    } else {
        g.game.group = vec![archer];
    }
    g.game
        .host_mut()
        .queue_line(me, &format!("control {archer}"));
    g.game
        .host_mut()
        .queue_line(me, "order 0 0 upgrade_to darkarcher");
    // Turns run until the world has the dark archer, without the
    // interface taking them: the turn that replaced it waits.
    for _ in 0..2000 {
        if g.game.host().world().entity(archer).is_none() {
            break;
        }
        g.game.run_turn(&mut h.client);
    }
    let selected: &[EntityId] = if loaded { &[] } else { &[archer] };
    assert_eq!(g.game.group, selected, "the turn is not taken yet");
    let dark = unit(&g, me, "darkarcher");
    let path = g.game.save_game(&mut h.client, Slot::New).unwrap();
    assert_eq!(read_save(&path).unwrap().header.selection, [dark]);
}
