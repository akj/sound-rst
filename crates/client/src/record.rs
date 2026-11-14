//! The record of a game on a server: what its world was built from, then
//! the orders and the digest of each turn. A record runs the same game
//! again on any machine, without the game data, and [`GameRecord::check`]
//! says where this machine computes something different.
//!
//! The text is JSON lines: the setup on the first line, then one turn a
//! line, so that a plain diff of two players' records starts at the first
//! turn where they differ.
//!
//! A record is a file anyone can hand over, so reading one is bounded by
//! the limits below, and a record that makes the simulation fail is
//! reported as invalid like any other.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use serde::{Deserialize, Serialize};
use soundrts_protocol::{Digest, MAX_ORDER_BYTES, MAX_TURN_ORDER_BYTES, SlotIndex, Turn};
use soundrts_world::{Controller, ControllerKind, GameSetup, Host, SIM_VERSION};

use crate::package::read_file;
use crate::session::SessionKind;

/// The largest record file read, in bytes. A turn with no order takes
/// about 50 bytes, so a game of 40 hours at the fastest speed (75 ms a
/// turn) takes about 100 MB.
pub const MAX_RECORD_BYTES: u64 = 256 * 1024 * 1024;

/// The most turns a record may hold: more than 41 hours at the fastest
/// speed.
pub const MAX_RECORD_TURNS: usize = 2_000_000;

/// The most orders a turn may hold. A server passes on at most
/// [`MAX_TURN_ORDER_BYTES`] of orders from each of at most 255 players
/// for a turn, and an order is at least one byte.
pub const MAX_RECORD_ORDERS_PER_TURN: usize = 255 * MAX_TURN_ORDER_BYTES;

/// What a game's world was built from, and who recorded it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setup {
    /// The simulation's version ([`SIM_VERSION`]) of the recorder.
    pub sim_version: u32,
    /// The rules text.
    pub rules: String,
    /// The computer players' text.
    pub ai: String,
    /// The map's definition.
    pub map: String,
    pub seed: i64,
    /// In slot order.
    pub players: Vec<RecordedPlayer>,
    /// The recorder's slot. It does not change the game.
    pub you: SlotIndex,
}

/// A player of a slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedPlayer {
    #[serde(flatten)]
    pub who: Who,
    pub faction: String,
    pub alliance: Option<String>,
}

/// Who plays a slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    /// A person, by login.
    Human(String),
    /// A computer, by AI type.
    Computer(String),
}

/// An order queued for a turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedOrder {
    pub slot: SlotIndex,
    pub order: String,
}

/// A turn run: the orders queued before it, in order, and the digest of
/// the world after it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedTurn {
    /// The server's number for the turn.
    pub turn: Turn,
    pub orders: Vec<RecordedOrder>,
    pub digest: Digest,
}

/// The first line of a record: the setup, with a `format` field that names
/// the format and its version.
#[derive(Serialize, Deserialize)]
#[serde(tag = "format")]
enum SetupLine {
    #[serde(rename = "soundrts-game-record-1")]
    V1(Setup),
}

/// A game on a server, as one player ran it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRecord {
    pub setup: Setup,
    /// The turns run, in order.
    pub turns: Vec<RecordedTurn>,
}

/// A turn whose digest differs from the recorded one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Difference {
    pub turn: Turn,
    pub recorded: Digest,
    pub replayed: Digest,
}

/// What replaying a record found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// Another version of the simulation made the record: it was not
    /// replayed.
    OtherVersion { recorded: u32 },
    /// The record cannot be replayed: its map cannot be built, or an order
    /// names a slot that does not exist.
    Invalid(String),
    /// The turns replayed, which stop after the first that differs.
    Replayed {
        turns: usize,
        first_difference: Option<Difference>,
    },
}

impl Check {
    /// Every turn was replayed and matched.
    pub fn passed(&self) -> bool {
        matches!(
            self,
            Check::Replayed {
                first_difference: None,
                ..
            }
        )
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Check::OtherVersion { recorded } => write!(
                f,
                "This record was made with simulation version {recorded}, and this program has version {SIM_VERSION}. It cannot replay the record."
            ),
            Check::Invalid(reason) => write!(f, "The record cannot be replayed: {reason}."),
            Check::Replayed {
                turns,
                first_difference: None,
            } => write!(f, "Replayed {turns} turns. Every turn matches the record."),
            Check::Replayed {
                turns,
                first_difference: Some(d),
            } => write!(
                f,
                "Replayed {turns} turns. Turn {} differs: the record has digest {:016x}, and this machine computes {:016x}.",
                d.turn, d.recorded, d.replayed
            ),
        }
    }
}

