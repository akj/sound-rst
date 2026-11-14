//! The first campaign through the real menus: the prologue cut scene, then
//! chapter 1 won with console cheats, and the bookmark moving on; and a
//! mission's resources unloaded after it. This checks what the client
//! sends to its voice, not what a screen reader says.

mod common;

use common::{N, ScriptedPlayer, Step, install};
use soundrts::input::Key;
use soundrts::mainmenu::main_menu;
use soundrts::testkit::{Harness, LiveKeys, Script};

#[test]
fn the_prologue_then_chapter_one_won_with_cheats_moves_the_bookmark() {
    let script = Script::new()
        .keep_open()
        .at(5.0)
        .tap(Key::Down)
        .at(15.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::Down)
        .at(40.0)
        .tap(Key::Return)
        .at(55.0)
        .tap(Key::Down)
        .at(65.0)
        .tap(Key::Return);
    let live = LiveKeys::default();
    let mut h = Harness::new("en", script.with_live(live.clone()).deadline(5000.0));
    h.client.game.seed = Some(3);
    let ini = h.client.paths.campaigns_config();
    assert!(!ini.exists());
    let steps = vec![
        Step::Said("new objective build 1 barracks", 20),
        Step::Key("objectives", Key::F9, N),
        Step::Console("farm", 3.0, "a farm"),
        Step::Said("objective complete build 1 farm", 200),
        Step::Keys(
            "after",
            vec![
                (300.0, Key::Escape, N),
                (320.0, Key::Escape, N),
                (340.0, Key::Escape, N),
            ],
        ),
        Step::Console("barracks", 3.0, "a barracks"),
    ];
    let player = ScriptedPlayer::new(steps, live, h.utterance_log.clone());
    install(&mut h.client, &player);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let all = h.said_since(0);
    let p = player.borrow();
    // The campaign menu, the prologue (a cut scene), then chapter 1 with
    // its intro and objectives.
    assert_eq!(
        all[4..18],
        [
            "campaign",
            "campaign",
            "campaign",
            "prologue ,",
            "prologue",
            "prologue",
            "A horde of enemies has arrived from nowhere and destroys everything! Something needs to be done...",
            "chapter 1",
            "Your lord have sent you to build a base near the frontier. Don't wander too far, for a few enemies have been spotted nearby.",
            "a 1 , forest , 1 town hall and 1 peasant , 1 gold mine and 1 wood",
            "new objective build 1 farm",
            ", 1 wood at c 1",
            "new objective build 1 barracks",
            ", 1 wood at c 1",
        ]
    );
    assert_eq!(
        p.said_at("objectives"),
        ["objective: , build 1 farm , build 1 barracks ,"]
    );
    // A cheat adds a farm: the first objective is complete.
    let farm = p.said_at("farm");
    assert_eq!(farm[..3], ["Enter a command.", "a . a", "  . a "]);
    assert_eq!(
        farm[7..],
        ["a farm", "objective complete build 1 farm", "farm 2 at a 1"]
    );
    // A barracks: the cut scene, the second objective, the mission, the
    // score, then the victory menu (Escape: quit) and the menus.
    let end = p.said_at("barracks");
    let a = end.iter().position(|s| s == "a barracks").expect("typed");
    assert_eq!(
        end[a..],
        [
            "a barracks",
            "Nicely done!",
            "barracks 1 at a 1",
            "objective complete build 1 barracks",
            "barracks 1 at a 1",
            "mission complete",
            "barracks 1 at a 1",
            "Victory! at 0 minutes 11 seconds",
            "2 units produced , 0 lost , 0 neutralized",
            "4 building produced , 0 lost , 0 neutralized",
            "5 gold gathered , 0 consumed",
            "10 wood gathered , 0 consumed",
            "score 21 press F5 to listen again",
            "please make a selection",
            "Quit",
            "please make a selection",
            "go back to the previous menu",
            "SoundRTS 1.3.8.1  , please make a selection",
            "quit,",
        ]
    );
    // The prologue unlocked chapter 1; the victory unlocked chapter 2.
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "[campaign]\nchapter = 2\n\n"
    );
    assert_eq!(h.log_text(), "", "nothing logged");
}

#[test]
fn a_mission_packages_resources_are_unloaded_when_it_ends() {
    use std::io::Write;
    // A campaign whose only chapter is a map package (chapter 1 of the
    // first campaign): its resources are a layer while it is played.
    let dir = tempfile::tempdir().unwrap();
    let campaign = dir.path().join("packaged");
    std::fs::create_dir(&campaign).unwrap();
    let mut w = zip::ZipWriter::new(std::fs::File::create(campaign.join("0.zip")).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    w.start_file("map.txt", options).unwrap();
    let map = soundrts::testkit::upstream_root().join("single/campaign/1.txt");
    w.write_all(&std::fs::read(map).unwrap()).unwrap();
    w.finish().unwrap();
    let package = soundrts::package::Package::from_path(&campaign).unwrap();
    let campaign = std::rc::Rc::new(soundrts::campaign::Campaign::new(package, "packaged"));
    // The game menu, "cancel game", then "quit" after the defeat.
    let script = Script::new()
        .keep_open()
        .deadline(2000.0)
        .at(100.0)
        .tap(Key::F10)
        .at(105.0)
        .tap(Key::Down)
        .at(110.0)
        .tap(Key::Return)
        .at(300.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    h.client.res.set_campaign(Some(&campaign));
    assert_eq!(
        soundrts::campaign::run_chapter(&mut h.client, &campaign, 0),
        Ok(())
    );
    assert!(
        h.said_since(0).contains(&"Quit".to_owned()),
        "the mission ended"
    );
    let layers: Vec<&str> = h
        .client
        .res
        .layers()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(layers, ["default", "packaged"]);
}
