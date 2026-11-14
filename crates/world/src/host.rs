//! The host of one game: it owns the world, takes the players' commands and
//! steps the game a turn at a time.

use std::sync::Arc;

use crate::command::{Cheat, Command};
use crate::entity::PlayerId;
use crate::events::Event;
use crate::message::{GameEnd, GameStart, TurnMessage};
use crate::nofloat::PRECISION;
use crate::player::ControllerKind;
use crate::setup::GameSetup;
use crate::world::World;

/// The host of one game. It owns the world: everything that changes the
/// game goes through it, as a [`Command`] for a player, and the game moves
/// on only when the host is stepped.
///
/// Order: the commands queued before a [`Host::step`] run at the start of
/// that step, in the order they were queued, and before the turn itself.
/// The exception is the cheats that change the game at once
/// ([`Cheat::Resources`], [`Cheat::AllTechs`], [`Cheat::InfiniteMana`],
/// [`Cheat::PlayAs`]): the host carries them out when they are queued,
/// between steps, as the single-player console always has, and tells the
/// seat at once what it knows now.
///
/// Nothing steps the host on its own: the game loop and the tests call
/// [`Host::step`] when a turn is due.
///
/// What a seat is told is plain data, as a host over a network would send
/// it: [`Host::game_start`] once, a [`TurnMessage`] with each step, and
/// [`Host::game_end`] when the game is over for the seat.
pub struct Host {
    world: World,
    start: Arc<GameStart>,
    seats: Vec<PlayerId>,
    warnings: Vec<String>,
}

/// What a step gives back.
#[derive(Debug)]
pub struct TurnReport {
    /// The end of the seat's turn: everything as of then, with what the
    /// seat perceived during the turn. `None` for a seat that was no longer
    /// in the game at the end of the turn. Only a local human has one.
    pub end_of_turn: Option<Arc<TurnMessage>>,
    /// What the seat perceived after the end of its turn: the players
    /// leaving, when the turn ended the game. For a seat without an end of
    /// turn, everything it perceived during the step.
    pub then: Vec<Event>,
    /// What the seat knows after the step: the end of its turn, unless the
    /// game ended then or the seat had no end of turn.
    pub message: Arc<TurnMessage>,
    /// What went wrong in the game since the last step: a refused or
    /// broken command, a map problem... For the log; a player does not
    /// hear them.
    pub warnings: Vec<String>,
}

/// Whether a cheat changes the game at once.
enum AtOnce {
    /// It waits for the next step.
    No,
    /// The game refused it.
    Refused,
    /// It changed the game; this seat gave it, or now plays.
    Done(PlayerId),
}

impl Host {
    /// Builds the game of a set-up. The error is the map's. The warnings
    /// of building wait for [`Host::drain_warnings`] or the first step.
    pub fn new(setup: &GameSetup) -> Result<Host, String> {
        let (world, seats) = setup.build_world()?;
        Ok(Host {
            start: Arc::new(GameStart::of(&world)),
            world,
            seats,
            warnings: Vec::new(),
        })
    }

    /// The player of each controller of the set-up, in seat order.
    pub fn seats(&self) -> &[PlayerId] {
        &self.seats
    }

    /// Queues a command of `player` for the next step, or carries out at
    /// once a cheat that changes the game between turns (see [`Host`]).
    /// After such a cheat, returns what the seat is told of the game now:
    /// after [`Cheat::PlayAs`], the seat it plays as.
    #[must_use = "a cheat carried out at once changes what the seat is told now"]
    pub fn queue(&mut self, player: PlayerId, command: Command) -> Option<TurnMessage> {
        if let Command::Cheat(cheat) = &command {
            match self.cheat_at_once(player, cheat) {
                AtOnce::No => {}
                AtOnce::Refused => return None,
                AtOnce::Done(seat) => return Some(self.message(seat)),
            }
        }
        self.world.queue(player, command);
        None
    }

    /// Queues a command of `player` received as text: the orders of a game
    /// on a server, and those of a game record. A line that is not a
    /// command is a warning. Temporary: it goes when a game on a server
    /// sends commands as values.
    pub fn queue_line(&mut self, player: PlayerId, line: &str) {
        self.world.queue_command(player, line);
    }

