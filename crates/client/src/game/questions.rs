//! The questions a player asks to learn what a sighted player reads off
//! the edge of the screen: what is in production, what the workers do,
//! the other players and their factions, every resource, how the fight on
//! a square goes, and how a unit fights.
//! Each is answered in one sentence, the most important word first, from
//! what the player knows.

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::nofloat::py_int;
use soundrts_world::orders::Keyword;
use soundrts_world::rules::Stats;
use soundrts_world::{KnownEntity, KnownTarget, PlayerId, SeatView};

use crate::client::Client;
use crate::net::words::{self, text};

use super::command::UnitFilter;
use super::order::{PRECISION, substitute_args};
use super::selection::TypeFilter;
use super::squares::summary;
use super::{GameInterface, compute_title, style_msg};

/// How the strength on the player's side of a fight compares with the
/// enemies'.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Balance {
    Stronger,
    Even,
    Weaker,
}

impl Balance {
    /// Twice as strong or more is stronger, as the simulation makes a
    /// unit flee from twice its side's strength.
    fn of(ours: i64, theirs: i64) -> Balance {
        if ours < theirs.saturating_mul(2) && theirs < ours.saturating_mul(2) {
            Balance::Even
        } else if ours > theirs {
            Balance::Stronger
        } else {
            Balance::Weaker
        }
    }

    fn word(self) -> &'static str {
        match self {
            Balance::Stronger => words::STRONGER,
            Balance::Even => words::EVEN,
            Balance::Weaker => words::WEAKER,
        }
    }
}

impl GameInterface {
    /// What each unit the player controls trains, researches or upgrades
    /// to now, soonest done first, with the seconds left and where: "recruit
    /// peasant 7 seconds at town hall 1". Production waiting for food
    /// comes last.
    pub(crate) fn cmd_production_status(&self, cx: &mut Client) {
        let style = cx.res.style();
        let mut productions: Vec<((bool, i64), Msg)> = Vec::new();
        for id in self.units(cx, true, false) {
            let v = &self.dobjets[&id];
            let Some((o, left)) = v.orders().first().and_then(|o| Some((o, o.time_left?))) else {
                continue;
            };
            let mut m = style_msg(style, o.keyword.as_str(), "title");
            if let Some(t) = &o.type_name {
                m = substitute_args(m, &style_msg(style, t, "title"));
            }
            if o.is_deferred {
                m.extend(style_msg(style, "messages", "production_deferred"));
            } else {
                m.extend(nb2msg((left + 999) / 1000));
                m.extend(msg(mp::SECONDS));
            }
            m.extend(msg(mp::AT));
            m.extend(self.title(cx, v));
            productions.push(((o.is_deferred, left), m));
        }
        productions.sort_by_key(|(k, _)| *k);
        let m = join(productions.into_iter().map(|(_, m)| m));
        cx.voice.item(&m);
    }