impl RecordedPlayer {
    /// The record of a slot's controller.
    pub fn of(c: &Controller) -> RecordedPlayer {
        let who = match &c.kind {
            ControllerKind::Human { login, .. } => Who::Human(login.clone()),
            ControllerKind::Computer { ai_type } => Who::Computer(ai_type.clone()),
            ControllerKind::Neutral => unreachable!("a slot has a human or a computer"),
        };
        RecordedPlayer {
            who,
            faction: c.faction.clone(),
            alliance: c.alliance.clone(),
        }
    }

    /// The controller of the slot. No human is local: a replay has no one
    /// to tell what happens.
    fn controller(&self) -> Controller {
        let mut c = match &self.who {
            Who::Human(login) => Controller::human(login, false),
            Who::Computer(ai_type) => Controller::computer(ai_type),
        };
        c.faction = self.faction.clone();
        c.alliance = self.alliance.clone();
        c
    }
}

impl GameRecord {
    /// A record with no turn yet.
    pub fn new(setup: Setup) -> GameRecord {
        GameRecord {
            setup,
            turns: Vec::new(),
        }
    }

    /// The record as JSON lines.
    #[expect(
        clippy::expect_used,
        reason = "strings, numbers and lists of them always serialize"
    )]
    pub fn to_text(&self) -> String {
        let mut text =
            serde_json::to_string(&SetupLine::V1(self.setup.clone())).expect("a setup is JSON");
        text.push('\n');
        for turn in &self.turns {
            text.push_str(&serde_json::to_string(turn).expect("a turn is JSON"));
            text.push('\n');
        }
        text
    }

    /// Reads the text of [`GameRecord::to_text`], within
    /// [`MAX_RECORD_TURNS`], [`MAX_RECORD_ORDERS_PER_TURN`] and
    /// [`MAX_ORDER_BYTES`].
    pub fn parse(text: &str) -> Result<GameRecord, String> {
        Self::parse_within(text, MAX_RECORD_TURNS, MAX_RECORD_ORDERS_PER_TURN)
    }

    fn parse_within(text: &str, max_turns: usize, max_orders: usize) -> Result<GameRecord, String> {
        let mut lines = text.lines();
        let first = lines.next().ok_or("the record is empty")?;
        let SetupLine::V1(setup) =
            serde_json::from_str(first).map_err(|e| format!("line 1: {e}"))?;
        let mut turns: Vec<RecordedTurn> = Vec::new();
        for (n, line) in (2_usize..).zip(lines) {
            if turns.len() >= max_turns {
                return Err(format!("more than {max_turns} turns"));
            }
            let turn: RecordedTurn =
                serde_json::from_str(line).map_err(|e| format!("line {n}: {e}"))?;
            if turn.orders.len() > max_orders {
                return Err(format!("line {n}: more than {max_orders} orders"));
            }
            if turn.orders.iter().any(|o| o.order.len() > MAX_ORDER_BYTES) {
                return Err(format!(
                    "line {n}: an order longer than {MAX_ORDER_BYTES} bytes"
                ));
            }
            turns.push(turn);
        }
        Ok(GameRecord { setup, turns })
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }

    /// Reads a record file of at most [`MAX_RECORD_BYTES`].
    pub fn read(path: &Path) -> Result<GameRecord, String> {
        let bytes = read_file(path, MAX_RECORD_BYTES)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let text =
            String::from_utf8(bytes).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        GameRecord::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Builds the world as the game did, runs each turn with its orders,
    /// and compares each digest with the recorded one, up to the first that
    /// differs. A record that makes the simulation fail is invalid.
    pub fn check(&self) -> Check {
        guarded(|| self.replay())
    }

    fn replay(&self) -> Check {
        let s = &self.setup;
        if s.sim_version != SIM_VERSION {
            return Check::OtherVersion {
                recorded: s.sim_version,
            };
        }
        // As `Session::multiplayer` and `Session::start` build a game on a
        // server.
        let kind = SessionKind::Multiplayer;
        let setup = GameSetup {
            rules: s.rules.clone(),
            ai: s.ai.clone(),
            map: s.map.clone(),
            seed: s.seed,
            controllers: s.players.iter().map(RecordedPlayer::controller).collect(),
            default_triggers: kind.default_triggers(),
            allow_cheatmode: false,
            random_starts: true,
            equivalents: kind.equivalents(),
        };
        let mut host = match Host::new(&setup) {
            Ok(host) => host,
            Err(e) => return Check::Invalid(format!("map error: {e}")),
        };
        // No player of a replay is local: nobody receives events.
        let Some(&recorder) = host.seats().get(s.you) else {
            return Check::Invalid(format!("no slot {}", s.you));
        };
        for (i, t) in self.turns.iter().enumerate() {
            for o in &t.orders {
                let Some(&p) = host.seats().get(o.slot) else {
                    return Check::Invalid(format!("turn {}: no slot {}", t.turn, o.slot));
                };
                host.queue_line(p, &o.order);
            }
            host.step(recorder);
            let replayed = host.digest();
            if replayed != t.digest {
                return Check::Replayed {
                    turns: i.saturating_add(1),
                    first_difference: Some(Difference {
                        turn: t.turn,
                        recorded: t.digest,
                        replayed,
                    }),
                };
            }
        }
        Check::Replayed {
            turns: self.turns.len(),
            first_difference: None,
        }
    }
}

