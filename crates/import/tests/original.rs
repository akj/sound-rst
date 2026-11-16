//! The import of the original game, as `scripts/fetch-upstream.ps1`
//! fetched it into `vendor/soundrts-upstream`, and the table of names
//! against it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use soundrts_import::names::{GAME, Row, names};
use soundrts_import::{Files, Report, import_game_files, numbers_of_the_original, read_zip};
use soundrts_world::msg::{Cue, Phrase, is_name};

fn original() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        soundrts_import::is_original_game(&root),
        "The original game is missing at {}. Run scripts/fetch-upstream.ps1 first.",
        root.display()
    );
    root
}

/// The files of the original's game folders, by path.
fn original_files() -> Files {
    let mut files = Files::new();
    for folder in ["res", "mods", "single"] {
        for (path, bytes) in soundrts_import::read_folder(&original().join(folder)).unwrap() {
            files.insert(format!("{folder}/{path}"), bytes);
        }
    }
    files
}

fn imported() -> (Files, Report) {
    let mut report = Report::default();
    let files = import_game_files(&original_files(), &mut report).unwrap();
    (files, report)
}

fn text(files: &Files, path: &str) -> String {
    let bytes = files.get(path).unwrap_or_else(|| panic!("no {path}"));
    String::from_utf8_lossy(bytes).into_owned()
}

/// The names that the language files and the recordings of a package
/// give: phrases, and sounds.
fn names_of(files: &Files, prefix: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (path, bytes) in files {
        let Some(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        if rest.starts_with("text/") {
            for line in String::from_utf8_lossy(bytes).lines() {
                let (name, _) = line.split_once(" = ").unwrap();
                assert!(is_name(name) && !name.contains('/'), "{path}: {line}");
                found.insert(name.to_owned());
            }
        }
        let sound = rest
            .split_once("sounds/")
            .filter(|(before, _)| before.is_empty() || before.starts_with("lang/"));
        if let Some((_, name)) = sound {
            let name = name.strip_suffix(".ogg").unwrap();
            // A recording in a language can stand for a phrase of the English.
            assert!(is_name(name), "{path}");
            found.insert(name.to_owned());
        }
    }
    found
}

#[test]
fn the_88_maps_the_mods_and_the_campaigns_import_without_an_error() {
    let (files, report) = imported();
    // One map of the original names a number that is nothing.
    assert_eq!(
        report.warnings,
        [
            "res/multi/pm6.txt line 11: 2 names no phrase and no sound; it stays a number, said as written"
        ]
    );
    let maps: Vec<&String> = files
        .keys()
        .filter(|p| p.starts_with("res/multi/"))
        .collect();
    // 88 maps that are one text file, and one with files of its own.
    assert_eq!(maps.iter().filter(|p| p.ends_with(".txt")).count(), 88);
    assert_eq!(maps.iter().filter(|p| p.ends_with(".zip")).count(), 1);
    // Every file of the original becomes one file.
    assert_eq!(files.len(), original_files().len() + 1);
    assert_eq!(text(&files, "import.txt").trim(), soundrts_import::stamp());
    for mods in ["mods/orc", "mods/flyingtownhall", "mods/teleportation"] {
        assert!(files.contains_key(&format!("{mods}/rules.txt")), "{mods}");
    }
    assert!(files.contains_key("mods/soundpack/sounds/footstep/grass_1.ogg"));
    assert!(files.contains_key("single/campaign/text/en.txt"));
    assert!(files.contains_key("single/campaign/lang/de/sounds/nicely_done.ogg"));
}

#[test]
fn every_word_that_names_something_is_a_name_that_exists_or_a_count() {
    let (files, _) = imported();
    let game = names_of(&files, "res/");
    let check = |path: &str, line: usize, word: &str, own: &BTreeSet<String>| {
        if word.bytes().all(|b| b.is_ascii_digit()) {
            return;
        }
        assert!(
            is_name(word),
            "{path} line {line}: {word} is no name and no count"
        );
        assert!(
            game.contains(word) || own.contains(word),
            "{path} line {line}: {word} does not exist"
        );
    };
    let mut titles = 0;
    for (path, bytes) in &files {
        let package = ["mods/", "single/"]
            .iter()
            .find(|g| path.starts_with(**g))
            .and_then(|g| {
                let name = path.strip_prefix(g)?.split('/').next()?;
                Some(format!("{g}{name}/"))
            });
        let own = package.map_or_else(BTreeSet::new, |p| names_of(&files, &p));
        let is_map = path.starts_with("res/multi/") && path.ends_with(".txt")
            || path.starts_with("single/") && path.ends_with(".txt") && {
                let stem = path.rsplit('/').next().unwrap().trim_end_matches(".txt");
                stem.bytes().all(|b| b.is_ascii_digit())
            };
        if is_map {
            let text = String::from_utf8_lossy(bytes);
            for (i, line) in text.lines().enumerate() {
                let code = line.split(';').next().unwrap();
                let mut words = code.split_whitespace();
                if matches!(
                    words.next(),
                    Some("title" | "objective" | "intro" | "sequence")
                ) {
                    titles += 1;
                    for word in words {
                        check(path, i + 1, word, &own);
                    }
                }
            }
        }
        if path.ends_with("ui/game_help.txt") || path.ends_with("ui/menu_help.txt") {
            let text = String::from_utf8_lossy(bytes);
            for (i, line) in text.lines().enumerate() {
                for word in line.split_whitespace() {
                    check(path, i + 1, word, &own);
                }
            }
        }
    }
    assert!(titles > 180, "{titles}");
}

#[test]
fn a_map_keeps_everything_but_the_numbers_that_name_something() {
    let (files, _) = imported();
    let before = original_files();
    let lines = |files: &Files, path: &str| -> Vec<String> {
        text(files, path).lines().map(str::to_owned).collect()
    };
    let (old, new) = (
        lines(&before, "single/campaign/1.txt"),
        lines(&files, "single/campaign/1.txt"),
    );
    assert_eq!(old.len(), new.len());
    let changed: Vec<(&String, &String)> = old.iter().zip(&new).filter(|(a, b)| a != b).collect();
    let pairs: Vec<(&str, &str)> = changed
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("title 4271 3001", "title chapter numeral_1"),
            ("intro 7501", "intro your_lord_has_sent_you"),
            (
                "trigger player1 (timer 0) (add_objective 1 101 151 83)",
                "trigger player1 (timer 0) (add_objective 1 build one_f farm)"
            ),
            (
                "trigger player1 (timer 0) (add_objective 2 101 151 99)",
                "trigger player1 (timer 0) (add_objective 2 build one_f barracks)"
            ),
            (
                "trigger player1 (has 2 farm barracks) (cut_scene 7502)",
                "trigger player1 (has 2 farm barracks) (cut_scene nicely_done)"
            ),
        ]
    );
    // A count of the original is the count itself.
    assert!(text(&before, "res/ui/game_help.txt").contains("1000010"));
    let help = text(&files, "res/ui/game_help.txt");
    assert!(help.starts_with("letter_f 10 to open_the_menu"), "{help}");
    // The rules and the computer players are not touched.
    for path in [
        "res/rules.txt",
        "res/ai.txt",
        "mods/orc/rules.txt",
        "res/ui/bindings.txt",
    ] {
        assert_eq!(files[path], before[path], "{path}");
    }
}

