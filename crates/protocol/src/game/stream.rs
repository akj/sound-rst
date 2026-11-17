//! A seat's game stream, checked: the frames a seat is sent, taken in
//! order, and what the interface may trust of them.
//!
//! A seat reads its stream in three stages, each a type that takes the
//! frames of its stage and gives the next:
//!
//! - [`JoinStream`]: a seat that joins the game, once its seat came. The
//!   start, then the seat's whole message. It gives a [`SeatStream`].
//! - [`SeatStream`]: the seat in the game. A change at each turn, the
//!   news of the game, and the end once.
//! - [`ReturnStream`]: a seat that returns after a lost connection. Its
//!   seat, the start and its whole message again, on the new connection,
//!   each the same as before. It gives a [`SeatStream`] again.
//!
//! A frame out of that order, or one that cannot be right for the game,
//! is refused with the reason, for the log. A refused stream is not read
//! further: the seat leaves the game.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use soundrts_world::{GameEnd, GameStart, PlayerId, Rules, SquareId, TurnMessage};

use super::{Role, RulesHash, SeatFrame, encode};
use crate::{
    ClockChange, Invalid, Login, MapHash, SeatChange, SeatToken, SlotIndex, Speed, StopReason,
};

/// What every stage knows of the seat, and checks a seat given back
/// against.
#[derive(Clone, Debug)]
struct Seat {
    you: Role,
    map: MapHash,
    /// The hash of the rules the seat reads the stream with.
    rules: RulesHash,
}

/// The stream of a seat that joins a game, from the frame after its seat:
/// the start of the game, then the seat's whole message.
///
/// The start must be of the rules the seat was given: the stream names
/// unit types, upgrades and factions by their place in them.
#[derive(Clone, Debug)]
pub struct JoinStream {
    seat: Seat,
    next: Join,
}

#[derive(Clone, Debug)]
enum Join {
    /// The start, read with these rules.
    Start(Rules),
    Whole(Arc<GameStart>),
}

/// What a frame of a joining seat's stream came to.
#[derive(Debug)]
pub enum Joining {
    /// The frame was the start: the whole message comes next.
    Started(JoinStream),
    /// The seat is in the game.
    Joined(Joined),
}

/// A seat in the game, with its first message.
#[derive(Debug)]
pub struct Joined {
    pub stream: SeatStream,
    pub message: Arc<TurnMessage>,
}

/// Why a joining seat cannot read the stream.
#[derive(Debug, PartialEq)]
pub enum JoinRefused {
    /// The game was built from other rules than the seat's.
    RulesDiffer { theirs: RulesHash, ours: RulesHash },
    /// What came cannot be right.
    Invalid(Invalid),
}

impl fmt::Display for JoinRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinRefused::RulesDiffer { theirs, ours } => {
                write!(f, "its rules are {theirs:?}, ours {ours:?}")
            }
            JoinRefused::Invalid(e) => e.fmt(f),
        }
    }
}

fn invalid<T>(why: impl Into<String>) -> Result<T, Invalid> {
    Err(Invalid(why.into()))
}

impl JoinStream {
    /// The stream of the seat `you` of a game on the map of hash `map`,
    /// read with the rules of the text `rules`, every layer joined as
    /// [`RulesHash::of`] takes it.
    pub fn new(you: Role, map: MapHash, rules: &str) -> JoinStream {
        JoinStream {
            seat: Seat {
                you,
                map,
                rules: RulesHash::of(rules),
            },
            next: Join::Start(Rules::load(&[rules])),
        }
    }

    /// Takes the next frame of the stream.
    pub fn take(self, frame: SeatFrame) -> Result<Joining, JoinRefused> {
        let JoinStream { seat, next } = self;
        let refused = |why: String| JoinRefused::Invalid(Invalid(why));
        match (next, frame) {
            (
                Join::Start(rules),
                SeatFrame::Start {
                    start,
                    rules: theirs,
                },
            ) => {
                start
                    .check()
                    .map_err(|e| refused(format!("the start: {}", e.0)))?;
                if theirs != seat.rules {
                    return Err(JoinRefused::RulesDiffer {
                        theirs,
                        ours: seat.rules,
                    });
                }
                let start = Arc::new(GameStart::from_message(start, rules));
                Ok(Joining::Started(JoinStream {
                    seat,
                    next: Join::Whole(start),
                }))
            }
            (Join::Whole(start), SeatFrame::Whole(m)) => {
                start
                    .check(&m)
                    .map_err(|e| refused(format!("the first message: {}", e.0)))?;
                let message = Arc::new(m);
                Ok(Joining::Joined(Joined {
                    stream: SeatStream {
                        seat,
                        start,
                        last: Arc::clone(&message),
                        over: false,
                    },
                    message,
                }))
            }
            (_, other) => Err(refused(format!("before the game: {}", kind(&other)))),
        }
    }
}

