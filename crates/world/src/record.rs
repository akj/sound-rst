//! The record of a game: what it was built from, then the commands of each
//! turn that had any, in the order the host ran them, each with its player.
//! Nothing else: no state, no events, no digests. A [`Host`] keeps the
//! record of its game as it runs ([`Host::record`]), and a host is built
//! again from a record by replaying it ([`Host::replay`]). A saved game and
//! a server's record of a game are both a record.
//!
//! A record replays only on the version of the simulation that made it
//! ([`crate::SIM_VERSION`]): a record of another version is refused when
//! it is read, and never replayed.
//!
//! # The file
//!
//! With the `serde` feature, a record is written and read as JSON lines,
//! so that a record cut short by a crash still reads up to its last whole
//! line, and a person can read it:
//!
//! 1. `{"format":"soundrts-record-1","sim_version":10}`
//! 2. the set-up ([`GameSetup`]), with the texts of the rules, the
//!    computer players' scripts and the map, so that a record replays
//!    without the game's files;
//! 3. one line for each turn that had commands, in turn order:
//!    `{"turn":12,"commands":[[0,{"Control":[31]}],...]}`, each command
//!    after its player, as the host ran them;
//! 4. last, `{"end":4000}`: the number of turns run.
//!
//! A line is whole when its end of line is written. A record whose last
//! line is not whole, or that has no `end` line, was cut short: it reads
//! up to its last whole turn line, and the commands of that turn wait for
//! the next step, as if the game had been saved just after they were given.
//!
//! A record is a file anyone can hand over, so reading one is bounded by
//! the limits below, and nothing read is trusted: replaying a record whose
//! commands the game refuses, or that makes the simulation fail, says so
//! ([`ReplayError`]).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::command::Command;
use crate::entity::PlayerId;
use crate::host::Host;
use crate::rules::VIRTUAL_TIME_INTERVAL;
use crate::setup::GameSetup;

/// The name and version of the file format, on its first line.
pub const RECORD_FORMAT: &str = "soundrts-record-1";

/// The largest record read, in bytes. A turn with one command takes about
/// 50 bytes, so a game with a command every turn for 40 hours at the
/// fastest speed takes about 100 MB.
pub const MAX_RECORD_BYTES: usize = 256 * 1024 * 1024;

/// The longest line of a record read, in bytes: the set-up's line holds
/// the rules, the scripts and the map.
pub const MAX_RECORD_LINE_BYTES: usize = 16 * 1024 * 1024;

/// The largest text of a set-up read (the rules, the scripts, the map), in
/// bytes. The game's own rules and largest map are under 20 KB each.
pub const MAX_RECORD_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// The most players a set-up read may have.
pub const MAX_RECORD_PLAYERS: usize = 256;

/// The most default triggers a set-up read may have.
pub const MAX_RECORD_TRIGGERS: usize = 1000;

/// The most turns a record read may have run: more than 41 hours at the
/// fastest speed (75 ms a turn).
pub const MAX_RECORD_TURNS: u64 = 2_000_000;

/// The most commands one turn of a record read may have. A server takes
/// at most 32 KB of commands a turn from each player, and a command takes
/// a few bytes at least.
pub const MAX_RECORD_COMMANDS_PER_TURN: usize = 1 << 20;

/// The commands given before one step of a game, in the order the host
/// ran them, each with its player. Never empty.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedTurn {
    number: u64,
    commands: Vec<(PlayerId, Command)>,
}

impl RecordedTurn {
    /// How many turns had run when the commands were given: the commands of
    /// turn 0 run with the first step.
    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn commands(&self) -> &[(PlayerId, Command)] {
        &self.commands
    }
}

/// The record of a game, made by this version of the simulation: its
/// set-up, the turns that had commands, and how many turns have run.
///
/// The turns are in order, each of a different number, none after the
/// turns run. A turn of the number of turns run holds the commands given
/// since the last step: they wait for the next one, except the cheats that
/// act at once, which have already taken effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    setup: GameSetup,
    turns: Vec<RecordedTurn>,
    turns_run: u64,
}

impl Record {
    /// The record of a game just built from `setup`.
    pub(crate) fn new(setup: GameSetup) -> Record {
        Record {
            setup,
            turns: Vec::new(),
            turns_run: 0,
        }
    }

    /// What the game was built from.
    pub fn setup(&self) -> &GameSetup {
        &self.setup
    }

    /// The turns that had commands, in order.
    pub fn turns(&self) -> &[RecordedTurn] {
        &self.turns
    }

    /// How many turns have run.
    pub fn turns_run(&self) -> u64 {
        self.turns_run
    }

    /// The game time the turns run take, in milliseconds.
    pub fn game_time_ms(&self) -> u64 {
        self.turns_run
            .saturating_mul(VIRTUAL_TIME_INTERVAL.unsigned_abs())
    }

