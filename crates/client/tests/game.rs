//! A whole training game through the real menus, on the real game data,
//! with a recording voice and a test clock. These tests check what the
//! client sends to its voice (texts and recorded sounds), not what a screen
//! reader says.

mod common;

use common::{N, ScriptedPlayer, Step, install};
use soundrts::input::Key;
use soundrts::mainmenu::main_menu;
use soundrts::testkit::{Harness, LiveKeys, Script};

/// The position of `map` in the training menu's map list.
fn map_index(map: &str) -> usize {
    let mut h = Harness::new("en", Script::new());
    h.client
        .res
        .multiplayer_maps()
        .iter()
        .position(|m| m.name == map)
        .expect("the map exists")
}

fn peasant_orders(g: &soundrts::game::GameInterface) -> String {
    let p = g.world.player(g.me);
    p.units
        .iter()
        .filter_map(|&u| g.world.entity(u))
        .filter(|e| &*e.type_name == "peasant")
        .map(|e| {
            e.orders
                .iter()
                .map(|o| o.keyword.as_str())
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[test]
fn a_training_game_on_jl1_through_the_menus() {
    let index = map_index("jl1");
    let mut script = Script::new()
        .keep_open()
        .at(5.0)
        .tap(Key::Down)
        .at(15.0)
        .tap(Key::Return)
        .at(30.0)
        .type_text("s")
        .at(50.0)
        .tap(Key::Return);
    let mut t = 65.0;
    for _ in 0..=index {
        script = script.at(t).tap(Key::Down);
        t += 10.0;
    }
    script = script
        .at(t)
        .tap(Key::Return)
        .at(t + 15.0)
        .tap(Key::Down)
        .at(t + 25.0)
        .tap(Key::Return)
        .at(t + 40.0)
        .tap(Key::Down)
        .at(t + 50.0)
        .tap(Key::Return);
    let live = LiveKeys::default();
    let mut h = Harness::new("en", script.with_live(live.clone()).deadline(3000.0));
    h.client.game.seed = Some(1);
    let steps = vec![
        Step::Turn(3),
        Step::Key("west", Key::Left, N),
        Step::Key("east", Key::Right, N),
        Step::Key("north", Key::Up, N),
        Step::Key("gold", Key::Z, N),
        Step::Key("wood", Key::X, N),
        Step::Key("food", Key::C, N),
        Step::Key("select town hall", Key::W, N),
        Step::Key("train menu", Key::A, N),
        Step::Key("train", Key::Return, N),
        Step::Said("ready", 600),
        Step::Key("select peasant", Key::E, N),
        Step::Key("order go", Key::A, N),
        Step::Key("order gather", Key::A, N),
        Step::Key("to the gold mine", Key::Left, N),
        Step::KeyUntil("target", Key::Tab, N, "gold mine", 10),
        Step::Key("gather", Key::Return, N),
        Step::Probe("gathering", peasant_orders),
        Step::Key("back home", Key::Right, N),
        Step::Key("order build", Key::A, N),
        Step::Key("order build 2", Key::A, N),
        Step::Key("order build 3", Key::A, N),
        Step::Key("meadow", Key::Tab, N),
        Step::Key("build", Key::Return, N),
        Step::Probe("building", peasant_orders),
        Step::Probe("peasants", |g| {
            g.world
                .player(g.me)
                .units
                .iter()
                .filter(|&&u| {
                    g.world
                        .entity(u)
                        .is_some_and(|e| &*e.type_name == "peasant")
                })
                .count()
                .to_string()
        }),
        Step::Key("units", Key::Space, N),
        Step::Keys(
            "game menu",
            vec![
                (0.0, Key::F10, N),
                (3.0, Key::Down, N),
                (6.0, Key::Return, N),
                (400.0, Key::Escape, N),
                (420.0, Key::Escape, N),
            ],
        ),
    ];
    let player = ScriptedPlayer::new(steps, live, h.utterance_log.clone());
    install(&mut h.client, &player);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let p = player.borrow();
    let all = h.said_since(0);
    // The menus, up to the start of the game.
    let start = all
        .iter()
        .position(|s| s == "start")
        .expect("the game started");
    assert_eq!(
        all[..4],
        [
            "SoundRTS 1.3.8.1  , please make a selection",
            "single player, , play on your computer",
            "single player,",
            "please make a selection",
        ]
    );
    assert_eq!(
        all[start - 7..=start],
        [
            "jl1 1 ,",
            "jl1 1",
            "please make a selection",
            "invite quiet computer ,",
            "invite quiet computer",
            "please make a selection",
            "start ,",
            "start",
        ]
    );
    // The objective, the first square, then what was seen around, by
    // square.
    assert_eq!(
        all[start + 1..start + 6],
        [
            "objective: neutralized enemy",
            "b 1 , meadows , 1 town hall , 1 farm and 1 peasant",
            ", 1 gold mine at a 1",
            ", 1 wood at c 1",
            ", 1 gold mine and 2 wood at c 2",
        ]
    );
    // Squares: a path west (its sound first), no path north.
    assert_eq!(
        p.said_at("west"),
        ["<1091> a 1 , meadows , in the fog of war , 1 gold mine"]
    );
    assert_eq!(
        p.said_at("east"),
        ["<1091> b 1 , meadows , 1 town hall , 1 farm and 1 peasant"]
    );
    assert_eq!(
        p.said_at("north")[0],
        "<1027> b 1 , meadows , 1 town hall , 1 farm and 1 peasant"
    );
    // Status keys (news may follow what a key says).
    assert_eq!(p.said_at("gold")[0], "10 gold");
    assert_eq!(p.said_at("wood")[0], "10 wood");
    assert_eq!(p.said_at("food")[0], "9 food to 1");
    // Training a peasant: the order, the gold left, then the new peasant.
    assert_eq!(
        p.said_at("select town hall"),
        ["town hall 1 at b 1 , awaiting your orders"]
    );
    assert_eq!(
        p.said_at("train menu"),
        ["recruit peasant , requires 4 gold and 1 food , press enter to confirm"]
    );
    assert_eq!(
        p.said_at("train"),
        ["recruit peasant", "6 gold", "peasant 2 ready!"]
    );
    assert_eq!(p.note("peasants"), "2");
    // Gathering: the order menu, the target, the confirmation.
    assert_eq!(
        p.said_at("select peasant"),
        ["peasant 1 at b 1 , awaiting your orders"]
    );
    assert_eq!(
        p.said_at("order gather"),
        ["exploit , , press tab to select the target and then press enter to confirm"]
    );
    assert_eq!(
        p.said_at("target").last().map(String::as_str),
        Some("<1003> gold mine in the fog of war , to the southwest , contains 75 gold , exploit")
    );
    assert_eq!(
        p.said_at("gather"),
        ["exploit gold mine in the fog of war ,"]
    );
    assert_eq!(p.note("gathering"), "gather;");
    // Building a farm on a meadow.
    assert_eq!(
        p.said_at("order build 3"),
        [
            "build farm on , requires 5 gold and 5 wood , press tab to select the target and then press enter to confirm"
        ]
    );
    assert_eq!(
        p.said_at("meadow"),
        ["<1003> a meadow to the northwest , build farm on"]
    );
    assert_eq!(p.said_at("build"), ["build farm on a meadow"]);
    assert_eq!(p.note("building"), "build;");
    assert_eq!(
        p.said_at("units"),
        ["a 1 You control peasant 1 at a 1 , build farm on a meadow ,"]
    );
    // The game menu, the score, and the menus again.
    let end = p.said_at("game menu");
    assert_eq!(end[..3], ["Menu", "cancel game ,", "cancel game"]);
    assert!(end[3].starts_with("Defeat at 2 minutes "), "{}", end[3]);
    assert_eq!(end[4], "2 units produced , 0 lost , 0 neutralized");
    assert!(
        end[6].ends_with(" gold gathered , 9 consumed"),
        "{}",
        end[6]
    );
    assert_eq!(end[7], "10 wood gathered , 5 consumed");
    assert!(end[8].starts_with("score "), "{}", end[8]);
    assert_eq!(
        end[9..],
        [
            "please make a selection",
            "go back to the previous menu",
            "SoundRTS 1.3.8.1  , please make a selection",
            "quit,",
        ]
    );
    assert_eq!(h.log_text(), "", "nothing logged");
}
