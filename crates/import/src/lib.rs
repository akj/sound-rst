//! The importer: it converts the files of the original SoundRTS into the
//! files the game reads.
//!
//! The original names every phrase and every sound with a number. The game
//! names them with words, through the table of [`names`]. The conversion
//! goes one way, and the game reads only what it produces.
//!
//! What a package of the original holds, and what it becomes:
//!
//! | The original | The game |
//! |---|---|
//! | `ui/tts.txt`, `ui-fr/tts.txt` | `text/en.txt`, `text/fr.txt`: lines `name = words` |
//! | `ui/…/1053.ogg` | `sounds/footstep/grass_1.ogg` |
//! | `ui-de/…/1071.ogg` | `lang/de/sounds/voice/own/order_ok_1.ogg` |
//! | `ui/style.txt`, `ui-fr/style.txt` | `ui/style.txt`, `lang/fr/ui/style.txt`, names for numbers |
//! | `ui/game_help.txt`, `ui/menu_help.txt` | the same paths, names for numbers |
//! | a map, a campaign chapter | the same path, names for numbers in its `title`, `objective` and `intro` lines and in its `add_objective` and `cut_scene` actions |
//! | every other file | the same path, the same bytes |
//!
//! A number of 1,000,000 or more is a count in the original: `1000012` is
//! said "12". It becomes the count itself, `12`. A number that names
//! nothing in the original is said as written there, so it also stays, with
//! a warning.
//!
//! Every read is bounded, and no path of the input leaves the folder that
//! the output is written to.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

pub mod names;
mod zip_guard;

pub use zip_guard::too_many_entries;

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use soundrts_world::msg::is_name;

use crate::names::{GAME, names};

/// The version of the layout that the importer writes. Increase it when
/// the same input gives other files.
pub const FORMAT: u32 = 1;

/// The file at the root of imported game data. It holds [`stamp`].
pub const STAMP_FILE: &str = "import.txt";

/// The most bytes one file may hold. The original's largest is a sound of
/// 1.2 MB.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// The most bytes a package, or the game, may hold. The whole original is
/// 13 MB.
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// The most files a package, or the game, may hold. The whole original has
/// 1,101.
pub const MAX_FILES: usize = 20_000;

/// How deep folders are looked into, and how deep packages may be inside
/// packages.
const MAX_DEPTH: usize = 16;

/// In the original, a number from this value up is a count to read aloud.
const COUNT_SHIFT: u64 = 1_000_000;

/// What marks imported game data as made by this importer with this table
/// of names. The game reads only data with its own stamp: with another
/// table, the names in the code and in the data would differ.
pub fn stamp() -> String {
    // FNV-1a, 64 bits.
    let hash = names::TEXT.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("soundrts-import {FORMAT} {hash:016x}")
}

/// Whether `root` holds game data with this importer's [`stamp`].
pub fn is_imported(root: &Path) -> bool {
    std::fs::read_to_string(root.join(STAMP_FILE)).is_ok_and(|s| s.trim() == stamp())
}

/// The files of a package: the path of each, with `/`, and its bytes.
pub type Files = BTreeMap<String, Vec<u8>>;

/// What an import did, beside the files it wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// What the importer kept as it was or left out, with the file and the
    /// line. The import is complete with these.
    pub warnings: Vec<String>,
    /// How many files were written.
    pub files: usize,
    /// The bytes read from the zip files inside the package, and written
    /// as zip files inside the result: together they stay within
    /// [`MAX_TOTAL_BYTES`], however deep the zip files are in each other.
    expanded: u64,
}

fn plain_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(['\\', ':', '\0'])
        && !part.ends_with(['.', ' '])
}

/// Whether a path of the input can be a path of the output: plain parts
/// joined by `/`.
fn plain_path(path: &str) -> bool {
    path.split('/').count() <= MAX_DEPTH && path.split('/').all(plain_part)
}

