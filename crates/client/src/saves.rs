//! Saved games: a single-player game written to the player's `saves`
//! folder, and read back. The same file, written to the `records` folder,
//! keeps a finished game to listen to ([`listen`]).
//!
//! A save is the game's record so far ([`Record`]), after one line of its
//! own: what game it is and what the menus say of it, and the little of
//! the interface a record does not hold, the cursor's square and the units
//! selected. A save loads by replaying the record, so it loads only on the
//! version of the game that made it: the simulation refuses a record of
//! another version ([`ReadError::OtherVersion`]).
//!
//! A save is written beside its file, then renamed over it, so that a
//! save cut short never replaces a good one. A save is a file anyone can
//! hand over: reading one is bounded, and what it says is checked.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::record::MAX_RECORD_BYTES;
use soundrts_world::{
    Cheat, Command, Completeness, EntityId, PlayerId, ReadError, Record, SIM_VERSION, SquareId,
};

use crate::net::words::{self, text};
use crate::package::read_file;

/// The name and version of the format, on a save's first line.
pub const SAVE_FORMAT: &str = "soundrts-save-1";

/// The longest first line of a save read, in bytes.
pub const MAX_SAVE_HEADER_BYTES: usize = 64 * 1024;

/// The most saves the list shows, newest first.
pub const MAX_SAVES_LISTED: usize = 500;

/// The most parts of a save's title read.
pub const MAX_TITLE_PARTS: usize = 16;

/// The most characters of a word of a save's title read.
pub const MAX_TITLE_WORD_CHARS: usize = 64;

/// The most units selected a save may restore.
pub const MAX_SELECTION: usize = 10_000;

/// The extension of a save's file.
const EXTENSION: &str = "sav";

/// The kind of a single-player game, and where to find its map again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedGame {
    /// A training game on the multiplayer map of this name.
    Training { map: String },
    /// A chapter of the campaign of this name.
    Mission { campaign: String, chapter: usize },
}

impl SavedGame {
    /// The file of the game saved by itself when the player last quit a
    /// game of this kind: one for each kind.
    fn last_game_file(&self) -> &'static str {
        match self {
            SavedGame::Training { .. } => "last-training.sav",
            SavedGame::Mission { .. } => "last-mission.sav",
        }
    }

    /// The start of a save's file name: the map's name, or the campaign's
    /// and the chapter's number, in characters any file system takes.
    fn file_stem(&self) -> String {
        let raw = match self {
            SavedGame::Training { map } => map.clone(),
            SavedGame::Mission { campaign, chapter } => format!("{campaign}-{chapter}"),
        };
        let safe: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .take(40)
            .collect();
        if safe.is_empty() {
            "game".to_owned()
        } else {
            safe
        }
    }
}

/// What a game run here needs to be saved: what it is, and its title.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveInfo {
    pub game: SavedGame,
    /// What the list of saves says first: the map's or the mission's
    /// title.
    pub title: Msg,
}

/// Where a save goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// A new file, chosen in the game menu.
    New,
    /// The game saved by itself when the player quits, one of each kind,
    /// replaced each time.
    LastGame,
}

/// The first line of a save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveHeader {
    pub format: String,
    /// The version of the simulation that made the save.
    pub sim_version: u32,
    pub game: SavedGame,
    pub title: Msg,
    /// The game time saved, in milliseconds.
    pub game_time_ms: u64,
    /// When the game was saved, in seconds since 1970 (UTC).
    pub saved_at: u64,
    /// Saved by itself when the player quit.
    pub last_game: bool,
    /// The square under the cursor.
    pub cursor: Option<SquareId>,
    /// The units selected.
    pub selection: Vec<EntityId>,
}

/// What the interface keeps beside the record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InterfaceState {
    pub cursor: Option<SquareId>,
    pub selection: Vec<EntityId>,
}

