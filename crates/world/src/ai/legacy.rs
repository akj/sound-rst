//! The digest of a computer's state, fed as it was before its types became
//! ids and its orders tasks, so that the recorded digests check the change.
//! It goes once the state takes its final shape.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use super::{Computer, Sel, Task};
use crate::entity::EntityId;
use crate::rules::TypeId;

/// [`Sel`] as it was: by names.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LegacySel {
    List(Vec<String>),
    One(String),
    Workers,
}

impl Hash for Computer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let Computer {
            plan,
            line_nb,
            previous_linechange,
            watchdog,
            constant_attacks,
            research,
            attacked_places,
            orders,
            previous_choose,
            sensible_building,
            workers,
            gathered_deposits,
            building_sites,
            raise_dead_users,
            teleportation_users,
            cataclysm_users,
            detector_users,
            summon_users,
            time_has_come,
            safe_cnt,
            lacking,
            names,
        } = self;
        let name = |t: TypeId| names.get(t.0 as usize).cloned().unwrap_or_default();
        let sel = |s: &Sel| match s {
            Sel::List(l) => LegacySel::List(l.iter().map(|&t| name(t)).collect()),
            Sel::One(t) => LegacySel::One(name(*t)),
            Sel::Workers => LegacySel::Workers,
        };
        let words = |t: &Task| {
            let mut w = vec![t.keyword().as_str().to_owned()];
            match *t {
                Task::Repair(site) => w.push(site.to_string()),
                Task::Build(b, meadow) => w.extend([name(b), meadow.to_string()]),
                Task::Train(x) | Task::Research(x) | Task::UpgradeTo(x) => w.push(name(x)),
                Task::Use(a, target) => {
                    w.push(name(a));
                    w.extend(target.map(|u| u.to_string()));
                }
            }
            w
        };
        let users = |l: &[(EntityId, TypeId)]| -> Vec<(EntityId, String)> {
            l.iter().map(|&(u, a)| (u, name(a))).collect()
        };
        plan.as_ref().map(|p| p.lines()).hash(state);
        line_nb.hash(state);
        previous_linechange.hash(state);
        watchdog.hash(state);
        constant_attacks.hash(state);
        research.hash(state);
        attacked_places.hash(state);
        let orders: BTreeMap<_, _> = orders
            .iter()
            .map(|((s, t), f)| ((sel(s), words(t)), f))
            .collect();
        orders.hash(state);
        previous_choose.hash(state);
        sensible_building.hash(state);
        workers.hash(state);
        gathered_deposits.hash(state);
        building_sites.hash(state);
        users(raise_dead_users).hash(state);
        users(teleportation_users).hash(state);
        users(cataclysm_users).hash(state);
        users(detector_users).hash(state);
        users(summon_users).hash(state);
        time_has_come.hash(state);
        safe_cnt.hash(state);
        lacking.hash(state);
    }
}
