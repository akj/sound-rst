//! Every command of the game's bindings is known to the interface and
//! reads its arguments, on the real `res/ui/bindings.txt`.

use soundrts::bindings::{Bindings, game_bindings_text};
use soundrts::game::command::{COMMAND_NAMES, GameCommand};
use soundrts::log::Log;
use soundrts::testkit::{Harness, Script, upstream_root};

/// Commands whose feature this client leaves out, and what they do.
const EXCLUDED: &[(&str, &str)] = &[];

/// Upstream methods that a player's own bindings could name, which this
/// client does not have (sound debugging).
const NOT_PORTED: &[&str] = &["select_sound", "sound_volume"];

#[test]
fn every_command_of_bindings_txt_is_parsed_into_a_handled_command() {
    let text = std::fs::read_to_string(upstream_root().join("res/ui/bindings.txt")).unwrap();
    // Every distinct name, as written in the file.
    let mut names: Vec<String> = Vec::new();
    let mut raw: Bindings<String> = Bindings::new();
    raw.load(&text, &|name, _| Ok(name.to_owned()), &Log::silent());
    for (_, name) in raw.iter() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names.sort();
    assert_eq!(names.len(), 44, "{names:?}");
    for n in &names {
        assert!(COMMAND_NAMES.contains(&n.as_str()), "{n} is not known");
    }
    // Each binding, with its arguments, reads into a command that runs.
    let h = Harness::new("en", Script::new());
    let mut typed: Bindings<GameCommand> = Bindings::new();
    let text = game_bindings_text(&h.client.res, &h.client.paths.custom_bindings());
    typed.load(&text, &GameCommand::parse, &h.client.log);
    // The client's own bindings come on top of the file's.
    let mut own: Bindings<String> = Bindings::new();
    own.load(
        soundrts::bindings::CLIENT_BINDINGS,
        &|name, _| Ok(name.to_owned()),
        &Log::silent(),
    );
    assert_eq!(typed.len(), raw.len() + own.len());
    for (chord, c) in typed.iter() {
        assert!(!matches!(c, GameCommand::Broken { .. }), "{chord:?}: {c:?}");
    }
    assert_eq!(h.log_text(), "", "no line of bindings.txt was ignored");
    for (name, _) in EXCLUDED {
        assert!(names.iter().any(|n| n == name), "{name} is bound");
    }
    for name in NOT_PORTED {
        assert!(GameCommand::parse(name, Vec::new()).is_err());
    }
}