/// The seconds since 1970 (UTC), by the system's clock.
pub fn wall_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes the game of `record` to `dir`, and returns the file. The file is
/// written beside, then renamed over the old one, so that a good save is
/// replaced only by a whole one.
pub fn write_save(
    dir: &Path,
    info: &SaveInfo,
    record: &Record,
    ui: &InterfaceState,
    saved_at: u64,
    slot: Slot,
) -> std::io::Result<PathBuf> {
    let header = SaveHeader {
        format: SAVE_FORMAT.to_owned(),
        sim_version: SIM_VERSION,
        game: info.game.clone(),
        title: info.title.clone(),
        game_time_ms: record.game_time_ms(),
        saved_at,
        last_game: slot == Slot::LastGame,
        cursor: ui.cursor,
        selection: ui.selection.clone(),
    };
    let mut bytes = serde_json::to_vec(&header).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    record.write(&mut bytes)?;
    std::fs::create_dir_all(dir)?;
    let path = match slot {
        Slot::LastGame => dir.join(info.game.last_game_file()),
        Slot::New => new_file(dir, &info.game, record.game_time_ms(), saved_at),
    };
    write_whole(&path, &bytes)?;
    Ok(path)
}

/// A file name not yet taken in `dir`: the game's map or mission, its
/// game time in minutes, and when it was saved.
fn new_file(dir: &Path, game: &SavedGame, game_time_ms: u64, saved_at: u64) -> PathBuf {
    let base = format!(
        "{}-{}min-{}",
        game.file_stem(),
        game_time_ms / 60_000,
        utc_stamp(saved_at)
    );
    let mut path = dir.join(format!("{base}.{EXTENSION}"));
    let mut n: u32 = 2;
    while path.exists() && n < 1000 {
        path = dir.join(format!("{base}-{n}.{EXTENSION}"));
        n = n.saturating_add(1);
    }
    path
}

/// `bytes` to `path`, written whole beside it first, then renamed over it.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".tmp");
    let beside = PathBuf::from(beside);
    let result = (|| {
        let mut file = std::fs::File::create(&beside)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&beside, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&beside);
    }
    result
}

/// `2026-11-15-143005` for a time in seconds since 1970, in UTC.
fn utc_stamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rest = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The year, month and day of a number of days since 1970-01-01 (Howard
/// Hinnant's algorithm, for days after the epoch).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days.saturating_add(719_468);
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe
        .saturating_sub(doe / 1460)
        .saturating_add(doe / 36_524)
        .saturating_sub(doe / 146_096))
        / 365;
    let doy = doe.saturating_sub(
        (365u64.saturating_mul(yoe))
            .saturating_add(yoe / 4)
            .saturating_sub(yoe / 100),
    );
    let mp = (doy.saturating_mul(5).saturating_add(2)) / 153;
    let d = doy
        .saturating_sub((153u64.saturating_mul(mp).saturating_add(2)) / 5)
        .saturating_add(1);
    let m = if mp < 10 {
        mp.saturating_add(3)
    } else {
        mp.saturating_sub(9)
    };
    let y = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(u64::from(m <= 2));
    (y, m, d)
}

/// A save in the folder: its file and its first line.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveEntry {
    pub path: PathBuf,
    pub header: SaveHeader,
}

/// Whether the folder has any save.
pub fn any_save(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .is_ok_and(|mut entries| entries.any(|e| e.is_ok_and(|e| is_save(&e.path()))))
}

fn is_save(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == EXTENSION) && path.is_file()
}

/// The saves of the folder whose first line reads, newest first, at most
/// [`MAX_SAVES_LISTED`].
pub fn list_saves(dir: &Path) -> Vec<SaveEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut saves: Vec<SaveEntry> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| is_save(p))
        .filter_map(|path| read_header(&path).map(|header| SaveEntry { path, header }))
        .collect();
    saves.sort_by(|a, b| {
        b.header
            .saved_at
            .cmp(&a.header.saved_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    saves.truncate(MAX_SAVES_LISTED);
    saves
}

/// The first line of a save, checked; `None` if it cannot be read.
fn read_header(path: &Path) -> Option<SaveHeader> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader =
        std::io::BufReader::new(std::io::Read::take(file, MAX_SAVE_HEADER_BYTES as u64));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).ok()?;
    let line = line.strip_suffix(b"\n")?;
    parse_header(line)
}

