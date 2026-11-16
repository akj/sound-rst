//! The translations: the words of the port's own phrases in each language,
//! and `--untranslated`, which lists the phrases that a language lacks.
//!
//! The original's phrases come from the imported game data. The port's own
//! ([`Phrase::OWN`]) come from the repository's `text` folder, built into
//! the program. A language file in the player's folder overrides both.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use soundrts_world::msg::Phrase;

use crate::client::open_resources;
use crate::log::Log;
use crate::options::Options;
use crate::package::TextFile;
use crate::paths::Paths;
use crate::probe::read_config;
use crate::resource::Resources;
use crate::sound_cache::read_language_file;

/// The words of the port's own phrases in each language but English, one
/// file a language. Their English is [`Phrase::english`].
pub const PORT_LANGUAGE_FILES: &[(&str, &str)] = &[
    ("be", include_str!("../../../text/be.txt")),
    ("cs", include_str!("../../../text/cs.txt")),
    ("de", include_str!("../../../text/de.txt")),
    ("es", include_str!("../../../text/es.txt")),
    ("fr", include_str!("../../../text/fr.txt")),
    ("it", include_str!("../../../text/it.txt")),
    ("pl", include_str!("../../../text/pl.txt")),
    ("pt-BR", include_str!("../../../text/pt-BR.txt")),
    ("ru", include_str!("../../../text/ru.txt")),
    ("sk", include_str!("../../../text/sk.txt")),
    ("vi", include_str!("../../../text/vi.txt")),
    ("zh", include_str!("../../../text/zh.txt")),
];

/// The port's language file of `language`, if it has one.
pub fn port_language_file(language: &str) -> Option<TextFile> {
    let (language, text) = PORT_LANGUAGE_FILES.iter().find(|(l, _)| *l == language)?;
    Some(TextFile {
        path: format!("sound-rst/text/{language}.txt"),
        text: (*text).to_owned(),
    })
}

/// A phrase of English that a language lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub id: String,
    pub english: String,
    /// The note for translators, empty for a phrase without one.
    pub note: &'static str,
}

impl Missing {
    /// Whether the port added this phrase, rather than the original.
    pub fn is_the_ports(&self) -> bool {
        Phrase::from_id(&self.id).is_some_and(|p| Phrase::OWN.contains(&p))
    }
}

/// What a language lacks, against English.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Untranslated {
    /// How many phrases English has.
    pub english: usize,
    /// The phrases the language lacks: the port's own, then the
    /// original's, each in the order of English.
    pub missing: Vec<Missing>,
    /// Each id of the language that English lacks, with its file.
    pub unknown: Vec<(String, String)>,
    /// Each file of the language that is refused, with the reason.
    pub refused: Vec<(String, String)>,
}

/// What the language of `files` lacks against the English of
/// `english_files`. English is the port's own phrases, then the files.
pub fn untranslated(english_files: &[TextFile], files: &[TextFile]) -> Untranslated {
    let mut english: Vec<(String, String)> = Phrase::OWN
        .iter()
        .map(|p| (p.id().to_owned(), p.english().to_owned()))
        .collect();
    let mut index: HashMap<String, usize> = english
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.clone(), i))
        .collect();
    let mut result = Untranslated::default();
    for file in english_files {
        let Ok(lines) = read_language_file(&file.text) else {
            continue;
        };
        for (id, words) in lines {
            match index.get(id).and_then(|&i| english.get_mut(i)) {
                Some(known) => known.1 = words.to_owned(),
                None => {
                    index.insert(id.to_owned(), english.len());
                    english.push((id.to_owned(), words.to_owned()));
                }
            }
        }
    }
    let mut translated = HashSet::new();
    for file in files {
        match read_language_file(&file.text) {
            Ok(lines) => {
                for (id, _) in lines {
                    if !index.contains_key(id) {
                        result.unknown.push((file.path.clone(), id.to_owned()));
                    }
                    translated.insert(id.to_owned());
                }
            }
            Err(e) => result.refused.push((file.path.clone(), e)),
        }
    }
    result.english = english.len();
    result.missing = english
        .into_iter()
        .filter(|(id, _)| !translated.contains(id))
        .map(|(id, english)| Missing {
            note: Phrase::from_id(&id).map_or("", Phrase::note),
            id,
            english,
        })
        .collect();
    result
}

/// What `language` lacks in the game as `res` loads it.
fn untranslated_in(res: &Resources, language: &str) -> Untranslated {
    let english = res.files_of_language("en");
    if language == "en" {
        // The English is what the other languages translate.
        let mut all = untranslated(&english, &english);
        all.missing.clear();
        return all;
    }
    untranslated(&english, &res.files_of_language(language))
}