/// The stream of a seat in the game.
///
/// - A change is applied to the seat's last message, and the result
///   checked against the game: no earlier in the game than the last, and
///   no larger than a message the server could send whole, so that a
///   stream cannot grow it without bound. A change keeps the seat's
///   player.
/// - A whole message during the game is a watcher's only: it now follows
///   the player of that message, who is in the game. Time does not go
///   back.
/// - The end is checked, and comes once: nothing follows it.
/// - The news of the game is the interface's to check against the room.
/// - A start or a seat during the game cannot be right.
#[derive(Clone, Debug)]
pub struct SeatStream {
    seat: Seat,
    start: Arc<GameStart>,
    /// The seat's last message.
    last: Arc<TurnMessage>,
    /// The end came.
    over: bool,
}

/// A frame of a seat's stream during the game, checked.
#[derive(Debug, PartialEq)]
pub enum Checked {
    /// A turn's change, applied: the seat's message from now on.
    Turn(Arc<TurnMessage>),
    /// A watcher follows the player of this whole message from now on.
    Follow(Arc<TurnMessage>),
    /// The game is over for the seat: its statistics and result.
    End(GameEnd),
    /// Something happened to a seat of the game.
    Seat { slot: SlotIndex, change: SeatChange },
    /// The room's host changed the game's clock.
    Clock { by: Login, change: ClockChange },
    /// The player of this slot is the room's host from now on.
    Host { slot: SlotIndex },
    /// `from`, an ally, points at `square`; or the seat's flare was sent.
    Flare { from: PlayerId, square: SquareId },
    /// The server stopped the game.
    Stopped(StopReason),
}

impl SeatStream {
    /// The seat's slot, or that it watches.
    pub fn role(&self) -> Role {
        self.seat.you
    }

    /// The start of the game, which a watcher's view of another player is
    /// made from.
    pub fn start(&self) -> &Arc<GameStart> {
        &self.start
    }

    /// Takes the next frame of the stream.
    pub fn take(&mut self, frame: SeatFrame) -> Result<Checked, Invalid> {
        if self.over {
            return invalid(format!("{} after the end", kind(&frame)));
        }
        let last = &self.last;
        let checked = match frame {
            SeatFrame::Change(change) => {
                let m = change
                    .apply(last)
                    .map_err(|e| Invalid(format!("a change: {}", e.0)))?;
                self.start
                    .check(&m)
                    .map_err(|e| Invalid(format!("a turn: {}", e.0)))?;
                if m.time < last.time {
                    return invalid("a turn back in time");
                }
                encode(&m).map_err(|e| Invalid(format!("a turn: {}", e.0)))?;
                let m = Arc::new(m);
                self.last = Arc::clone(&m);
                Checked::Turn(m)
            }
            SeatFrame::Whole(m) if self.seat.you == Role::Watcher => {
                self.start
                    .check(&m)
                    .map_err(|e| Invalid(format!("a whole message: {}", e.0)))?;
                if m.time < last.time {
                    return invalid("a whole message back in time");
                }
                if !m.state.players.contains(&m.seat) {
                    return invalid(format!("to follow player {}, not in the game", m.seat.0));
                }
                let m = Arc::new(m);
                self.last = Arc::clone(&m);
                Checked::Follow(m)
            }
            SeatFrame::End(end) => {
                end.check()
                    .map_err(|e| Invalid(format!("the end: {}", e.0)))?;
                self.over = true;
                Checked::End(end)
            }
            SeatFrame::Seat { slot, change } => Checked::Seat { slot, change },
            SeatFrame::Clock { by, change } => Checked::Clock { by, change },
            SeatFrame::Host { slot } => Checked::Host { slot },
            SeatFrame::Flare { from, square } => Checked::Flare { from, square },
            SeatFrame::Stopped(reason) => Checked::Stopped(reason),
            other @ (SeatFrame::Start { .. }
            | SeatFrame::Whole(_)
            | SeatFrame::Seated { .. }
            | SeatFrame::NotStarted(_)) => {
                return invalid(format!("{} during the game", kind(&other)));
            }
        };
        Ok(checked)
    }