fn add(files: &mut Files, total: &mut u64, path: String, bytes: Vec<u8>) -> Result<(), String> {
    *total = total.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    if *total > MAX_TOTAL_BYTES {
        return Err(format!("more than {MAX_TOTAL_BYTES} bytes"));
    }
    if files.len() >= MAX_FILES {
        return Err(format!("more than {MAX_FILES} files"));
    }
    files.insert(path, bytes);
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let at = |e: std::io::Error| format!("cannot read {}: {e}", path.display());
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(at)?
        .take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(at)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return Err(format!(
            "{} holds more than {MAX_FILE_BYTES} bytes",
            path.display()
        ));
    }
    Ok(bytes)
}

fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Files,
    total: &mut u64,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: folders nested too deep", dir.display()));
    }
    let at = |e: std::io::Error| format!("cannot read {}: {e}", dir.display());
    for entry in std::fs::read_dir(dir).map_err(at)? {
        let entry = entry.map_err(at)?;
        let kind = entry.file_type().map_err(at)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // A link is never followed.
        if kind.is_symlink() || !plain_part(&name) {
            continue;
        }
        let path = format!("{prefix}{name}");
        if kind.is_dir() {
            let deeper = depth.saturating_add(1);
            walk(&entry.path(), &format!("{path}/"), deeper, files, total)?;
        } else {
            add(files, total, path, read_file(&entry.path())?)?;
        }
    }
    Ok(())
}

/// The files of a folder. Links are left out.
pub fn read_folder(dir: &Path) -> Result<Files, String> {
    let mut files = Files::new();
    walk(dir, "", 0, &mut files, &mut 0)?;
    Ok(files)
}

/// The files of a zip file. An entry whose path is not plain is refused.
pub fn read_zip(bytes: &[u8]) -> Result<Files, String> {
    read_zip_within(bytes, &mut 0)
}

/// As [`read_zip`]. `expanded` counts the bytes read so far, of this zip
/// file and of those read before it: together they stay within
/// [`MAX_TOTAL_BYTES`].
fn read_zip_within(bytes: &[u8], expanded: &mut u64) -> Result<Files, String> {
    // Before the reader builds its index of the entries.
    if too_many_entries(bytes, MAX_FILES) {
        return Err(format!("more than {MAX_FILES} files"));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("not a zip file that can be read: {e}"))?;
    if archive.len() > MAX_FILES {
        return Err(format!("more than {MAX_FILES} files"));
    }
    let mut files = Files::new();
    let total = expanded;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        if entry.is_dir() {
            continue;
        }
        if !plain_path(&name) {
            return Err(format!("unsafe path: {name}"));
        }
        let left = MAX_FILE_BYTES.min(MAX_TOTAL_BYTES.saturating_sub(*total));
        let mut content = Vec::new();
        (&mut entry)
            .take(left.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|e| format!("{name}: {e}"))?;
        if u64::try_from(content.len()).unwrap_or(u64::MAX) > left {
            return Err(format!("{name} holds too many bytes"));
        }
        add(&mut files, total, name, content)?;
    }
    Ok(files)
}

/// The files as a zip file. The same files give the same bytes on every
/// machine, since a map is known to other players by the hash of its file:
/// the entries are sorted, stored as they are, and dated 1980.
pub fn write_zip(files: &Files) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for (path, bytes) in files {
        writer
            .start_file(path.as_str(), options)
            .map_err(|e| e.to_string())?;
        writer.write_all(bytes).map_err(|e| e.to_string())?;
    }
    Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Writes the files under `dir`, which is made.
pub fn write_folder(dir: &Path, files: &Files) -> Result<(), String> {
    for (path, bytes) in files {
        if !plain_path(path) {
            return Err(format!("unsafe path: {path}"));
        }
        let target = dir.join(path);
        let at = |e: std::io::Error| format!("cannot write {}: {e}", target.display());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(at)?;
        }
        std::fs::write(&target, bytes).map_err(at)?;
    }
    Ok(())
}

/// The names of the numbers of one package: the game's, then the
/// package's own, then those of the package it is inside.
struct Namer<'a> {
    own: BTreeMap<u64, String>,
    parent: Option<&'a Namer<'a>>,
}