#[test]
fn a_style_keeps_its_syntax_and_names_its_phrases_and_sounds() {
    let (files, _) = imported();
    let style = text(&files, "res/ui/style.txt");
    for line in [
        "not_enough_resource_0 not_enough gold",
        "title build $1 on",
        "recommended_maps jl8 jl7 jl6 jl5",
        "move_on_grass footstep/grass_1 footstep/grass_2",
        "proportion_0 if_me voice/own/health_0 else voice/other/health_0",
    ] {
        let code = |l: &str| l.split(';').next().unwrap().trim_end().to_owned();
        assert!(style.lines().any(|l| code(l) == line), "{line}");
    }
    // A noise: the interval of a repeat and the volume of a loop stay.
    let noises: Vec<&str> = style.lines().filter(|l| l.starts_with("noise")).collect();
    for noise in [
        "noise loop ambience/path",
        "noise_if_damaged loop building/damaged .5 ; half volume",
        "noise repeat 15 ambience/farm",
        "noise_if_damaged if_me repeat 12 voice/own/hurt else repeat 12 voice/other/hurt",
    ] {
        assert!(noises.contains(&noise), "{noise}: {noises:#?}");
    }
    assert!(!style.lines().any(|l| {
        let code = l.split(';').next().unwrap();
        code.split_whitespace()
            .skip(1)
            .any(|w| w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit()))
    }));
    // A language keeps only what differs in it.
    let french = text(&files, "res/lang/fr/ui/style.txt");
    assert!(
        french.starts_with("def a_deadly_fog\nshortcut n\n"),
        "{french}"
    );
    assert_eq!(
        text(&files, "res/text/fr.txt").lines().next(),
        Some("nothing = Rien !")
    );
}