/// A language code that names a file: letters, digits, `-` and `_`.
fn is_language_code(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 20
        && language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The answer to `--untranslated <language>`: each phrase of the game
/// that the language lacks, as a line of a language file with the English
/// words, under its note. With `all`, how many each language lacks.
pub fn report(options: &Options, language: &str) -> Result<String, String> {
    if language != "all" && !is_language_code(language) {
        return Err(format!(
            "{language:?} is not a language code such as fr or pt-BR"
        ));
    }
    let paths = Paths::from_environment(options.root.as_deref()).map_err(|e| e.to_string())?;
    let log = Log::silent();
    let config = read_config(&paths, &log);
    // The game without mods: the phrases every player has.
    let res = open_resources(&paths, config.parameters.clone(), "", "", None, &log);
    let mut out = String::new();
    if language == "all" {
        for language in res.available_languages().iter().filter(|l| *l != "en") {
            let u = untranslated_in(&res, language);
            let _ = writeln!(
                out,
                "{language} lacks {} of the {} phrases.",
                u.missing.len(),
                u.english
            );
        }
        return Ok(out);
    }
    let u = untranslated_in(&res, language);
    let file = res.player_language_file(language);
    for (path, why) in &u.refused {
        let _ = writeln!(
            out,
            "The game refuses {path} whole, so none of its lines count: {why}."
        );
    }
    if u.missing.is_empty() {
        let _ = writeln!(out, "{language} has all the {} phrases.", u.english);
    } else {
        let _ = writeln!(
            out,
            "{language} lacks {} of the {} phrases. Copy the lines that you translate into {}, and put your words in place of the English.",
            u.missing.len(),
            u.english,
            file.display()
        );
    }
    let (port, original): (Vec<&Missing>, Vec<&Missing>) =
        u.missing.iter().partition(|m| m.is_the_ports());
    for (title, missing) in [
        ("The phrases of sound-rst", port),
        ("The phrases of the original SoundRTS", original),
    ] {
        if missing.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n# {title}: {} missing.", missing.len());
        for m in missing {
            for line in m.note.lines() {
                let _ = writeln!(out, "# {line}");
            }
            let _ = writeln!(out, "{} = {}", m.id, m.english);
        }
    }
    if !u.unknown.is_empty() {
        let _ = writeln!(
            out,
            "\nThese lines name no phrase of the game, so the game never says them:"
        );
        for (path, id) in &u.unknown {
            let _ = writeln!(out, "{path}: {id}");
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> TextFile {
        TextFile {
            path: path.to_owned(),
            text: text.to_owned(),
        }
    }

    /// The repository's `text` folder holds one file for each language of
    /// [`PORT_LANGUAGE_FILES`], each file gives only the port's own
    /// phrases, and none gives a phrase twice. CI fails on any other line.
    #[test]
    fn the_ports_language_files_give_only_the_ports_phrases() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../text");
        let mut on_disk: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = PORT_LANGUAGE_FILES
            .iter()
            .map(|(l, _)| format!("{l}.txt"))
            .collect();
        listed.sort();
        assert_eq!(
            on_disk, listed,
            "each file of text/ needs a line in PORT_LANGUAGE_FILES, and English is Phrase::english"
        );
        let mut errors = Vec::new();
        for (language, text) in PORT_LANGUAGE_FILES {
            let lines = match read_language_file(text) {
                Ok(lines) => lines,
                Err(e) => {
                    errors.push(format!("text/{language}.txt, {e}"));
                    continue;
                }
            };
            let mut seen = HashSet::new();
            for (id, _) in lines {
                match Phrase::from_id(id) {
                    None => errors.push(format!("text/{language}.txt: {id} is no phrase")),
                    Some(p) if !Phrase::OWN.contains(&p) => errors.push(format!(
                        "text/{language}.txt: {id} is a phrase of the original, whose words come from its files"
                    )),
                    Some(_) if !seen.insert(id) => {
                        errors.push(format!("text/{language}.txt: {id} is given twice"))
                    }
                    Some(_) => {}
                }
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn a_language_lacks_each_english_phrase_that_none_of_its_files_gives() {
        let own = Phrase::OWN[0];
        let english = [
            file("res/text/en.txt", "hello = hello\nbye = bye\n"),
            file("me/text/en.txt", "hello = hi there\n"),
        ];
        let french = [
            file("res/text/fr.txt", "bye = au revoir\n"),
            file(
                "me/text/fr.txt",
                &format!("{} = mot\nhelo = typo\n", own.id()),
            ),
            file("bad/text/fr.txt", "no equals sign\n"),
        ];
        let u = untranslated(&english, &french);
        assert_eq!(u.english, Phrase::OWN.len() + 2);
        assert_eq!(u.missing.len(), Phrase::OWN.len());
        assert!(!u.missing.iter().any(|m| m.id == own.id()));
        let hello = u.missing.last().unwrap();
        assert_eq!(
            (hello.id.as_str(), hello.english.as_str(), hello.note),
            ("hello", "hi there", "")
        );
        assert!(!hello.is_the_ports());
        assert!(u.missing[0].is_the_ports());
        assert_eq!(u.missing[0].english, Phrase::OWN[1].english());
        assert_eq!(u.missing[0].note, Phrase::OWN[1].note());
        assert_eq!(
            u.unknown,
            [("me/text/fr.txt".to_owned(), "helo".to_owned())]
        );
        assert_eq!(
            u.refused,
            [(
                "bad/text/fr.txt".to_owned(),
                "line 1: not 'name = words'".to_owned()
            )]
        );
    }

    #[test]
    fn a_language_code_names_a_file_and_nothing_else() {
        for good in ["fr", "pt-BR", "zh_TW"] {
            assert!(is_language_code(good), "{good}");
        }
        for bad in ["", "../fr", "fr.txt", "a b", "f/r"] {
            assert!(!is_language_code(bad), "{bad}");
        }
    }
}