impl Namer<'_> {
    fn name(&self, number: u64) -> Option<&str> {
        let game = u32::try_from(number)
            .ok()
            .and_then(|n| names().game_name(n));
        game.or_else(|| self.own_name(number))
    }

    fn own_name(&self, number: u64) -> Option<&str> {
        let own = self.own.get(&number).map(String::as_str);
        own.or_else(|| self.parent.and_then(|p| p.own_name(number)))
    }

    /// What a word that names a phrase or a sound becomes: the name of its
    /// number, or a count. A word that stays is reported.
    fn word(&self, word: &str, at: &str, report: &mut Report) -> String {
        let number = word
            .parse::<u64>()
            .ok()
            .filter(|_| word.bytes().all(|b| b.is_ascii_digit()));
        let Some(number) = number else {
            report
                .warnings
                .push(format!("{at}: '{word}' is not a number; left as it is"));
            return word.to_owned();
        };
        if let Some(count) = number.checked_sub(COUNT_SHIFT) {
            return count.to_string();
        }
        match self.name(number) {
            Some(name) => name.to_owned(),
            None => {
                report.warnings.push(format!(
                    "{at}: {word} names no phrase and no sound; it stays a number, said as written"
                ));
                word.to_owned()
            }
        }
    }
}

/// The text of a file of the original: UTF-8, with `\n` line ends.
fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// The number at the start of each line of a `tts.txt`, with its words.
/// A line that is not `<number> <words>` is left out, as the original
/// never used it.
fn tts_lines(bytes: &[u8]) -> BTreeMap<u64, String> {
    let text = decode(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes));
    let mut lines = BTreeMap::new();
    for line in text.split('\n') {
        let Some((key, words)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let digits = key.bytes().all(|b| b.is_ascii_digit());
        if let (true, Ok(number), false) = (digits, key.parse(), words.trim().is_empty()) {
            lines.insert(number, words.trim().to_owned());
        }
    }
    lines
}

/// The folder of the interface files that a path of the original is in,
/// as its language (`None` for `ui`, the English one) and the rest of the
/// path.
fn ui_path(path: &str) -> Option<(Option<&str>, &str)> {
    let (folder, rest) = path.split_once('/')?;
    if folder == "ui" {
        return Some((None, rest));
    }
    let language = folder.strip_prefix("ui-")?;
    let plain = !language.is_empty()
        && language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    plain.then_some((Some(language), rest))
}

/// The number that a recording of the original is named with.
fn recording_number(rest: &str) -> Option<u64> {
    let stem = rest.rsplit('/').next()?.strip_suffix(".ogg")?;
    stem.parse()
        .ok()
        .filter(|_| stem.bytes().all(|b| b.is_ascii_digit()))
}

