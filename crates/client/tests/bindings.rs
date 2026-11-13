//! The game's bindings with the player's override file, read through the
//! resource stack on the real game data.

use soundrts::bindings::{Bindings, Chord, Command, game_bindings_text};
use soundrts::testkit::{Harness, Script};

fn command(name: &str, args: &str) -> Command {
    Command {
        name: name.into(),
        args: args.split_whitespace().map(str::to_owned).collect(),
    }
}

#[test]
fn the_players_bindings_file_overrides_the_games() {
    let h = Harness::new("en", Script::new());
    let custom = h.client.paths.custom_bindings();
    std::fs::write(&custom, "SHIFT s: escape\r\nF12: say_time\r\n").unwrap();
    let mut b = Bindings::new();
    b.load(
        &game_bindings_text(&h.client.res, &custom),
        &|_| true,
        &h.client.log,
    );
    let get = |s: &str| b.get(&Chord::parse(s).unwrap()).cloned();
    assert_eq!(get("SHIFT s"), Some(command("escape", "")));
    assert_eq!(get("F12"), Some(command("say_time", "")));
    assert_eq!(get("CTRL SHIFT F2"), Some(command("toggle_cheatmode", "")));
}

#[test]
fn without_a_players_file_the_games_bindings_stand() {
    let h = Harness::new("en", Script::new());
    let mut b = Bindings::new();
    b.load(
        &game_bindings_text(&h.client.res, &h.client.paths.custom_bindings()),
        &|_| true,
        &h.client.log,
    );
    assert_eq!(
        b.get(&Chord::parse("SHIFT s").unwrap()),
        Some(&command(
            "select_units",
            "local worker soldier1 soldier2 soldier3 soldier4 soldier5 soldier6 soldier7"
        ))
    );
    // 172 binding lines; "d" and "CTRL d" are each bound twice.
    assert_eq!(b.len(), 170);
}
