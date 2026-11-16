//! The single-player menu's "listen to a game": the saves, and the records
//! of the records folder, newest first. A game chosen is replayed for the
//! player to listen to, as a watcher hears a game (see
//! [`crate::game::replay`]); then the score of the player followed is
//! said, and the list comes back.
//!
//! The records folder holds the last finished game of each kind, kept by
//! itself when a game ends, the games kept with "keep this game's record",
//! and any record a player puts there: a save, or a record a server wrote.
//! A record is a file anyone can hand over: it is read as loading reads a
//! save, with the same limits and checks.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Msg, MsgPart, Phrase, comma, msg};
use soundrts_world::record::MAX_RECORD_BYTES;
use soundrts_world::{Completeness, ReadError, Record};

use super::{
    LoadError, MAX_SAVE_HEADER_BYTES, MAX_SAVES_LISTED, MAX_TITLE_WORD_CHARS, SaveHeader, SaveInfo,
    SavedGame, age, list_saves, read_save_bytes, spoken_entry_with, wall_now,
};
use crate::client::Client;
use crate::game::GameInterface;
use crate::game::replay::{Keep, Replay};
use crate::mapfile::Map;
use crate::menu::{Action, Exit, Flow, Menu};
use crate::package::read_file;

/// The extension of a record a server writes.
const RECORD_EXTENSION: &str = "jsonl";

/// A game the player can listen to.
#[derive(Clone, Debug, PartialEq)]
pub struct Listenable {
    pub path: PathBuf,
    pub what: What,
}

/// What a game to listen to is.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// A save of the saves folder.
    Save(SaveHeader),
    /// A save of the records folder: the last finished game, or one kept.
    Kept(SaveHeader),
    /// A record of the records folder without a save's first line, such
    /// as a server writes; when its file was last changed, in seconds
    /// since 1970.
    Record { modified: u64 },
}

impl What {
    /// When the game was saved, or its file written.
    fn time(&self) -> u64 {
        match self {
            What::Save(h) | What::Kept(h) => h.saved_at,
            What::Record { modified } => *modified,
        }
    }
}

/// Whether there is any game to listen to: a save, or a file in the
/// records folder.
pub fn any_to_listen(cx: &Client) -> bool {
    super::any_save(&cx.paths.saves()) || !list_records(&cx.paths.records()).is_empty()
}

/// The saves of `saves` and the records of `records`, newest first, at
/// most [`MAX_SAVES_LISTED`].
pub fn list_listenable(saves: &Path, records: &Path) -> Vec<Listenable> {
    let mut list: Vec<Listenable> = list_saves(saves)
        .into_iter()
        .map(|e| Listenable {
            path: e.path,
            what: What::Save(e.header),
        })
        .chain(list_records(records))
        .collect();
    list.sort_by(|a, b| {
        b.what
            .time()
            .cmp(&a.what.time())
            .then_with(|| a.path.cmp(&b.path))
    });
    list.truncate(MAX_SAVES_LISTED);
    list
}

/// The records of the folder: its saves whose first line reads, and its
/// files of records a server writes.
fn list_records(dir: &Path) -> Vec<Listenable> {
    let saves = list_saves(dir).into_iter().map(|e| Listenable {
        path: e.path,
        what: What::Kept(e.header),
    });
    let Ok(entries) = std::fs::read_dir(dir) else {
        return saves.collect();
    };
    let records = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == RECORD_EXTENSION) && p.is_file())
        .take(MAX_SAVES_LISTED)
        .map(|path| {
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            Listenable {
                path,
                what: What::Record { modified },
            }
        });
    saves.chain(records).collect()
}

/// What the list says of a game, as of `now`: a save as the load menu
/// says it, the last finished game as such, and a record by its file's
/// name and how long ago it was written.
pub fn spoken(entry: &Listenable, now: u64) -> Msg {
    match &entry.what {
        What::Save(h) => spoken_entry_with(h, now, Phrase::LastGame),
        What::Kept(h) => spoken_entry_with(h, now, Phrase::LastFinishedGame),
        What::Record { modified } => [
            readable_name(&entry.path),
            comma(),
            age(now.saturating_sub(*modified)),
        ]
        .concat(),
    }
}

