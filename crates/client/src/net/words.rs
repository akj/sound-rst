//! What the client says of a game on a server: what happens to the
//! players and the clock, and why a game stops or does not start.

use soundrts_protocol::{ClockChange, Login, SeatChange, Speed, StartRefusal, StopReason};
use soundrts_world::msg::{Msg, MsgPart, Phrase, msg, nb2msg};

use crate::game::server::Failure;

/// What happened to a player of the game, `who`.
pub fn seat_news(who: Msg, change: &SeatChange) -> Msg {
    let what = match change {
        SeatChange::Dropped => msg(Phrase::LostTheConnection),
        SeatChange::Returned => msg(Phrase::IsBack),
        SeatChange::GivenUp => msg(Phrase::DidNotComeBack),
        SeatChange::Left => msg(Phrase::LeftTheGame),
        SeatChange::Removed { by } => {
            [msg(Phrase::RemovedBy), vec![MsgPart::literal(by.as_str())]].concat()
        }
    };
    [who, what].concat()
}

/// A speed of the game, as said.
pub fn speed(speed: Speed) -> Msg {
    match speed {
        Speed::Slow => msg(Phrase::Slow),
        Speed::Normal => msg(Phrase::Normal),
        Speed::Fast2 => [msg(Phrase::Fast), nb2msg(2)].concat(),
        Speed::Fast4 => [msg(Phrase::Fast), nb2msg(4)].concat(),
    }
}

/// What the room's host, `by`, did to the clock.
pub fn clock_news(by: &Login, change: ClockChange) -> Msg {
    let what = match change {
        ClockChange::Paused => msg(Phrase::PausedBy),
        ClockChange::Resumed => msg(Phrase::ResumedBy),
        ClockChange::Speed { speed: s } => {
            [msg(Phrase::GameSpeed), speed(s), msg(Phrase::SetBy)].concat()
        }
    };
    [what, vec![MsgPart::literal(by.as_str())]].concat()
}

/// Who is the room's host from now on.
pub fn new_host(host: &Login) -> Msg {
    [
        vec![MsgPart::literal(host.as_str())],
        msg(Phrase::IsNowTheOrganizer),
    ]
    .concat()
}

/// Why the server stopped the game.
pub fn stopped(reason: StopReason) -> Msg {
    let why = match reason {
        StopReason::Crashed => Phrase::ItFailed,
        StopReason::TooSlow => Phrase::ATurnTookTooLong,
    };
    [msg(Phrase::GameStoppedByTheServer), msg(why)].concat()
}

/// Why the game did not start.
pub fn not_started(refusal: &StartRefusal) -> Msg {
    let why = match refusal {
        StartRefusal::CannotBuild { error } => [
            msg(Phrase::ServerCannotBuildIt),
            vec![MsgPart::literal(error.as_str())],
        ]
        .concat(),
        StartRefusal::TooSlow => msg(Phrase::TookTooLongToBuild),
        StartRefusal::Crashed => msg(Phrase::ItFailed),
        StartRefusal::Busy => msg(Phrase::ServerBusy),
    };
    [msg(Phrase::GameNotStarted), why].concat()
}

/// Why the player could not return to the game.
pub fn return_failed(why: Failure) -> Msg {
    let why = match why {
        Failure::GameOver => Phrase::TheGameIsOver,
        Failure::SeatGivenUp => Phrase::TheSeatWasGivenUp,
        Failure::TimeUp => Phrase::NoAnswerInTime,
    };
    [msg(Phrase::CouldNotReturn), msg(why)].concat()
}