    /// How many of the workers the player controls are idle, gather each
    /// resource and build: "1 idle , 2 on gold , 1 on wood".
    pub(crate) fn cmd_workers_status(&self, cx: &mut Client) {
        let filter = UnitFilter {
            keyboard_types: vec!["worker".into()],
            ..UnitFilter::default()
        };
        let TypeFilter::These(types) = self.arrange(cx, &filter) else {
            unreachable!("a filter with types")
        };
        let view = self.view();
        let workers: Vec<_> = self
            .units(cx, true, false)
            .into_iter()
            .map(|id| &self.dobjets[&id])
            .filter(|v| types.iter().any(|t| t == v.type_name()))
            .collect();
        if workers.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            return;
        }
        let mut idle = 0;
        let mut building = 0;
        let mut gathering = vec![0; view.resources().len()];
        for v in workers {
            let Some(o) = v.orders().first() else {
                idle += 1;
                continue;
            };
            let target = match &o.target {
                Some(KnownTarget::Entity(id)) => view.known(*id),
                Some(KnownTarget::Remembered(e)) => Some((**e).clone()),
                _ => None,
            };
            match o.keyword {
                Keyword::Build | Keyword::BuildPhaseTwo => building += 1,
                // Sent to help build a site.
                Keyword::Repair if target.as_ref().is_some_and(|t| t.is_building_site()) => {
                    building += 1
                }
                Keyword::Gather => {
                    // A worker bringing back its last load from a deposit
                    // now exhausted gathers what it carries.
                    let cargo = v.model.own.as_ref().and_then(|o| o.cargo).map(|c| c.0);
                    let r = target.and_then(|e| self.resource_type(&e)).or(cargo);
                    if let Some(n) = r.and_then(|r| gathering.get_mut(usize::try_from(r).ok()?)) {
                        *n += 1;
                    }
                }
                _ => {}
            }
        }
        let mut m = [nb2msg(idle), text(words::IDLE)].concat();
        for (i, &n) in gathering.iter().enumerate() {
            if n > 0 {
                m.extend(msg(mp::COMMA));
                m.extend(nb2msg(n));
                m.extend(msg(words::ON2));
                m.extend(style_msg(
                    cx.res.style(),
                    "parameters",
                    &format!("resource_{i}_title"),
                ));
            }
        }
        if building > 0 {
            m.extend(msg(mp::COMMA));
            m.extend(nb2msg(building));
            m.extend(msg(words::BUILDING));
        }
        cx.voice.item(&m);
    }

    /// The other players, each an ally or an enemy, and those defeated:
    /// the players in the game, then the defeated who left it.
    pub(crate) fn cmd_players_status(&self, cx: &mut Client) {
        let view = self.view();
        let defeated = view.defeated();
        let gone = defeated.iter().filter(|p| !view.players().contains(p));
        let mut players = Vec::new();
        for &p in view.players().iter().chain(gone) {
            if p == self.me {
                continue;
            }
            let mut m = view.player_name(p);
            m.extend(msg(if view.is_ally(p) { mp::ALLY } else { mp::ENEMY }));
            if defeated.contains(&p) {
                m.extend(msg(mp::HAS_BEEN_DEFEATED));
            }
            players.push(m);
        }
        cx.voice.item(&join(players.into_iter()));
    }

    /// Whether the rules of the game have more than one faction, so that
    /// the players' factions are worth saying.
    pub(crate) fn has_factions(&self) -> bool {
        self.view().game_start().rules.factions().len() > 1
    }

    /// The title of a player's faction: "orc". Its name in the rules if
    /// the style gives it no title; nothing for a player without one.
    pub(crate) fn faction_title(&self, cx: &Client, p: PlayerId) -> Msg {
        let factions = &self.view().game_start().player_factions;
        let Some(faction) = factions.get(p.0 as usize) else {
            return Msg::new();
        };
        let title = compute_title(cx.res.style(), faction);
        if title.is_empty() {
            vec![MsgPart::Literal(faction.clone())]
        } else {
            title
        }
    }

    /// Every resource the rules define: "120 gold , 80 wood , 10 stone".
    pub(crate) fn cmd_all_resources_status(&self, cx: &mut Client) {
        let style = cx.res.style();
        let resources = self.resources().into_iter().enumerate().map(|(i, n)| {
            [
                nb2msg(n),
                style_msg(style, "parameters", &format!("resource_{i}_title")),
            ]
            .concat()
        });
        cx.voice.item(&join(resources));
    }

    /// The units that can fight on the selected square (or in the zoom)
    /// that the player perceives now, the player's and its allies' against
    /// the enemies', after how the sides compare: "stronger , 3 footman
    /// against 1 archer". The strength of a side is its units' threat, as
    /// the simulation weighs it, from the rules: upgrades are left out.
    pub(crate) fn cmd_fight_status(&self, cx: &mut Client) {
        let view = self.view();
        let (mut ours, mut theirs) = (Vec::new(), Vec::new());
        let (mut our_threat, mut their_threat) = (0i64, 0i64);
        for v in self.dobjets.values() {
            if v.is_memory() || !self.is_in(v, self.place) {
                continue;
            }
            if self
                .zoom
                .as_ref()
                .is_some_and(|z| !z.contains(v.model.x, v.model.y))
            {
                continue;
            }
            let Some(p) = v.player() else { continue };
            let threat = threat(view, &v.model);
            let title = self.short_title(cx, v);
            if threat == 0 || title.is_empty() {
                continue;
            }
            if view.is_ally(p) {
                ours.push(title);
                our_threat = our_threat.saturating_add(threat);
            } else {
                theirs.push(title);
                their_threat = their_threat.saturating_add(threat);
            }
        }
        if ours.is_empty() && theirs.is_empty() {
            cx.voice.item(&msg(mp::NOTHING));
            return;
        }
        let side = |titles: &[Msg]| {
            if titles.is_empty() {
                msg(mp::NOTHING)
            } else {
                summary(titles, false)
            }
        };
        let m = [
            text(Balance::of(our_threat, their_threat).word()),
            msg(mp::COMMA),
            side(&ours),
            msg(words::AGAINST),
            side(&theirs),
        ]
        .concat();
        cx.voice.item(&m);
    }

    /// How the first unit the player controls fights, by the rules of its
    /// type and the upgrades that apply to it: "footman , damage 6 every
    /// 1.5 seconds , range 1 , armor 1.5 . 15 hit points , speed 1.5".
    pub(crate) fn cmd_combat_status(&self, cx: &mut Client) {
        let view = self.view();
        let unit = self.group.first().and_then(|&u| view.perceived(u));
        let Some((e, s)) = unit.and_then(|e| combat_stats(view, &e).map(|s| (e, s))) else {
            cx.voice.item(&msg(mp::NOTHING));
            return;
        };
        let mut m = compute_title(cx.res.style(), &e.type_name);
        m.extend(msg(mp::COMMA));
        if s.damage > 0 {
            m.extend(text(words::DAMAGE));
            m.extend(decimal(s.damage));
            m.extend(text(words::EVERY));
            m.extend(decimal(s.cooldown));
            m.extend(msg(mp::SECONDS));
            m.extend(msg(mp::COMMA));
            m.extend(text(words::RANGE));
            m.extend(decimal(s.range));
        } else {
            m.extend(text(words::NO_ATTACK));
        }
        m.extend(msg(mp::COMMA));
        m.extend(text(words::ARMOR));
        m.extend(decimal(s.armor));
        m.extend(msg(mp::PERIOD));
        m.extend(nb2msg(e.hp_max / PRECISION));
        m.extend(text(words::HIT_POINTS));
        if s.speed > 0 {
            m.extend(msg(mp::COMMA));
            m.extend(msg(mp::SPEED));
            m.extend(decimal(s.speed));
        }
        cx.voice.item(&m);
    }
}