fn parse_header(line: &[u8]) -> Option<SaveHeader> {
    let mut header: SaveHeader = serde_json::from_slice(line).ok()?;
    if header.format != SAVE_FORMAT || header.selection.len() > MAX_SELECTION {
        return None;
    }
    header.title = clean_title(&header.title);
    Some(header)
}

/// A title read from a file, kept to what a title is: a few words and
/// sound numbers, words of letters, digits, spaces and `._-`.
fn clean_title(title: &Msg) -> Msg {
    let clean = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
            .take(MAX_TITLE_WORD_CHARS)
            .collect()
    };
    title
        .iter()
        .take(MAX_TITLE_PARTS)
        .map(|p| match p {
            MsgPart::Num(n) => MsgPart::Num(*n),
            MsgPart::Text(s) => MsgPart::Text(clean(s)),
            MsgPart::Literal(s) => MsgPart::Literal(clean(s)),
        })
        .filter(|p| !matches!(p, MsgPart::Text(s) | MsgPart::Literal(s) if s.is_empty()))
        .collect()
}

/// Why a save cannot be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The file cannot be read.
    Unreadable(std::io::Error),
    /// The first line is not a save's.
    NotASave,
    /// The record cannot be read.
    Record(ReadError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Unreadable(e) => write!(f, "cannot read the file: {e}"),
            LoadError::NotASave => write!(f, "not a saved game"),
            LoadError::Record(e) => write!(f, "the record: {e}"),
        }
    }
}

impl LoadError {
    /// What the player hears: why, in a few words.
    pub fn spoken(&self) -> Msg {
        let why = match self {
            LoadError::Record(ReadError::OtherVersion { .. }) => words::ANOTHER_VERSION,
            LoadError::Unreadable(_) | LoadError::NotASave | LoadError::Record(_) => words::DAMAGED,
        };
        [text(words::CANNOT_LOAD), msg(mp::COMMA), text(why)].concat()
    }
}

/// A save read: its first line, its record, and whether the record was
/// whole.
#[derive(Clone, Debug)]
pub struct Saved {
    pub header: SaveHeader,
    pub record: Record,
    pub completeness: Completeness,
}

/// Reads a save. A record cut short reads up to its last whole turn.
pub fn read_save(path: &Path) -> Result<Saved, LoadError> {
    let limit = MAX_RECORD_BYTES.saturating_add(MAX_SAVE_HEADER_BYTES);
    let bytes = read_file(path, limit as u64).map_err(LoadError::Unreadable)?;
    read_save_bytes(&bytes)
}

/// Reads a save from its bytes.
pub fn read_save_bytes(bytes: &[u8]) -> Result<Saved, LoadError> {
    let end = bytes
        .iter()
        .take(MAX_SAVE_HEADER_BYTES)
        .position(|&b| b == b'\n')
        .ok_or(LoadError::NotASave)?;
    let (first, rest) = bytes.split_at(end);
    let header = parse_header(first).ok_or(LoadError::NotASave)?;
    let rest = rest.get(1..).unwrap_or_default();
    let (record, completeness) = Record::read(rest).map_err(LoadError::Record)?;
    Ok(Saved {
        header,
        record,
        completeness,
    })
}

/// The player a game's interface played when it was saved: the first
/// seat, then each player a cheat played as.
pub fn played_seat(record: &Record, first: PlayerId) -> PlayerId {
    let mut me = first;
    for turn in record.turns() {
        for (player, command) in turn.commands() {
            if *player == me
                && let Command::Cheat(Cheat::PlayAs(other)) = command
            {
                me = *other;
            }
        }
    }
    me
}

/// What the list of saves says of one: "last game" for the game saved
/// when the player quit, its title, its game time, and how long ago it was
/// saved, as of `now` (seconds since 1970).
pub fn spoken_entry(header: &SaveHeader, now: u64) -> Msg {
    spoken_entry_with(header, now, words::LAST_GAME)
}