/// A file's name as words: without its extension, its dashes and
/// underscores as spaces, and only letters, digits and spaces.
fn readable_name(path: &Path) -> Msg {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let words: String = stem
        .chars()
        .map(|c| if matches!(c, '-' | '_') { ' ' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .take(MAX_TITLE_WORD_CHARS)
        .collect();
    vec![MsgPart::literal(words.trim())]
}

/// The games to listen to, newest first, then back. Choosing one plays it
/// to the listener.
pub fn listen_menu(cx: &mut Client) -> Result<(), Exit> {
    let list = list_listenable(&cx.paths.saves(), &cx.paths.records());
    if list.is_empty() {
        cx.voice.item(&msg(Phrase::TheListIsEmpty));
        return Ok(());
    }
    let now = wall_now();
    let mut menu = Menu::new(msg(Phrase::ListenToAGame), Vec::new());
    for entry in list {
        let label = spoken(&entry, now);
        let entry = Rc::new(entry);
        menu.append(
            label,
            Action::run(move |cx| listen(cx, &entry).map(|()| Flow::CloseMenu)),
            None,
        );
    }
    menu.append(msg(Phrase::Back), Action::CloseMenu, None);
    menu.run(cx)
}

/// A game read to be listened to.
struct Read {
    record: Record,
    completeness: Completeness,
    /// What a save says it is; `None` for a record without a save's line.
    game: Option<SavedGame>,
    keep: Keep,
}

/// Reads the save or the record of `entry`, as loading reads a save.
fn read(entry: &Listenable) -> Result<Read, LoadError> {
    let limit = MAX_RECORD_BYTES.saturating_add(MAX_SAVE_HEADER_BYTES);
    let bytes = read_file(&entry.path, limit as u64).map_err(LoadError::Unreadable)?;
    match Record::read(&bytes) {
        Ok((record, completeness)) => Ok(Read {
            record,
            completeness,
            game: None,
            keep: Keep::AlreadyKept,
        }),
        // A save: its own line, then the record.
        Err(ReadError::NotARecord) => {
            let saved = read_save_bytes(&bytes)?;
            // A game kept in the records folder stays there; the last
            // finished game is replaced by the next one.
            let keep = match &entry.what {
                What::Kept(h) if !h.last_game => Keep::AlreadyKept,
                What::Save(_) | What::Kept(_) | What::Record { .. } => Keep::Save {
                    info: SaveInfo {
                        game: saved.header.game.clone(),
                        title: saved.header.title.clone(),
                    },
                    saved_at: saved.header.saved_at,
                },
            };
            Ok(Read {
                record: saved.record,
                completeness: saved.completeness,
                game: Some(saved.header.game),
                keep,
            })
        }
        Err(e) => Err(LoadError::Record(e)),
    }
}

/// Plays the game of `entry` to the listener, to its end, then says the
/// score of the player followed. What cannot be listened to is said, and
/// the menus come back.
pub fn listen(cx: &mut Client, entry: &Listenable) -> Result<(), Exit> {
    let path = &entry.path;
    let read = match read(entry) {
        Ok(read) => read,
        Err(e) => {
            cx.log
                .warning(format!("cannot listen to {}: {e}", path.display()));
            cx.voice.item(&e.spoken());
            return Ok(());
        }
    };
    if read.completeness == Completeness::CutShort {
        cx.log.warning(format!(
            "{} was cut short: it plays to turn {}",
            path.display(),
            read.record.turns_run()
        ));
    }
    let campaign = match &read.game {
        Some(SavedGame::Mission { campaign, .. }) => {
            cx.res.campaigns().into_iter().find(|c| &c.name == campaign)
        }
        Some(SavedGame::Training { .. }) | None => None,
    };
    if let Some(c) = &campaign {
        if let Some(mods) = &c.mods {
            cx.res.set_mods(mods);
        }
        cx.res.set_campaign(Some(c));
    }
    let result = match map_of(cx, &read, campaign.as_ref(), path) {
        Some(map) => {
            cx.res.set_map(Some(&map));
            let result = play(cx, read);
            cx.res.set_map(None);
            result
        }
        None => super::load::cannot_load(cx),
    };
    if campaign.is_some() {
        cx.res.set_campaign(None);
    }
    result
}

/// The map of a game to listen to: the game's own, if its files have it
/// with the same text, else the record's text, without its own sounds.
fn map_of(
    cx: &mut Client,
    read: &Read,
    campaign: Option<&crate::campaign::Campaign>,
    path: &Path,
) -> Option<Arc<Map>> {
    let text = &read.record.setup().map;
    if let (Some(c), Some(SavedGame::Mission { chapter, .. })) = (campaign, &read.game)
        && let Some(crate::campaign::Chapter::Mission { map, .. }) = c.chapters.get(*chapter)
        && &map.definition == text
    {
        return Some(Arc::new(map.clone()));
    }
    if let Some(map) = cx
        .res
        .multiplayer_maps()
        .into_iter()
        .find(|m| &m.definition == text)
    {
        return Some(map);
    }
    let name = match &read.game {
        Some(SavedGame::Training { map }) => map.clone(),
        Some(SavedGame::Mission { campaign, chapter }) => format!("{campaign}-{chapter}"),
        None => path
            .file_stem()
            .map_or_else(|| "record".to_owned(), |s| s.to_string_lossy().into_owned()),
    };
    let file_name = format!("{}.txt", crate::mapfile::name_from_path(&name));
    Map::from_bytes(text.clone().into_bytes(), &file_name)
        .ok()
        .map(Arc::new)
}

/// Replays the game read, from its start, following its first person,
/// until it ends or the listener leaves; then the score.
fn play(cx: &mut Client, read: Read) -> Result<(), Exit> {
    let mission = matches!(read.game, Some(SavedGame::Mission { .. }));
    let replay = match Replay::new(Arc::new(read.record), read.completeness, read.keep) {
        Ok(replay) => replay,
        Err(e) => {
            cx.log.warning(format!("cannot replay the record: {e}"));
            return super::load::cannot_load(cx);
        }
    };
    let Some(me) = replay.first_player() else {
        cx.log.warning("a record without a player");
        return super::load::cannot_load(cx);
    };
    let mut game = match GameInterface::listening(cx, replay, me) {
        Ok(game) => game,
        Err(e) => {
            cx.log.warning(format!("cannot replay the record: {e}"));
            return super::load::cannot_load(cx);
        }
    };
    game.now_following(cx);
    let intro = if mission {
        game.view()
            .map_intro()
            .iter()
            .map(|part| vec![part.clone()])
            .collect()
    } else {
        Vec::new()
    };
    crate::session::prepare(cx, &mut game, intro);
    game.run_loop(cx)?;
    crate::game::replay::finish_listening(cx, &mut game);
    cx.input.set_game(None);
    Ok(())
}

#[cfg(test)]
mod tests;
