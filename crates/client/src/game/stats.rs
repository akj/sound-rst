//! The score said at the end of a game, from the player's statistics.

use soundrts_world::SeatView;
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::nofloat::floordiv;
use soundrts_world::player::{StatEvent, StatTarget};

use crate::style::Style;

use super::order::PRECISION;
use super::style_msg;

/// The messages of the score: the outcome and the duration, units and
/// buildings, each resource, then the score. None before the game is over
/// for the player: the statistics come with its end.
pub fn score_msgs(view: &SeatView, style: &Style) -> Vec<Msg> {
    score_msgs_of(view, style, false)
}

/// As [`score_msgs`]; with `still_on`, the outcome is that the game was
/// not over for the player: a recording that ends while the player is
/// still in the game.
pub fn score_msgs_of(view: &SeatView, style: &Style, still_on: bool) -> Vec<Msg> {
    let Some(stats) = view.stats() else {
        return Vec::new();
    };
    let resources = view.resources();
    let outcome = if view.has_victory() {
        msg(mp::VICTORY)
    } else if still_on {
        crate::net::words::text(crate::net::words::NOT_OVER)
    } else {
        msg(mp::DEFEAT)
    };
    let (minutes, seconds) = stats.game_duration_in_minutes_seconds(view.time());
    let mut msgs = vec![
        [
            outcome,
            msg(mp::AT),
            nb2msg(minutes),
            msg(mp::MINUTES),
            nb2msg(seconds),
            msg(mp::SECONDS),
        ]
        .concat(),
    ];
    for (target, produced) in [
        (StatTarget::Unit, [mp::UNITS, mp::PRODUCED_F].concat()),
        (
            StatTarget::Building,
            [mp::BUILDINGS, mp::PRODUCED_M].concat(),
        ),
    ] {
        msgs.push(
            [
                nb2msg(stats.get(StatEvent::Produced, target)),
                msg(&produced),
                msg(mp::COMMA),
                nb2msg(stats.get(StatEvent::Lost, target)),
                msg(mp::LOST),
                msg(mp::COMMA),
                nb2msg(stats.get(StatEvent::Killed, target)),
                msg(mp::NEUTRALIZED),
            ]
            .concat(),
        );
    }
    for i in 0..resources.len() {
        let gathered = stats.get(StatEvent::Gathered, StatTarget::Resource(i));
        msgs.push(
            [
                nb2msg(floordiv(gathered, PRECISION)),
                style_msg(style, "parameters", &format!("resource_{i}_title")),
                msg(mp::GATHERED),
                msg(mp::COMMA),
                nb2msg(floordiv(stats.consumed(i, resources), PRECISION)),
                msg(mp::CONSUMED),
            ]
            .concat(),
        );
    }
    msgs.push(
        [
            msg(mp::SCORE),
            nb2msg(stats.score(resources)),
            msg(mp::HISTORY_EXPLANATION),
        ]
        .concat(),
    );
    msgs
}