/// The check `f` makes, or an invalid record if it panics.
fn guarded(f: impl FnOnce() -> Check) -> Check {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let reason = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("no reason given");
        Check::Invalid(format!("the simulation failed: {reason}"))
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    fn record() -> GameRecord {
        GameRecord {
            setup: Setup {
                sim_version: 1,
                rules: "def peasant\nclass worker\n".into(),
                ai: "def easy\n".into(),
                map: "square_width 12\n".into(),
                seed: 7,
                players: vec![
                    RecordedPlayer {
                        who: Who::Human("andrew".into()),
                        faction: "random_faction".into(),
                        alliance: Some("1".into()),
                    },
                    RecordedPlayer {
                        who: Who::Computer("easy".into()),
                        faction: "human".into(),
                        alliance: None,
                    },
                ],
                you: 0,
            },
            turns: vec![
                RecordedTurn {
                    turn: 0,
                    orders: vec![RecordedOrder {
                        slot: 1,
                        order: "go a1 \"quoted\"".into(),
                    }],
                    digest: u64::MAX,
                },
                RecordedTurn {
                    turn: 1,
                    orders: Vec::new(),
                    digest: 0x0123_4567_89ab_cdef,
                },
            ],
        }
    }

    #[test]
    fn a_record_read_back_from_its_text_is_equal() {
        let r = record();
        let text = r.to_text();
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(
            text.starts_with(r#"{"format":"soundrts-game-record-1","sim_version":1,"#),
            "{text}"
        );
        assert_eq!(
            text.lines().nth(2),
            Some(r#"{"turn":1,"orders":[],"digest":81985529216486895}"#)
        );
        assert_eq!(GameRecord::parse(&text), Ok(r));
    }

    #[test]
    fn a_record_of_another_simulation_version_is_not_replayed() {
        let mut r = record();
        r.setup.sim_version = SIM_VERSION + 1;
        let check = r.check();
        assert_eq!(
            check,
            Check::OtherVersion {
                recorded: SIM_VERSION + 1
            }
        );
        assert!(!check.passed());
        assert_eq!(
            check.to_string(),
            format!(
                "This record was made with simulation version {}, and this program has version {SIM_VERSION}. It cannot replay the record.",
                SIM_VERSION + 1
            )
        );
    }

    #[test]
    fn a_record_of_another_format_is_refused() {
        let text = record()
            .to_text()
            .replace("soundrts-game-record-1", "soundrts-game-record-2");
        let e = GameRecord::parse(&text).unwrap_err();
        assert!(e.starts_with("line 1: unknown variant"), "{e}");
    }

    #[test]
    fn a_record_over_the_limits_is_refused() {
        let text = record().to_text();
        assert!(GameRecord::parse_within(&text, 2, 1).is_ok());
        let e = GameRecord::parse_within(&text, 1, 1).unwrap_err();
        assert_eq!(e, "more than 1 turns");
        let e = GameRecord::parse_within(&text, 2, 0).unwrap_err();
        assert_eq!(e, "line 2: more than 0 orders");
        let mut r = record();
        r.turns[0].orders[0].order = "x".repeat(MAX_ORDER_BYTES + 1);
        let e = GameRecord::parse(&r.to_text()).unwrap_err();
        assert_eq!(
            e,
            format!("line 2: an order longer than {MAX_ORDER_BYTES} bytes")
        );
    }

    #[test]
    fn a_record_that_makes_the_simulation_fail_is_invalid() {
        let check = guarded(|| panic!("boom"));
        assert_eq!(check, Check::Invalid("the simulation failed: boom".into()));
        assert_eq!(
            check.to_string(),
            "The record cannot be replayed: the simulation failed: boom."
        );
        let check = guarded(|| panic!("{}", 7));
        assert_eq!(check, Check::Invalid("the simulation failed: 7".into()));
    }

    #[test]
    fn a_record_file_that_is_not_text_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.jsonl");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(
            GameRecord::read(&path)
                .unwrap_err()
                .starts_with("cannot read")
        );
    }
}