    /// The stream of this seat on a new connection, after the last one was
    /// lost: what the server sends when the seat's token is presented.
    pub fn returning(&self) -> ReturnStream {
        ReturnStream {
            seat: self.seat.clone(),
            start: Arc::clone(&self.start),
            last: Arc::clone(&self.last),
            next: Return::Seat,
        }
    }
}

/// The stream of a seat that returns after a lost connection, on the new
/// connection: the seat, the start and the seat's whole message, in that
/// order, each checked against the seat as it was.
///
/// - The seat is the same slot, or a watcher still, of a game on the same
///   map.
/// - The start is the same game's, of the same rules.
/// - The whole message is checked against the game, and is no earlier in
///   it than the seat's last. It is the same player's, except that a
///   watcher may have been moved on to another player meanwhile, who is in
///   the game.
///
/// The server may stop the game at any point instead.
#[derive(Clone, Debug)]
pub struct ReturnStream {
    seat: Seat,
    start: Arc<GameStart>,
    last: Arc<TurnMessage>,
    next: Return,
}

#[derive(Clone, Debug)]
enum Return {
    Seat,
    Start(Reseated),
    Whole(Reseated),
}

/// The seat as the server gives it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Reseated {
    pub speed: Speed,
    pub paused: bool,
    /// How long the server holds the seat once it notices a loss.
    pub hold: Duration,
    /// What takes the seat back after the next loss.
    pub token: SeatToken,
}

/// What a frame of a returning seat's stream came to.
#[derive(Debug)]
pub enum Returning {
    /// The return goes on.
    More(ReturnStream),
    /// The seat is back.
    Back(Back),
    /// The server stopped the game.
    Stopped(StopReason),
}

/// A seat back in the game: its stream from now on, its whole message, and
/// the seat as the server gives it back.
#[derive(Debug)]
pub struct Back {
    pub stream: SeatStream,
    pub message: Arc<TurnMessage>,
    pub seated: Reseated,
}

impl ReturnStream {
    /// Takes the next frame of the stream.
    pub fn take(self, frame: SeatFrame) -> Result<Returning, Invalid> {
        let ReturnStream {
            seat,
            start,
            last,
            next,
        } = self;
        let next = match (next, frame) {
            (_, SeatFrame::Stopped(reason)) => return Ok(Returning::Stopped(reason)),
            (
                Return::Seat,
                SeatFrame::Seated {
                    you,
                    map,
                    speed,
                    paused,
                    hold,
                    token,
                },
            ) => {
                if you != seat.you || map.hash != seat.map {
                    return invalid(format!("the seat {you:?}"));
                }
                Return::Start(Reseated {
                    speed,
                    paused,
                    hold,
                    token,
                })
            }
            (Return::Start(seated), SeatFrame::Start { start: s, rules }) => {
                if rules != seat.rules || s != start.message() {
                    return invalid("the start of another game");
                }
                Return::Whole(seated)
            }
            (Return::Whole(seated), SeatFrame::Whole(m)) => {
                start
                    .check(&m)
                    .map_err(|e| Invalid(format!("the whole message: {}", e.0)))?;
                if m.seat != last.seat {
                    if seat.you != Role::Watcher {
                        return invalid(format!("the message of player {}", m.seat.0));
                    }
                    if !m.state.players.contains(&m.seat) {
                        return invalid(format!("to follow player {}, not in the game", m.seat.0));
                    }
                }
                if m.time < last.time {
                    return invalid("a message back in time");
                }
                let message = Arc::new(m);
                return Ok(Returning::Back(Back {
                    stream: SeatStream {
                        seat,
                        start,
                        last: Arc::clone(&message),
                        over: false,
                    },
                    message,
                    seated,
                }));
            }
            (_, other) => return invalid(format!("{} before the seat was back", kind(&other))),
        };
        Ok(Returning::More(ReturnStream {
            seat,
            start,
            last,
            next,
        }))
    }
}

/// What a frame is, for the log.
fn kind(frame: &SeatFrame) -> &'static str {
    match frame {
        SeatFrame::Start { .. } => "a start",
        SeatFrame::Whole(_) => "a whole message",
        SeatFrame::Change(_) => "a change",
        SeatFrame::End(_) => "an end",
        SeatFrame::Seated { .. } => "a seat",
        SeatFrame::Seat { .. } => "news of a seat",
        SeatFrame::Clock { .. } => "news of the clock",
        SeatFrame::Host { .. } => "news of the host",
        SeatFrame::Flare { .. } => "a flare",
        SeatFrame::Stopped(_) => "a stop",
        SeatFrame::NotStarted(_) => "a refused start",
    }
}
