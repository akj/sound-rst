//! Saving a game run here: the game menu's "save game", and the game saved
//! by itself when the player leaves it before it is over (see
//! [`crate::saves`]).

use soundrts_world::msg::{Phrase, comma, msg};
use std::path::PathBuf;

use super::GameInterface;
use crate::client::Client;
use crate::saves::{self, InterfaceState, Slot};

impl GameInterface {
    /// Whether the game can be saved: a game run here that knows what it
    /// is.
    pub(crate) fn can_be_saved(&self) -> bool {
        self.source.local().is_some_and(|l| l.save.is_some())
    }

    /// Whether the game is still on for the player: in the game, without a
    /// victory.
    pub fn goes_on(&self) -> bool {
        self.view().players().contains(&self.me) && !self.view().has_victory()
    }

    /// What the interface keeps beside the record in a save. A game
    /// loaded and saved again before its first end of turn keeps the
    /// selection it has yet to take back.
    fn interface_state(&self) -> InterfaceState {
        let selection = match self.source.local() {
            Some(l) if !l.selection_to_restore.is_empty() => &l.selection_to_restore,
            _ => &self.group,
        };
        InterfaceState {
            cursor: self.place,
            selection: selection.clone(),
        }
    }

    /// Writes the game so far to `slot`. The commands given since the
    /// last turn go to the host first, as at the next turn, so that the
    /// save has them.
    pub(crate) fn save_game(&mut self, cx: &mut Client, slot: Slot) -> std::io::Result<PathBuf> {
        if !self.can_be_saved() {
            return Err(std::io::Error::other("this game cannot be saved"));
        }
        for c in std::mem::take(&mut self.pending_commands) {
            self.queue_at_once(c);
        }
        let ui = self.interface_state();
        let Some(l) = self.source.local() else {
            return Err(std::io::Error::other("a game not run here"));
        };
        let Some(info) = &l.save else {
            return Err(std::io::Error::other("this game cannot be saved"));
        };
        saves::write_save(
            &cx.paths.saves(),
            info,
            l.host.record(),
            &ui,
            saves::wall_now(),
            slot,
        )
    }

    /// The game menu's "save game": a new save, and "saved", or "cannot
    /// save". The game goes on.
    pub(crate) fn cmd_save_game(&mut self, cx: &mut Client) {
        match self.save_game(cx, Slot::New) {
            Ok(path) => {
                cx.log.info(format!("saved the game to {}", path.display()));
                cx.voice.item(&msg(Phrase::Saved));
            }
            Err(e) => {
                cx.log.warning(format!("cannot save the game: {e}"));
                cx.voice.item(&msg(Phrase::CannotSave));
            }
        }
    }

    /// Keeps the record of a game run here that has ended, won, lost or
    /// left, silently, as the last finished game of its kind in the
    /// records folder, where the player can listen to it.
    pub fn keep_finished(&mut self, cx: &mut Client) {
        let Some(l) = self.source.local() else {
            return;
        };
        let Some(info) = &l.save else {
            return;
        };
        let written = saves::write_save(
            &cx.paths.records(),
            info,
            l.host.record(),
            &InterfaceState::default(),
            saves::wall_now(),
            Slot::LastGame,
        );
        match written {
            Ok(path) => cx
                .log
                .info(format!("kept the finished game in {}", path.display())),
            Err(e) => cx
                .log
                .warning(format!("cannot keep the finished game: {e}")),
        }
    }

    /// Saves the game as the last game, silently, if it can be saved and
    /// is not over: the player is leaving it.
    pub fn save_on_leaving(&mut self, cx: &mut Client) {
        if !self.can_be_saved() || !self.goes_on() {
            return;
        }
        match self.save_game(cx, Slot::LastGame) {
            Ok(path) => cx
                .log
                .info(format!("saved the game left to {}", path.display())),
            Err(e) => cx.log.warning(format!("cannot save the game left: {e}")),
        }
    }

    /// Takes back what the interface kept in a save: the cursor's square,
    /// if the map has it, and the units selected, from the first end of
    /// turn, which keeps those the player still has. What is told when the
    /// game goes on: "loaded", and the square under the cursor.
    pub(crate) fn resume(&mut self, ui: InterfaceState) -> soundrts_world::msg::Msg {
        let squares = self.view().square_count();
        self.place = ui
            .cursor
            .filter(|s| usize::try_from(s.0).is_ok_and(|i| i < squares));
        if let Some(l) = self.source.local_mut() {
            l.selection_to_restore = ui.selection;
        }
        let mut told = msg(Phrase::Loaded);
        if let Some(s) = self.place {
            told.extend(comma());
            told.extend(self.view().square(s).title().iter().cloned());
        }
        told
    }
}
