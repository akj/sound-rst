//! The game's bindings with the player's override file, read through the
//! resource stack on the real game data.

use soundrts::bindings::{Bindings, Chord, game_bindings_text};
use soundrts::game::command::GameCommand;
use soundrts::testkit::{Harness, Script};

fn command(name: &str, args: &str) -> GameCommand {
    GameCommand::parse(name, args.split_whitespace().map(str::to_owned).collect()).unwrap()
}

fn load(h: &Harness) -> Bindings<GameCommand> {
    let mut b = Bindings::new();
    b.load(
        &game_bindings_text(&h.client.res, &h.client.paths.custom_bindings()),
        &GameCommand::parse,
        &h.client.log,
    );
    b
}

#[test]
fn the_players_bindings_file_overrides_the_games() {
    let h = Harness::new("en", Script::new());
    std::fs::write(
        h.client.paths.custom_bindings(),
        "SHIFT s: escape\r\nF12: say_time\r\n",
    )
    .unwrap();
    let b = load(&h);
    let get = |s: &str| b.get(&Chord::parse(s).unwrap()).cloned();
    assert_eq!(get("SHIFT s"), Some(GameCommand::Escape));
    assert_eq!(get("F12"), Some(GameCommand::SayTime));
    assert_eq!(get("CTRL SHIFT F2"), Some(GameCommand::ToggleCheatmode));
}

#[test]
fn without_a_players_file_the_games_bindings_stand() {
    let h = Harness::new("en", Script::new());
    let b = load(&h);
    assert_eq!(
        b.get(&Chord::parse("SHIFT s").unwrap()),
        Some(&command(
            "select_units",
            "local worker soldier1 soldier2 soldier3 soldier4 soldier5 soldier6 soldier7"
        ))
    );
    // 172 binding lines; "d" and "CTRL d" are each bound twice. Then the
    // client's own three: reading the chat again, and chat to the allies.
    assert_eq!(b.len(), 173);
    assert_eq!(
        b.get(&Chord::parse("SHIFT F7").unwrap()),
        Some(&GameCommand::SayAllies)
    );
    assert_eq!(
        b.get(&Chord::parse("CTRL F5").unwrap()),
        Some(&GameCommand::ChatPrevious)
    );
}

#[test]
fn the_players_file_can_rebind_the_clients_own_keys() {
    let h = Harness::new("en", Script::new());
    std::fs::write(h.client.paths.custom_bindings(), "SHIFT F7: say\n").unwrap();
    let b = load(&h);
    assert_eq!(
        b.get(&Chord::parse("SHIFT F7").unwrap()),
        Some(&GameCommand::Say)
    );
}