/// The statistics of an entity by the rules of its type, with the effects
/// of the upgrades that apply to it, as the simulation applies them to a
/// unit: an upgrade's `bonus` adds its value, `apply_bonus` adds the type's
/// own bonus for the statistic. Each upgrade counts once, as researched
/// once. `None` for an entity without a type.
fn combat_stats(view: &SeatView, e: &KnownEntity) -> Option<Stats> {
    let t = view.unit_type(&e.type_name)?;
    let mut stats = t.stats.clone();
    for u in &e.upgrades {
        let Some(effect) = view.unit_type(u).and_then(|u| u.effect.as_deref()) else {
            continue;
        };
        let (stat, value) = match effect {
            [kind, stat, base, ..] if kind == "bonus" => (stat, py_int(base).unwrap_or(0)),
            [kind, stat, ..] if kind == "apply_bonus" => (stat, t.bonus(stat)),
            _ => continue,
        };
        if let Some(v) = stats.get_mut(stat) {
            *v = v.saturating_add(value);
        }
    }
    Some(stats)
}

/// A value in thousandths as a number said: 1500 is "1.5". A whole
/// number is said as the other numbers are.
fn decimal(thousandths: i64) -> Msg {
    if thousandths % PRECISION == 0 {
        nb2msg(thousandths / PRECISION)
    } else {
        vec![MsgPart::Text(
            (thousandths as f64 / PRECISION as f64).to_string(),
        )]
    }
}

/// The danger a creature represents by the rules of its type, as the
/// simulation's menace weighs it: its damage, or for a transport, what it
/// can carry.
fn threat(view: &SeatView, e: &KnownEntity) -> i64 {
    let Some(s) = view.unit_type(&e.type_name).map(|t| &t.stats) else {
        return 0;
    };
    if !e.is_creature() {
        0
    } else if s.damage != 0 {
        s.damage
    } else {
        s.transport_capacity * PRECISION * 2
    }
}

