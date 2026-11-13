//! A game on a server (`worldclient.Coordinator`): the commands the player
//! writes go to the server as orders, and a turn runs only when the server
//! has sent it, with every player's orders, so that every client runs the
//! same game.

use soundrts_protocol::{ClientMsg, Digest, SlotIndex, Turn};
use soundrts_world::PlayerId;

use super::GameInterface;
use crate::client::Client;

/// What runs the turns of a game on a server.
pub struct NetDriver {
    /// The player's slot.
    pub you: SlotIndex,
    /// The world's player of each slot.
    pub slots: Vec<PlayerId>,
    /// The real time between two turns, in seconds.
    pub interval: f64,
    /// The digest after each turn run, as told to the server.
    pub digests: Vec<(Turn, Digest)>,
}

impl GameInterface {
    /// Sends the commands written, then runs the next turn received if its
    /// time has come. Turns that piled up run closer together until the
    /// game has caught up. The game ends if the server ended it or the
    /// connection is lost.
    pub(crate) fn net_update(&mut self, cx: &mut Client) {
        let over = match &cx.net {
            None => true,
            Some(n) => n.closed.is_some() || n.game.as_ref().is_none_or(|g| g.ended),
        };
        if over {
            if !self.end_loop {
                self.srv_quit(cx);
            }
            return;
        }
        crate::net::send_orders(cx, &mut self.pending_commands);
        let now = cx.clock.now();
        if self.waiting_for_world_update || now < self.next_update {
            return;
        }
        let Some(link) = crate::net::game_link(cx) else {
            return;
        };
        let Some((turn, orders)) = link.turns.pop_front() else {
            return;
        };
        let backlog = link.turns.len();
        let driver = self.net.as_ref().expect("a game on a server");
        let interval = driver.interval;
        for so in orders {
            let Some(&p) = driver.slots.get(so.slot) else {
                cx.log
                    .warning(format!("orders for an unknown slot: {}", so.slot));
                continue;
            };
            // As the original: a player out of the game gets no order.
            if !self.world.players.contains(&p) {
                continue;
            }
            for order in so.orders.as_slice() {
                self.world.queue_command(p, order.as_str());
            }
        }
        self.next_update = now + interval / (1 + backlog) as f64;
        self.run_turn(cx);
        let digest = self.world.digest_u64();
        if let Some(driver) = &mut self.net {
            driver.digests.push((turn, digest));
        }
        crate::net::send(cx, ClientMsg::TurnDone { turn, digest });
    }
}
