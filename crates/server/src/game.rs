//! One task per running game: the wait for every player to load, the turn
//! clock, the orders for the next turn, the wait for slow players, and the
//! comparison of digests.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use soundrts_protocol::{
    Digest, Login, MAX_ORDERS_PER_MESSAGE, MAX_TURN_ORDER_BYTES, Order, Orders, Refusal, RoomId,
    ServerMsg, SlotIndex, SlotOrders, Turn,
};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tracing::{info, warn};

use crate::connection::{ConnId, Outbox, encode};
use crate::lobby::{LobbyInput, LobbyTx};

/// What the lobby passes to a game, for the slot of the player who sent it.
pub(crate) enum GameInput {
    Loaded {
        slot: SlotIndex,
    },
    Orders {
        slot: SlotIndex,
        orders: Orders,
    },
    TurnDone {
        slot: SlotIndex,
        turn: Turn,
        digest: Digest,
    },
    Leave {
        slot: SlotIndex,
    },
}

pub(crate) struct Settings {
    pub turn_interval: Duration,
    pub max_lead: u32,
    pub lag_timeout: Duration,
}

/// A human in a game, as the lobby hands them over.
pub(crate) struct Player {
    pub slot: SlotIndex,
    pub conn: ConnId,
    pub login: Login,
    pub outbox: Outbox,
}

/// Starts a game's task. The game reports to the lobby each human who
/// leaves it ([`LobbyInput::LeftGame`]), then [`LobbyInput::GameOver`] when
/// none is left.
pub(crate) fn spawn(
    room: RoomId,
    players: Vec<Player>,
    settings: Settings,
    lobby: LobbyTx,
) -> mpsc::UnboundedSender<GameInput> {
    let (tx, rx) = mpsc::unbounded_channel();
    let humans = players
        .into_iter()
        .map(|p| {
            let human = Human {
                conn: p.conn,
                login: p.login,
                outbox: p.outbox,
                last_done: None,
                holding_since: None,
            };
            (p.slot, human)
        })
        .collect();
    let game = Game {
        room,
        humans,
        next_turn: 0,
        pending: BTreeMap::new(),
        phase: Phase::Loading {
            deadline: Instant::now() + settings.lag_timeout,
            loaded: BTreeSet::new(),
        },
        digests: BTreeMap::new(),
        settings,
        lobby,
    };
    tokio::spawn(game.run(rx));
    tx
}

struct Human {
    conn: ConnId,
    login: Login,
    outbox: Outbox,
    last_done: Option<Turn>,
    /// Since when this player has kept the game waiting.
    holding_since: Option<Instant>,
}

enum Phase {
    /// The clock has not started: the game waits for every human to build
    /// the world. A human who has not loaded by `deadline` is taken out.
    Loading {
        deadline: Instant,
        loaded: BTreeSet<SlotIndex>,
    },
    /// The clock runs. `waiting` while it is stopped for slow players.
    Running { clock: Interval, waiting: bool },
}

impl Phase {
    /// When the game next has something to do on its own.
    async fn wake(&mut self) {
        match self {
            Phase::Loading { deadline, .. } => tokio::time::sleep_until(*deadline).await,
            Phase::Running { clock, .. } => {
                clock.tick().await;
            }
        }
    }
}

struct Game {
    room: RoomId,
    /// The humans still in the game.
    humans: BTreeMap<SlotIndex, Human>,
    next_turn: Turn,
    /// The orders for `next_turn`, in arrival order for each slot.
    pending: BTreeMap<SlotIndex, Vec<Order>>,
    phase: Phase,
    /// The digests of the turns some human still in the game has not
    /// reported.
    digests: BTreeMap<Turn, BTreeMap<SlotIndex, Digest>>,
    settings: Settings,
    lobby: LobbyTx,
}

impl Game {
    async fn run(mut self, mut inputs: mpsc::UnboundedReceiver<GameInput>) {
        info!(room = self.room.0, "game loading");
        while !self.humans.is_empty() {
            tokio::select! {
                _ = self.phase.wake() => self.wake(Instant::now()),
                input = inputs.recv() => match input {
                    Some(input) => self.on_input(input),
                    None => return,
                },
            }
        }
        info!(room = self.room.0, turns = self.next_turn, "game over");
        let _ = self.lobby.send(LobbyInput::GameOver { room: self.room });
    }