/// The messages separated by commas; "nothing!" without any.
fn join(messages: impl Iterator<Item = Msg>) -> Msg {
    let mut m = Vec::new();
    for x in messages {
        if !m.is_empty() {
            m.extend(msg(mp::COMMA));
        }
        m.extend(x);
    }
    if m.is_empty() { msg(mp::NOTHING) } else { m }
}

#[cfg(test)]
mod tests {
    use soundrts_world::{Cheat, Command, EntityId, PlayerId, SquareId};

    use super::super::testing::{cheat, enemy, run_until, training, training_allied, unit};
    use super::Balance;
    use crate::clock::Clock;
    use crate::input::{Key, Mods};
    use crate::testkit::{Harness, LiveKeys, Script, TestGame};

    /// The square of the player's town hall, and its name.
    fn home(g: &TestGame, p: PlayerId) -> (SquareId, String) {
        let w = g.game.host().world();
        let square = w.entity(unit(g, p, "townhall")).unwrap().square().unwrap();
        (square, g.game.view().square(square).name().to_owned())
    }

    /// The units of `p` of this type, in the whole world.
    fn units(g: &TestGame, p: PlayerId, type_name: &str) -> Vec<EntityId> {
        let w = g.game.host().world();
        w.player(p)
            .units
            .iter()
            .copied()
            .filter(|&u| w.entity(u).is_some_and(|e| &*e.type_name == type_name))
            .collect()
    }

    /// Gives `unit` of `p` an order, as the interface sends it.
    fn order(g: &mut TestGame, p: PlayerId, unit: EntityId, order: &str) {
        g.game
            .host_mut()
            .queue_line(p, &format!("control {}", unit.0));
        g.game
            .host_mut()
            .queue_line(p, &format!("order 0 0 {order}"));
    }

    /// What a key answers: the first thing said after it, before the news
    /// that follows.
    fn ask(h: &mut Harness, g: &mut TestGame, key: Key, mods: Mods) -> String {
        h.key(g, key, mods).remove(0)
    }

    fn run_turns(h: &mut Harness, g: &mut TestGame, turns: i64) {
        let turn = g.game.view().turn() + turns;
        h.run_until_turn(g, turn);
    }

