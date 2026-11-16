//! Hostile packages. A package to import can come from anyone: a mod or a
//! map that a stranger made. The importer refuses a bad one with a message,
//! or imports what it can and says what it left. It never panics, never
//! writes outside the folder it was given, and never reads without a limit.
//!
//! The changed texts come from a fixed seed, so a failure reproduces.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

use soundrts_import::{
    Files, MAX_FILES, Report, import, import_files, import_game_files, import_map_text, read_zip,
    write_zip,
};

fn files(entries: &[(&str, &[u8])]) -> Files {
    entries
        .iter()
        .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
        .collect()
}

/// A zip file of these entries, whatever their names.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        w.start_file(*name, options).unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn imported(entries: &[(&str, &[u8])]) -> (Files, Report) {
    let mut report = Report::default();
    let out = import_files(&files(entries), &mut report).unwrap();
    (out, report)
}

fn text(files: &Files, path: &str) -> String {
    String::from_utf8(files[path].clone()).unwrap()
}

#[test]
fn a_package_of_a_stranger_gets_the_games_names_and_names_of_its_own() {
    let (out, report) = imported(&[
        ("rules.txt", b"def golem\nclass soldier\n"),
        (
            "ui/tts.txt",
            b"\xef\xbb\xbf; coding: utf-8\n5 Press Alt F4 now.\n9000 golem\n",
        ),
        ("ui-fr/tts.txt", b"9000 golem de pierre\n"),
        (
            "ui/style.txt",
            b"def golem ; a golem\ntitle 9000 5 9999 12345\nnoise loop 9001 0.5\n",
        ),
        ("ui/sub/9001.ogg", b"a sound"),
        ("ui-fr/9001.ogg", b"un son"),
        ("ui/1053.ogg", b"a footstep"),
    ]);
    // A number that the game has keeps the game's name: the package says
    // other words for that name, and the loader decides whether it may.
    assert_eq!(
        text(&out, "text/en.txt"),
        "what_are_your_orders = Press Alt F4 now.\nphrase_9000 = golem\n"
    );
    assert_eq!(text(&out, "text/fr.txt"), "phrase_9000 = golem de pierre\n");
    assert_eq!(
        text(&out, "ui/style.txt"),
        "def golem ; a golem\ntitle phrase_9000 what_are_your_orders silence/long 12345\n\
         noise loop unnamed/sound_9001 0.5\n"
    );
    assert_eq!(out["sounds/unnamed/sound_9001.ogg"], b"a sound");
    assert_eq!(out["lang/fr/sounds/unnamed/sound_9001.ogg"], b"un son");
    assert_eq!(out["sounds/footstep/grass_1.ogg"], b"a footstep");
    assert_eq!(out["rules.txt"], b"def golem\nclass soldier\n");
    assert_eq!(
        report.warnings,
        [
            "ui/style.txt line 2: 12345 names no phrase and no sound; it stays a number, said as written"
        ]
    );
}

#[test]
fn a_path_that_leaves_the_package_is_refused() {
    for name in [
        "../outside.txt",
        "ui/../../outside.txt",
        "/absolute.txt",
        "C:/windows.txt",
        "ui\\..\\..\\outside.txt",
        "ui/con:stream",
        "ui/./tts.txt",
        "ui//tts.txt",
        "ui/trailing./tts.txt",
        "ui/trailing /tts.txt",
        "a/a/a/a/a/a/a/a/a/a/a/a/a/a/a/a/a/deep.txt",
    ] {
        let error = read_zip(&zip_of(&[("map.txt", b"title 1\n"), (name, b"x")])).unwrap_err();
        assert!(error.contains("unsafe path"), "{name}: {error}");
        // The same path, given without a zip file.
        let mut report = Report::default();
        let result = import_files(&files(&[(name, b"x")]), &mut report);
        assert!(result.unwrap_err().contains("unsafe path"), "{name}");
    }
}