#[test]
fn a_map_with_files_of_its_own_is_a_zip_file_with_the_same_bytes_each_time() {
    let (files, _) = imported();
    let (again, _) = imported();
    assert_eq!(files["res/multi/jl4.zip"], again["res/multi/jl4.zip"]);
    let inside = read_zip(&files["res/multi/jl4.zip"]).unwrap();
    let paths: Vec<&str> = inside.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        [
            "ai.txt",
            "map.txt",
            "readme.txt",
            "rules.txt",
            "sounds/creature/stone_golem.ogg",
            "text/cs.txt",
            "text/de.txt",
            "text/en.txt",
            "text/fr.txt",
            "text/it.txt",
            "text/pt-BR.txt",
            "text/ru.txt",
            "text/sk.txt",
            "ui/bindings.txt",
            "ui/style.txt",
        ]
    );
    assert_eq!(text(&inside, "text/en.txt"), "stone_golem = stone golem\n");
    let style = text(&inside, "ui/style.txt");
    assert!(style.contains("title stone_golem"), "{style}");
    assert!(style.contains("creature/stone_golem"), "{style}");
    // The names the game has stay the game's.
    assert!(style.contains("resource_2_title stone\n"), "{style}");
}

#[test]
fn every_number_of_the_original_has_a_row_and_every_row_a_number() {
    let names = names();
    let numbers = numbers_of_the_original(&original()).unwrap();
    let game = &numbers[GAME];
    for (package, found) in &numbers {
        // A package lists the numbers the game lacks.
        let found: BTreeMap<u64, bool> = found
            .iter()
            .filter(|(n, _)| package == GAME || !game.contains_key(n))
            .map(|(n, phrase)| (*n, *phrase))
            .collect();
        let rows: BTreeMap<u64, bool> = names
            .rows(package)
            .iter()
            .map(|r| (u64::from(r.number), !r.is_sound()))
            .collect();
        assert_eq!(rows, found, "{package}: number, and whether a phrase");
    }
    for (package, _) in names.packages() {
        assert!(
            numbers.contains_key(package),
            "{package} is no package of the original"
        );
    }
}

#[test]
fn every_phrase_of_the_original_has_its_number_and_its_english() {
    let (files, _) = imported();
    let english: BTreeMap<String, String> = text(&files, "res/text/en.txt")
        .lines()
        .map(|l| l.split_once(" = ").unwrap())
        .map(|(name, words)| (name.to_owned(), words.to_owned()))
        .collect();
    for &phrase in Phrase::ORIGINAL {
        let id = phrase.id();
        assert!(names().game_number(id).is_some(), "{id} has no row");
        assert_eq!(
            english.get(id).map(String::as_str),
            Some(phrase.english()),
            "{id}"
        );
    }
    assert_eq!(
        names().game_number(Phrase::AwaitingYourOrders.id()),
        Some(4202)
    );
    assert_eq!(names().game_number(Phrase::AtDistance.id()), Some(54));
    assert_eq!(names().game_number(Phrase::LetterZ.id()), Some(5025));
}

#[test]
fn no_phrase_of_the_port_takes_a_name_of_the_original() {
    for &phrase in Phrase::OWN {
        for (package, rows) in names().packages() {
            let taken = rows.iter().any(|r| r.name == phrase.id());
            assert!(!taken, "{} is a name of {package}", phrase.id());
        }
    }
}

#[test]
fn every_cue_is_a_sound_of_the_game() {
    let (files, _) = imported();
    let game = names().rows(GAME);
    for cue in Cue::ALL {
        let row = game.iter().find(|r| r.name == cue.sound());
        assert!(row.is_some_and(Row::is_sound), "{}", cue.sound());
        let path = format!("res/sounds/{}.ogg", cue.sound());
        assert!(files.contains_key(&path), "{path}");
    }
    assert_eq!(names().game_number(Cue::Beep.sound()), Some(1029));
    assert_eq!(names().game_number(Cue::Select.sound()), Some(6115));
}
