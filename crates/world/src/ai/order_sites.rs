//! The orders the computer player gives where the recorded games do not
//! reach, each with the order it gives, as the words the order stores.

use super::*;
use crate::entity::PlayerId;
use crate::player::Controller;
use crate::world::World;
use crate::{AiScripts, Rules};

/// The type of the rules with this name.
fn ty(w: &World, name: &str) -> TypeId {
    w.rules.type_id(name).unwrap()
}

/// jl1 with an easy computer, then a human.
fn game() -> (World, PlayerId, PlayerId) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
    let read = |f: &str| {
        let bytes = std::fs::read(dir.join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let rules = Rules::load(&[read("res/rules.txt")]);
    let ai = AiScripts::load(&[read("res/ai.txt")]);
    let mut w = World::new(rules, ai, Vec::new(), 0);
    w.load_and_build_map(&read("res/multi/jl1.txt")).unwrap();
    let players = w
        .populate_map(
            &[Controller::computer("easy"), Controller::human("me", true)],
            false,
            true,
        )
        .unwrap();
    let c = players[0];
    w.plm(c).cheat_has_all = true;
    w.plm(c).resources = vec![1_000_000; w.pl(c).resources.len()];
    (w, c, players[1])
}

/// The square of a player's first unit.
fn home(w: &World, p: PlayerId) -> SquareId {
    match w.e(w.pl(p).units[0]).place {
        Some(Place::Square(s)) => s,
        _ => panic!("on a square"),
    }
}

/// The square farthest from `s` by the ground.
fn far_from(w: &mut World, s: SquareId) -> SquareId {
    let mut far = (0, s);
    for i in 0..w.squares.len() {
        let t = SquareId(i as u32);
        let path = w.shortest_path_places(s, t, None, AirGround::Ground, false);
        far = far.max((path.unwrap().len(), t));
    }
    far.1
}

fn add(w: &mut World, type_name: &str, p: PlayerId, s: SquareId) -> EntityId {
    let t = w.rules.type_id(type_name).unwrap();
    let (x, y) = (w.square(s).x, w.square(s).y);
    let u = w.create_creature(t, Some(p), Place::Square(s), x, y);
    u.unwrap()
}

/// The orders of a unit as the words they store: keyword, type, target.
fn orders(w: &World, u: EntityId) -> Vec<Vec<String>> {
    w.e(u).orders().iter().map(|o| o.words(w)).collect()
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|&s| s.to_owned()).collect()
}

fn id(w: &World, s: SquareId) -> String {
    w.square(s).id.to_string()
}

/// Enemies of `c` on `s`, seen by `c`.
fn seen_enemies(w: &mut World, you: PlayerId, c: PlayerId, s: SquareId, n: usize) {
    for _ in 0..n {
        let e = add(w, "knight", you, s);
        w.plm(c).perception.insert(e);
    }
}

/// What `c` knows of the battlefield, found again as at its update.
fn update_knowledge(w: &mut World, c: PlayerId) {
    w.update_enemy_menace(c);
    w.update_tactical_knowledge(c);
}

#[test]
fn raise_dead_is_used_on_the_square_of_the_user() {
    let (mut w, c, you) = game();
    let s = home(&w, c);
    let n = add(&mut w, "necromancer", c, s);
    w.computer_state(c).raise_dead_users = vec![(n, ty(&w, "a_raise_dead"))];
    // It sees a corpse there.
    let dead = add(&mut w, "footman", you, s);
    w.die(dead, None).unwrap();
    let corpse = w
        .square(s)
        .objects
        .iter()
        .copied()
        .find(|&o| w.e(o).is_corpse());
    w.plm(c).perception.insert(corpse.unwrap());
    update_knowledge(&mut w, c);
    w.door(c).raise_dead().unwrap();
    assert_eq!(orders(&w, n), [words(&["use", "a_raise_dead", &id(&w, s)])]);
}

#[test]
fn teleportation_is_used_on_the_place_units_are_sent_to() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let far = far_from(&mut w, s);
    let mage = add(&mut w, "mage", c, s);
    let footman = add(&mut w, "footman", c, s);
    // The order does not depend on the ability: the mage's own is used.
    w.computer_state(c).teleportation_users = vec![(mage, ty(&w, "a_meteors"))];
    w.door(c).send_units(vec![footman], far).unwrap();
    assert_eq!(
        orders(&w, mage),
        [words(&["use", "a_meteors", &id(&w, far)])]
    );
}

#[test]
fn a_cataclysm_and_a_detector_are_used_on_the_place_units_are_sent_to() {
    let (mut w, c, you) = game();
    let s = home(&w, c);
    let far = far_from(&mut w, s);
    let mage = add(&mut w, "mage", c, s);
    seen_enemies(&mut w, you, c, far, 1);
    w.computer_state(c).cataclysm_users = vec![(mage, ty(&w, "a_meteors"))];
    w.computer_state(c).detector_users = vec![(mage, ty(&w, "a_meteors"))];
    w.door(c).send_units(vec![mage], far).unwrap();
    let use_far = words(&["use", "a_meteors", &id(&w, far)]);
    let o = orders(&w, mage);
    assert_eq!(o.len(), 4, "{o:?}");
    assert_eq!(o[0][0], "go");
    assert_eq!(o[1], use_far);
    assert_eq!(o[2], use_far);
    assert_eq!(o[3], words(&["go", &id(&w, far)]));
}