/// Rewrites the words of `code` that `convert` gives another word for.
/// A word ends at a blank, and at a parenthesis if `parentheses`.
fn rewrite(
    code: &str,
    parentheses: bool,
    mut convert: impl FnMut(&str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(code.len());
    let mut word = String::new();
    let mut flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            let new = convert(word);
            out.push_str(new.as_deref().unwrap_or(word));
            word.clear();
        }
    };
    for c in code.chars() {
        if c.is_whitespace() || (parentheses && matches!(c, '(' | ')')) {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Applies `line` to the code of each line of a definition file, and keeps
/// its comments. `line` gets the number of the line, from 1, whether the
/// line continues the one before (which ended with a backslash), and the
/// code.
fn each_line(text: &str, mut line: impl FnMut(usize, bool, &str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut continues = false;
    for (i, whole) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (code, comment) = match whole.find(';') {
            Some(at) => whole.split_at(at),
            None => (whole, ""),
        };
        if code.trim().is_empty() {
            // A comment or a blank line does not end a continued line.
            out.push_str(whole);
            continue;
        }
        out.push_str(&line(i.saturating_add(1), continues, code));
        out.push_str(comment);
        continues = code.trim_end().ends_with('\\');
    }
    out
}

/// The attributes of a style whose words name no phrase and no sound.
const STYLE_PLAIN: [&str; 9] = [
    "is_a",
    "index",
    "shortcut",
    "keyboard",
    "color",
    "ground",
    "shape",
    "recommended_maps",
    "can_upgrade_to",
];

/// The words of a style that are part of its syntax.
const STYLE_SYNTAX: [&str; 5] = ["if_me", "else", "ambient", "loop", "repeat"];

fn convert_style(text: &str, namer: &Namer, file: &str, report: &mut Report) -> String {
    let mut attribute = String::new();
    // In a noise, the word after `loop` is a sound and the next one a
    // volume. The word after `repeat` is an interval. A line can go on
    // after a backslash, so this lasts as long as the attribute.
    let mut skip = 0_u8;
    let mut after_loop = false;
    each_line(text, |number, continues, code| {
        let mut first = !continues;
        if !continues {
            skip = 0;
            after_loop = false;
        }
        rewrite(code, false, |word| {
            if std::mem::take(&mut first) {
                attribute = word.to_owned();
                return None;
            }
            let plain = matches!(attribute.as_str(), "def" | "clear")
                || STYLE_PLAIN.contains(&attribute.as_str());
            if plain || word == "\\" || word.starts_with('$') {
                return None;
            }
            if STYLE_SYNTAX.contains(&word) {
                after_loop = word == "loop";
                skip = u8::from(word == "repeat");
                return None;
            }
            if skip > 0 {
                skip = skip.saturating_sub(1);
                return None;
            }
            if attribute.starts_with("noise") && std::mem::take(&mut after_loop) {
                skip = 1;
            }
            Some(namer.word(word, &format!("{file} line {number}"), report))
        })
    })
}

/// A help file: every word names a phrase or a sound.
fn convert_help(text: &str, namer: &Namer, file: &str, report: &mut Report) -> String {
    each_line(text, |number, _, code| {
        rewrite(code, false, |word| {
            Some(namer.word(word, &format!("{file} line {number}"), report))
        })
    })
}

/// Rewrites the words of a map that name a phrase or a sound, or are a
/// count: those of its `title`, `objective` and `intro` lines, and of its
/// `add_objective` and `cut_scene` actions. For a chapter that is a cut
/// scene, those of its `title` and `sequence` lines. `name` gets the line of
/// each word, counted from 1, and gives the word to write in its place, if
/// another.
fn map_words(text: &str, mut name: impl FnMut(usize, &str) -> Option<String>) -> String {
    let cut_scene = text.split('\n').next() == Some("cut_scene_chapter");
    let mut keyword = String::new();
    // Inside an action that says something: the words of the action that
    // come before those it says.
    let mut saying: Option<usize> = None;
    each_line(text, |number, continues, code| {
        if !continues {
            keyword = code.split_whitespace().next().unwrap_or("").to_owned();
            saying = None;
        }
        let mut name = |word: &str| (word != "\\").then(|| name(number, word)).flatten();
        let named = match keyword.as_str() {
            "title" => true,
            "sequence" => cut_scene,
            "objective" | "intro" => !cut_scene,
            _ => false,
        };
        if named {
            let mut is_keyword = !continues;
            return rewrite(code, false, |word| {
                if std::mem::take(&mut is_keyword) {
                    None
                } else {
                    name(word)
                }
            });
        }
        if keyword != "trigger" || cut_scene {
            return code.to_owned();
        }
        // An action ends at a parenthesis: `add_objective`, the
        // objective's number and its words, or `cut_scene` and its words.
        let mut out = String::with_capacity(code.len());
        for piece in code.split_inclusive(['(', ')']) {
            out.push_str(&rewrite(piece, true, |word| match (word, saying) {
                ("add_objective", _) => {
                    saying = Some(1);
                    None
                }
                ("cut_scene", _) => {
                    saying = Some(0);
                    None
                }
                (_, Some(0)) => name(word),
                (_, Some(n)) => {
                    saying = Some(n.saturating_sub(1));
                    None
                }
                (_, None) => None,
            }));
            if piece.ends_with([')', '(']) {
                saying = None;
            }
        }
        out
    })
}

fn convert_map(text: &str, namer: &Namer, file: &str, report: &mut Report) -> String {
    map_words(text, |line, word| {
        Some(namer.word(word, &format!("{file} line {line}"), report))
    })
}

/// The words of a map of the game that name a phrase or a sound, or are a
/// count, each with its line, counted from 1.
pub fn words_a_map_says(text: &str) -> Vec<(usize, String)> {
    let mut words = Vec::new();
    map_words(text, |line, word| {
        words.push((line, word.to_owned()));
        None
    });
    words
}

/// The `title` line of a `campaign.txt`.
fn convert_campaign(text: &str, namer: &Namer, file: &str, report: &mut Report) -> String {
    each_line(text, |number, _, code| {
        let mut words = 0_usize;
        let title = code.split_whitespace().next() == Some("title");
        rewrite(code, false, |word| {
            words = words.saturating_add(1);
            (title && words > 1).then(|| namer.word(word, &format!("{file} line {number}"), report))
        })
    })
}

/// A language file: a line `name = words` for each line of a `tts.txt`
/// whose number has a name, in the order of the numbers.
fn convert_tts(bytes: &[u8], namer: &Namer, file: &str, report: &mut Report) -> String {
    let mut out = String::new();
    for (number, words) in tts_lines(bytes) {
        match namer.name(number) {
            Some(name) => {
                out.push_str(name);
                out.push_str(" = ");
                out.push_str(&words);
                out.push('\n');
            }
            None => report
                .warnings
                .push(format!("{file}: {number} has no name; left out")),
        }
    }
    out
}

/// The names of the numbers that a package has and the game lacks: those
/// of the table for `key`, and for a number the table lacks, `phrase_<n>`
/// or `unnamed/sound_<n>`.
fn own_names(files: &Files, key: &str, parent: &Namer) -> BTreeMap<u64, String> {
    let mut own: BTreeMap<u64, String> = names()
        .rows(key)
        .iter()
        .filter(|_| key != GAME)
        .map(|r| (u64::from(r.number), r.name.to_owned()))
        .collect();
    let mut found: BTreeMap<u64, bool> = BTreeMap::new();
    for (path, bytes) in files {
        let Some((_, rest)) = ui_path(path) else {
            continue;
        };
        if rest == "tts.txt" {
            found.extend(tts_lines(bytes).into_keys().map(|n| (n, true)));
        } else if let Some(n) = recording_number(rest) {
            found.entry(n).or_insert(false);
        }
    }
    let named = Namer {
        own: own.clone(),
        parent: Some(parent),
    };
    for (number, phrase) in found {
        if named.name(number).is_none() {
            let name = if phrase {
                format!("phrase_{number}")
            } else {
                format!("unnamed/sound_{number}")
            };
            own.insert(number, name);
        }
    }
    own
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The packages inside a package: its `multi`, `mods` and `single`
/// folders hold one in each folder or zip file.
const GROUPS: [&str; 3] = ["multi", "mods", "single"];

fn put(out: &mut Files, path: String, bytes: Vec<u8>, from: &str) -> Result<(), String> {
    if !plain_path(&path) {
        return Err(format!("{from}: unsafe path {path}"));
    }
    if out.insert(path.clone(), bytes).is_some() {
        return Err(format!("{from}: a second file becomes {path}"));
    }
    Ok(())
}

/// Converts the files of one package. `key` is its name in the table of
/// names, and `at` is where it is, for a message.
fn import_package(
    files: &Files,
    key: &str,
    at: &str,
    parent: &Namer,
    depth: usize,
    report: &mut Report,
) -> Result<Files, String> {
    if depth > MAX_DEPTH {
        return Err(format!("{at}: packages nested too deep"));
    }
    let namer = Namer {
        own: own_names(files, key, parent),
        parent: Some(parent),
    };
    let join = |a: &str, b: &str| {
        if a.is_empty() {
            b.to_owned()
        } else {
            format!("{a}/{b}")
        }
    };
    let mut out = Files::new();
    let mut inside: BTreeMap<String, Files> = BTreeMap::new();
    for (path, bytes) in files {
        let from = join(at, path);
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        // A package in a folder of a group.
        if let [group, name, rest] = parts.as_slice()
            && GROUPS.contains(group)
        {
            let files = inside.entry(format!("{group}/{name}")).or_default();
            files.insert((*rest).to_owned(), bytes.clone());
            continue;
        }
        let stem = path.strip_suffix(".zip");
        let chapter = stem.is_some_and(is_number);
        let in_group = matches!(parts.as_slice(), [group, _] if GROUPS.contains(group));
        // A package in a zip file: of a group, or a chapter of a campaign.
        if let Some(stem) = stem.filter(|_| chapter || in_group) {
            let files =
                read_zip_within(bytes, &mut report.expanded).map_err(|e| format!("{from}: {e}"))?;
            let deeper = depth.saturating_add(1);
            let new = import_package(&files, &join(key, stem), &from, &namer, deeper, report)?;
            let zip = write_zip(&new)?;
            let size = u64::try_from(zip.len()).unwrap_or(u64::MAX);
            report.expanded = report.expanded.saturating_add(size);
            if report.expanded > MAX_TOTAL_BYTES {
                return Err(format!("{from}: more than {MAX_TOTAL_BYTES} bytes"));
            }
            put(&mut out, path.clone(), zip, &from)?;
            continue;
        }
        let text = || decode(bytes);
        let stem = path.strip_suffix(".txt");
        let is_map = path == "map.txt"
            || stem.is_some_and(is_number)
            || (in_group && stem.is_some() && parts.first() == Some(&"multi"));
        let (to, new) = if let Some((language, rest)) = ui_path(path) {
            let folder = language.map_or(String::new(), |l| format!("lang/{l}/"));
            if rest == "tts.txt" {
                let to = format!("text/{}.txt", language.unwrap_or("en"));
                (to, convert_tts(bytes, &namer, &from, report).into_bytes())
            } else if let Some(stem) = rest.strip_suffix(".ogg") {
                let name = match recording_number(rest) {
                    Some(n) => namer.name(n).map(str::to_owned),
                    None => is_name(stem).then(|| stem.to_owned()),
                };
                let Some(name) = name else {
                    report
                        .warnings
                        .push(format!("{from}: the recording has no name; left out"));
                    continue;
                };
                (format!("{folder}sounds/{name}.ogg"), bytes.clone())
            } else {
                let new = match rest {
                    "style.txt" => convert_style(&text(), &namer, &from, report).into_bytes(),
                    "game_help.txt" | "menu_help.txt" => {
                        convert_help(&text(), &namer, &from, report).into_bytes()
                    }
                    _ => bytes.clone(),
                };
                (format!("{folder}ui/{rest}"), new)
            }
        } else if is_map {
            let new = convert_map(&text(), &namer, &from, report);
            (path.clone(), new.into_bytes())
        } else if path == "campaign.txt" {
            let new = convert_campaign(&text(), &namer, &from, report);
            (path.clone(), new.into_bytes())
        } else {
            (path.clone(), bytes.clone())
        };
        put(&mut out, to, new, &from)?;
    }
    for (path, files) in inside {
        let from = join(at, &path);
        let deeper = depth.saturating_add(1);
        let new = import_package(&files, &join(key, &path), &from, &namer, deeper, report)?;
        for (rest, bytes) in new {
            put(&mut out, format!("{path}/{rest}"), bytes, &from)?;
        }
    }
    Ok(out)
}

const NO_NAMES: Namer = Namer {
    own: BTreeMap::new(),
    parent: None,
};

/// Converts the files of a package of the original that is not part of
/// the game: a mod, a campaign, or a map with files of its own. The
/// numbers that the game has get the game's names. The others get
/// `phrase_<n>` or `unnamed/sound_<n>`, for a person to rename.
pub fn import_files(files: &Files, report: &mut Report) -> Result<Files, String> {
    import_package(files, "", "", &NO_NAMES, 0, report)
}

/// Converts the text of a map of the original that is one file.
pub fn import_map_text(text: &[u8], report: &mut Report) -> String {
    convert_map(&decode(text), &NO_NAMES, "the map", report)
}

/// The folders of the original's game that hold what the game reads.
const GAME_FOLDERS: [&str; 3] = ["res", "mods", "single"];

/// Whether `dir` is the folder of the original's game.
pub fn is_original_game(dir: &Path) -> bool {
    dir.join("res/rules.txt").is_file() && dir.join("res/ui/tts.txt").is_file()
}

/// Converts the files of the original's game: its `res`, `mods` and
/// `single` folders. The result holds [`STAMP_FILE`].
pub fn import_game_files(files: &Files, report: &mut Report) -> Result<Files, String> {
    let mut out = Files::new();
    for folder in GAME_FOLDERS {
        let prefix = format!("{folder}/");
        let inside: Files = files
            .iter()
            .filter_map(|(p, b)| Some((p.strip_prefix(&prefix)?.to_owned(), b.clone())))
            .collect();
        let new = if folder == GAME {
            import_package(&inside, GAME, GAME, &NO_NAMES, 0, report)?
        } else {
            // The folder is a group of packages, as inside a package.
            let grouped: Files = inside
                .into_iter()
                .map(|(p, b)| (format!("{prefix}{p}"), b))
                .collect();
            let new = import_package(&grouped, "", "", &NO_NAMES, 0, report)?;
            new.into_iter()
                .filter_map(|(p, b)| Some((p.strip_prefix(&prefix)?.to_owned(), b)))
                .collect()
        };
        for (path, bytes) in new {
            put(&mut out, format!("{prefix}{path}"), bytes, folder)?;
        }
    }
    out.insert(STAMP_FILE.to_owned(), format!("{}\n", stamp()).into_bytes());
    Ok(out)
}

fn read_game(dir: &Path) -> Result<Files, String> {
    let mut files = Files::new();
    let mut total = 0;
    for folder in GAME_FOLDERS {
        let inside = dir.join(folder);
        if inside.is_dir() {
            walk(&inside, &format!("{folder}/"), 1, &mut files, &mut total)?;
        }
    }
    Ok(files)
}

fn partial_of(to: &Path) -> PathBuf {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    to.with_file_name(name)
}

/// Writes `files` as the folder `to`: into `<to>.partial` first, so that
/// `to` is complete or absent. A folder that is there, `to` or what an
/// import that stopped left in `<to>.partial`, is replaced only if
/// `replace` says that the importer made it, or if it is empty.
fn write_whole_folder(
    to: &Path,
    files: &Files,
    replace: impl Fn(&Path) -> bool,
) -> Result<(), String> {
    let partial = partial_of(to);
    let gone = |dir: &Path| match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("cannot remove {}: {e}", dir.display()))
        }
        _ => Ok(()),
    };
    let is_empty = |dir: &Path| std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none());
    for dir in [to, partial.as_path()] {
        if dir.exists() && !replace(dir) && !is_empty(dir) {
            return Err(format!(
                "{} is there, and an import did not make it. Give a folder that does not exist.",
                dir.display()
            ));
        }
    }
    gone(&partial)?;
    if let Err(e) = write_folder(&partial, files) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    gone(to)?;
    std::fs::rename(&partial, to).map_err(|e| format!("cannot make {}: {e}", to.display()))
}

