//! Saving a game run here: the game menu's "save game", and the game saved
//! by itself when the player leaves it before it is over (see
//! [`crate::saves`]).

use std::path::PathBuf;

use super::{GameInterface, Source};
use crate::client::Client;
use crate::net::words::{self, text};
use crate::saves::{self, InterfaceState, SaveInfo, Slot};

impl GameInterface {
    /// Gives a game run here what it needs to be saved. A game on a
    /// server is not saved.
    pub fn set_save_info(&mut self, info: SaveInfo) {
        if let Source::Local(l) = &mut self.source {
            l.save = Some(info);
        }
    }

    /// Whether the game can be saved: a game run here that knows what it
    /// is.
    pub(crate) fn can_be_saved(&self) -> bool {
        matches!(&self.source, Source::Local(l) if l.save.is_some())
    }

    /// Whether the game is still on for the player: in the game, without a
    /// victory.
    pub fn goes_on(&self) -> bool {
        self.view().players().contains(&self.me) && !self.view().has_victory()
    }

    /// What the interface keeps beside the record in a save. A unit of
    /// the selection replaced in a turn not yet taken (an upgrade) is
    /// saved as the unit that took its place, as the game has it.
    fn interface_state(&self) -> InterfaceState {
        InterfaceState {
            cursor: self.place,
            selection: self.group_now().into_iter().map(|(_, now)| now).collect(),
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
        let Source::Local(l) = &self.source else {
            return Err(std::io::Error::other("a game on a server"));
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
                cx.voice.item(&text(words::SAVED));
            }
            Err(e) => {
                cx.log.warning(format!("cannot save the game: {e}"));
                cx.voice.item(&text(words::CANNOT_SAVE));
            }
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
    /// if the map has it, and the units selected, which the first end of
    /// turn keeps to those the player still has. What is told when the
    /// game goes on: "loaded", and the square under the cursor.
    pub(crate) fn resume(&mut self, ui: InterfaceState) -> soundrts_world::msg::Msg {
        let squares = self.view().square_count();
        self.place = ui
            .cursor
            .filter(|s| usize::try_from(s.0).is_ok_and(|i| i < squares));
        self.group = ui.selection;
        let mut told = text(words::LOADED);
        if let Some(s) = self.place {
            told.extend(soundrts_world::msg::msg(soundrts_world::msgparts::COMMA));
            told.extend(self.view().square(s).title().iter().cloned());
        }
        told
    }
}