    fn on_input(&mut self, input: GameInput) {
        match input {
            GameInput::Orders { slot, orders } => {
                let Some(human) = self.humans.get(&slot) else {
                    return;
                };
                let pending = self.pending.entry(slot).or_default();
                let bytes = |list: &[Order]| list.iter().map(|o| o.as_str().len()).sum::<usize>();
                if bytes(pending) + bytes(orders.as_slice()) > MAX_TURN_ORDER_BYTES {
                    human.outbox.send(&ServerMsg::Refused {
                        reason: Refusal::SlowDown,
                    });
                    return;
                }
                pending.extend(Vec::from(orders));
            }
            GameInput::Loaded { slot } => self.loaded(slot),
            GameInput::TurnDone { slot, turn, digest } => self.turn_done(slot, turn, digest),
            GameInput::Leave { slot } => self.leave(slot),
        }
    }

    /// How many turns `human` is behind the last turn sent.
    fn lag(&self, human: &Human) -> i64 {
        i64::from(self.next_turn) - 1 - human.last_done.map_or(-1, i64::from)
    }

    fn laggards(&self) -> Vec<SlotIndex> {
        self.humans
            .iter()
            .filter(|(_, h)| self.lag(h) > i64::from(self.settings.max_lead))
            .map(|(&slot, _)| slot)
            .collect()
    }

    fn loaded(&mut self, slot: SlotIndex) {
        let Some(human) = self.humans.get(&slot) else {
            return;
        };
        let newly = match &mut self.phase {
            Phase::Loading { loaded, .. } => loaded.insert(slot),
            Phase::Running { .. } => false,
        };
        if newly {
            self.start_if_all_loaded();
        } else {
            human.outbox.send(&ServerMsg::Refused {
                reason: Refusal::NotNow,
            });
        }
    }

    /// Starts the clock once every human still in the game has loaded. The
    /// first tick is immediate, so turn 0 goes out at once.
    fn start_if_all_loaded(&mut self) {
        let Phase::Loading { loaded, .. } = &self.phase else {
            return;
        };
        if self.humans.is_empty() || !self.humans.keys().all(|s| loaded.contains(s)) {
            return;
        }
        info!(room = self.room.0, "game clock started");
        let mut clock = tokio::time::interval(self.settings.turn_interval);
        clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
        self.phase = Phase::Running {
            clock,
            waiting: false,
        };
    }

    fn wake(&mut self, now: Instant) {
        match &self.phase {
            Phase::Loading { loaded, .. } => {
                let late: Vec<SlotIndex> = self
                    .humans
                    .keys()
                    .filter(|s| !loaded.contains(s))
                    .copied()
                    .collect();
                for slot in late {
                    info!(room = self.room.0, login = %self.humans[&slot].login, "did not load in time; taken out of the game");
                    self.leave(slot);
                }
            }
            Phase::Running { .. } => self.tick(now),
        }
    }

    fn tick(&mut self, now: Instant) {
        let laggards = self.laggards();
        if laggards.is_empty() {
            self.send_turn();
            return;
        }
        if let Phase::Running { waiting, .. } = &mut self.phase
            && !*waiting
        {
            *waiting = true;
            let players = laggards
                .iter()
                .map(|s| self.humans[s].login.clone())
                .collect::<Vec<_>>();
            info!(room = self.room.0, ?players, "game waiting");
            self.broadcast(&ServerMsg::Waiting { players });
        }
        for slot in laggards {
            let human = self.humans.get_mut(&slot).expect("a laggard is a human");
            let since = *human.holding_since.get_or_insert(now);
            if now.duration_since(since) >= self.settings.lag_timeout {
                info!(room = self.room.0, login = %human.login, "too slow; taken out of the game");
                self.leave(slot);
            }
        }
    }

    fn send_turn(&mut self) {
        let mut orders = Vec::new();
        for (slot, list) in std::mem::take(&mut self.pending) {
            for chunk in list.chunks(MAX_ORDERS_PER_MESSAGE) {
                orders.push(SlotOrders {
                    slot,
                    orders: Orders::new(chunk.to_vec()).expect("a chunk fits in a message"),
                });
            }
        }
        self.broadcast(&ServerMsg::Turn {
            turn: self.next_turn,
            orders,
        });
        self.next_turn += 1;
    }

    fn turn_done(&mut self, slot: SlotIndex, turn: Turn, digest: Digest) {
        let next_turn = self.next_turn;
        let max_lead = i64::from(self.settings.max_lead);
        let Some(human) = self.humans.get_mut(&slot) else {
            return;
        };
        let expected = human.last_done.map_or(0, |t| t + 1);
        if turn != expected || turn >= next_turn {
            human.outbox.send(&ServerMsg::Refused {
                reason: Refusal::NotNow,
            });
            return;
        }
        human.last_done = Some(turn);
        if i64::from(next_turn) - 1 - i64::from(turn) <= max_lead {
            human.holding_since = None;
        }
        self.digests.entry(turn).or_default().insert(slot, digest);
        self.compare_digests();
        self.resume_if_nobody_behind();
    }

