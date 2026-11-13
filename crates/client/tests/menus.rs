//! The main menu and the options menu, run with scripted keys on the real
//! game data. These check what the client sends to the speech backend (a
//! recording fake), not what a screen reader says.

use soundrts::input::{Key, Mods};
use soundrts::mainmenu::main_menu;
use soundrts::menu::{Action, Choice, Exit, first_letter_of};
use soundrts::testkit::{Harness, Script, SpeechEvent};
use soundrts_world::msg::MsgPart;

const TITLE: &str = "SoundRTS 1.3.8.1  , please make a selection";

#[test]
fn the_main_menu_says_its_title_and_the_arrows_walk_its_items() {
    let script = Script::new()
        .at(10.0)
        .tap(Key::Down)
        .at(20.0)
        .tap(Key::Down)
        .at(30.0)
        .tap(Key::Down)
        .at(40.0)
        .tap(Key::Up)
        .at(50.0)
        .tap(Key::Tab)
        .at(60.0)
        .tap_with(Key::Tab, Mods::SHIFT)
        .at(70.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken(),
        [
            TITLE,
            "single player, , play on your computer",
            "multiplayer, , play on the Internet",
            "options, , modify your parameters",
            "multiplayer, , play on the Internet",
            "options, , modify your parameters",
            "multiplayer, , play on the Internet",
            "quit,",
        ]
    );
    // Each move plays the select sound; the confirmation plays its own.
    let sounds: Vec<String> = h.audio.played().into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        sounds,
        ["6115", "6115", "6115", "6115", "6115", "6115", "6116"]
    );
    assert!(h.audio.played().iter().all(|(channel, _)| *channel != 0));
}

#[test]
fn up_first_selects_the_last_item_but_enter_then_does_nothing_as_upstream() {
    let script = Script::new()
        .at(10.0)
        .tap(Key::Up)
        .at(20.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(h.spoken(), [TITLE, "quit, , quit this program", "quit,"]);
}

#[test]
fn a_key_cuts_the_title_short_and_escape_confirms_quit() {
    let script = Script::new().at(0.5).tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.speech.events(),
        [
            SpeechEvent::Speak(TITLE.into()),
            SpeechEvent::Stop,
            SpeechEvent::Speak("quit,".into()),
        ]
    );
}

#[test]
fn closing_the_window_ends_the_menus() {
    let mut h = Harness::new("en", Script::new());
    assert_eq!(main_menu(&mut h.client), Err(Exit));
    assert_eq!(h.spoken(), [TITLE]);
}

#[test]
fn options_then_modify_login_with_typed_characters() {
    let script = Script::new()
        .at(5.0)
        .tap(Key::Down)
        .at(7.0)
        .tap(Key::Down)
        .at(10.0)
        .tap(Key::Down)
        .at(20.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::Down)
        .at(40.0)
        .tap(Key::Return)
        // Each key press clears the queue, so the letters are typed apart.
        .at(50.0)
        .type_text("b")
        .at(51.0)
        .type_text("o")
        .at(52.0)
        .type_text("b")
        .at(53.0)
        .type_text("-")
        .at(54.0)
        .tap(Key::Backspace)
        .at(55.0)
        .type_text("2")
        .at(60.0)
        .tap(Key::Return)
        .at(70.0)
        .tap(Key::Left)
        .at(80.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken(),
        [
            TITLE,
            "single player, , play on your computer",
            "multiplayer, , play on the Internet",
            "options, , modify your parameters",
            "options,",
            "options menu",
            "Modify your login",
            "Modify your login",
            "enter your new login name use letters and numbers only",
            "b . b",
            "o . b o",
            "b . b o b",
            // "-" is not allowed: a beep (a sound), then the login so far.
            ". b o b",
            "b o",
            "2 . b o 2",
            "bo2",
            "new login bo2",
            "options menu",
            "go back to the previous menu",
            TITLE,
            "quit,",
        ]
    );
    assert!(
        h.audio.played().contains(&(0, "1029".to_owned())),
        "the beep plays on the voice channel"
    );
    assert_eq!(h.client.config.login.as_str(), "bo2");
    let ini = std::fs::read_to_string(h.user_dir.path().join("SoundRTS.ini")).unwrap();
    assert!(ini.contains("\nlogin = bo2\n"), "{ini}");
}

#[test]
fn escape_while_typing_keeps_the_login() {
    let script = Script::new()
        .at(5.0)
        .tap(Key::Down)
        .at(7.0)
        .tap(Key::Down)
        .at(10.0)
        .tap(Key::Down)
        .at(20.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::Down)
        .at(40.0)
        .tap(Key::Return)
        .at(50.0)
        .type_text("x")
        .at(60.0)
        .tap(Key::Escape)
        .at(70.0)
        .tap(Key::Escape)
        .at(80.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken()[9..],
        [
            "x . x",
            "the current login was kept",
            "options menu",
            "go back to the previous menu",
            TITLE,
            "quit,",
        ]
    );
    assert_eq!(h.client.config.login.as_str(), "player");
}

