//! The single-player menu's "load a game": the saves, newest first. A save
//! chosen is replayed, while the player hears "loading" and how far it
//! has got, and Escape gives up; then the game goes on as an ordinary
//! single-player game. Delete removes a save once the player confirms.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::{Completeness, ControllerKind, Host, Record, ReplayError};

use super::{InterfaceState, SaveInfo, Saved, SavedGame, list_saves, played_seat, read_save};
use super::{spoken_entry, wall_now};
use crate::client::Client;
use crate::input::{Event, InputPoint, Key};
use crate::mapfile::Map;
use crate::menu::{Action, Choice, Exit, Flow, Menu};
use crate::net::words::{self, text};
use crate::session::{LoadedGame, Session, SessionKind};

/// How many turns are replayed between two looks at the keys.
const TURNS_BETWEEN_KEYS: u64 = 50;

/// How often the player hears how far loading has got, in seconds.
const PROGRESS_EVERY: f64 = 2.0;

/// The saves, newest first, then back. Choosing one loads it and plays it;
/// Delete on one asks to confirm, then removes it.
pub fn load_menu(cx: &mut Client) -> Result<(), Exit> {
    loop {
        let list = list_saves(&cx.paths.saves());
        if list.is_empty() {
            cx.voice.item(&msg(words::THE_LIST_IS_EMPTY));
            return Ok(());
        }
        let now = wall_now();
        let again = Rc::new(Cell::new(false));
        let mut menu = Menu::new(text(words::LOAD_A_GAME), Vec::new());
        for entry in list {
            let path = Rc::new(entry.path);
            let load = {
                let path = path.clone();
                Action::run(move |cx| load_game(cx, &path).map(|()| Flow::CloseMenu))
            };
            let delete = {
                let again = again.clone();
                Rc::new(move |cx: &mut Client| {
                    confirm_and_delete(cx, &path)?;
                    again.set(true);
                    Ok(Flow::Stay)
                })
            };
            menu.choices.push(Choice {
                on_delete: Some(delete),
                ..Choice::new(spoken_entry(&entry.header, now), load)
            });
        }
        menu.append(msg(mp::BACK), Action::CloseMenu, None);
        menu.run(cx)?;
        if !again.take() {
            return Ok(());
        }
    }
}

/// Asks to confirm, then deletes the save and says so.
fn confirm_and_delete(cx: &mut Client, path: &Path) -> Result<(), Exit> {
    let yes = Rc::new(Cell::new(false));
    let mut menu = Menu::new(text(words::DELETE_THIS_SAVE), Vec::new());
    let chosen = yes.clone();
    menu.append(
        text(words::DELETE),
        Action::run(move |_| {
            chosen.set(true);
            Ok(Flow::CloseMenu)
        }),
        None,
    );
    menu.append(msg(mp::CANCEL), Action::Nothing, None);
    menu.run(cx)?;
    if yes.get() {
        match std::fs::remove_file(path) {
            Ok(()) => cx.voice.item(&text(words::DELETED)),
            Err(e) => {
                cx.log
                    .warning(format!("cannot delete {}: {e}", path.display()));
                cx.voice.item(&msg(mp::BEEP));
            }
        }
    }
    Ok(())
}

/// Loads the save at `path` and plays it to its end. What cannot be
/// loaded is said, and the menus come back.
pub fn load_game(cx: &mut Client, path: &Path) -> Result<(), Exit> {
    let saved = match read_save(path) {
        Ok(saved) => saved,
        Err(e) => {
            cx.log
                .warning(format!("cannot load {}: {e}", path.display()));
            cx.voice.item(&e.spoken());
            return Ok(());
        }
    };
    if saved.completeness == Completeness::CutShort {
        cx.log.warning(format!(
            "{} was cut short: loaded to turn {}",
            path.display(),
            saved.record.turns_run()
        ));
    }
    match saved.header.game.clone() {
        SavedGame::Training { map } => {
            let found = cx
                .res
                .multiplayer_maps()
                .into_iter()
                .find(|m| m.name == map && m.definition == saved.record.setup().map);
            let Some(map) = found.or_else(|| map_of(&saved)) else {
                return cannot_load(cx);
            };
            match replayed(cx, SessionKind::Training, map, saved)? {
                Some(mut game) => game.run(cx),
                None => Ok(()),
            }
        }
        SavedGame::Mission { campaign, chapter } => {
            let found = cx.res.campaigns().into_iter().find(|c| c.name == campaign);
            match found {
                Some(campaign) => {
                    crate::campaign::resume_mission(cx, Rc::new(campaign), chapter, saved)
                }
                None => {
                    let Some(map) = map_of(&saved) else {
                        return cannot_load(cx);
                    };
                    match replayed(cx, SessionKind::Mission, map, saved)? {
                        Some(mut game) => game.run(cx),
                        None => Ok(()),
                    }
                }
            }
        }
    }
}

