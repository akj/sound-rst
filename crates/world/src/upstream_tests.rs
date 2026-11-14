//! The original game's tests of the simulation, ported. Module and test
//! names follow the original files and test names. Tests that need the AI
//! or the client are listed in the crate's porting notes instead.

use std::path::PathBuf;

use crate::entity::{EntityId, Place, PlayerId, SquareId, Target};
use crate::nofloat::PRECISION;
use crate::orders::Keyword;
use crate::player::Controller;
use crate::rules::{AiScripts, AirGround, Rules};
use crate::world::World;

fn upstream() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        dir.join("res/rules.txt").is_file(),
        "the original game is missing from {}: run scripts/fetch-upstream.ps1",
        dir.display()
    );
    dir
}

fn read(relative: &str) -> String {
    let path = upstream().join(relative);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `World()` then `load_and_build_map` with the default rules.
fn world_text(text: &str) -> World {
    let rules = Rules::load(&[read("res/rules.txt")]);
    let ai = AiScripts::load(&[read("res/ai.txt")]);
    let mut w = World::new(rules, ai, Vec::new(), 0);
    w.load_and_build_map(text).unwrap();
    w
}

fn world(relative: &str) -> World {
    world_text(&read(relative))
}

fn sq(w: &World, name: &str) -> SquareId {
    w.square_by_name(name).unwrap()
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

/// `rules.unit_class(t)(player, place, place.x, place.y)`.
fn create(w: &mut World, t: &str, p: PlayerId, s: SquareId) -> EntityId {
    let (x, y) = (w.square(s).x, w.square(s).y);
    let tid = w.rules.type_id(t).unwrap();
    w.create_creature(tid, Some(p), Place::Square(s), x, y)
        .unwrap()
}

/// `Human(world, DummyClient())`: a player outside the game's players.
fn human(w: &mut World) -> PlayerId {
    w.new_player(&Controller::human("ai_timers", false))
        .unwrap()
}

fn find(w: &World, p: PlayerId, t: &str, index: usize) -> Option<EntityId> {
    w.player(p)
        .units
        .iter()
        .copied()
        .filter(|&u| &*w.e(u).type_name == t)
        .nth(index)
}

fn nb(w: &World, p: PlayerId, t: &str) -> usize {
    w.player(p)
        .units
        .iter()
        .filter(|&&u| &*w.e(u).type_name == t)
        .count()
}

fn take(w: &mut World, u: EntityId, order: &str) {
    w.take_order(u, words(order), true, false, None).unwrap();
}

fn keywords(w: &World, u: EntityId) -> Vec<&'static str> {
    w.e(u).orders.iter().map(|o| o.keyword.as_str()).collect()
}

/// `is_perceiving_method` of the original tests.
fn is_perceiving(w: &mut World, p: PlayerId, o: EntityId) -> bool {
    w.update_buckets();
    w.update_cloaking();
    w.update_detection();
    w.update_perception_and_memory(p).unwrap();
    w.player(p).perception.contains(&o)
}

fn move_to(w: &mut World, u: EntityId, s: SquareId) {
    w.move_to(u, Some(Place::Square(s)), None, 90).unwrap();
}

fn place_of(w: &World, u: EntityId) -> SquareId {
    w.e(u).square().unwrap()
}

mod test_worldunit {
    use super::*;

    #[test]
    fn test_next_stage() {
        let mut w = world("soundrts/tests/jl1.txt");
        let p = human(&mut w);
        let a1 = sq(&w, "a1");
        let unit = create(&mut w, "footman", p, a1);
        assert!(w.next_stage(unit, None, false).unwrap().is_none());
        assert!(
            w.next_stage(unit, Some(&Target::Square(a1)), false)
                .unwrap()
                .is_none()
        );
        let target = create(&mut w, "footman", p, a1);
        let stage = w
            .next_stage(unit, Some(&Target::Entity(target)), false)
            .unwrap();
        assert!(matches!(stage, Some(Target::Entity(t)) if t == target));
        w.delete(target);
        assert!(
            w.next_stage(unit, Some(&Target::Entity(target)), false)
                .unwrap()
                .is_none()
        );
        let b1 = sq(&w, "b1");
        let other = create(&mut w, "footman", p, b1);
        let transport = create(&mut w, "new_flyingmachine", p, a1);
        w.load(transport, unit);
        assert!(
            w.next_stage(unit, Some(&Target::Entity(other)), false)
                .unwrap()
                .is_none()
        );
    }

    fn counterattack_setup() -> (World, EntityId) {
        let mut w = world("soundrts/tests/jl1.txt");
        let p = human(&mut w);
        let b1 = sq(&w, "b1");
        let unit = create(&mut w, "footman", p, b1);
        (w, unit)
    }

    #[test]
    fn test_counterattack() {
        let (mut w, unit) = counterattack_setup();
        let place = sq(&w, "a1");
        w.counterattack(unit, place).unwrap();
        let e = w.e(unit);
        assert_eq!(keywords(&w, unit), ["go", "go"]);
        assert_eq!(e.orders[0].args, [w.square(place).id.to_string()]);
        let here = w.square(place_of(&w, unit)).id;
        assert_eq!(e.orders[1].args, [format!("zoom-{here}-{}-{}", e.x, e.y)]);
    }

    #[test]
    fn test_counterattack_if_cannot_go() {
        let (mut w, unit) = counterattack_setup();
        // No path leads from b1 to b2 through one square.
        let place = sq(&w, "b2");
        w.counterattack(unit, place).unwrap();
        assert!(w.e(unit).orders.is_empty());
    }

    #[test]
    fn test_counterattack_if_defensive_mode() {
        let (mut w, unit) = counterattack_setup();
        w.em(unit).ai_mode = Some(crate::entity::AiMode::Defensive);
        let place = sq(&w, "a1");
        w.counterattack(unit, place).unwrap();
        assert!(w.e(unit).orders.is_empty());
    }

    const DAMAGE_VS_RULES: &str = "
def pikeman
class soldier
damage 6
damage_vs cavalry 12

def cavalry
class soldier

def light_cavalry
is_a cavalry

def light_cavalry_subtype
is_a light_cavalry

def horse_archer
is_a cavalry archer
";

    fn rules_world(text: &str) -> World {
        World::new(Rules::load(&[text]), AiScripts::default(), Vec::new(), 0)
    }

    #[test]
    fn test_damage_vs() {
        let w = rules_world(DAMAGE_VS_RULES);
        let e = |t: &str| w.entity_from_nowhere(t).unwrap();
        let pikeman = e("pikeman");
        assert_eq!(
            w.base_damage_versus_entities(&pikeman, &pikeman),
            6 * PRECISION
        );
        assert_eq!(
            w.base_damage_versus_entities(&pikeman, &e("cavalry")),
            12 * PRECISION
        );
        assert_eq!(
            w.base_damage_versus_entities(&pikeman, &e("light_cavalry")),
            12 * PRECISION
        );
        assert_eq!(
            w.base_damage_versus_entities(&pikeman, &e("light_cavalry_subtype")),
            12 * PRECISION
        );
        assert_eq!(
            w.base_damage_versus_entities(&pikeman, &e("horse_archer")),
            12 * PRECISION
        );
    }

    const ARMOR_VS_RULES: &str = "
def archer
class soldier

def horse_archer
is_a cavalry archer

def heavy
class soldier
armor 1
armor_vs archer 2
";

    #[test]
    fn test_armor_vs() {
        let w = rules_world(ARMOR_VS_RULES);
        let e = |t: &str| w.entity_from_nowhere(t).unwrap();
        let heavy = e("heavy");
        assert_eq!(w.armor_versus_entities(&heavy, &heavy), PRECISION);
        assert_eq!(w.armor_versus_entities(&heavy, &e("archer")), 2 * PRECISION);
        assert_eq!(
            w.armor_versus_entities(&heavy, &e("horse_archer")),
            2 * PRECISION
        );
    }
}

mod test_world_unit {
    use crate::map::convert_and_split_first_numbers as f;

    #[test]
    fn test_convert_and_split_first_numbers() {
        let w = |s: &str| super::words(s);
        assert_eq!(
            f(&w("1 9 a1 2 peasant")),
            (vec![1000, 9000], w("a1 2 peasant"))
        );
        assert_eq!(f(&w("1 9 a1")), (vec![1000, 9000], w("a1")));
        assert_eq!(f(&w("1 a1")), (vec![1000], w("a1")));
        assert_eq!(f(&w("a1")), (vec![], w("a1")));
        assert_eq!(f(&[]), (vec![], vec![]));
        assert_eq!(f(&w("a1 2 peasant")), (vec![], w("a1 2 peasant")));
        assert_eq!(f(&w("1 9")), (vec![1000, 9000], vec![]));
    }
}

mod test_worldplayerbase {
    use super::*;

    fn setup() -> (World, PlayerId, PlayerId, EntityId) {
        let mut w = world("soundrts/tests/jl1.txt");
        let p = human(&mut w);
        let enemy = human(&mut w);
        let b1 = sq(&w, "b1");
        let unit = create(&mut w, "footman", p, b1);
        (w, p, enemy, unit)
    }

    #[test]
    fn test_on_unit_attacked() {
        let (mut w, p, enemy, unit) = setup();
        let a1 = sq(&w, "a1");
        let hostile = create(&mut w, "footman", enemy, a1);
        w.plm(p).perception.insert(hostile);
        w.on_unit_attacked(p, unit, hostile);
        assert!(w.counterattack_places(p).contains(&a1));
    }

    #[test]
    fn test_update_counterattacks() {
        let (mut w, p, _, unit) = setup();
        w.plm(p).counterattack_places = vec![sq(&w, "a1")];
        w.update_counterattacks(p).unwrap();
        assert!(w.counterattack_places(p).is_empty());
        assert_eq!(keywords(&w, unit), ["go", "go"]);
    }

    #[test]
    fn test_update_counterattacks_if_not_nearby() {
        let (mut w, p, _, unit) = setup();
        w.plm(p).counterattack_places = vec![sq(&w, "c4")];
        w.update_counterattacks(p).unwrap();
        assert!(w.counterattack_places(p).is_empty());
        assert!(w.e(unit).orders.is_empty());
    }

    #[test]
    fn test_update_counterattacks_if_closer_attackers() {
        let (mut w, p, _, unit) = setup();
        w.plm(p).counterattack_places = vec![sq(&w, "a1"), place_of(&w, unit)];
        w.update_counterattacks(p).unwrap();
        assert!(w.counterattack_places(p).is_empty());
        assert!(w.e(unit).orders.is_empty());
    }
}

mod test_worldplayerstats {
    use crate::nofloat::PRECISION;
    use crate::player::{PlayerStats, StatEvent::*, StatTarget::*};

    const GOLD: usize = 0;

    #[test]
    fn test_add_and_get() {
        let mut stats = PlayerStats::default();
        assert_eq!(stats.get(Produced, Unit), 0);
        stats.add(Produced, Unit, 1);
        assert_eq!(stats.get(Produced, Unit), 1);
        assert_eq!(stats.get(Gathered, Resource(GOLD)), 0);
        stats.add(Gathered, Resource(GOLD), 5);
        assert_eq!(stats.get(Gathered, Resource(GOLD)), 5);
    }

    #[test]
    fn test_consumed() {
        let mut stats = PlayerStats::default();
        assert_eq!(stats.consumed(GOLD, &[0, 0]), 0);
        stats.add(Gathered, Resource(GOLD), 5);
        assert_eq!(stats.consumed(GOLD, &[0, 0]), 5);
        assert_eq!(stats.consumed(GOLD, &[5, 0]), 0);
    }

    #[test]
    fn test_score() {
        let mut stats = PlayerStats::default();
        let r = [0, 0];
        stats.add(Gathered, Resource(GOLD), 100 * PRECISION);
        assert_eq!(stats.score(&r), 200);
        let steps = [
            (Produced, Unit, 201),
            (Lost, Unit, 200),
            (Killed, Unit, 201),
            (Produced, Building, 202),
            (Lost, Building, 201),
            (Killed, Building, 202),
        ];
        for (e, t, expected) in steps {
            stats.add(e, t, 1);
            assert_eq!(stats.score(&r), expected);
        }
    }

    /// The numbers `score_msgs` says; the client builds the messages.
    #[test]
    fn test_score_msgs() {
        let mut stats = PlayerStats::default();
        let r = [0, 0];
        stats.add(Gathered, Resource(GOLD), 100 * PRECISION);
        for t in [Unit, Building] {
            for e in [Produced, Lost, Killed] {
                stats.add(e, t, 1);
            }
        }
        stats.freeze(15 * PRECISION);
        // The time of the world after the freeze is ignored.
        assert_eq!(
            stats.game_duration_in_minutes_seconds(30 * PRECISION),
            (0, 15)
        );
        assert_eq!(stats.get(Gathered, Resource(0)) / PRECISION, 100);
        assert_eq!(stats.consumed(0, &r) / PRECISION, 100);
        assert_eq!(stats.get(Gathered, Resource(1)) / PRECISION, 0);
        assert_eq!(stats.score(&r), 202);
    }
}

mod test_worldplayer {
    use super::*;
    use crate::player::Objective;

    const TINY_MAP: &str = "
square_width 12
nb_columns 1
nb_lines 1
nb_meadows_by_square 9

nb_players_min 1
nb_players_max 1
starting_squares a1
starting_units townhall farm peasant
starting_resources 10 10
";

    #[test]
    fn objective_test_init() {
        let o = Objective {
            number: "1".into(),
            description: vec![12, 13, 14],
        };
        assert_eq!(o.number, "1");
        assert_eq!(o.description, [12, 13, 14]);
    }

    fn tiny_set_up() -> (World, PlayerId) {
        let mut w = world_text(TINY_MAP);
        let p = w
            .populate_map(&[Controller::computer("timers")], true, false)
            .unwrap()[0];
        (w, p)
    }

    fn add(w: &mut World, p: PlayerId, items: &str) {
        w.lang_add_units(p, &words(items), None, 0, None, true);
    }

    fn set_rule(w: &mut World, t: &str, f: impl FnOnce(&mut crate::rules::Stats)) {
        f(&mut w.rules.unit_type_mut(t).unwrap().stats);
    }

    fn run_until_idle(w: &mut World, units: &[EntityId], n: usize) {
        for _ in 0..n {
            w.update();
            if units.iter().all(|&u| w.e(u).orders.is_empty()) {
                break;
            }
        }
    }

    #[test]
    fn test_storage_bonus() {
        let (mut w, cp) = tiny_set_up();
        w.update();
        assert_eq!(w.player(cp).storage_bonus[1], 0);
        for _ in 0..2 {
            add(&mut w, cp, "a1 lumbermill");
            w.update();
            assert_eq!(w.player(cp).storage_bonus[1], PRECISION);
        }
    }

    #[test]
    fn test_no_count_limit() {
        let (mut w, cp) = tiny_set_up();
        let th = find(&w, cp, "townhall", 0).unwrap();
        take(&mut w, th, "train peasant");
        assert!(!w.e(th).orders.is_empty());
    }

    #[test]
    fn test_count_limit() {
        let (mut w, cp) = tiny_set_up();
        set_rule(&mut w, "peasant", |s| s.count_limit = 1);
        w.update();
        let th = find(&w, cp, "townhall", 0).unwrap();
        assert!(
            !w.order_menu(th, Keyword::Train, false)
                .contains(&"train peasant".to_owned())
        );
        take(&mut w, th, "train peasant");
        assert!(w.e(th).orders.is_empty());
        set_rule(&mut w, "peasant", |s| s.count_limit = 2);
        w.update();
        assert!(
            w.order_menu(th, Keyword::Train, false)
                .contains(&"train peasant".to_owned())
        );
        take(&mut w, th, "train peasant");
        assert!(!w.e(th).orders.is_empty());
    }

    #[test]
    fn test_count_limit_build() {
        let (mut w, cp) = tiny_set_up();
        add(&mut w, cp, "a1 peasant");
        set_rule(&mut w, "peasant", |s| s.speed = 10 * PRECISION);
        set_rule(&mut w, "barracks", |s| s.time_cost = PRECISION);
        set_rule(&mut w, "farm", |s| s.time_cost = PRECISION);
        w.plm(cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        set_rule(&mut w, "barracks", |s| s.count_limit = 1);
        w.update();
        let p1 = find(&w, cp, "peasant", 0).unwrap();
        let p2 = find(&w, cp, "peasant", 1).unwrap();
        for p in [p1, p2] {
            assert!(w.e(p).orders.is_empty());
        }
        assert!(
            w.order_menu(p1, Keyword::Build, false)
                .contains(&"build barracks".to_owned())
        );
        for p in [p1, p2] {
            take(&mut w, p, "build barracks a1");
            assert!(!w.e(p).orders.is_empty());
        }
        run_until_idle(&mut w, &[p1, p2], 50);
        for p in [p1, p2] {
            assert!(w.e(p).orders.is_empty());
        }
        assert_eq!(nb(&w, cp, "barracks"), 1);
        assert!(
            !w.order_menu(p1, Keyword::Build, false)
                .contains(&"build barracks".to_owned())
        );
        for p in [p1, p2] {
            take(&mut w, p, "build barracks a1");
            assert!(w.e(p).orders.is_empty());
        }
        assert_eq!(nb(&w, cp, "barracks"), 1);
        set_rule(&mut w, "barracks", |s| s.count_limit = 2);
        w.update();
        assert_eq!(w.future_count(cp, "barracks"), 1);
        for p in [p1, p2] {
            take(&mut w, p, "build barracks a1");
            assert_eq!(w.e(p).orders[0].keyword, Keyword::Build);
            assert_eq!(w.e(p).orders[0].type_name.as_deref(), Some("barracks"));
        }
        assert_eq!(w.future_count(cp, "barracks"), 1);
        run_until_idle(&mut w, &[p1, p2], 50);
        for p in [p1, p2] {
            assert!(w.e(p).orders.is_empty());
        }
        assert_eq!(w.future_count(cp, "barracks"), 2);
        assert_eq!(nb(&w, cp, "barracks"), 2);
        assert_eq!(nb(&w, cp, "farm"), 1);
        take(&mut w, p1, "build farm a1");
        run_until_idle(&mut w, &[p1, p2], 50);
        assert_eq!(nb(&w, cp, "farm"), 2);
    }

    #[test]
    fn test_count_limit_build_queued() {
        let (mut w, cp) = tiny_set_up();
        set_rule(&mut w, "peasant", |s| s.speed = 10 * PRECISION);
        set_rule(&mut w, "barracks", |s| s.time_cost = PRECISION);
        add(&mut w, cp, "a1 peasant");
        w.plm(cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        set_rule(&mut w, "barracks", |s| s.count_limit = 1);
        w.update();
        let p1 = find(&w, cp, "peasant", 0).unwrap();
        let p2 = find(&w, cp, "peasant", 1).unwrap();
        for p in [p1, p2] {
            assert!(w.e(p).orders.is_empty());
            take(&mut w, p, "build barracks a1");
            w.take_order(p, words("build barracks a1"), false, false, None)
                .unwrap();
            assert_eq!(w.e(p).orders.len(), 2);
        }
        run_until_idle(&mut w, &[p1, p2], 1000);
        assert!(w.e(p1).orders.is_empty());
        assert!(w.e(p2).orders.is_empty());
        assert_eq!(nb(&w, cp, "barracks"), 1);
    }

    #[test]
    fn test_count_limit_upgrade_to() {
        let (mut w, cp) = tiny_set_up();
        for t in ["lumbermill", "magestower", "archer", "archer"] {
            add(&mut w, cp, &format!("a1 {t}"));
        }
        w.plm(cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        set_rule(&mut w, "darkarcher", |s| s.count_limit = 1);
        w.update();
        let a1 = find(&w, cp, "archer", 0).unwrap();
        let a2 = find(&w, cp, "archer", 1).unwrap();
        take(&mut w, a1, "upgrade_to darkarcher");
        take(&mut w, a2, "upgrade_to darkarcher");
        assert_eq!(w.e(a1).orders[0].keyword, Keyword::UpgradeTo);
        assert!(w.e(a2).orders.is_empty());
    }

    #[test]
    fn test_count_limit_summon() {
        let (mut w, cp) = tiny_set_up();
        add(&mut w, cp, "a1 dragon");
        assert_eq!(nb(&w, cp, "dragon"), 1);
        set_rule(&mut w, "dragon", |s| s.count_limit = 2);
        add(&mut w, cp, "a1 mage");
        w.plm(cp).upgrades.push("u_summon_dragon".into());
        w.update();
        let m = find(&w, cp, "mage", 0).unwrap();
        take(&mut w, m, "use a_summon_dragon a1");
        w.update();
        assert_eq!(nb(&w, cp, "dragon"), 2);
    }

    fn kill_ten_footmen(w: &mut World, cp: PlayerId) {
        add(w, cp, "a1 10 footman");
        for _ in 0..10 {
            let f = find(w, cp, "footman", 0).unwrap();
            assert_eq!(w.square(place_of(w, f)).name, "a1");
            w.die(f, None).unwrap();
        }
    }

    #[test]
    fn test_count_limit_raise_dead() {
        let (mut w, cp) = tiny_set_up();
        kill_ten_footmen(&mut w, cp);
        assert_eq!(nb(&w, cp, "zombie"), 0);
        set_rule(&mut w, "zombie", |s| s.count_limit = 1);
        add(&mut w, cp, "a1 necromancer");
        w.plm(cp).upgrades.push("u_raise_dead".into());
        w.update();
        let n = find(&w, cp, "necromancer", 0).unwrap();
        take(&mut w, n, "use a_raise_dead a1");
        w.update();
        assert_eq!(nb(&w, cp, "zombie"), 1);
    }

    #[test]
    fn test_count_limit_resurrection() {
        let (mut w, cp) = tiny_set_up();
        w.plm(cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        kill_ten_footmen(&mut w, cp);
        assert_eq!(nb(&w, cp, "footman"), 0);
        set_rule(&mut w, "footman", |s| s.count_limit = 2);
        w.update();
        w.update();
        add(&mut w, cp, "a1 priest");
        w.plm(cp).upgrades.push("u_resurrection".into());
        let p = find(&w, cp, "priest", 0).unwrap();
        take(&mut w, p, "use a_resurrection a1");
        w.update();
        assert_eq!(nb(&w, cp, "footman"), 2);
    }

    #[test]
    fn test_count_limit_resurrection_with_train_queue() {
        let (mut w, cp) = tiny_set_up();
        w.plm(cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        kill_ten_footmen(&mut w, cp);
        assert_eq!(nb(&w, cp, "footman"), 0);
        set_rule(&mut w, "footman", |s| s.count_limit = 2);
        add(&mut w, cp, "a1 barracks");
        w.update();
        let b = find(&w, cp, "barracks", 0).unwrap();
        take(&mut w, b, "train footman");
        w.update();
        add(&mut w, cp, "a1 priest");
        w.plm(cp).upgrades.push("u_resurrection".into());
        let p = find(&w, cp, "priest", 0).unwrap();
        take(&mut w, p, "use a_resurrection a1");
        w.update();
        assert_eq!(nb(&w, cp, "footman"), 1);
    }

    struct Setup {
        w: World,
        cp: PlayerId,
        cp2: PlayerId,
    }

    fn set_up(alliance: (&str, &str), cloak: bool, map_name: &str, ai: (&str, &str)) -> Setup {
        let mut w = world(&format!("soundrts/tests/{map_name}.txt"));
        if cloak {
            w.rules
                .unit_type_mut("new_flyingmachine")
                .unwrap()
                .stats
                .is_a_cloaker = 1;
        }
        let mut c1 = Controller::computer(ai.0);
        let mut c2 = Controller::computer(ai.1);
        c1.alliance = Some(alliance.0.into());
        c2.alliance = Some(alliance.1.into());
        w.populate_map(&[c1, c2], false, false).unwrap();
        let (cp, cp2) = (w.players[0], w.players[1]);
        assert_eq!(w.players.len(), 2);
        w.update_buckets();
        Setup { w, cp, cp2 }
    }

    fn default_set_up() -> Setup {
        set_up(("1", "2"), false, "jl1_extended", ("easy", "easy"))
    }

    #[test]
    fn computer_test_init_and_get_a_peasant() {
        let mut s = default_set_up();
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(
            sorted(s.w.rules.get_makers("peasant")),
            ["castle", "keep", "townhall"]
        );
        assert_eq!(sorted(s.w.rules.get_makers("footman")), ["barracks"]);
        assert_eq!(sorted(s.w.rules.get_makers("barracks")), ["peasant"]);
        let th = find(&s.w, s.cp, "townhall", 0).unwrap();
        assert!(keywords(&s.w, th).is_empty());
        assert!(!s.w.ai_get(s.cp, 1000, "peasant").unwrap());
        assert_eq!(keywords(&s.w, th), ["train"]);
    }

    #[test]
    fn computer_test_init_and_get_a_footman() {
        let mut s = default_set_up();
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        assert!(keywords(&s.w, p).is_empty());
        assert!(!s.w.ai_get(s.cp, 1000, "footman").unwrap());
        s.w.update();
        assert!(!keywords(&s.w, p).is_empty());
    }

    #[test]
    fn computer_test_init_and_upgrade_to_a_keep() {
        let mut s = set_up(("1", "2"), false, "jl1", ("easy", "easy"));
        let w = &mut s.w;
        w.plm(s.cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        let th = find(w, s.cp, "townhall", 0).unwrap();
        assert!(keywords(w, th).is_empty());
        assert!(!w.ai_get(s.cp, 1, "keep").unwrap());
        // the keep requires barracks
        assert!(keywords(w, th).is_empty());
        add(w, s.cp, "a1 barracks");
        assert!(find(w, s.cp, "barracks", 0).is_some());
        assert!(!w.ai_get(s.cp, 1, "knight").unwrap()); // => get keep
        assert_eq!(keywords(w, th), ["upgrade_to"]);
    }

    fn archer_with_upgrade_buildings() -> (Setup, EntityId) {
        let mut s = set_up(("1", "2"), false, "jl1", ("easy", "easy"));
        let w = &mut s.w;
        w.plm(s.cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        add(w, s.cp, "a1 archer");
        add(w, s.cp, "a1 lumbermill");
        add(w, s.cp, "a1 magestower");
        let archer = find(w, s.cp, "archer", 0).unwrap();
        (s, archer)
    }

    #[test]
    fn computer_test_upgrade_to_if_autoexploring() {
        let (mut s, archer) = archer_with_upgrade_buildings();
        let w = &mut s.w;
        take(w, archer, "auto_explore");
        assert_eq!(keywords(w, archer), ["auto_explore"]);
        assert!(!w.ai_get(s.cp, 1, "darkarcher").unwrap());
        assert_eq!(keywords(w, archer)[0], "upgrade_to");
    }

    #[test]
    fn computer_test_no_autoexploring_if_upgrade_to() {
        let (mut s, archer) = archer_with_upgrade_buildings();
        let w = &mut s.w;
        assert!(!w.ai_get(s.cp, 1, "darkarcher").unwrap());
        assert_eq!(keywords(w, archer)[0], "upgrade_to");
        assert!(!w.best_explorers(s.cp).contains(&archer));
    }

    #[test]
    fn computer_test_is_a_is_not_is() {
        let mut s = set_up(("1", "2"), false, "jl1", ("easy", "easy"));
        let w = &mut s.w;
        w.plm(s.cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        let first = w.player(s.cp).units[0];
        w.die(first, None).unwrap();
        assert_eq!(w.computer_nb(s.cp, "townhall"), 0);
        assert!(!w.has(s.cp, "townhall"));
        add(w, s.cp, "a1 keep"); // is a townhall
        assert_eq!(w.computer_nb(s.cp, "townhall"), 0);
        assert!(w.has(s.cp, "townhall"));
    }

    #[test]
    fn computer_test_is_a_fills_the_requirement() {
        let mut s = set_up(("1", "2"), false, "jl1", ("easy", "easy"));
        let w = &mut s.w;
        w.plm(s.cp).resources = vec![1000 * PRECISION, 1000 * PRECISION];
        let first = w.player(s.cp).units[0];
        w.die(first, None).unwrap();
        assert_eq!(w.computer_nb(s.cp, "townhall"), 0);
        assert!(!w.has(s.cp, "townhall"));
        add(w, s.cp, "a1 keep"); // is a townhall
        assert_eq!(w.computer_nb(s.cp, "peasant"), 1);
        let p = find(w, s.cp, "peasant", 0).unwrap();
        assert!(keywords(w, p).is_empty());
        assert_eq!(w.computer_nb(s.cp, "blacksmith"), 0);
        assert!(w.has(s.cp, "townhall"));
        w.update_effect_users_and_workers(s.cp).unwrap();
        w.update_buckets();
        w.update_perception(s.cp).unwrap();
        assert!(!w.ai_get(s.cp, 1, "blacksmith").unwrap()); // requires a townhall
        assert_eq!(w.e(p).orders[0].keyword, Keyword::Build);
        assert_eq!(w.e(p).orders[0].type_name.as_deref(), Some("blacksmith"));
    }

    #[test]
    fn computer_test_menace() {
        let s = default_set_up();
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        assert!(s.w.e(p).menace() > 0);
        let th = find(&s.w, s.cp, "townhall", 0).unwrap();
        assert_eq!(s.w.e(th).menace(), 0);
    }

    #[test]
    fn computer_test_immediate_order() {
        let mut s = default_set_up();
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        take(&mut s.w, p, "go 1");
        assert!(!s.w.e(p).orders.is_empty());
        take(&mut s.w, p, "mode_offensive");
        assert!(!s.w.e(p).orders.is_empty());
    }

    #[test]
    fn computer_test_perception() {
        let mut s = default_set_up();
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        assert!(is_perceiving(w, cp, p));
        let p2 = find(w, cp2, "peasant", 0).unwrap();
        let p2place = place_of(w, p2);
        assert!(!is_perceiving(w, cp, p2));
        assert!(
            !w.player(cp2)
                .observed_before_squares
                .contains(&place_of(w, p))
        );
        move_to(w, p2, place_of(w, p));
        assert!(is_perceiving(w, cp, p2));
        assert!(is_perceiving(w, cp2, p));
        assert!(
            w.player(cp2)
                .observed_before_squares
                .contains(&place_of(w, p))
        );
        let fm = find(w, cp, "new_flyingmachine", 0).unwrap();
        assert!(!w.e(p).is_inside());
        w.load(fm, p);
        assert!(w.e(p).is_inside());
        assert!(!is_perceiving(w, cp2, p));
        w.unload_all(fm, None);
        assert!(!w.e(p).is_inside());
        assert!(is_perceiving(w, cp2, p));
        move_to(w, p2, p2place);
        assert!(!is_perceiving(w, cp, p2));
        assert!(!is_perceiving(w, cp2, p));
        move_to(w, p2, place_of(w, p));
        assert!(is_perceiving(w, cp, p2));
        assert!(is_perceiving(w, cp2, p));
        w.die(p2, None).unwrap();
        assert!(!is_perceiving(w, cp2, p));
        w.load_all(fm, None);
        w.unload_all(fm, None);
        w.load_all(fm, None);
        w.unload_all(fm, None);
    }

    fn wait_until_not_perceived(w: &mut World, p: PlayerId, o: EntityId) -> usize {
        let mut n = 0;
        while is_perceiving(w, p, o) {
            w.update();
            n += 1;
            assert!(n < 20);
        }
        n
    }

    #[test]
    fn computer_test_perception_by_observe() {
        let mut s = default_set_up();
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        assert!(is_perceiving(w, cp, p));
        let p2 = find(w, cp2, "peasant", 0).unwrap();
        assert!(!is_perceiving(w, cp, p2));
        assert!(
            !w.player(cp2)
                .observed_before_squares
                .contains(&place_of(w, p))
        );
        w.hit(p2, p);
        assert!(is_perceiving(w, cp, p2));
        assert!(!is_perceiving(w, cp2, p));
        let n = wait_until_not_perceived(w, cp, p2);
        assert!(n == 11 || n == 12, "{n}");
        assert!(!is_perceiving(w, cp, p2));
        assert!(!is_perceiving(w, cp2, p));
        assert!(w.remembers(cp, p2));
    }

    fn plateau_setup(one_shot: bool) -> (Setup, EntityId, EntityId) {
        let mut s = set_up(("1", "2"), false, "height", ("easy", "easy"));
        let sw = s.w.square_width();
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        {
            let e = s.w.em(p);
            e.stats.bonus_height = 1;
            if one_shot {
                e.stats.damage = e.stats.hp_max * 2;
            }
            e.stats.range = 2 * PRECISION;
            e.stats.sight_range = 2 * sw;
        }
        let p2 = find(&s.w, s.cp2, "peasant", 0).unwrap();
        (s, p, p2)
    }

    #[test]
    fn computer_test_perception_by_observe_with_plateau() {
        let (mut s, p, p2) = plateau_setup(false);
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        assert_eq!(w.height(p), Some(2));
        assert_eq!(w.square(place_of(w, p)).name, "a1");
        assert_eq!(w.square(place_of(w, p2)).name, "b1");
        assert!(w.height(p) > w.height(p2));
        assert!(is_perceiving(w, cp, p2));
        let (pp, p2p) = (place_of(w, p), place_of(w, p2));
        assert_eq!(w.enemy_menace(cp, p2p), 0);
        w.update();
        assert!(w.enemy_menace(cp, p2p) != 0);
        assert!(!is_perceiving(w, cp2, p));
        assert_eq!(w.enemy_menace(cp2, pp), 0);
        assert!(!w.square_is_dangerous(cp2, pp));
        assert_eq!(w.enemy_menace(cp2, p2p), 0);
        assert!(!w.square_is_dangerous(cp2, p2p));
        assert!(
            w.next_stage(p2, Some(&Target::Square(pp)), true)
                .unwrap()
                .is_some()
        );
        w.hit(p, p2);
        assert!(is_perceiving(w, cp2, p));
        let n = wait_until_not_perceived(w, cp2, p);
        assert!(n == 11 || n == 12, "{n}");
        assert!(!is_perceiving(w, cp2, p));
        assert!(w.remembers(cp2, p));
        assert!(w.enemy_menace(cp2, pp) != 0);
        assert!(w.square_is_dangerous(cp2, pp));
        assert!(w.enemy_menace(cp2, p2p) != 0);
        assert!(w.square_is_dangerous(cp2, p2p));
        assert!(
            w.next_stage(p2, Some(&Target::Square(pp)), true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn computer_test_perception_by_observe_with_plateau_one_shot() {
        let (mut s, p, p2) = plateau_setup(true);
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        assert_eq!(w.square(place_of(w, p)).name, "a1");
        assert_eq!(w.square(place_of(w, p2)).name, "b1");
        assert!(is_perceiving(w, cp, p2));
        let (pp, p2place) = (place_of(w, p), place_of(w, p2));
        assert_eq!(w.enemy_menace(cp, p2place), 0);
        w.update();
        assert!(w.enemy_menace(cp, p2place) != 0);
        assert!(!is_perceiving(w, cp2, p));
        assert_eq!(w.enemy_menace(cp2, pp), 0);
        assert!(!w.square_is_dangerous(cp2, pp));
        assert_eq!(w.enemy_menace(cp2, p2place), 0);
        assert!(!w.square_is_dangerous(cp2, p2place));
        assert!(
            w.next_stage(p2, Some(&Target::Square(pp)), true)
                .unwrap()
                .is_some()
        );
        w.hit(p, p2);
        assert!(is_perceiving(w, cp2, p));
        let n = wait_until_not_perceived(w, cp2, p);
        assert!(n == 11 || n == 12, "{n}");
        assert!(!is_perceiving(w, cp2, p));
        assert!(w.remembers(cp2, p));
        assert!(w.enemy_menace(cp2, pp) != 0);
        assert!(w.square_is_dangerous(cp2, pp));
        assert!(w.enemy_menace(cp2, p2place) != 0);
        assert!(w.square_is_dangerous(cp2, p2place));
    }

    fn memory_models(w: &World, p: PlayerId) -> Vec<EntityId> {
        w.player(p).memory.keys().copied().collect()
    }

    fn after_unit_disappears(convert: bool) {
        let mut s = default_set_up();
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        assert!(is_perceiving(w, cp, p));
        assert!(w.player(cp).perception.contains(&p));
        assert!(!memory_models(w, cp).contains(&p));
        let p2 = find(w, cp2, "peasant", 0).unwrap();
        assert!(!memory_models(w, cp).contains(&p2));
        assert!(!w.player(cp).perception.contains(&p2));
        move_to(w, p, place_of(w, p2));
        assert!(is_perceiving(w, cp, p));
        assert!(w.player(cp).perception.contains(&p));
        let th2 = find(w, cp2, "townhall", 0).unwrap();
        assert!(w.player(cp).perception.contains(&th2));
        if convert {
            w.set_player(p, Some(cp2));
        } else {
            w.die(p, None).unwrap();
        }
        w.update();
        assert!(!w.player(cp).perception.contains(&th2));
        assert!(memory_models(w, cp).contains(&th2));
        assert!(!is_perceiving(w, cp, p));
        assert!(!w.player(cp).perception.contains(&p));
        if !convert {
            assert!(!memory_models(w, cp).contains(&p));
        }
    }

    #[test]
    fn computer_test_perception_after_unit_death() {
        after_unit_disappears(false);
    }

    #[test]
    fn computer_test_perception_after_unit_ownership_change() {
        after_unit_disappears(true);
    }

    #[test]
    fn computer_test_has() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let c = find(w, cp, "castle", 0).unwrap();
        for t in ["castle", "keep", "townhall"] {
            assert!(w.has(cp, t));
        }
        let th = find(w, cp, "townhall", 0).unwrap();
        w.delete(th);
        for t in ["castle", "keep", "townhall"] {
            assert!(w.has(cp, t));
        }
        w.delete(c);
        for t in ["castle", "keep", "townhall"] {
            assert!(!w.has(cp, t));
        }
    }

    #[test]
    fn computer_test_has2() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let c = find(w, cp, "castle", 0).unwrap();
        for t in ["castle", "keep", "townhall"] {
            assert!(w.has(cp, t));
        }
        w.delete(c);
        assert!(!w.has(cp, "castle"));
        assert!(!w.has(cp, "keep"));
        assert!(w.has(cp, "townhall"));
    }

    #[test]
    fn computer_test_react() {
        let mut s = default_set_up();
        let w = &mut s.w;
        let p = find(w, s.cp, "peasant", 0).unwrap();
        let p2 = find(w, s.cp2, "peasant", 0).unwrap();
        assert!(w.is_an_enemy(p2, &Target::Entity(p)));
        move_to(w, p, place_of(w, p2));
        w.update();
        assert!(w.can_attack(p2, &Target::Entity(p)).unwrap());
        assert!(matches!(w.e(p2).action_target(), Some(Target::Entity(t)) if *t == p));
    }

    fn footman_beside_peasant(map: &str, ai: &str) -> (Setup, EntityId) {
        let mut s = set_up(("1", "2"), false, map, (ai, ai));
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        let name = s.w.square(place_of(&s.w, p)).name.clone();
        add(&mut s.w, s.cp2, &format!("{name} footman"));
        let f = find(&s.w, s.cp2, "footman", 0).unwrap();
        (s, f)
    }

    #[test]
    fn computer_test_defensive_worker_must_flee() {
        let mut s = set_up(("1", "2"), false, "jl1", ("timers", "timers"));
        let p = find(&s.w, s.cp, "peasant", 0).unwrap();
        let name = s.w.square(place_of(&s.w, p)).name.clone();
        add(&mut s.w, s.cp2, &format!("{name} footman"));
        s.w.update();
        assert_eq!(s.w.e(p).orders[0].keyword, Keyword::Go);
    }

    #[test]
    fn computer_test_offensive_smart_unit_must_sometimes_attack() {
        let (mut s, f) = footman_beside_peasant("jl1", "easy");
        s.w.update();
        assert!(s.w.e(f).orders.is_empty());
        assert!(s.w.e(f).is_attacking());
    }

    #[test]
    fn computer_test_offensive_smart_unit_must_sometimes_flee() {
        let (mut s, f) = footman_beside_peasant("jl1_extended", "easy");
        s.w.update();
        assert_eq!(s.w.e(f).orders[0].keyword, Keyword::Go);
    }

    #[test]
    fn computer_test_offensive_not_smart_unit_must_always_attack() {
        let (mut s, f) = footman_beside_peasant("jl1_extended", "timers");
        s.w.update();
        assert!(s.w.e(f).orders.is_empty());
        assert!(s.w.e(f).is_attacking());
    }

    #[test]
    fn computer_test_offensive_not_smart_unit_must_always_attack2() {
        let mut s = set_up(("1", "2"), false, "jl1_extended", ("timers", "timers"));
        let w = &mut s.w;
        w.rules.unit_type_mut("footman").unwrap().stats.speed = 10 * PRECISION;
        let p = find(w, s.cp, "peasant", 0).unwrap();
        let pp = place_of(w, p);
        let n0 = w.square(pp).neighbors[0];
        let name = w.square(n0).name.clone();
        add(w, s.cp2, &format!("{name} footman"));
        let f = find(w, s.cp2, "footman", 0).unwrap();
        let go = format!("go {}", w.square(pp).id);
        take(w, f, &go);
        for _ in 0..100 {
            w.update();
            if place_of(w, f) == pp {
                break;
            }
        }
        assert_eq!(place_of(w, f), pp);
        for _ in 0..100 {
            w.update();
            if w.e(f).is_attacking() {
                break;
            }
        }
        assert_eq!(place_of(w, f), pp);
        assert!(w.e(f).is_attacking());
        for _ in 0..100 {
            w.update();
            if w.e(f).hp < w.e(f).stats.hp_max {
                break;
            }
        }
        assert!(w.e(f).hp < w.e(f).stats.hp_max);
    }

    #[test]
    fn computer_test_upgrade_to() {
        let mut s = set_up(("1", "2"), false, "jl1", ("easy", "easy"));
        let cp = s.cp;
        let w = &mut s.w;
        let th = find(w, cp, "townhall", 0).unwrap();
        let name = w.square(place_of(w, th)).name.clone();
        add(w, cp, &format!("{name} barracks"));
        assert_ne!(&*w.e(th).type_name, "keep");
        assert_eq!(&*w.e(th).type_name, "townhall");
        assert_eq!(nb(w, cp, "keep"), 0);
        assert_eq!(nb(w, cp, "barracks"), 1);
        w.plm(cp).resources = vec![100_000, 100_000];
        take(w, th, "upgrade_to keep");
        assert!(!w.e(th).orders.is_empty());
        assert!(!w.e(th).orders[0].is_impossible);
        for _ in 0..10000 {
            w.creature_update(th).unwrap();
            if w.e(th).orders.is_empty() {
                break;
            }
        }
        assert!(w.e(th).orders.is_empty());
        assert!(w.e(th).place.is_none());
        assert!(
            !w.player(cp).units.contains(&th),
            "townhall still belongs to the player"
        );
        assert_eq!(nb(w, cp, "keep"), 1);
        assert_eq!(nb(w, cp, "townhall"), 0);
        assert!(w.has(cp, "townhall"));
        let k = find(w, cp, "keep", 0).unwrap();
        assert_eq!(&*w.e(k).type_name, "keep");
    }

    #[test]
    fn computer_test_allied() {
        let mut s = set_up(("1", "1"), true, "jl1_extended", ("easy", "easy"));
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        let p2 = find(w, cp2, "peasant", 0).unwrap();
        let th = find(w, cp, "townhall", 0).unwrap();
        assert!(!w.is_an_enemy(p, &Target::Entity(p2)));
        assert!(!w.player_is_an_enemy(cp, cp2));
        assert!(is_perceiving(w, cp, p2));
        w.em(th).stats.heal_level = 1;
        w.em(p2).hp = 0;
        move_to(w, p2, place_of(w, p));
        w.update();
        assert!(w.e(p2).hp > 0);
        assert!(w.e(p).is_cloaked);
        assert!(w.e(p2).is_cloaked);
        assert!(w.lang(cp, "no_enemy_left", &[]).unwrap());
    }

    #[test]
    fn computer_test_allied2() {
        let mut s = set_up(("1", "2"), true, "jl1_extended", ("easy", "easy"));
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        let p2 = find(w, cp2, "peasant", 0).unwrap();
        let th = find(w, cp, "townhall", 0).unwrap();
        assert!(w.is_an_enemy(p, &Target::Entity(p2)));
        assert!(w.player_is_an_enemy(cp, cp2));
        assert!(!is_perceiving(w, cp, p2));
        w.em(p2).hp = 0;
        move_to(w, p2, place_of(w, p));
        w.creature_update(th).unwrap();
        w.entity_slow_update(th).unwrap();
        assert_eq!(w.e(p2).hp, 0);
        w.update();
        assert!(w.e(p).is_cloaked);
        assert!(!w.e(p2).is_cloaked);
        assert!(!w.lang(cp, "no_enemy_left", &[]).unwrap());
    }

    #[test]
    fn computer_test_allied_observer_after_defeat() {
        let mut s = set_up(("1", "1"), false, "jl1", ("easy", "easy"));
        let (cp, cp2) = (s.cp, s.cp2);
        let w = &mut s.w;
        w.plm(cp).observer_if_defeated = true;
        w.plm(cp2).observer_if_defeated = true;
        let first_object_of_a2 = w.square(sq(w, "a2")).objects[0];
        assert!(!is_perceiving(w, cp, first_object_of_a2));
        assert!(!is_perceiving(w, cp2, first_object_of_a2));
        w.defeat(cp2, false);
        assert!(!is_perceiving(w, cp, first_object_of_a2));
        assert!(!is_perceiving(w, cp2, first_object_of_a2));
        w.defeat(cp, false);
    }

    #[test]
    fn computer_test_ai_nearest_warehouse() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let th = find(w, cp, "townhall", 0).unwrap();
        let place = w.e(th).place.unwrap();
        assert_eq!(w.nearest_warehouse(cp, place, 0, false).unwrap(), Some(th));
        assert_eq!(w.nearest_warehouse(cp, place, 1, false).unwrap(), Some(th));
        let s = place_of(w, th);
        assert_eq!(
            w.shortest_path_distance_to(s, s, None, AirGround::Ground, false)
                .unwrap(),
            0
        );
    }

    fn imperative_go(attacker: &str, ai: &str, world_update: bool) {
        let mut s = set_up(("1", "2"), false, "jl1_extended", (ai, ai));
        let cp = s.cp;
        let w = &mut s.w;
        let th = find(w, cp, "townhall", 0).unwrap();
        let f = find(w, cp, attacker, 0).unwrap();
        w.take_order(f, words(&format!("go {th}")), true, true, None)
            .unwrap();
        assert_eq!(w.e(th).hp, w.e(th).stats.hp_max);
        for _ in 0..100 {
            if world_update {
                w.update();
            } else {
                w.creature_update(f).unwrap();
            }
            if w.e(th).hp != w.e(th).stats.hp_max {
                break;
            }
        }
        assert_ne!(w.e(th).hp, w.e(th).stats.hp_max);
    }

    #[test]
    fn computer_test_imperative_go() {
        imperative_go("peasant", "timers", true);
    }

    #[test]
    fn computer_test_imperative_go2() {
        imperative_go("flyingmachine", "easy", false);
    }

    #[test]
    fn computer_test_imperative_go3() {
        imperative_go("dragon", "timers", true);
    }

    #[test]
    fn computer_test_move_slowly() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        w.em(p).stats.speed /= 100;
        let (a1, a2) = (sq(w, "a1"), sq(w, "a2"));
        move_to(w, p, a2);
        let go = format!("go {}", w.square(a1).id);
        take(w, p, &go);
        let pos = |w: &World| (w.e(p).x, w.e(p).y);
        let start = pos(w);
        w.update();
        assert_eq!(start, pos(w));
        w.update();
        assert_ne!(start, pos(w));
        let second = pos(w);

        // a second time: the same result
        w.move_to(p, Some(Place::Square(a2)), Some(start), 90)
            .unwrap();
        assert_eq!(start, pos(w));
        w.set_action_target(p, None);
        take(w, p, &go);
        assert_eq!(start, pos(w));
        w.update();
        assert_eq!(start, pos(w));
        w.update();
        assert_ne!(start, pos(w));
        assert_eq!(second, pos(w));

        // without collision: the same result
        let (a, x, y) = (w.e(p).airground_type, w.e(p).x, w.e(p).y);
        w.collision_mut(a).remove(x, y);
        w.em(p).stats.collision = 0;
        w.move_to(p, Some(Place::Square(a2)), Some(start), 90)
            .unwrap();
        assert_eq!(start, pos(w));
        w.set_action_target(p, None);
        take(w, p, &go);
        assert_eq!(start, pos(w));
        w.update();
        assert_eq!(start, pos(w));
        w.update();
        assert_ne!(start, pos(w));
        assert_eq!(second, pos(w));
    }

    #[test]
    fn computer_test_die_to_air_transport() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        let f = find(w, cp, "new_flyingmachine", 0).unwrap();
        let pl = place_of(w, f);
        w.load(f, p);
        w.die(f, None).unwrap();
        let last = *w.square(pl).objects.last().unwrap();
        assert!(w.e(last).is_corpse());
        assert_eq!(
            &*w.e(last).corpse_unit.as_ref().unwrap().type_name,
            "peasant"
        );
        assert_ne!(w.e(p).place, Some(Place::Square(pl)));
    }

    #[test]
    fn computer_test_survive_to_ground_transport() {
        let mut s = default_set_up();
        let cp = s.cp;
        let w = &mut s.w;
        let p = find(w, cp, "peasant", 0).unwrap();
        let f = find(w, cp, "new_flyingmachine", 0).unwrap();
        let pl = place_of(w, f);
        w.em(f).airground_type = AirGround::Ground;
        w.load(f, p);
        w.die(f, None).unwrap();
        let last = *w.square(pl).objects.last().unwrap();
        assert!(!w.e(last).is_corpse());
        assert_eq!(w.e(p).place, Some(Place::Square(pl)));
    }
}

mod test_worldplayercomputer {
    use super::*;

    #[test]
    fn test_is_ok_for_warehouse() {
        let mut w = world_text(
            "square_width 12\nnb_columns 1\nnb_lines 1\nnb_meadows_by_square 0\n\
             nb_players_min 1\nnb_players_max 1\nstarting_squares a1\n",
        );
        let a1 = sq(&w, "a1");
        assert_eq!(w.square(a1).name, "a1");
        assert!(w.square(a1).objects.is_empty());
        let owner = human(&mut w);
        assert!(!w.is_ok_for_warehouse(a1, 0));
        let gold = w.rules.type_id("goldmine").unwrap();
        w.create_deposit(gold, a1, "75").unwrap();
        assert!(w.is_ok_for_warehouse(a1, 0));
        create(&mut w, "lumbermill", owner, a1); // stores resource 1
        assert!(w.is_ok_for_warehouse(a1, 0));
        create(&mut w, "townhall", owner, a1); // stores resource 0
        assert!(!w.is_ok_for_warehouse(a1, 0));
    }
}

mod test_autorepair {
    use super::*;

    fn setup(
        square: &str,
        resources: [i64; 2],
        other: &str,
        wounded: bool,
    ) -> (World, EntityId, EntityId) {
        let mut w = world("soundrts/tests/jl1_cyclic.txt");
        let place = sq(&w, square);
        let player = human(&mut w);
        let p = create(&mut w, "peasant", player, place);
        let o = create(&mut w, other, player, place);
        if wounded {
            w.em(o).hp /= 2;
        }
        w.plm(player).resources = resources.to_vec();
        (w, p, o)
    }

    #[test]
    fn test_decide_repair() {
        let (mut w, p, _) = setup("a2", [1000, 1000], "townhall", true);
        w.decide(p).unwrap();
        assert_eq!(w.e(p).orders[0].keyword, Keyword::Repair);
    }

    #[test]
    fn test_decide_no_repair_if_no_resource() {
        let (mut w, p, _) = setup("a2", [0, 0], "townhall", true);
        w.decide(p).unwrap();
        assert!(w.e(p).orders.is_empty());
    }

    #[test]
    fn test_decide_gather_if_no_repair() {
        let (mut w, p, th) = setup("a1", [1000, 1000], "townhall", false);
        w.decide(p).unwrap();
        assert_eq!(w.e(p).orders[0].keyword, Keyword::Gather);
        w.em(th).hp /= 2;
        w.decide(p).unwrap();
        assert_eq!(w.e(p).orders[0].keyword, Keyword::Repair);
    }

    #[test]
    fn test_repair_unit() {
        let (mut w, p, c) = setup("a2", [10000, 10000], "catapult", true);
        w.decide(p).unwrap();
        assert_eq!(w.e(p).orders[0].keyword, Keyword::Repair);
        assert!(w.e(p).orders[0].target.is_some());
        for _ in 0..75 {
            w.creature_update(p).unwrap();
        }
        assert_eq!(w.e(c).hp, w.e(c).stats.hp_max);
    }

    #[test]
    fn test_high_ground_hit_chance() {
        let mut w = world("soundrts/tests/jl1_cyclic.txt");
        let place = sq(&w, "a1");
        let player = human(&mut w);
        let a = create(&mut w, "archer", player, place);
        let place2 = sq(&w, "a2");
        let player2 = human(&mut w);
        let a2 = create(&mut w, "archer", player2, place2);
        assert_eq!(w.chance_to_hit(a, a2), Some(100));
        assert_eq!((0..100).filter(|_| w.has_hit(a, a2).unwrap()).count(), 100);
        w.sqm(place2).high_ground = true;
        assert_eq!(w.chance_to_hit(a, a2), Some(50));
        let hits = (0..1000).filter(|_| w.has_hit(a, a2).unwrap()).count();
        assert!(450 < hits && hits < 550, "{hits}");
        let f = create(&mut w, "footman", player, place);
        assert_eq!(w.chance_to_hit(f, a2), Some(100));
    }
}

mod test_cyclic_map {
    use super::*;
    use crate::pathfinding::Node;

    fn other_side_square(w: &World, n: Option<Node>) -> SquareId {
        let Some(Node::Exit(e)) = n else {
            panic!("not an exit")
        };
        let os = w.exit_other_side(e).unwrap();
        w.e(os).square().unwrap()
    }

    #[test]
    fn test_shortest_path() {
        let mut w = world("soundrts/tests/jl1_cyclic.txt");
        for (a, b) in [("b4", "b1"), ("b1", "b4"), ("c2", "a2"), ("a2", "c2")] {
            let (sa, sb) = (sq(&w, a), sq(&w, b));
            let n = w
                .shortest_path_to(sa, sb, None, AirGround::Ground, false)
                .unwrap();
            assert_eq!(other_side_square(&w, n), sb, "{a} -> {b}");
        }
    }

    #[test]
    fn test_unit_move() {
        let mut w = world("soundrts/tests/jl1_cyclic.txt");
        let player = human(&mut w);
        let (b4, b1) = (sq(&w, "b4"), sq(&w, "b1"));
        let u = create(&mut w, "peasant", player, b4);
        w.em(u).o = 0;
        w.start_moving_to(u, Some(&Target::Square(b1)), false)
            .unwrap();
        let speed = w.e(u).stats.speed;
        w.em(u).actual_speed = speed;
        w.action_update(u).unwrap();
        assert_eq!(w.e(u).o, 90);
        for _ in 0..12 {
            w.action_update(u).unwrap();
        }
        assert_eq!(w.e(u).place, Some(Place::Square(b1)));
    }
}

mod test_world {
    use super::*;

    fn name_after(w: &mut World, a: &str, b: &str) -> Option<String> {
        let (sa, sb) = (sq(w, a), sq(w, b));
        let n = w
            .shortest_path_to(sa, sb, None, AirGround::Ground, false)
            .unwrap()?;
        let crate::pathfinding::Node::Exit(e) = n else {
            return None;
        };
        let os = w.exit_other_side(e)?;
        Some(w.square(w.e(os).square()?).name.clone())
    }

    #[test]
    fn test_shortest_path() {
        let mut w = world("soundrts/tests/jl1.txt");
        let d = |w: &mut World, a: &str, b: &str| {
            let (sa, sb) = (sq(w, a), sq(w, b));
            w.shortest_path_distance_to(sa, sb, None, AirGround::Ground, false)
                .unwrap()
        };
        assert_eq!(d(&mut w, "a1", "a2"), d(&mut w, "a1", "b1"));
        assert_eq!(name_after(&mut w, "a1", "a4").as_deref(), Some("a2"));
        assert_eq!(name_after(&mut w, "b1", "c2").as_deref(), Some("c1"));
        assert_eq!(name_after(&mut w, "b1", "a3").as_deref(), Some("a1"));
        let mut w2 = world("soundrts/tests/jl2.txt");
        for (a, b) in [("a1", "c1"), ("c1", "a1")] {
            let (sa, sb) = (sq(&w2, a), sq(&w2, b));
            assert_eq!(
                w2.shortest_path_to(sa, sb, None, AirGround::Ground, false)
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn test_check_string() {
        let empty = World::new(Rules::default(), AiScripts::default(), Vec::new(), 0);
        empty.get_digest();
        let w = world("soundrts/tests/jl1.txt");
        let w2 = world("soundrts/tests/jl2.txt");
        assert_eq!(w.get_digest(), w.get_digest());
        assert_ne!(w.get_digest(), w2.get_digest());
    }
}

mod test_order {
    use super::*;

    const TINY_MAP: &str = "
square_width 12
nb_columns 2
nb_lines 1
nb_meadows_by_square 9
west_east_paths a1

nb_players_min 1
nb_players_max 1

player 10 10 a1 guardtower b1 peasant
";

    #[test]
    fn test_enter_building_from_another_square() {
        let mut w = world_text(TINY_MAP);
        let p = w
            .populate_map(&[Controller::computer("timers")], true, false)
            .unwrap()[0];
        let units = w.player(p).units.clone();
        let (tower, unit) = (units[0], units[1]);
        w.take_default_order(unit, &tower.to_string(), true, false, None)
            .unwrap();
        assert!(!w.e(unit).orders.is_empty());
        assert!(!w.e(tower).orders.is_empty());
        w.update();
        assert!(!w.e(unit).orders[0].is_complete);
        assert!(w.e(tower).orders[0].is_complete);
        for _ in 0..100 {
            w.update();
            if w.e(unit).is_inside() {
                break;
            }
        }
        assert!(w.e(unit).is_inside());
    }
}

mod test_perception {
    use super::*;

    fn setup() -> (World, EntityId) {
        let mut w = world("soundrts/tests/height.txt");
        let player = human(&mut w);
        let b1 = sq(&w, "b1");
        let p = create(&mut w, "peasant", player, b1);
        (w, p)
    }

    #[test]
    fn test_must_not_see_plateau_from_below_even_if_path_exists() {
        let (mut w, p) = setup();
        let a1 = sq(&w, "a1");
        assert!(!w.get_observed_squares(p, false).unwrap().contains(&a1));
    }

    #[test]
    fn test_must_see_diagonal_if_path_exists() {
        let (mut w, p) = setup();
        let a2 = sq(&w, "a2");
        assert!(w.get_observed_squares(p, false).unwrap().contains(&a2));
    }
}

mod test_definitions {
    use crate::rules::Rules;

    #[test]
    fn test_load_rules() {
        // The original loads a class on Python's `int`; the rules classes
        // here are the game's base classes.
        let rules = Rules::load(&["def number\nclass soldier\ndef other\nclass int"]);
        assert!(rules.unit_type("number").is_some());
        assert!(rules.unit_type("other").is_none());
        assert!(rules.classnames().contains(&"other"));
    }

    #[test]
    fn test_copy_rules() {
        let rules = Rules::load(&["def number\nclass soldier"]);
        let copy = rules.clone();
        assert!(copy.unit_type("number").is_some());
    }
}