    #[test]
    fn production_is_said_for_every_building_soonest_done_first() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let (_, name) = home(&g, me);
        g.game.cheat_at_once(Cheat::Resources);
        cheat(&mut g, me, &format!("add_units {name} barracks"));
        run_turns(&mut h, &mut g, 1);
        let barracks = unit(&g, me, "barracks");
        order(&mut g, me, barracks, "train footman");
        run_turns(&mut h, &mut g, 10);
        let townhall = unit(&g, me, "townhall");
        order(&mut g, me, townhall, "train peasant");
        run_turns(&mut h, &mut g, 2);
        assert_eq!(
            ask(&mut h, &mut g, Key::B, Mods::NONE),
            "recruit footman 9 seconds at barracks 1 , recruit peasant 10 seconds at town hall 1"
        );
    }

    #[test]
    fn nothing_in_production_is_said_as_nothing() {
        let (mut h, mut g) = training();
        assert_eq!(ask(&mut h, &mut g, Key::B, Mods::NONE), "nothing!");
    }

    #[test]
    fn the_workers_are_counted_idle_on_each_resource_and_building() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let (square, name) = home(&g, me);
        // Cheat mode shows the whole map: the deposits are known.
        let _ = g
            .game
            .host_mut()
            .queue(me, Command::Cheat(Cheat::ToggleMode));
        g.game.cheat_at_once(Cheat::Resources);
        cheat(&mut g, me, &format!("add_units {name} 5 peasant"));
        run_turns(&mut h, &mut g, 1);
        let find = |g: &TestGame, type_name: &str| {
            let w = g.game.host().world();
            w.entities()
                .find(|e| &*e.type_name == type_name)
                .map(|e| e.id)
        };
        let (gold, wood) = (find(&g, "goldmine").unwrap(), find(&g, "wood").unwrap());
        let peasants = units(&g, me, "peasant");
        assert_eq!(peasants.len(), 6);
        order(&mut g, me, peasants[0], &format!("gather {}", gold.0));
        order(&mut g, me, peasants[1], &format!("gather {}", gold.0));
        order(&mut g, me, peasants[2], &format!("gather {}", wood.0));
        let to = g.game.view().square(square).protocol_id();
        order(&mut g, me, peasants[3], &format!("build farm {to}"));
        run_until(&mut h, &mut g, |game| {
            game.host()
                .world()
                .entities()
                .any(|e| &*e.type_name == "buildingsite")
        });
        // Sent to help build: a repair of the site.
        let site = find(&g, "buildingsite").unwrap();
        order(&mut g, me, peasants[4], &format!("repair {}", site.0));
        run_turns(&mut h, &mut g, 2);
        assert_eq!(
            ask(&mut h, &mut g, Key::N, Mods::NONE),
            "1 idle , 2 on gold , 1 on wood , 2 building"
        );
    }

    #[test]
    fn the_health_of_a_group_is_said_as_a_whole() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let (_, name) = home(&g, me);
        cheat(&mut g, me, &format!("add_units {name} 2 footman 1 archer"));
        cheat(&mut g, them, &format!("add_units {name} 3 footman"));
        run_turns(&mut h, &mut g, 1);
        let mine: Vec<EntityId> = [units(&g, me, "footman"), units(&g, me, "archer")].concat();
        g.game.group = mine.clone();
        run_until(&mut h, &mut g, |game| {
            mine.iter().any(|u| {
                game.view()
                    .perceived(*u)
                    .is_some_and(|e| e.hp * 2 < e.hp_max)
            })
        });
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::NONE),
            "22 hit points on 40 , 1 badly hurt , 2 footman and 1 archer"
        );
    }

    #[test]
    fn the_other_players_are_said_ally_or_enemy_and_defeated() {
        let (mut h, mut g) = training_allied("jl2", &["easy"; 3], &["1", "1", "2", "2"]);
        let them = g.game.view().players()[3];
        let _ = g.game.host_mut().queue(them, Command::Quit);
        run_turns(&mut h, &mut g, 2);
        assert_eq!(
            ask(&mut h, &mut g, Key::F12, Mods::NONE),
            "quiet computer allied , quiet computer enemy , quiet computer enemy has been defeated"
        );
    }

    #[test]
    fn every_resource_of_the_rules_is_said() {
        // jl4 adds stone. The keys are pressed as a player presses them:
        // Shift down, X, Shift up.
        let live = LiveKeys::default();
        let mut h = Harness::new("en", Script::new().keep_open().with_live(live.clone()));
        let mut g = h.start_training("jl4", &["easy"], 1);
        h.run_until_turn(&mut g, 3);
        h.run_until_voice_idle(&mut g);
        let (from, t) = (h.utterances().len(), h.clock.now());
        live.down_at(t, Key::LShift, Mods::SHIFT);
        live.tap_at(t + 0.1, Key::X, Mods::SHIFT);
        live.up_at(t + 0.2, Key::LShift);
        h.run_for(&mut g, 0.3);
        assert_eq!(h.said_since(from)[0], "10 gold , 10 wood , 0 stone");
    }

    #[test]
    fn the_fight_here_compares_the_sides_the_player_perceives() {
        let (mut h, mut g) = training();
        let (me, them) = (g.game.me, enemy(&g));
        let (square, name) = home(&g, me);
        let (theirs, their_name) = home(&g, them);
        cheat(&mut g, me, &format!("add_units {name} 3 footman"));
        cheat(&mut g, them, &format!("add_units {name} 1 archer"));
        cheat(&mut g, them, &format!("add_units {their_name} 1 footman"));
        run_turns(&mut h, &mut g, 1);
        assert_eq!(g.game.place, Some(square));
        assert_eq!(
            ask(&mut h, &mut g, Key::M, Mods::NONE),
            "stronger , 1 peasant and 3 footman against 1 archer"
        );
        // The computer's home, where the player sees nothing: its footman
        // there is not counted.
        assert!(g.game.view().perceived(unit(&g, them, "footman")).is_none());
        g.game.place = Some(theirs);
        assert_eq!(ask(&mut h, &mut g, Key::M, Mods::NONE), "nothing!");
    }

    #[test]
    fn how_the_controlled_unit_fights_counts_its_upgrades() {
        let (mut h, mut g) = training();
        let me = g.game.me;
        let (_, name) = home(&g, me);
        cheat(&mut g, me, &format!("add_units {name} 1 footman 1 archer"));
        run_turns(&mut h, &mut g, 1);
        let (footman, archer) = (unit(&g, me, "footman"), unit(&g, me, "archer"));
        assert_eq!(ask(&mut h, &mut g, Key::V, Mods::SHIFT), "nothing!");
        g.game.group = vec![footman];
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::SHIFT),
            "footman , damage 3.5 every 1.5 seconds , range 1 , armor 0.5 . 15 hit points , speed 1.5"
        );
        g.game.group = vec![archer, footman];
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::SHIFT),
            "archer , damage 2.5 every 1.5 seconds , range 4 , armor 0 . 10 hit points , speed 1.5"
        );
        // The upgrades, then new units, which have them: the footman's
        // weapon and armor add the bonuses of its type, the archer's add
        // their own values, as the simulation does.
        cheat(
            &mut g,
            me,
            &format!(
                "add_units {name} melee_weapon melee_armor archer_weapon archer_armor archer_range"
            ),
        );
        run_turns(&mut h, &mut g, 1);
        cheat(&mut g, me, &format!("add_units {name} 1 footman 1 archer"));
        run_turns(&mut h, &mut g, 1);
        let newest = |g: &TestGame, t: &str| *units(g, me, t).last().unwrap();
        let (footman, archer) = (newest(&g, "footman"), newest(&g, "archer"));
        g.game.group = vec![archer];
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::SHIFT),
            "archer , damage 4 every 1.5 seconds , range 5 , armor 0.5 . 10 hit points , speed 1.5"
        );
        g.game.group = vec![footman];
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::SHIFT),
            "footman , damage 6 every 1.5 seconds , range 1 , armor 1.5 . 15 hit points , speed 1.5"
        );
        let w = g.game.host().world();
        let stats = |u: EntityId| {
            let s = &w.entity(u).unwrap().stats;
            (s.damage, s.range, s.armor)
        };
        assert_eq!(stats(archer), (4000, 5000, 500));
        assert_eq!(stats(footman), (6000, 1000, 1500));
        // A building that does not attack.
        g.game.group = vec![unit(&g, me, "townhall")];
        assert_eq!(
            ask(&mut h, &mut g, Key::V, Mods::SHIFT),
            "town hall , no attack , armor 0 . 400 hit points"
        );
    }

    #[test]
    fn the_factions_are_said_when_the_rules_have_more_than_one() {
        // The original's rules have one faction: the list is the names.
        let (mut h, mut g) = training();
        assert_eq!(
            ask(&mut h, &mut g, Key::F11, Mods::NONE),
            "player , ai_quiet ,"
        );
        assert_eq!(ask(&mut h, &mut g, Key::F11, Mods::SHIFT), "human");
        // The orc mod adds a faction; each random choice was drawn.
        let mut h = Harness::new("en", Script::new().keep_open());
        h.client.res.set_mods("orc");
        let mut g = h.start_training("jl1", &["quiet"], 1);
        h.run_until_turn(&mut g, 3);
        let factions: Vec<String> = g
            .game
            .view()
            .players()
            .iter()
            .map(|&p| g.game.host().world().faction_name(p))
            .collect();
        assert_eq!(factions, ["orc_faction", "human_faction"]);
        assert_eq!(
            ask(&mut h, &mut g, Key::F11, Mods::NONE),
            "player orc , ai_quiet human ,"
        );
        assert_eq!(ask(&mut h, &mut g, Key::F11, Mods::SHIFT), "orc");
    }

    #[test]
    fn a_side_twice_as_strong_or_more_is_stronger() {
        assert_eq!(Balance::of(2, 1), Balance::Stronger);
        assert_eq!(Balance::of(3, 2), Balance::Even);
        assert_eq!(Balance::of(1, 1), Balance::Even);
        assert_eq!(Balance::of(2, 3), Balance::Even);
        assert_eq!(Balance::of(1, 2), Balance::Weaker);
        assert_eq!(Balance::of(1, 0), Balance::Stronger);
        assert_eq!(Balance::of(0, 1), Balance::Weaker);
    }
}