    /// Runs one turn: the commands queued, then 300 ms of game time.
    /// Returns what `seat` was told and the warnings since the last step.
    pub fn step(&mut self, seat: PlayerId) -> TurnReport {
        self.world.update();
        let mut events = self.world.drain_events(seat);
        let end = self.world.player_mut(seat).end_of_turn.take();
        let (end_of_turn, then, message) = match end {
            Some(end) => {
                let then = events.split_off(end.after_events);
                let end = Arc::new(TurnMessage {
                    events,
                    ..end.message
                });
                // A seat leaves the game after the end of its turn only
                // when that turn ended the game.
                let message = if self.world.players.contains(&seat) {
                    Arc::clone(&end)
                } else {
                    Arc::new(self.message(seat))
                };
                (Some(end), then, message)
            }
            None => (None, events, Arc::new(self.message(seat))),
        };
        TurnReport {
            end_of_turn,
            then,
            message,
            warnings: self.drain_warnings(),
        }
    }

    /// What every seat is sent once, when the game starts.
    pub fn game_start(&self) -> Arc<GameStart> {
        Arc::clone(&self.start)
    }

    /// What `seat` is told of the game as it is now, without events: what
    /// a seat is sent when it joins, before its first step.
    pub fn message(&self, seat: PlayerId) -> TurnMessage {
        self.world.turn_message(seat, Vec::new())
    }

    /// What `seat` is sent when the game is over for it.
    pub fn game_end(&self, seat: PlayerId) -> GameEnd {
        self.world.game_end(seat)
    }

    /// The warnings since the last call or step.
    pub fn drain_warnings(&mut self) -> Vec<String> {
        let mut w = std::mem::take(&mut self.warnings);
        w.extend(self.world.drain_warnings());
        w
    }

    /// The text line of a command, as a game on a server sends it and the
    /// world reads it. `None` for the cheats the host carries out itself
    /// ([`Cheat::Resources`], [`Cheat::AllTechs`], [`Cheat::InfiniteMana`],
    /// [`Cheat::PlayAs`]), which have no text; a game on a server allows no
    /// cheat. Temporary: it goes when a game on a server sends commands as
    /// values.
    pub fn line(&self, command: &Command) -> Option<String> {
        command.line(&self.world)
    }

    /// A sum of the whole state of the game, to check that two machines
    /// play the same game: see [`World::digest_u64`].
    pub fn digest(&self) -> u64 {
        self.world.digest_u64()
    }

    /// The whole world, to check a game's results in tests. Only the
    /// `truth` feature has it, and the interface is built without that
    /// feature: a seat knows only what it is told.
    #[cfg(feature = "truth")]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Carries out a cheat that changes the game at once, or refuses it if
    /// the game allows no cheat. The other cheats wait for the next step.
    fn cheat_at_once(&mut self, player: PlayerId, cheat: &Cheat) -> AtOnce {
        if matches!(
            cheat,
            Cheat::ToggleMode | Cheat::Victory | Cheat::AddUnits { .. } | Cheat::Trigger(_)
        ) {
            return AtOnce::No;
        }
        if !self.world.allow_cheatmode {
            self.warnings
                .push(format!("cheat mode is not allowed: {cheat:?}"));
            return AtOnce::Refused;
        }
        let w = &mut self.world;
        match cheat {
            Cheat::Resources => {
                for r in &mut w.player_mut(player).resources {
                    *r += 1000 * PRECISION;
                }
            }
            Cheat::AllTechs => w.player_mut(player).cheat_has_all = true,
            Cheat::InfiniteMana => {
                for u in w.player(player).units.clone() {
                    if let Some(e) = w.entity_mut(u) {
                        e.stats.mana_regen *= 1000;
                    }
                }
            }
            &Cheat::PlayAs(other) => {
                if !w.players.contains(&other) {
                    self.warnings
                        .push(format!("no player {} to play as", other.0));
                    return AtOnce::Refused;
                }
                if other != player {
                    let login = w.player(other).controller.login();
                    w.player_mut(other).controller.kind =
                        ControllerKind::Human { login, local: true };
                    if let ControllerKind::Human { local, .. } =
                        &mut w.player_mut(player).controller.kind
                    {
                        *local = false;
                    }
                }
                return AtOnce::Done(other);
            }
            Cheat::ToggleMode | Cheat::Victory | Cheat::AddUnits { .. } | Cheat::Trigger(_) => {}
        }
        AtOnce::Done(player)
    }
}

#[cfg(test)]
mod tests;
