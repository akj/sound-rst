//! Every command of the game's bindings is known to the interface and
//! reads its arguments, on the real `res/ui/bindings.txt`.

use soundrts::bindings::{Bindings, game_bindings_text};
use soundrts::game::command::{GameCommand, is_command};
use soundrts::log::Log;
use soundrts::package::TextFile;
use soundrts::testkit::{Harness, Script, game_root};

/// One file of bindings.
fn file(path: &str, text: &str) -> [TextFile; 1] {
    [TextFile {
        path: path.to_owned(),
        text: text.to_owned(),
    }]
}

/// Commands whose feature this client leaves out, and what they do.
const EXCLUDED: &[(&str, &str)] = &[];

/// Upstream methods that a player's own bindings could name, which this
/// client does not have (sound debugging).
const NOT_PORTED: &[&str] = &["select_sound", "sound_volume"];

#[test]
fn every_command_of_bindings_txt_is_parsed_into_a_handled_command() {
    let text = std::fs::read_to_string(game_root().join("res/ui/bindings.txt")).unwrap();
    // Every distinct name, as written in the file.
    let mut names: Vec<String> = Vec::new();
    let mut raw: Bindings<String> = Bindings::new();
    raw.load(
        &file("res/ui/bindings.txt", &text),
        &|name, _| Ok(name.to_owned()),
        &Log::silent(),
    );
    for (_, name) in raw.iter() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names.sort();
    assert_eq!(names.len(), 44, "{names:?}");
    for n in &names {
        assert!(is_command(n), "{n} is not known");
    }
    // Each binding, with its arguments, reads into a command that runs.
    let h = Harness::new("en", Script::new());
    let mut typed: Bindings<GameCommand> = Bindings::new();
    let text = game_bindings_text(&h.client.res, &h.client.paths.custom_bindings());
    typed.load(&text, &GameCommand::parse, &h.client.log);
    // The client's own bindings come on top of the file's, and replace
    // its six modifiers pressed alone.
    let mut own: Bindings<String> = Bindings::new();
    own.load(
        &file("client", soundrts::bindings::CLIENT_BINDINGS),
        &|name, _| Ok(name.to_owned()),
        &Log::silent(),
    );
    let replaced = own.iter().filter(|(c, _)| raw.get(c).is_some()).count();
    assert_eq!(replaced, 6);
    assert_eq!(typed.len(), raw.len() + own.len() - replaced);
    assert_eq!(h.log_text(), "", "no line of bindings.txt was ignored");
    for (name, _) in EXCLUDED {
        assert!(names.iter().any(|n| n == name), "{name} is bound");
    }
    for name in NOT_PORTED {
        assert!(GameCommand::parse(name, Vec::new()).is_err());
    }
}
