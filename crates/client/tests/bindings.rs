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
fn a_refused_line_of_the_players_bindings_is_logged_with_its_file_and_its_line() {
    let h = Harness::new("en", Script::new());
    let custom = h.client.paths.custom_bindings();
    std::fs::write(
        &custom,
        "F12: say_time\r\n; the next key\r\n\r\nWIN a: escape\r\nF11: no_such_command\r\n",
    )
    .unwrap();
    let b = load(&h);
    assert_eq!(
        b.get(&Chord::parse("F12").unwrap()),
        Some(&GameCommand::SayTime)
    );
    let log = h.log_text();
    let custom = custom.display();
    for refused in [
        format!("{custom} line 4 is ignored ('WIN' is not an allowed key modifier): WIN a: escape"),
        format!(
            "{custom} line 5 is ignored ('no_such_command' is not a command): F11: no_such_command"
        ),
    ] {
        assert!(log.contains(&refused), "{refused} in:\n{log}");
    }
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
    // client's own forty-two, none of them a key of the original's:
    // reading the chat again, chat and a flare to the allies, the news log
    // and copying from it, the questions, key help, the previous
    // objective, the fourteen keys of idle soldiers, a watcher's two to
    // follow another player, and a listener's seven for the pace, the
    // pause and the jumps.
    assert_eq!(b.len(), 212);
    let news = [
        ("F4", GameCommand::Unheard),
        ("COMMA", GameCommand::ReviewPrevious),
        ("PERIOD", GameCommand::ReviewNext),
        ("SLASH", GameCommand::ReviewCategory),
        ("CTRL SHIFT c", GameCommand::CopyMessage),
        ("DELETE", GameCommand::JumpToAlert),
        ("b", GameCommand::ProductionStatus),
        ("n", GameCommand::WorkersStatus),
        ("m", GameCommand::FightStatus),
        ("SHIFT x", GameCommand::AllResourcesStatus),
        ("F12", GameCommand::PlayersStatus),
        ("CTRL F1", GameCommand::KeyHelp),
        ("F9", GameCommand::Objectives(1)),
        ("SHIFT F9", GameCommand::Objectives(-1)),
        ("SHIFT v", GameCommand::CombatStatus),
        ("SHIFT F11", GameCommand::SayFaction),
        ("CTRL F12", GameCommand::FollowPlayer(1)),
        ("CTRL SHIFT F12", GameCommand::FollowPlayer(-1)),
        ("CTRL F7", GameCommand::Flare),
        ("MINUS", GameCommand::ListenSpeed(-1)),
        ("EQUALS", GameCommand::ListenSpeed(1)),
        ("SHIFT SPACE", GameCommand::ListenPause),
        ("LEFTBRACKET", GameCommand::ListenJump(-1)),
        ("RIGHTBRACKET", GameCommand::ListenJump(1)),
        ("SHIFT LEFTBRACKET", GameCommand::ListenJump(-5)),
        ("SHIFT RIGHTBRACKET", GameCommand::ListenJump(5)),
    ];
    for (key, c) in news {
        assert_eq!(b.get(&Chord::parse(key).unwrap()), Some(&c), "{key}");
    }
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