#[test]
fn a_recording_or_a_language_that_would_leave_its_folder_is_left_out_or_refused() {
    // A recording whose name is no name is left out, and the report says so.
    let (out, report) = imported(&[
        ("ui/Evil Name.ogg", b"x"),
        ("ui/7.5.ogg", b"x"),
        ("ui/../x.ogg", b"x"),
    ]);
    assert!(out.is_empty(), "{:?}", out.keys());
    assert_eq!(
        report.warnings,
        [
            "ui/../x.ogg: the recording has no name; left out",
            "ui/7.5.ogg: the recording has no name; left out",
            "ui/Evil Name.ogg: the recording has no name; left out"
        ]
    );
    // A language is letters, digits and '-'. A folder that only starts like
    // one is a folder like any other.
    let (out, _) = imported(&[("ui-fr_FR!/tts.txt", b"5 hello\n")]);
    assert_eq!(out.keys().collect::<Vec<_>>(), ["ui-fr_FR!/tts.txt"]);
    for name in ["ui/../x.txt", "ui-../tts.txt", "ui-fr/../../style.txt"] {
        let mut report = Report::default();
        let result = import_files(&files(&[(name, b"5 hello\n")]), &mut report);
        assert!(result.unwrap_err().contains("unsafe path"), "{name}");
    }
}

#[test]
fn two_files_that_become_one_are_refused() {
    let mut report = Report::default();
    let result = import_files(
        &files(&[("ui/a/1053.ogg", b"one"), ("ui/b/1053.ogg", b"two")]),
        &mut report,
    );
    let error = result.unwrap_err();
    assert!(
        error.contains("a second file becomes sounds/footstep/grass_1.ogg"),
        "{error}"
    );
    let result = import_files(
        &files(&[("ui/tts.txt", b"5 a\n"), ("text/en.txt", b"x = y\n")]),
        &mut report,
    );
    assert!(
        result
            .unwrap_err()
            .contains("a second file becomes text/en.txt")
    );
}

#[test]
fn what_is_not_a_zip_file_or_holds_too_much_is_refused() {
    assert!(read_zip(b"title 1\n").is_err());
    assert!(read_zip(b"").is_err());
    let names: Vec<String> = (0..=MAX_FILES).map(|i| format!("{i}.txt")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
    let error = read_zip(&zip_of(&entries)).unwrap_err();
    assert!(error.contains("more than"), "{error}");
    // A zip file inside a package is read with the same limits.
    let mut report = Report::default();
    let result = import_files(&files(&[("multi/bad.zip", b"not a zip file")]), &mut report);
    assert!(result.unwrap_err().starts_with("multi/bad.zip: "));
}

#[test]
fn zip_files_inside_zip_files_are_read_only_so_deep() {
    let mut bytes = zip_of(&[("map.txt", b"title 3001\n")]);
    for _ in 0..40 {
        bytes = zip_of(&[("0.zip", &bytes)]);
    }
    let mut report = Report::default();
    let result = import_files(&files(&[("0.zip", &bytes)]), &mut report);
    assert!(result.unwrap_err().contains("nested too deep"));
    // A few levels are a campaign chapter that is a map with its own files.
    let chapter = zip_of(&[("map.txt", b"title 3001\n")]);
    let (out, _) = imported(&[("1.zip", &chapter)]);
    let inside = read_zip(&out["1.zip"]).unwrap();
    assert_eq!(text(&inside, "map.txt"), "title numeral_1\n");
}

#[test]
fn the_folder_to_write_is_never_one_that_the_import_did_not_make() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("package");
    std::fs::create_dir_all(package.join("ui")).unwrap();
    std::fs::write(package.join("ui/tts.txt"), "5 hello\n").unwrap();
    let mine = dir.path().join("mine");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("precious.txt"), "keep").unwrap();
    let error = import(&package, &mine).unwrap_err();
    assert!(error.contains("is there already"), "{error}");
    assert_eq!(std::fs::read(mine.join("precious.txt")).unwrap(), b"keep");

    let out = dir.path().join("out");
    let report = import(&package, &out).unwrap();
    assert_eq!(report.files, 1);
    assert_eq!(
        std::fs::read_to_string(out.join("text/en.txt")).unwrap(),
        "what_are_your_orders = hello\n"
    );
    assert!(!dir.path().join("out.partial").exists());
    let zipped = dir.path().join("out.zip");
    import(&package, &zipped).unwrap();
    assert_eq!(
        read_zip(&std::fs::read(zipped).unwrap())
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["text/en.txt"]
    );
    // What is not the original game is not imported as the game.
    let error = soundrts_import::import_game(&package, &dir.path().join("game")).unwrap_err();
    assert!(error.contains("is not the original's game"), "{error}");
}

