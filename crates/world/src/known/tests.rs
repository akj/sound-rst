use super::*;
use crate::events::{EntityEvent, Event};
use crate::seat_view::tests::{command, game, remember, seat, unit};

#[test]
fn an_enemys_entity_has_no_own_part() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    let b2 = w.square_by_name("b2").unwrap();
    let b2 = w.square(b2).id;
    command(
        &mut w,
        them,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 go {b2}"),
        ],
    );
    assert_eq!(w.entity(peasant).unwrap().orders.len(), 1);
    // The player is made to see the computer's peasant.
    w.player_mut(me).perception.insert(peasant);
    let e = seat(&w, me).perceived(peasant).unwrap();
    assert_eq!(
        (&*e.type_name, e.player, e.hp, e.hp_max, e.last_seen),
        ("peasant", Some(them), 4000, 4000, None)
    );
    assert_eq!(e.own, None);
    // Its owner knows its orders.
    let e = seat(&w, them).perceived(peasant).unwrap();
    assert_eq!(e.own.unwrap().orders.len(), 1);
}

#[test]
fn an_allys_entity_has_its_own_part_once_the_player_controls_the_allys_units() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, them, "peasant");
    let townhall = unit(&w, me, "townhall");
    // The other player's peasant goes to the player's town hall, as that
    // player remembers it.
    remember(&mut w, them, townhall);
    command(
        &mut w,
        them,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 go {}", townhall.0),
        ],
    );
    let target = w.entity(peasant).unwrap().orders[0].target();
    assert!(matches!(target, Some(Target::Memory(m)) if m.owner == them));
    w.player_mut(me).perception.insert(peasant);
    assert_eq!(seat(&w, me).perceived(peasant).unwrap().own, None);
    w.player_mut(me).allied_control.push(them);
    let own = seat(&w, me).perceived(peasant).unwrap().own.unwrap();
    // The other player's memory is not the player's: the target is the
    // town hall as the player knows it.
    let targets: Vec<_> = own.orders.iter().map(|o| o.target.clone()).collect();
    assert_eq!(targets, [Some(KnownTarget::Entity(townhall))]);
    assert_eq!(own.stance, Some(AiMode::Defensive));
}

#[test]
fn the_players_own_unit_has_its_orders_cargo_and_the_rest() {
    let (mut w, me, them) = game();
    let peasant = unit(&w, me, "peasant");
    let farm = unit(&w, me, "farm");
    let townhall = unit(&w, them, "townhall");
    // Orders on its farm, and on the computer's town hall as the player
    // remembers it.
    remember(&mut w, me, townhall);
    command(
        &mut w,
        me,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 go {}", farm.0),
            format!("order 1 0 go {}", townhall.0),
        ],
    );
    let e = w.entity_mut(peasant).unwrap();
    e.cargo = Some((0, 5000));
    e.group = Some("1".into());
    let distance_to_goal = e.distance_to_goal;

    let e = seat(&w, me).perceived(peasant).unwrap();
    let own = e.own.unwrap();
    let [go_farm, go_townhall] = &own.orders[..] else {
        panic!("two orders: {:?}", own.orders);
    };
    assert_eq!(
        go_farm,
        &KnownOrder {
            keyword: Keyword::Go,
            type_id: None,
            is_deferred: false,
            is_imperative: false,
            time_left: None,
            target: Some(KnownTarget::Entity(farm)),
            patrol: None,
        }
    );
    let Some(KnownTarget::Remembered(th)) = &go_townhall.target else {
        panic!("the town hall as remembered: {go_townhall:?}");
    };
    assert_eq!(
        (go_townhall.keyword, th.id, th.last_seen, &th.own),
        (Keyword::Go, townhall, Some(1500), &None)
    );
    assert_eq!(
        (
            own.has_basic_abilities,
            own.cargo,
            own.inside,
            own.stance,
            own.group,
        ),
        (
            true,
            Some((0, 5000)),
            None,
            Some(AiMode::Defensive),
            Some("1".into())
        )
    );
    assert_eq!(own.distance_to_goal, distance_to_goal);
    assert_eq!(e.number, Some(1));
}

#[test]
fn a_remembered_entity_is_the_copy_as_remembered_with_its_time() {
    let (mut w, me, them) = game();
    let townhall = unit(&w, them, "townhall");
    remember(&mut w, me, townhall);
    // Seen with 400 hit points at x 18 m; it is damaged and moved since.
    let seen = (400_000, 18_000);
    let e = w.entity_mut(townhall).unwrap();
    assert_eq!((e.hp, e.x), seen);
    (e.hp, e.x) = (1000, 18_500);

    let r = seat(&w, me).remembered(townhall).unwrap();
    assert_eq!(
        (r.id, (r.hp, r.x), r.last_seen, r.is_memory(), &r.own),
        (townhall, seen, Some(1500), true, &None)
    );
    // The end of the next turn tells the same.
    w.update();
    let end = w.player_mut(me).end_of_turn.take().unwrap().message;
    let r = end.remembered.iter().find(|e| e.id == townhall).unwrap();
    assert_eq!(((r.hp, r.x), r.last_seen), (seen, Some(1500)));
    assert!(!end.perceived.iter().any(|e| e.id == townhall));
}

#[test]
fn an_event_tells_the_entity_as_it_was_when_it_happened() {
    let (mut w, me, _) = game();
    let peasant = unit(&w, me, "peasant");
    let square = |name| w.square(w.square_by_name(name).unwrap()).id;
    let (b2, c2) = (square("b2"), square("c2"));
    command(
        &mut w,
        me,
        &[
            format!("control {}", peasant.0),
            format!("order 0 0 go {b2}"),
        ],
    );
    for _ in 0..3 {
        w.update();
    }
    let before = (16_650, 6_000);
    assert_eq!(
        (w.entity(peasant).unwrap().x, w.entity(peasant).unwrap().y),
        before
    );
    w.drain_events(me);
    // A second order is taken at the start of the turn, while the peasant
    // walks to b2; it walks on in the rest of the turn.
    command(&mut w, me, &[format!("order 1 0 go {c2}")]);
    let after = (16_200, 6_000);
    assert_eq!(
        (w.entity(peasant).unwrap().x, w.entity(peasant).unwrap().y),
        after
    );
    let events = w.drain_events(me);
    let ok = events
        .iter()
        .find_map(|e| match e {
            Event::Entity {
                entity,
                event: EntityEvent::OrderOk,
            } if entity.id == peasant => Some(entity),
            _ => None,
        })
        .unwrap();
    assert_eq!((ok.x, ok.y), before);
    assert_eq!((ok.last_seen, ok.own.is_some()), (None, true));
    // The end of the turn tells where it is now.
    let end = w.player_mut(me).end_of_turn.take().unwrap().message;
    let now = end.perceived.iter().find(|e| e.id == peasant).unwrap();
    assert_eq!((now.x, now.y), after);
}