    /// Notes a command the host has taken, for the next step.
    pub(crate) fn push(&mut self, player: PlayerId, command: Command) {
        match self.turns.last_mut() {
            Some(t) if t.number == self.turns_run => t.commands.push((player, command)),
            _ => self.turns.push(RecordedTurn {
                number: self.turns_run,
                commands: vec![(player, command)],
            }),
        }
    }

    /// Notes a step.
    pub(crate) fn step(&mut self) {
        self.turns_run = self.turns_run.saturating_add(1);
    }
}

/// Why a record cannot be replayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The game cannot be built from the set-up: the map's error.
    Build(String),
    /// The game refused a command of the record: it is not a record this
    /// game made, or it was changed.
    Refused { turn: u64 },
    /// The simulation failed while replaying: what it said.
    Failed(String),
    /// The caller stopped the replay.
    Stopped,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Build(e) => write!(f, "the game cannot be built: {e}"),
            ReplayError::Refused { turn } => {
                write!(f, "the game refused a command of turn {turn}")
            }
            ReplayError::Failed(e) => write!(f, "the game failed: {e}"),
            ReplayError::Stopped => write!(f, "stopped"),
        }
    }
}

impl std::error::Error for ReplayError {}

impl Host {
    /// Builds the game of `record` again: from its set-up, then, for each
    /// turn run, the turn's commands queued in order and a step; last, the
    /// commands given since the last step are queued. The host keeps the
    /// record, and goes on recording from there.
    ///
    /// `after_step` is called after each step with the host as it is then
    /// and may stop the replay. What the steps tell the seats is dropped:
    /// nothing of the replayed game is told, and the host's warnings are
    /// dropped too.
    ///
    /// A record the game refuses a command of, or that makes the
    /// simulation fail, is an error, never a panic.
    pub fn replay(
        record: &Record,
        mut after_step: impl FnMut(&Host) -> ControlFlow<()>,
    ) -> Result<Host, ReplayError> {
        let run = || -> Result<Host, ReplayError> {
            let mut host = Host::new(&record.setup).map_err(ReplayError::Build)?;
            let mut turns = record.turns.iter().peekable();
            for step in 0..=record.turns_run {
                if let Some(turn) = turns.next_if(|t| t.number == step) {
                    for (player, command) in &turn.commands {
                        // A cheat that acts at once tells the seat what it
                        // knows now: nothing is told while replaying.
                        let _told = host.queue(*player, command.clone());
                    }
                }
                if step == record.turns_run {
                    break;
                }
                host.step_seats(&[]);
                host.drain_warnings();
                if after_step(&host).is_break() {
                    return Err(ReplayError::Stopped);
                }
            }
            host.drain_warnings();
            // Each command recorded was taken when it was given, so the
            // game takes it again; one refused is not in the new record.
            if let Some(turn) = first_difference(&record.turns, &host.record().turns) {
                return Err(ReplayError::Refused { turn });
            }
            Ok(host)
        };
        catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic".to_owned());
            Err(ReplayError::Failed(why))
        })
    }
}

/// The number of the first turn where two records' turns differ.
fn first_difference(a: &[RecordedTurn], b: &[RecordedTurn]) -> Option<u64> {
    let mut a = a.iter();
    let mut b = b.iter();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return None,
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => return Some(x.number.min(y.number)),
            (Some(t), None) | (None, Some(t)) => return Some(t.number),
        }
    }
}

#[cfg(feature = "serde")]
pub use file::{Completeness, ReadError};

/// The file form of a record: see the module's documentation.
#[cfg(feature = "serde")]
mod file {
    use serde::{Deserialize, Serialize};

    use super::*;

