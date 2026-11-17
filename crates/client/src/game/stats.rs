//! The score said at the end of a game, from the player's statistics.

use soundrts_world::SeatView;
use soundrts_world::msg::{Msg, MsgPart, Phrase, comma, msg, nb2msg};
use soundrts_world::nofloat::floordiv;
use soundrts_world::player::{StatEvent, StatTarget};

use crate::style::Style;

use super::order::PRECISION;

/// The messages of the score: the outcome and the duration, units and
/// buildings, each resource, then the score. None before the game is over
/// for the player: the statistics come with its end. With `still_on`, the
/// outcome is that the game was not over for the player: a recording that
/// ends while the player is still in the game.
pub fn score_msgs(view: &SeatView, style: &Style, still_on: bool) -> Vec<Msg> {
    let Some(stats) = view.stats() else {
        return Vec::new();
    };
    let resources = view.resources();
    let outcome = if view.has_victory() {
        msg(Phrase::Victory)
    } else if still_on {
        msg(Phrase::NotOver)
    } else {
        msg(Phrase::Defeat)
    };
    let (minutes, seconds) = stats.game_duration_in_minutes_seconds(view.time());
    let mut msgs = vec![
        [
            outcome,
            msg(Phrase::At),
            nb2msg(minutes),
            msg(Phrase::Minutes),
            nb2msg(seconds),
            msg(Phrase::Seconds),
        ]
        .concat(),
    ];
    for (target, produced) in [
        (StatTarget::Unit, [Phrase::Units, Phrase::ProducedF]),
        (StatTarget::Building, [Phrase::Buildings, Phrase::ProducedM]),
    ] {
        msgs.push(
            [
                nb2msg(stats.get(StatEvent::Produced, target)),
                produced.map(MsgPart::from).to_vec(),
                comma(),
                nb2msg(stats.get(StatEvent::Lost, target)),
                msg(Phrase::Lost),
                comma(),
                nb2msg(stats.get(StatEvent::Killed, target)),
                msg(Phrase::Neutralized),
            ]
            .concat(),
        );
    }
    for i in 0..resources.len() {
        let gathered = stats.get(StatEvent::Gathered, StatTarget::Resource(i));
        msgs.push(
            [
                nb2msg(floordiv(gathered, PRECISION)),
                style.get("parameters").resource_title_msg(i),
                msg(Phrase::Gathered),
                comma(),
                nb2msg(floordiv(stats.consumed(i, resources), PRECISION)),
                msg(Phrase::Consumed),
            ]
            .concat(),
        );
    }
    msgs.push(
        [
            msg(Phrase::Score),
            nb2msg(stats.score(resources)),
            msg(Phrase::HistoryExplanation),
        ]
        .concat(),
    );
    msgs
}
