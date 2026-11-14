//! The questions a player asks to learn what a sighted player reads off
//! the edge of the screen: what is in production, what the workers do,
//! the other players, every resource, and how the fight on a square goes.
//! Each is answered in one sentence, the most important word first, from
//! what the player knows.

use soundrts_world::KnownTarget;
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;
use soundrts_world::orders::Keyword;

use crate::client::Client;
use crate::net::words::{self, text};

use super::command::UnitFilter;
use super::order::substitute_args;
use super::selection::TypeFilter;
use super::squares::summary;
use super::{GameInterface, R, style_msg};

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
    pub(crate) fn cmd_production_status(&self, cx: &mut Client) -> R {
        let style = cx.res.style();
        let view = self.view();
        let mut productions: Vec<((bool, i64), Msg)> = Vec::new();
        for id in self.units(cx, true, false) {
            let v = &self.dobjets[&id];
            let (Some(left), Some(o)) = (view.production_time_left(id), v.orders().first()) else {
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
            m.extend(self.title(cx, v)?);
            productions.push(((o.is_deferred, left), m));
        }
        productions.sort_by_key(|(k, _)| *k);
        let m = join(productions.into_iter().map(|(_, m)| m));
        cx.voice.item(&m);
        Ok(())
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
            match o.keyword {
                Keyword::Build | Keyword::BuildPhaseTwo => building += 1,
                Keyword::Gather => {
                    let deposit = match &o.target {
                        Some(KnownTarget::Entity(id)) => view.known(*id),
                        Some(KnownTarget::Remembered(e)) => Some((**e).clone()),
                        _ => None,
                    };
                    let r = deposit.and_then(|e| self.resource_type(&e));
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
    /// the ones still watching, then the ones who left. A player who quit
    /// undefeated is not said, as F11 does not say them.
    pub(crate) fn cmd_players_status(&self, cx: &mut Client) {
        let view = self.view();
        let defeated_and_gone = view
            .former_players()
            .iter()
            .filter(|&&p| view.is_defeated(p));
        let mut players = Vec::new();
        for &p in view.players().iter().chain(defeated_and_gone) {
            if p == self.me {
                continue;
            }
            let mut m = view.player_name(p);
            m.extend(msg(if view.is_ally(p) { mp::ALLY } else { mp::ENEMY }));
            if view.is_defeated(p) {
                m.extend(msg(mp::HAS_BEEN_DEFEATED));
            }
            players.push(m);
        }
        cx.voice.item(&join(players.into_iter()));
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
    /// the simulation weighs it.
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
            let threat = view.threat(v.id());
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

    use super::super::testing::{
        cheat, enemy, run_until, training, training_allied, training_on, unit,
    };
    use super::Balance;
    use crate::input::{Key, Mods};
    use crate::testkit::{Harness, TestGame};

    /// The square of the player's town hall, and its name.
    fn home(g: &TestGame, p: PlayerId) -> (SquareId, String) {
        let w = g.game.host.world();
        let square = w.entity(unit(g, p, "townhall")).unwrap().square().unwrap();
        (square, g.game.view().square(square).name().to_owned())
    }

    /// The units of `p` of this type, in the whole world.
    fn units(g: &TestGame, p: PlayerId, type_name: &str) -> Vec<EntityId> {
        let w = g.game.host.world();
        w.player(p)
            .units
            .iter()
            .copied()
            .filter(|&u| w.entity(u).is_some_and(|e| &*e.type_name == type_name))
            .collect()
    }

    /// Gives `unit` of `p` an order, as the interface sends it.
    fn order(g: &mut TestGame, p: PlayerId, unit: EntityId, order: &str) {
        g.game.host.queue_line(p, &format!("control {}", unit.0));
        g.game.host.queue_line(p, &format!("order 0 0 {order}"));
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
        g.game.host.queue(me, Command::Cheat(Cheat::Resources));
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
        let (_, name) = home(&g, me);
        // Cheat mode shows the whole map: the deposits are known.
        g.game.host.queue(me, Command::Cheat(Cheat::ToggleMode));
        cheat(&mut g, me, &format!("add_units {name} 3 peasant"));
        run_turns(&mut h, &mut g, 1);
        let deposit = |type_name: &str| {
            let w = g.game.host.world();
            w.entities()
                .find(|e| &*e.type_name == type_name)
                .map(|e| e.id)
                .unwrap()
        };
        let (gold, wood) = (deposit("goldmine"), deposit("wood"));
        let peasants = units(&g, me, "peasant");
        assert_eq!(peasants.len(), 4);
        order(&mut g, me, peasants[0], &format!("gather {}", gold.0));
        order(&mut g, me, peasants[1], &format!("gather {}", gold.0));
        order(&mut g, me, peasants[2], &format!("gather {}", wood.0));
        run_turns(&mut h, &mut g, 2);
        assert_eq!(
            ask(&mut h, &mut g, Key::N, Mods::NONE),
            "1 idle , 2 on gold , 1 on wood"
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
        g.game.host.queue(them, Command::Quit);
        run_turns(&mut h, &mut g, 2);
        assert_eq!(
            ask(&mut h, &mut g, Key::F12, Mods::NONE),
            "quiet computer allied , quiet computer enemy , quiet computer enemy has been defeated"
        );
    }

    #[test]
    fn every_resource_of_the_rules_is_said() {
        let (mut h, mut g) = training_on("jl4");
        assert_eq!(
            ask(&mut h, &mut g, Key::X, Mods::SHIFT),
            "10 gold , 10 wood , 0 stone"
        );
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