#[test]
fn the_same_menu_speaks_french() {
    let script = Script::new()
        .at(5.0)
        .tap(Key::Down)
        .at(7.0)
        .tap(Key::Down)
        .at(10.0)
        .tap(Key::Down)
        .at(20.0)
        .tap(Key::Return)
        .at(30.0)
        .tap(Key::Down)
        .at(40.0)
        .tap(Key::Escape)
        .at(50.0)
        .tap(Key::Escape);
    let mut h = Harness::new("fr", script);
    assert_eq!(h.client.res.language, "fr");
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken(),
        [
            "SoundRTS 1.3.8.1  , Que souhaitez-vous ?",
            "Un seul joueur, , jouer seul sur votre ordinateur",
            "Multijoueur, , jouer à plusieurs sur Internet",
            "Options, , modifier vos paramètres",
            "Options,",
            "menu options",
            "Modifier votre pseudo.",
            "retour",
            "SoundRTS 1.3.8.1  , Que souhaitez-vous ?",
            "Quitter,",
        ]
    );
}

#[test]
fn a_letter_selects_the_next_item_starting_with_it() {
    let script = Script::new()
        .at(10.0)
        .type_text("q")
        .at(20.0)
        .type_text("o")
        .at(30.0)
        .type_text("z")
        .at(40.0)
        .type_text("Q")
        .at(50.0)
        .tap(Key::F2)
        .at(60.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken(),
        [
            TITLE,
            "quit, , quit this program",
            "options, , modify your parameters",
            // "z" matches nothing: silence. Shift+Q searches backwards.
            "quit, , quit this program",
            // F2 says the last help line of ui/menu_help.txt.
            "shift Tab to select backward",
            "quit,",
        ]
    );
}

#[test]
fn home_and_end_change_the_volume_and_say_it() {
    let script = Script::new()
        .at(10.0)
        .tap(Key::Home)
        .at(20.0)
        .tap(Key::End)
        .at(30.0)
        .tap(Key::End)
        .at(40.0)
        .tap(Key::Escape);
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        h.spoken(),
        [
            TITLE,
            "60 percent of the volume",
            "50 percent of the volume",
            "40 percent of the volume",
            "quit,",
        ]
    );
}

#[test]
fn test_first_letter() {
    let h = Harness::new("en", Script::new());
    let first =
        |label: Vec<MsgPart>| first_letter_of(&h.client, &Choice::new(label, Action::Nothing));
    assert_eq!(first(vec![MsgPart::Num(4031)]).as_deref(), Some("s"));
    assert_eq!(first(vec![MsgPart::from("4267")]).as_deref(), Some("c"));
    assert_eq!(first(vec![MsgPart::from("orc")]).as_deref(), Some("o"));
    assert_eq!(first(vec![MsgPart::from("Orc")]).as_deref(), Some("o"));
    assert_eq!(
        first(vec![MsgPart::from("4267"), MsgPart::from("editor")]).as_deref(),
        Some("c")
    );
    assert_eq!(
        first(vec![MsgPart::from("orc"), MsgPart::from("editor")]).as_deref(),
        Some("o")
    );
    // 1000 is a sound, not a text: it is skipped.
    assert_eq!(
        first(vec![
            MsgPart::Num(1000),
            MsgPart::from("orc"),
            MsgPart::from("editor")
        ])
        .as_deref(),
        Some("o")
    );
}

#[test]
fn in_debug_mode_the_log_has_what_the_voice_said() {
    let config = soundrts::config::Config {
        debug_mode: 1,
        ..Default::default()
    };
    let script = Script::new()
        .at(10.0)
        .tap(Key::Down)
        .at(20.0)
        .tap(Key::Escape);
    let mut h = Harness::with_config("en", script, config);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let said: Vec<String> = h
        .log_text()
        .lines()
        .filter_map(|l| l.split_once(" SAID: ").map(|(_, t)| t.to_owned()))
        .collect();
    assert_eq!(
        said,
        [TITLE, "single player, , play on your computer", "quit,"]
    );

    let mut h = Harness::new("en", Script::new().at(10.0).tap(Key::Escape));
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert!(!h.log_text().contains(" SAID: "));
}

#[test]
fn the_window_title_follows_the_mods() {
    let mut h = Harness::new("en", Script::new());
    assert_eq!(
        h.desktop.titles.borrow().last().map(String::as_str),
        Some("SoundRTS 1.3.8.1  ")
    );
    h.client.res.set_mods("orc,nonexistent");
    assert_eq!(h.client.res.mods, "orc");
    assert_eq!(
        h.desktop.titles.borrow().last().map(String::as_str),
        Some("SoundRTS 1.3.8.1 orc ")
    );
    assert!(
        h.client.res.rules_text().contains(
            &std::fs::read_to_string(soundrts::testkit::upstream_root().join("mods/orc/rules.txt"))
                .unwrap()
        )
    );
}
