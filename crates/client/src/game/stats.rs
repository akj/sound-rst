//! The score said at the end of a game, from the player's statistics.

use soundrts_world::View;
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::nofloat::floordiv;
use soundrts_world::player::{StatEvent, StatTarget};

use crate::style::Style;

use super::order::PRECISION;
use super::style_msg;

/// The messages of the score: the outcome and the duration, units and
/// buildings, each resource, then the score.
pub fn score_msgs(view: View, style: &Style) -> Vec<Msg> {
    let stats = view.stats();
    let resources = view.resources();
    let outcome = if view.has_victory() {
        mp::VICTORY
    } else {
        mp::DEFEAT
    };
    let (minutes, seconds) = stats.game_duration_in_minutes_seconds(view.time());
    let mut msgs = vec![
        [
            msg(outcome),
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