#[cfg(unix)]
#[test]
fn a_link_in_a_folder_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(secret.join("password.txt"), "hunter2").unwrap();
    let package = dir.path().join("package");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("rules.txt"), "def a\n").unwrap();
    std::os::unix::fs::symlink(&secret, package.join("leak")).unwrap();
    let read = soundrts_import::read_folder(&package).unwrap();
    assert_eq!(read.keys().collect::<Vec<_>>(), ["rules.txt"]);
}

/// SplitMix64: small, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const PIECES: [&[u8]; 16] = [
    b"\n",
    b";",
    b"\\\n",
    b"(",
    b")",
    b" ",
    b"title ",
    b"trigger ",
    b"(add_objective ",
    b"(cut_scene ",
    b"if_me ",
    b"noise loop ",
    b"99999999999999999999999999 ",
    b"\xff\xfe",
    b"\r",
    b"1000000 ",
];

fn mutate(text: &[u8], r: &mut Rng) -> Vec<u8> {
    let mut out = text.to_vec();
    for _ in 0..=r.below(8) {
        let at = r.below(out.len() + 1);
        match r.below(3) {
            0 => {
                let piece = PIECES[r.below(PIECES.len())];
                out.splice(at..at, piece.iter().copied());
            }
            1 if at < out.len() => {
                let end = (at + r.below(40)).min(out.len());
                out.drain(at..end);
            }
            _ if at < out.len() => out[at] = (r.next() & 0xff) as u8,
            _ => {}
        }
    }
    out
}

#[test]
fn changed_texts_are_imported_without_a_panic() {
    let seeds: [(&str, &[u8]); 5] = [
        (
            "map.txt",
            b"title 4271 3001\nobjective 145 88\nintro 7501\n\
              trigger player1 (timer 0) (add_objective 1 101 151 83)\n\
              trigger player1 (has 2 farm) \\\n (cut_scene 7502) ; done\n",
        ),
        (
            "0.txt",
            b"cut_scene_chapter\ntitle 4272\nsequence 75005005 1000010 2011 2036 \n",
        ),
        (
            "ui/style.txt",
            b"def peasant ; a worker\ntitle 85 $1\nnoise if_me repeat 5 1001 1002 else loop 1003 .5\n\
              alert if_me 1078 else 1079\nshortcut p\nmove_on_grass 1053 1054\n",
        ),
        ("ui/tts.txt", b"\xef\xbb\xbf; coding: utf-8\n5 What are your orders?\n9000\tgolem\n+139 x\n"),
        ("ui/game_help.txt", b"5005 1000010 2011 2036 \n5015 5012\n"),
    ];
    let mut r = Rng(0x696d_706f_7274);
    for (path, seed) in seeds {
        for case in 0..400 {
            let bytes = mutate(seed, &mut r);
            let run = || {
                let mut report = Report::default();
                let package = files(&[(path, &bytes), ("rules.txt", b"def a\n")]);
                let out = import_files(&package, &mut report).unwrap();
                // What came out is read again as a package of its own.
                let _ = write_zip(&out).and_then(|z| read_zip(&z));
                let _ = import_map_text(&bytes, &mut report);
                let game: Files = package
                    .iter()
                    .map(|(p, b)| (format!("res/{p}"), b.clone()))
                    .collect();
                let _ = import_game_files(&game, &mut report);
                assert!(report.warnings.len() < 10_000);
            };
            let result = catch_unwind(AssertUnwindSafe(run));
            assert!(
                result.is_ok(),
                "{path} case {case}: {:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }
}
