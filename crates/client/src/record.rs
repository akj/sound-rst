//! The record of a game on a server: what its world was built from, then
//! the orders and the digest of each turn. A record runs the same game
//! again on any machine, without the game data, and [`GameRecord::check`]
//! says where this machine computes something different.
//!
//! The text is JSON lines: the setup on the first line, then one turn a
//! line, so that a plain diff of two players' records starts at the first
//! turn where they differ.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use soundrts_protocol::{Digest, SlotIndex, Turn};
use soundrts_world::{Controller, ControllerKind, SIM_VERSION};

use crate::session::{SessionKind, WorldInputs, build_world};

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

    /// Reads the text of [`GameRecord::to_text`].
    pub fn parse(text: &str) -> Result<GameRecord, String> {
        let mut lines = text.lines();
        let first = lines.next().ok_or("the record is empty")?;
        let SetupLine::V1(setup) =
            serde_json::from_str(first).map_err(|e| format!("line 1: {e}"))?;
        let turns = lines
            .enumerate()
            .map(|(i, line)| serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 2)))
            .collect::<Result<_, _>>()?;
        Ok(GameRecord { setup, turns })
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }

    pub fn read(path: &Path) -> Result<GameRecord, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        GameRecord::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Builds the world as the game did, runs each turn with its orders,
    /// and compares each digest with the recorded one, up to the first that
    /// differs.
    pub fn check(&self) -> Check {
        let s = &self.setup;
        if s.sim_version != SIM_VERSION {
            return Check::OtherVersion {
                recorded: s.sim_version,
            };
        }
        let controllers: Vec<Controller> =
            s.players.iter().map(RecordedPlayer::controller).collect();
        // As `Session::multiplayer` and `Session::start` build a game on a
        // server.
        let kind = SessionKind::Multiplayer;
        let built = build_world(WorldInputs {
            rules: &s.rules,
            ai: &s.ai,
            default_triggers: kind.default_triggers(),
            seed: s.seed,
            allow_cheatmode: false,
            map_definition: &s.map,
            controllers: &controllers,
            random_starts: true,
            equivalents: kind.equivalents(),
        });
        let (mut world, players) = match built {
            Ok(built) => built,
            Err(e) => return Check::Invalid(format!("map error: {e}")),
        };
        for (i, t) in self.turns.iter().enumerate() {
            for o in &t.orders {
                let Some(&p) = players[..controllers.len()].get(o.slot) else {
                    return Check::Invalid(format!("turn {}: no slot {}", t.turn, o.slot));
                };
                world.queue_command(p, &o.order);
            }
            world.update();
            let replayed = world.digest_u64();
            if replayed != t.digest {
                return Check::Replayed {
                    turns: i + 1,
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

#[cfg(test)]
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
}