    /// Whether a record read was whole.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Completeness {
        Whole,
        /// The file was cut short: it was read up to its last whole turn.
        CutShort,
    }

    /// Why a file cannot be read as a record.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ReadError {
        /// More than [`MAX_RECORD_BYTES`].
        TooLarge,
        /// The first line does not name the format.
        NotARecord,
        /// A record of another version of the simulation.
        OtherVersion { found: u32 },
        /// The file ends before the set-up is whole.
        CutShort,
        /// A whole line that cannot be read, or that breaks the limits or
        /// the order of turns: its number, from 1, and why.
        Damaged { line: usize, why: String },
    }

    impl std::fmt::Display for ReadError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                ReadError::TooLarge => write!(f, "larger than {MAX_RECORD_BYTES} bytes"),
                ReadError::NotARecord => write!(f, "not a game record"),
                ReadError::OtherVersion { found } => write!(
                    f,
                    "made by version {found} of the simulation, this is version {}",
                    crate::SIM_VERSION
                ),
                ReadError::CutShort => write!(f, "cut short before the game's set-up"),
                ReadError::Damaged { line, why } => write!(f, "line {line}: {why}"),
            }
        }
    }

    impl std::error::Error for ReadError {}

    #[derive(Serialize, Deserialize)]
    struct Header {
        format: String,
        sim_version: u32,
    }

    /// A line after the set-up.
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Line {
        Turn(TurnLine),
        End(EndLine),
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TurnLine {
        turn: u64,
        commands: Vec<(PlayerId, Command)>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct EndLine {
        end: u64,
    }

    fn json_line<T: Serialize>(out: &mut impl std::io::Write, value: &T) -> std::io::Result<()> {
        serde_json::to_writer(&mut *out, value)?;
        out.write_all(b"\n")
    }

    impl Record {
        /// Writes the record as its file.
        pub fn write(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
            json_line(
                out,
                &Header {
                    format: RECORD_FORMAT.to_owned(),
                    sim_version: crate::SIM_VERSION,
                },
            )?;
            json_line(out, &self.setup)?;
            for t in &self.turns {
                json_line(
                    out,
                    &Line::Turn(TurnLine {
                        turn: t.number,
                        commands: t.commands.clone(),
                    }),
                )?;
            }
            json_line(
                out,
                &Line::End(EndLine {
                    end: self.turns_run,
                }),
            )
        }

        /// The record's file, as bytes.
        pub fn to_bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            // Writing to memory fails only if a value cannot be written as
            // JSON, which a record's values all can.
            if self.write(&mut out).is_err() {
                out.clear();
            }
            out
        }

        /// Reads a record's file. A file cut short reads up to its last
        /// whole turn line (see the module's documentation).
        pub fn read(bytes: &[u8]) -> Result<(Record, Completeness), ReadError> {
            if bytes.len() > MAX_RECORD_BYTES {
                return Err(ReadError::TooLarge);
            }
            // Only lines with their end of line are whole.
            let mut lines = bytes.split_inclusive(|&b| b == b'\n');
            let mut number = 0usize;
            let mut next_whole = || -> Option<(usize, &[u8])> {
                let line = lines.next()?;
                number = number.saturating_add(1);
                line.strip_suffix(b"\n").map(|l| (number, l))
            };
            let damaged = |line: usize, why: &str| ReadError::Damaged {
                line,
                why: why.to_owned(),
            };

            let (_, first) = next_whole().ok_or(ReadError::NotARecord)?;
            let header: Header =
                serde_json::from_slice(first).map_err(|_| ReadError::NotARecord)?;
            if header.format != RECORD_FORMAT {
                return Err(ReadError::NotARecord);
            }
            if header.sim_version != crate::SIM_VERSION {
                return Err(ReadError::OtherVersion {
                    found: header.sim_version,
                });
            }

            let (n, line) = next_whole().ok_or(ReadError::CutShort)?;
            if line.len() > MAX_RECORD_LINE_BYTES {
                return Err(damaged(n, "the line is too long"));
            }
            let setup: GameSetup =
                serde_json::from_slice(line).map_err(|e| damaged(n, &e.to_string()))?;
            check_setup(&setup).map_err(|why| damaged(n, why))?;

            let mut record = Record::new(setup);
            let mut end = None;
            while let Some((n, line)) = next_whole() {
                if end.is_some() {
                    return Err(damaged(n, "a line after the end"));
                }
                if line.len() > MAX_RECORD_LINE_BYTES {
                    return Err(damaged(n, "the line is too long"));
                }
                match serde_json::from_slice(line).map_err(|e| damaged(n, &e.to_string()))? {
                    Line::Turn(t) => {
                        let after_last = record.turns.last().is_none_or(|l| l.number < t.turn);
                        if !after_last || t.turn > MAX_RECORD_TURNS {
                            return Err(damaged(n, "a turn out of order"));
                        }
                        if t.commands.is_empty() || t.commands.len() > MAX_RECORD_COMMANDS_PER_TURN
                        {
                            return Err(damaged(n, "a turn with no commands or too many"));
                        }
                        record.turns.push(RecordedTurn {
                            number: t.turn,
                            commands: t.commands,
                        });
                    }
                    Line::End(e) => {
                        let after_turns = record.turns.last().is_none_or(|l| l.number <= e.end);
                        if !after_turns || e.end > MAX_RECORD_TURNS {
                            return Err(damaged(n, "an end before the last turn"));
                        }
                        end = Some(e.end);
                    }
                }
            }
            Ok(match end {
                Some(end) => {
                    record.turns_run = end;
                    (record, Completeness::Whole)
                }
                None => {
                    record.turns_run = record.turns.last().map_or(0, |t| t.number);
                    (record, Completeness::CutShort)
                }
            })
        }
    }

    /// Whether a set-up read keeps within the limits.
    fn check_setup(s: &GameSetup) -> Result<(), &'static str> {
        if [&s.rules, &s.ai, &s.map]
            .iter()
            .any(|t| t.len() > MAX_RECORD_TEXT_BYTES)
        {
            return Err("a text of the set-up is too long");
        }
        if s.controllers.len() > MAX_RECORD_PLAYERS {
            return Err("too many players");
        }
        if s.default_triggers.len() > MAX_RECORD_TRIGGERS {
            return Err("too many default triggers");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
