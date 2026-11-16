//! What each skill level changes in how a computer plays.

use super::skill::{MoreMakers, Settings, Workers};
use super::tests::world;
use super::*;
use crate::entity::{PlayerId, Target};
use crate::player::Controller;
use crate::rules::VIRTUAL_TIME_INTERVAL;
use crate::world::World;

/// One setting of each level, from the weakest.
fn settings_of(f: impl Fn(Settings) -> i64) -> [i64; 5] {
    Skill::ALL.map(|s| f(s.settings()))
}

#[test]
fn each_level_has_its_settings_and_normal_is_the_original() {
    assert_eq!(settings_of(|s| s.grace), [600_000, 300_000, 0, 0, 0]);
    assert_eq!(
        settings_of(|s| s.turns_between_decisions),
        [40, 30, 10, 10, 10]
    );
    assert_eq!(
        Skill::ALL.map(|s| s.settings().more_makers),
        [
            MoreMakers::AsTheOriginal,
            MoreMakers::AsTheOriginal,
            MoreMakers::AsTheOriginal,
            MoreMakers::AsTheOriginal,
            MoreMakers::CountingSites(3),
        ]
    );
    assert_eq!(
        Skill::ALL.map(|s| s.settings().workers),
        [
            Workers::AtMost(6),
            Workers::AtMost(8),
            Workers::AtLeast(10),
            Workers::WithSpare {
                kept: 10,
                spare: 15
            },
            Workers::WithSpare {
                kept: 10,
                spare: 15
            },
        ]
    );
}

/// An aggressive computer at `skill` on a1 with a farm, three footmen, a
/// peasant and a flying machine, its explorer. Another flying machine of
/// its own stands on b1 and shows it the farm of a human there. A flying
/// machine harms nothing.
fn facing_a_farm(skill: Skill) -> (World, PlayerId, SquareId) {
    let mut w = world(
        "
square_width 12
nb_columns 2
nb_lines 1
west_east_paths a1
nb_meadows_by_square 0
nb_players_min 2
nb_players_max 2
starting_squares a1 b1
starting_units farm
starting_resources 0 0
",
    );
    let computer = Controller::computer_at("aggressive", skill);
    let human = Controller::human("someone", true);
    let p = w.populate_map(&[computer, human], false, false).unwrap()[0];
    let [a1, b1] = ["a1", "b1"].map(|n| w.square_by_name(n).unwrap());
    // Of two equal explorers, the last one made explores.
    for (t, s) in [
        ("new_flyingmachine", b1),
        ("footman", a1),
        ("footman", a1),
        ("footman", a1),
        ("peasant", a1),
        ("new_flyingmachine", a1),
    ] {
        let (x, y) = (w.square(s).x, w.square(s).y);
        let tid = w.rules.type_id(t).unwrap();
        w.create_creature(tid, Some(p), Place::Square(s), x, y)
            .unwrap();
    }
    (w, p, b1)
}

/// The game time at which `p` first sends a soldier to `s`, if it does
/// within `turns` turns.
fn first_sent_to(w: &mut World, p: PlayerId, s: SquareId, turns: usize) -> Option<i64> {
    for _ in 0..turns {
        w.update();
        let sent = w.pl(p).units.iter().any(|&u| {
            w.e(u).class() == EntityClass::Soldier
                && w.e(u).orders().iter().any(|o| {
                    o.keyword() == Keyword::Go
                        && matches!(o.target(), Some(Target::Square(t)) if *t == s)
                })
        });
        if sent {
            return Some(w.time);
        }
    }
    None
}

#[test]
fn the_lowest_level_starts_no_attack_before_ten_minutes_where_normal_attacks_at_once() {
    let (mut w, p, b1) = facing_a_farm(Skill::NORMAL);
    assert_eq!(first_sent_to(&mut w, p, b1, 2200), Some(6_300));
    let (mut w, p, b1) = facing_a_farm(Skill::Level1);
    // Ten minutes are 2,000 turns. It decides once in 40 turns: at its
    // first decision after the grace, it attacks.
    assert_eq!(first_sent_to(&mut w, p, b1, 2200), Some(600_300));
}

#[test]
fn level_two_starts_no_attack_before_five_minutes() {
    let (mut w, p, b1) = facing_a_farm(Skill::Level2);
    assert_eq!(first_sent_to(&mut w, p, b1, 1200), Some(306_300));
}

/// A computer at `skill` of the script `easy`, alone on a square with
/// these units, 100 gold and 100 wood, room for 8 buildings, and this
/// script.
fn with_a_town_hall(skill: Skill, script: &str, units: &str) -> (World, PlayerId) {
    let mut w = world(&format!(
        "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 8
goldmines 1000 a1
woods 1000 a1
nb_players_min 1
nb_players_max 1
starting_squares a1
starting_units {units}
starting_resources 100 100
"
    ));
    w.ai_scripts = crate::AiScripts::load(&[script]);
    let p = w
        .populate_map(&[Controller::computer_at("easy", skill)], false, false)
        .unwrap()[0];
    (w, p)
}

#[test]
fn each_level_keeps_its_number_of_workers_whatever_its_script_asks() {
    let mut kept = Vec::new();
    for skill in Skill::ALL {
        let (mut w, p) = with_a_town_hall(
            skill,
            "def easy\nget 9 peasant\n",
            "townhall 3 farm peasant",
        );
        for _ in 0..1000 {
            w.update();
        }
        kept.push(w.computer_nb(p, "peasant"));
    }
    // The script's 9 are more than levels 1 and 2 keep, and fewer than
    // the others keep.
    assert_eq!(kept, [6, 8, 10, 15, 15]);
}

#[test]
fn a_level_decides_once_in_its_number_of_turns() {
    let (mut w, p, _) = facing_a_farm(Skill::NORMAL);
    let mut decisions = Vec::new();
    for skill in Skill::ALL {
        let PlayerKind::Computer { skill: s, .. } = &mut w.plm(p).kind else {
            unreachable!("a computer");
        };
        *s = skill;
        let mut n = 0;
        for turn in 0..120 {
            w.time = turn * VIRTUAL_TIME_INTERVAL;
            if w.door(p).should_play_this_turn() {
                n += 1;
            }
        }
        decisions.push(n);
    }
    assert_eq!(decisions, [3, 4, 12, 12, 12]);
}

#[test]
fn the_top_level_builds_more_barracks_while_it_can_pay_for_their_footmen() {
    let mut barracks = Vec::new();
    for skill in [Skill::NORMAL, Skill::Level5] {
        let (mut w, p) = with_a_town_hall(
            skill,
            "def easy\nget 20 footman\n",
            "townhall barracks 3 farm 6 peasant",
        );
        // 50 gold: eight footmen, or a barracks and seven.
        w.plm(p).resources = vec![50_000, 100_000];
        for _ in 0..100 {
            w.update();
        }
        let t = Sel::One(w.rules.type_id("barracks").unwrap());
        barracks.push(w.door(p).future_nb(&t));
    }
    // It counts the barracks being built too: with one more being built,
    // it can pay for no more than three footmen for each, and stops.
    // Counting only those built, it would build two more.
    assert_eq!(barracks, [1, 2]);
}
