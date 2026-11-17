//! What each skill level changes in how a computer plays.

use super::skill::{Defence, MoreMakers, Research, Settings, Workers};
use super::tests::world;
use super::*;
use crate::entity::{PlayerId, Target};
use crate::player::Controller;
use crate::rules::VIRTUAL_TIME_INTERVAL;
use crate::world::World;

/// One setting of each level, from the weakest.
fn settings_of(f: impl Fn(Settings) -> i64) -> [i64; 4] {
    Skill::ALL.map(|s| f(s.settings()))
}

#[test]
fn each_level_has_its_settings_and_normal_is_the_original() {
    assert_eq!(settings_of(|s| s.grace), [600_000, 300_000, 0, 0]);
    assert_eq!(settings_of(|s| s.turns_between_decisions), [40, 30, 10, 10]);
    assert_eq!(
        Skill::ALL.map(|s| s.settings().workers),
        [
            Workers::AtMost(6),
            Workers::AtMost(8),
            Workers::AtLeast(10),
            Workers::WithSpare {
                kept: 10,
                spare: 15
            }
        ]
    );
    let original = MoreMakers::AsTheOriginal;
    assert_eq!(
        Skill::ALL.map(|s| s.settings().more_makers),
        [original, original, original, MoreMakers::CountingSites(3)]
    );
    let original = Research::AsTheOriginal;
    assert_eq!(
        Skill::ALL.map(|s| s.settings().research),
        [original, original, original, Research::AsSoonAsAffordable]
    );
    let original = Defence::AsTheOriginal;
    assert_eq!(
        Skill::ALL.map(|s| s.settings().defence),
        [original, original, original, Defence::WhenOutnumbered]
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
/// these units, 100 gold and 100 wood, and this script.
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
    // Normal keeps. Level 4 has resources to spare for 5 more.
    assert_eq!(kept, [6, 8, 10, 15]);
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
    assert_eq!(decisions, [3, 4, 12, 12]);
}

#[test]
fn level_four_builds_more_barracks_while_it_can_pay_for_their_footmen() {
    let mut barracks = Vec::new();
    for skill in [Skill::NORMAL, Skill::Level4] {
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
    assert_eq!(barracks, [1, 2]);
}

#[test]
fn level_four_researches_an_upgrade_as_soon_as_it_can_pay_for_it() {
    let mut researching = Vec::new();
    for skill in [Skill::NORMAL, Skill::Level4] {
        let (mut w, p) = with_a_town_hall(
            skill,
            "def easy\nresearch 1\n",
            "townhall lumbermill 3 farm 15 peasant",
        );
        // An upgrade of the lumbermill costs 8 gold and 10 wood: enough
        // for one, not for four. It has all its spare workers.
        w.plm(p).resources = vec![10_000, 12_000];
        for _ in 0..20 {
            w.update();
        }
        let weapon = w.rules.type_id("archer_weapon").unwrap();
        researching.push(w.door(p).future_nb(&Sel::List(vec![weapon])));
    }
    // Normal waits until it could pay for it four times.
    assert_eq!(researching, [0, 1]);
}

/// A computer at `skill` of the script `easy` on a1, with a town hall,
/// `maker`, three farms, 10 peasants and 50 gold and wood, and a flying
/// machine of its own on b1, where it sees three footmen of a human. A
/// flying machine harms nothing, and footmen cannot reach it.
fn outnumbered(skill: Skill, maker: &str) -> (World, PlayerId) {
    let mut w = world(
        "
square_width 12
nb_columns 2
nb_lines 1
west_east_paths a1
nb_meadows_by_square 4
goldmines 1000 a1
woods 1000 a1
nb_players_min 2
nb_players_max 2
starting_squares a1 b1
starting_units farm
starting_resources 0 0
",
    );
    w.ai_scripts = crate::AiScripts::load(&["def easy\nget 10 peasant\n"]);
    let computer = Controller::computer_at("easy", skill);
    let human = Controller::human("someone", true);
    let players = w.populate_map(&[computer, human], false, false).unwrap();
    let (p, h) = (players[0], players[1]);
    let [a1, b1] = ["a1", "b1"].map(|n| w.square_by_name(n).unwrap());
    let mut units = vec![
        ("townhall", p, a1),
        (maker, p, a1),
        ("farm", p, a1),
        ("farm", p, a1),
    ];
    units.extend(std::iter::repeat_n(("peasant", p, a1), 10));
    units.extend(std::iter::repeat_n(("footman", h, b1), 3));
    units.push(("new_flyingmachine", p, b1));
    for (t, owner, s) in units {
        let (x, y) = (w.square(s).x, w.square(s).y);
        let tid = w.rules.type_id(t).unwrap();
        w.create_creature(tid, Some(owner), Place::Square(s), x, y)
            .unwrap();
    }
    w.plm(p).resources = vec![50_000, 50_000];
    (w, p)
}

#[test]
fn outnumbered_level_four_trains_soldiers_where_normal_follows_its_script() {
    let mut trained = Vec::new();
    for skill in [Skill::NORMAL, Skill::Level4] {
        let (mut w, p) = outnumbered(skill, "barracks");
        for _ in 0..60 {
            w.update();
        }
        let footman = Sel::One(w.rules.type_id("footman").unwrap());
        let d = w.door(p);
        trained.push((d.future_nb(&footman), d.future_nb(&Sel::Workers)));
    }
    // Three footmen, and no fighter of its own: its flying machine cannot
    // attack. It works toward three footmen, the cheapest fighter its
    // barracks can train, one after the other, and gets no spare worker
    // meanwhile.
    assert_eq!(trained, [(0, 10), (2, 10)]);
}

#[test]
fn outnumbered_level_four_trains_no_flying_machine_as_a_defender() {
    let (mut w, p) = outnumbered(Skill::Level4, "workshop");
    for _ in 0..60 {
        w.update();
    }
    let nb = |w: &mut World, t: &str| {
        let t = Sel::One(w.rules.type_id(t).unwrap());
        w.door(p).future_nb(&t)
    };
    // Of what its workshop trains, the flying machine costs least but
    // cannot attack: it trains a catapult, and keeps its one flying
    // machine.
    let trained = [nb(&mut w, "new_flyingmachine"), nb(&mut w, "catapult")];
    assert_eq!(trained, [1, 1]);
}

#[test]
fn level_four_takes_no_neutral_soldiers_for_enemies() {
    // Its town hall, barracks, farms and peasants on a1, and three footmen
    // of a neutral player on b1, which its flying machine sees. It has no
    // fighter, and no enemy fighter: it trains no footman, and starts its
    // first spare worker.
    let mut w = world(
        "
square_width 12
nb_columns 2
nb_lines 1
west_east_paths a1
nb_meadows_by_square 4
goldmines 1000 a1
woods 1000 a1
nb_players_min 1
nb_players_max 1
player 50 50 a1 townhall barracks 3 farm 10 peasant
computer_only 0 0 b1 3 footman
",
    );
    w.ai_scripts = crate::AiScripts::load(&["def easy\nget 10 peasant\n"]);
    let computer = Controller::computer_at("easy", Skill::Level4);
    let p = w.populate_map(&[computer], false, false).unwrap()[0];
    let b1 = w.square_by_name("b1").unwrap();
    let (x, y) = (w.square(b1).x, w.square(b1).y);
    let machine = w.rules.type_id("new_flyingmachine").unwrap();
    w.create_creature(machine, Some(p), Place::Square(b1), x, y)
        .unwrap();
    for _ in 0..60 {
        w.update();
    }
    let footman = Sel::One(w.rules.type_id("footman").unwrap());
    let d = w.door(p);
    assert_eq!(
        (
            d.fighters_known(),
            d.future_nb(&footman),
            d.future_nb(&Sel::Workers)
        ),
        ((0, 0), 0, 11)
    );
}