#[test]
fn a_cataclysm_on_a_place_too_strong_to_attack_is_cast_from_nearby() {
    let (mut w, c, you) = game();
    let s = home(&w, c);
    let far = far_from(&mut w, s);
    let mage = add(&mut w, "mage", c, s);
    // Too strong to attack.
    seen_enemies(&mut w, you, c, far, 10);
    update_knowledge(&mut w, c);
    w.computer_state(c).cataclysm_users = vec![(mage, ty(&w, "a_meteors"))];
    w.door(c)
        .eventually_attack(vec![Some(Place::Square(far))])
        .unwrap();
    let o = orders(&w, mage);
    assert_eq!(o.len(), 3, "{o:?}");
    // A square on the way, before the target.
    assert_eq!(o[0][0], "go");
    let before = w.square_by_obj_id(o[0][1].parse().unwrap()).unwrap();
    assert!(before != s && before != far);
    assert_eq!(o[1], words(&["use", "a_meteors", &id(&w, far)]));
    assert_eq!(o[2], words(&["go", &id(&w, s)]));
}

#[test]
fn workers_repair_a_building_site_without_workers() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let farm = w.rules.type_id("farm").unwrap();
    let (x, y) = (w.square(s).x, w.square(s).y);
    let site = w
        .create_building_site(c, Place::Square(s), x, y, farm)
        .unwrap();
    w.update_effect_users_and_workers(c);
    w.door(c)
        .send_workers_to_forgotten_building_sites()
        .unwrap();
    let repair = words(&["repair", &site.to_string()]);
    let workers = w.computer_state(c).workers.clone();
    assert!(!workers.is_empty());
    for u in workers {
        assert_eq!(orders(&w, u).first(), Some(&repair));
    }
}

#[test]
fn workers_near_a_deposit_repair_the_warehouse_being_built_there() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    // A deposit away from the computer's warehouse.
    let (far, deposit) = w
        .squares
        .iter()
        .enumerate()
        .filter(|&(i, _)| SquareId(i as u32) != s)
        .find_map(|(i, sq)| {
            let d = sq.objects.iter().copied().find(|&o| w.e(o).is_deposit());
            d.map(|d| (SquareId(i as u32), d))
        })
        .unwrap();
    let worker = add(&mut w, "peasant", c, far);
    w.computer_state(c).workers = vec![worker];
    let townhall = w.rules.type_id("townhall").unwrap();
    let (x, y) = (w.square(far).x, w.square(far).y);
    let site = w
        .create_building_site(c, Place::Square(far), x, y, townhall)
        .unwrap();
    // The worker sees the deposit and the site.
    w.update_perception_and_memory(c).unwrap();
    w.door(c)
        .build_a_warehouse_for(&Mark::of(crate::entity::Target::Entity(deposit)), townhall)
        .unwrap();
    assert_eq!(orders(&w, worker), [words(&["repair", &site.to_string()])]);
}

#[test]
fn workers_build_a_gate_on_the_exit_nearest_the_sensible_building() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let townhall = w
        .pl(c)
        .units
        .iter()
        .copied()
        .find(|&u| &*w.e(u).type_name == "townhall")
        .unwrap();
    w.update_effect_users_and_workers(c);
    w.computer_state(c).sensible_building = Some(townhall);
    let (bx, by) = (w.e(townhall).x, w.e(townhall).y);
    let mut exits = w.square(s).exits.clone();
    exits.sort_by_key(|&x| square_of_distance(bx, by, w.e(x).x, w.e(x).y));
    w.door(c).defensive_routine().unwrap();
    let gate = words(&["build", "gate", &exits[0].to_string()]);
    let workers = w.computer_state(c).workers.clone();
    assert!(!workers.is_empty());
    for u in workers {
        assert_eq!(orders(&w, u).first(), Some(&gate));
    }
}

#[test]
fn a_research_is_ordered_to_a_building_that_can_do_it() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let blacksmith = add(&mut w, "blacksmith", c, s);
    let t = ty(&w, "melee_weapon");
    w.door(c)
        .build_or_train_or_upgradeto_or_summon(t, 1, 0)
        .unwrap();
    assert_eq!(
        orders(&w, blacksmith),
        [words(&["research", "melee_weapon"])]
    );
}

#[test]
fn a_summoning_aimed_by_the_player_is_aimed_at_its_first_unit() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let priest = add(&mut w, "priest", c, s);
    let t = ty(&w, "holy_vision");
    w.door(c)
        .build_or_train_or_upgradeto_or_summon(t, 1, 0)
        .unwrap();
    let first = w.pl(c).units[0];
    assert_eq!(
        orders(&w, priest),
        [words(&["use", "a_holy_vision", &first.to_string()])]
    );
}

#[test]
fn an_explorer_stops_before_it_upgrades() {
    let (mut w, c, _) = game();
    let s = home(&w, c);
    let footman = add(&mut w, "footman", c, s);
    w.take_order(footman, Request::auto_explore(), true, false, None)
        .unwrap();
    assert_eq!(orders(&w, footman), [words(&["auto_explore"])]);
    let (footman_t, knight) = (ty(&w, "footman"), ty(&w, "knight"));
    w.door(c)
        .order(1, Sel::One(footman_t), Task::UpgradeTo(knight), false)
        .unwrap();
    assert!(
        orders(&w, footman).iter().all(|o| o[0] != "auto_explore"),
        "{:?}",
        orders(&w, footman)
    );
}
