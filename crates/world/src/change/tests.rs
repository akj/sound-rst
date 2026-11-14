//! The change form without any encoding: applying the change between two
//! messages gives back the second, over a whole game; a remembered entity
//! seen again is named, not sent; and a change that names what is not
//! there is refused. The encoded form is proved in the protocol crate.

use super::*;
use crate::test_games::{Game, allies_sharing_control, cheat_mode_and_defeat};
use crate::view::tests::read;

fn entities(m: &TurnMessage) -> impl Iterator<Item = &KnownEntity> {
    m.perceived.iter().chain(&m.remembered)
}

/// Allies who share control, then a person in cheat mode, defeated, with
/// woods cut: every seat's change from 1 and from 10 turns back gives back
/// its message.
#[test]
fn a_change_gives_back_the_message_over_a_whole_game() {
    for case in [allies_sharing_control(400), cheat_mode_and_defeat(800)] {
        play(&mut Game::new(&case, read, true));
    }
}

fn play(game: &mut Game) {
    let mut history: Vec<Vec<TurnMessage>> = game.messages().into_iter().map(|m| vec![m]).collect();
    let (mut seen_again, mut changed, mut counting_down) = (0, 0, 0);
    while let Some(now) = game.step() {
        for (h, m) in history.iter_mut().zip(now) {
            for back in [1, 10] {
                let Some(old) = h.len().checked_sub(back).and_then(|i| h.get(i)) else {
                    continue;
                };
                let change = TurnChange::between(old, &m);
                if back == 1 {
                    seen_again += change.seen_again.len();
                    changed += change.known.changed.iter().count();
                    counting_down += change
                        .known
                        .changed
                        .iter()
                        .filter(|(_, f)| {
                            f.own_fields
                                .as_ref()
                                .is_some_and(|o| o.times_left.is_some())
                        })
                        .count();
                }
                assert_eq!(change.apply(old).as_ref(), Ok(&m));
            }
            h.push(m);
            if h.len() > 10 {
                h.remove(0);
            }
        }
    }
    eprintln!(
        "{seen_again} groups of entities seen again, {changed} entities changed, \
         {counting_down} of them only counting down production"
    );
    assert!(changed > 0 && counting_down > 0);
}

/// The game's first messages, and those after a few turns, of the
/// person's seat.
fn two_messages() -> (TurnMessage, TurnMessage) {
    let mut game = Game::new(&cheat_mode_and_defeat(100), read, false);
    let first = game.messages().remove(0);
    let mut now = None;
    for _ in 0..60 {
        now = game.step();
    }
    (first, now.unwrap().remove(0))
}

#[test]
fn a_remembered_entity_seen_again_is_named_with_the_time() {
    let (_, mut last) = two_messages();
    let e = last
        .remembered
        .first()
        .expect("a remembered entity")
        .clone();
    let mut now = last.clone();
    now.remembered[0].last_seen = Some(now.time);
    let change = TurnChange::between(&last, &now);
    assert_eq!(
        change.seen_again,
        vec![SeenAgain {
            time: now.time,
            ids: [e.id].into_iter().collect()
        }]
    );
    assert!(change.known.changed.is_empty());
    assert_eq!(change.apply(&last).as_ref(), Ok(&now));

    // Perceived until now, and remembered as it was: that is not seen
    // again, and goes as a change.
    let p = last.perceived.pop().unwrap();
    let mut now = last.clone();
    now.remembered.push(KnownEntity {
        last_seen: Some(5),
        ..p.clone()
    });
    now.remembered.sort_by_key(|e| e.id);
    last.perceived.push(p);
    let change = TurnChange::between(&last, &now);
    assert!(change.seen_again.is_empty());
    assert_eq!(change.apply(&last).as_ref(), Ok(&now));
}

#[test]
fn a_change_that_names_what_is_not_there_is_refused() {
    let (before, first) = two_messages();
    let good = TurnChange::between(&before, &first);
    let before = &before;
    let absent = EntityId(u32::MAX);
    let known = entities(&first).next().unwrap().clone();
    let without_own = entities(&first).find(|e| e.own.is_none()).unwrap().id;
    let perceived = first.perceived.first().unwrap().id;
    let fields = EntityFields {
        hp: Some(1),
        ..Default::default()
    };
    let cases: Vec<(&str, TurnChange)> = vec![
        ("gone, not there", {
            let mut c = good.clone();
            c.known.gone = [absent].into_iter().collect();
            c
        }),
        ("changed, not there", {
            let mut c = good.clone();
            c.known.changed = [(absent, fields.clone())].into_iter().collect();
            c
        }),
        ("changed nothing", {
            let mut c = TurnChange::between(&first, &first);
            c.known.changed = [(known.id, EntityFields::default())].into_iter().collect();
            c
        }),
        ("new, already there", {
            let mut c = TurnChange::between(&first, &first);
            c.known.new = vec![known.clone()];
            c
        }),
        ("seen again, perceived", {
            let mut c = TurnChange::between(&first, &first);
            c.seen_again = vec![SeenAgain {
                time: 3,
                ids: [perceived].into_iter().collect(),
            }];
            c
        }),
        ("seen again, not there", {
            let mut c = TurnChange::between(&first, &first);
            c.seen_again = vec![SeenAgain {
                time: 3,
                ids: [absent].into_iter().collect(),
            }];
            c
        }),
        ("own fields without an own part", {
            let mut c = TurnChange::between(&first, &first);
            let own = OwnFields {
                is_idle: Some(true),
                ..Default::default()
            };
            let f = EntityFields {
                own_fields: Some(own),
                ..Default::default()
            };
            c.known.changed = [(without_own, f)].into_iter().collect();
            c
        }),
        ("times left for orders it does not have", {
            let mut c = TurnChange::between(&first, &first);
            let own = OwnFields {
                times_left: Some(vec![Some(300); 40]),
                ..Default::default()
            };
            let with_own = entities(&first).find(|e| e.own.is_some()).unwrap().id;
            let f = EntityFields {
                own_fields: Some(own),
                ..Default::default()
            };
            c.known.changed = [(with_own, f)].into_iter().collect();
            c
        }),
        ("terrain forgotten that was not known", {
            let mut c = TurnChange::between(&first, &first);
            c.squares.terrain = Some(vec![(SquareId(u32::MAX), None)]);
            c
        }),
        ("terrain out of order", {
            let mut c = TurnChange::between(&first, &first);
            c.squares.terrain = Some(vec![
                (SquareId(2), Some("_forest".into())),
                (SquareId(1), Some("_forest".into())),
            ]);
            c
        }),
    ];
    assert!(good.apply(before).is_ok());
    for (what, c) in cases {
        assert!(c.apply(&first).is_err(), "{what}");
    }
    let mut both = first.clone();
    both.remembered.push(first.perceived[0].clone());
    assert!(TurnChange::between(&first, &first).apply(&both).is_err());
}