/// Says that the save cannot be loaded because it is damaged.
pub(crate) fn cannot_load(cx: &mut Client) -> Result<(), Exit> {
    cx.voice.item(&cannot_load_msg());
    Ok(())
}

fn cannot_load_msg() -> Msg {
    [
        text(words::CANNOT_LOAD),
        msg(mp::COMMA),
        text(words::DAMAGED),
    ]
    .concat()
}

/// The map of a save from its record alone, when the game's own files no
/// longer have it: the map's text, without its own sounds.
pub fn map_of(saved: &Saved) -> Option<Arc<Map>> {
    let name = match &saved.header.game {
        SavedGame::Training { map } => map.clone(),
        SavedGame::Mission { campaign, chapter } => format!("{campaign}-{chapter}"),
    };
    let file_name = format!("{}.txt", crate::mapfile::name_from_path(&name));
    let bytes = saved.record.setup().map.clone().into_bytes();
    Map::from_bytes(bytes, &file_name).ok().map(Arc::new)
}

/// Replays a save, saying "loading" and how far it has got; Escape gives
/// up. The game ready to go on, or `None` if it was given up or cannot be
/// replayed, which is said.
pub fn replayed(
    cx: &mut Client,
    kind: SessionKind,
    map: Arc<Map>,
    saved: Saved,
) -> Result<Option<Session>, Exit> {
    let Some(host) = replay_with_progress(cx, &saved.record)? else {
        return Ok(None);
    };
    // A game saved here has the local human first; a save without one
    // would never tell its interface a turn.
    let local_first = host
        .record()
        .setup()
        .controllers
        .first()
        .is_some_and(|c| matches!(c.kind, ControllerKind::Human { local: true, .. }));
    let (Some(&first), true) = (host.seats().first(), local_first) else {
        cx.log.warning("a saved game without a local player first");
        return cannot_load(cx).map(|()| None);
    };
    let me = played_seat(host.record(), first);
    let save = SaveInfo {
        game: saved.header.game,
        title: saved.header.title,
    };
    let ui = InterfaceState {
        cursor: saved.header.cursor,
        selection: saved.header.selection,
    };
    let game = LoadedGame { host, me, ui };
    Ok(Some(Session::resumed(kind, map, save, game)))
}

/// The host of a record, replayed: "loading", then how far it has got
/// every few seconds. Nothing of the game replayed is said or played.
/// Escape gives up ("loading stopped"); a record the game cannot replay is
/// said to be damaged. Closing the window stops it too.
fn replay_with_progress(cx: &mut Client, record: &Record) -> Result<Option<Host>, Exit> {
    cx.voice.item(&msg(mp::LOADING));
    let total = record.turns_run().max(1);
    let mut said_at = cx.clock.now();
    let mut closed = false;
    let result = Host::replay(record, |host| {
        let done = host.record().turns_run();
        if done % TURNS_BETWEEN_KEYS != 0 {
            return ControlFlow::Continue(());
        }
        cx.input.input_point(InputPoint::Menu, cx.voice.is_idle());
        while let Some(e) = cx.input.poll() {
            match e {
                Event::Quit => {
                    closed = true;
                    return ControlFlow::Break(());
                }
                Event::KeyDown(k) if k.key == Key::Escape => return ControlFlow::Break(()),
                Event::KeyDown(_) | Event::KeyUp(_) => {}
            }
        }
        let now = cx.clock.now();
        if now - said_at >= PROGRESS_EVERY {
            said_at = now;
            let percent = done.saturating_mul(100).checked_div(total).unwrap_or(100);
            let m = [
                nb2msg(i64::try_from(percent).unwrap_or(100)),
                text(words::PERCENT),
            ]
            .concat();
            cx.voice.item(&m);
        }
        cx.voice.update();
        ControlFlow::Continue(())
    });
    if closed {
        return Err(Exit);
    }
    match result {
        Ok(host) => Ok(Some(host)),
        Err(ReplayError::Stopped) => {
            cx.voice.item(&text(words::LOADING_STOPPED));
            Ok(None)
        }
        Err(e) => {
            cx.log.warning(format!("cannot replay the saved game: {e}"));
            cx.voice.item(&cannot_load_msg());
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests;