/// As [`spoken_entry`], with `last` for the game kept by itself.
pub fn spoken_entry_with(header: &SaveHeader, now: u64, last: &str) -> Msg {
    let mut m = Vec::new();
    if header.last_game {
        m.extend(text(last));
        m.extend(msg(mp::COMMA));
    }
    m.extend(header.title.iter().cloned());
    m.extend(msg(mp::COMMA));
    let minutes = header.game_time_ms / 60_000;
    m.extend(nb2msg(i64::try_from(minutes).unwrap_or(i64::MAX)));
    m.extend(msg(mp::MINUTES));
    m.extend(msg(mp::COMMA));
    m.extend(age(now.saturating_sub(header.saved_at)));
    if header.sim_version != SIM_VERSION {
        m.extend(msg(mp::COMMA));
        m.extend(text(words::ANOTHER_VERSION));
    }
    m
}

/// "saved just now", "saved 3 hours ago"...
fn age(seconds: u64) -> Msg {
    let (n, unit) = match seconds {
        0..60 => return text(words::SAVED_JUST_NOW),
        60..3600 => (seconds / 60, words::MINUTES_AGO),
        3600..86_400 => (seconds / 3600, words::HOURS_AGO),
        _ => (seconds / 86_400, words::DAYS_AGO),
    };
    [
        text(words::SAVED_WORD),
        nb2msg(i64::try_from(n).unwrap_or(i64::MAX)),
        text(unit),
    ]
    .concat()
}

/// Replays the record or the save at `path`, for the command line
/// (`--check-record`): how many turns ran and how the game ended, or why it
/// cannot be replayed. This is how a game a bug report brings is played
/// again.
pub fn check_record(path: &Path) -> Result<String, String> {
    let named = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let limit = MAX_RECORD_BYTES.saturating_add(MAX_SAVE_HEADER_BYTES);
    let bytes = read_file(path, limit as u64).map_err(|e| named(&e))?;
    let (record, completeness) = match Record::read(&bytes) {
        Ok(read) => read,
        // A save: its own line, then the record.
        Err(ReadError::NotARecord) => {
            let saved = read_save_bytes(&bytes).map_err(|e| named(&e))?;
            (saved.record, saved.completeness)
        }
        Err(e) => return Err(named(&e)),
    };
    let started = std::time::Instant::now();
    let host = soundrts_world::Host::replay(&record, |_| std::ops::ControlFlow::Continue(()))
        .map_err(|e| named(&format!("cannot be replayed: {e}")))?;
    let took = started.elapsed().as_secs_f64();
    let seconds = record.game_time_ms() / 1000;
    let mut out = format!(
        "{}: {} turns replayed, {} min {} s of game time, in {took:.1} s",
        path.display(),
        record.turns_run(),
        seconds / 60,
        seconds % 60,
    );
    if completeness == Completeness::CutShort {
        out.push_str("; the record was cut short and replayed to its last whole turn");
    }
    out.push_str(".\n");
    out.push_str(&how_it_ended(&host));
    out.push('\n');
    Ok(out)
}

/// Who is still in the game and who won, by the logins of the set-up.
fn how_it_ended(host: &soundrts_world::Host) -> String {
    let seats = host.seats();
    let Some(&first) = seats.first() else {
        return "The game has no player.".to_owned();
    };
    let controllers = &host.record().setup().controllers;
    let name = |p: PlayerId| -> String {
        seats
            .iter()
            .position(|&s| s == p)
            .and_then(|i| controllers.get(i))
            .map_or_else(|| format!("player {}", p.0), |c| c.login())
    };
    let list = |players: Vec<PlayerId>| -> String {
        players.into_iter().map(name).collect::<Vec<_>>().join(", ")
    };
    let in_game = host.message(first).state.players;
    let winners: Vec<PlayerId> = seats
        .iter()
        .copied()
        .filter(|&s| host.message(s).state.has_victory)
        .collect();
    let mut out = if in_game.is_empty() {
        "The game is over: no player is left in it.".to_owned()
    } else {
        format!("In the game at the end: {}.", list(in_game))
    };
    if !winners.is_empty() {
        out.push_str(&format!(" Won: {}.", list(winners)));
    }
    out
}

pub mod listen;
pub mod load;

#[cfg(test)]
mod tests;