/// Converts the folder of the original's game `from` into the folder `to`.
/// Safe to run again: imported data that is there is replaced.
pub fn import_game(from: &Path, to: &Path) -> Result<Report, String> {
    if !is_original_game(from) {
        return Err(format!(
            "{} is not the original's game: it has no res/rules.txt and res/ui/tts.txt",
            from.display()
        ));
    }
    let mut report = Report::default();
    let files = import_game_files(&read_game(from)?, &mut report)?;
    write_whole_folder(to, &files, |dir| dir.join(STAMP_FILE).is_file())?;
    report.files = files.len();
    Ok(report)
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

/// Converts `from` into `to`. `from` is the folder of the original's game,
/// or one of its packages as a folder or a zip file, or a map as a `.txt`
/// file. A package becomes a zip file if `to` ends with `.zip`, else a
/// folder. A package that is there is not replaced.
pub fn import(from: &Path, to: &Path) -> Result<Report, String> {
    if from.is_dir() && is_original_game(from) {
        return import_game(from, to);
    }
    let mut report = Report::default();
    if to.exists() {
        return Err(format!("{} is there already", to.display()));
    }
    let write = |bytes: &[u8]| {
        std::fs::write(to, bytes).map_err(|e| format!("cannot write {}: {e}", to.display()))
    };
    if has_extension(from, "txt") {
        let text = import_map_text(&read_file(from)?, &mut report);
        write(text.as_bytes())?;
        report.files = 1;
        return Ok(report);
    }
    let files = if from.is_dir() {
        read_folder(from)?
    } else {
        read_zip(&read_file(from)?).map_err(|e| format!("{}: {e}", from.display()))?
    };
    let files = import_files(&files, &mut report)?;
    if has_extension(to, "zip") {
        write(&write_zip(&files)?)?;
    } else {
        write_whole_folder(to, &files, |_| false)?;
    }
    report.files = files.len();
    Ok(report)
}

/// The numbers that the packages of the original have, by the name of each
/// package in the table: true for a line of a `tts.txt`, false for a
/// recording. For the tests of the table.
pub fn numbers_of_the_original(
    dir: &Path,
) -> Result<BTreeMap<String, BTreeMap<u64, bool>>, String> {
    fn collect(files: &Files, key: &str, all: &mut BTreeMap<String, BTreeMap<u64, bool>>) {
        let mut inside: BTreeMap<String, Files> = BTreeMap::new();
        let mut numbers = BTreeMap::new();
        let mut is_package = false;
        for (path, bytes) in files {
            let parts: Vec<&str> = path.splitn(3, '/').collect();
            if let [group, name, rest] = parts.as_slice()
                && GROUPS.contains(group)
            {
                let files = inside.entry(format!("{group}/{name}")).or_default();
                files.insert((*rest).to_owned(), bytes.clone());
            } else if let ([group, _], Some(stem)) = (parts.as_slice(), path.strip_suffix(".zip"))
                && GROUPS.contains(group)
                && let Ok(files) = read_zip(bytes)
            {
                inside.insert(stem.to_owned(), files);
            } else if let Some((_, rest)) = ui_path(path) {
                is_package = true;
                if rest == "tts.txt" {
                    numbers.extend(tts_lines(bytes).into_keys().map(|n| (n, true)));
                } else if let Some(n) = recording_number(rest) {
                    numbers.entry(n).or_insert(false);
                }
            }
        }
        if is_package {
            all.insert(key.to_owned(), numbers);
        }
        for (path, files) in inside {
            let key = if key.is_empty() {
                path
            } else {
                format!("{key}/{path}")
            };
            collect(&files, &key, all);
        }
    }
    let files = read_game(dir)?;
    let mut all = BTreeMap::new();
    let game: Files = files
        .iter()
        .filter_map(|(p, b)| Some((p.strip_prefix("res/")?.to_owned(), b.clone())))
        .collect();
    collect(&game, GAME, &mut all);
    let others: Files = files
        .into_iter()
        .filter(|(p, _)| !p.starts_with("res/"))
        .collect();
    collect(&others, "", &mut all);
    Ok(all)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn zipped(path: &str, bytes: &[u8]) -> Vec<u8> {
        let files: Files = [(path.to_owned(), bytes.to_vec())].into();
        write_zip(&files).unwrap()
    }

    #[test]
    fn the_zip_files_inside_a_package_share_one_limit() {
        let chapter = zipped("map.txt", &[b' '; 1000]);
        let package: Files = (0..4)
            .map(|i| (format!("{i}.zip"), chapter.clone()))
            .collect();
        let mut report = Report::default();
        assert_eq!(import_files(&package, &mut report).unwrap().len(), 4);
        // Each chapter is read, then written: more than 8,000 bytes.
        assert!(report.expanded > 8000, "{}", report.expanded);
        // The same package, when what was read before leaves room for
        // three chapters and not for four.
        let mut report = Report {
            expanded: MAX_TOTAL_BYTES - 7000,
            ..Report::default()
        };
        let error = import_files(&package, &mut report).unwrap_err();
        assert!(error.starts_with("3.zip: "), "{error}");
    }

    #[test]
    fn a_noise_that_goes_on_after_a_backslash_keeps_its_volume_and_its_interval() {
        let mut report = Report::default();
        let text = "def a\nnoise loop 1000 \\\n 1\nnoise_if_damaged repeat \\\n 5 1000\ntitle 1\n";
        assert_eq!(
            convert_style(text, &NO_NAMES, "style.txt", &mut report),
            "def a\nnoise loop ambience/path \\\n 1\nnoise_if_damaged repeat \\\n 5 ambience/path\n\
             title you_are_capitalized\n"
        );
    }
}