    /// Takes a human out of the game, then does what the game was waiting
    /// on them for.
    fn leave(&mut self, slot: SlotIndex) {
        if self.take_out(slot) {
            self.compare_digests();
            self.resume_if_nobody_behind();
            self.start_if_all_loaded();
        }
    }

    /// Takes a human out of the game: their `quit` goes into the next turn.
    /// False if they were not in it.
    fn take_out(&mut self, slot: SlotIndex) -> bool {
        let Some(human) = self.humans.remove(&slot) else {
            return false;
        };
        info!(room = self.room.0, login = %human.login, "left the game");
        self.pending.entry(slot).or_default().push(Order::quit());
        self.broadcast(&ServerMsg::PlayerLeftGame { slot });
        let _ = self.lobby.send(LobbyInput::LeftGame {
            room: self.room,
            conn: human.conn,
        });
        true
    }

    fn resume_if_nobody_behind(&mut self) {
        let waiting = matches!(self.phase, Phase::Running { waiting: true, .. });
        if waiting && !self.humans.is_empty() && self.laggards().is_empty() {
            if let Phase::Running { waiting, .. } = &mut self.phase {
                *waiting = false;
            }
            for human in self.humans.values_mut() {
                human.holding_since = None;
            }
            info!(room = self.room.0, "game resumed");
            self.broadcast(&ServerMsg::Resumed);
        }
    }

    /// Compares the digests of each turn every human still in the game has
    /// reported, oldest first. The humans whose digest differs are told,
    /// with the others, and taken out of the game; the others play on.
    fn compare_digests(&mut self) {
        while let Some(entry) = self.digests.first_entry() {
            if !self.humans.keys().all(|s| entry.get().contains_key(s)) {
                return;
            }
            let turn = *entry.key();
            let digests: BTreeMap<SlotIndex, Digest> = entry
                .remove()
                .into_iter()
                .filter(|(s, _)| self.humans.contains_key(s))
                .collect();
            if let Some(slots) = odd_slots(&digests) {
                warn!(room = self.room.0, turn, ?slots, "out of sync");
                self.broadcast(&ServerMsg::OutOfSync {
                    turn,
                    slots: slots.clone(),
                });
                // All of them before the next turn is compared: with only
                // some of them out, the others could look like a majority.
                for slot in slots {
                    self.take_out(slot);
                }
            }
        }
    }

    fn broadcast(&self, msg: &ServerMsg) {
        let text = encode(msg);
        for human in self.humans.values() {
            human.outbox.send_text(text.clone());
        }
    }
}

/// The slots whose digest differs from the most common one, or every slot
/// when two digests tie for most common. `None` when all agree.
fn odd_slots(digests: &BTreeMap<SlotIndex, Digest>) -> Option<Vec<SlotIndex>> {
    let mut counts: HashMap<Digest, usize> = HashMap::new();
    for d in digests.values() {
        *counts.entry(*d).or_default() += 1;
    }
    if counts.len() <= 1 {
        return None;
    }
    let most = counts.values().copied().max().expect("not empty");
    let mut leaders = counts.iter().filter(|(_, n)| **n == most);
    let leader = leaders.next().map(|(d, _)| *d);
    let common = if leaders.next().is_some() {
        None
    } else {
        leader
    };
    Some(
        digests
            .iter()
            .filter(|(_, d)| Some(**d) != common)
            .map(|(s, _)| *s)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digests(list: &[Digest]) -> BTreeMap<SlotIndex, Digest> {
        list.iter().copied().enumerate().collect()
    }

    #[test]
    fn agreeing_digests_are_in_sync() {
        assert_eq!(odd_slots(&digests(&[7, 7, 7])), None);
        assert_eq!(odd_slots(&digests(&[7])), None);
        assert_eq!(odd_slots(&BTreeMap::new()), None);
    }

    #[test]
    fn the_odd_one_out_is_named() {
        assert_eq!(odd_slots(&digests(&[7, 7, 8])), Some(vec![2]));
        assert_eq!(odd_slots(&digests(&[1, 7, 7, 8])), Some(vec![0, 3]));
    }

    #[test]
    fn on_a_tie_every_slot_is_named() {
        assert_eq!(odd_slots(&digests(&[7, 8])), Some(vec![0, 1]));
        assert_eq!(odd_slots(&digests(&[7, 7, 8, 8])), Some(vec![0, 1, 2, 3]));
    }
}
