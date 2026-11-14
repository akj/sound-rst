//! A game on a server (`worldclient.Coordinator`): the commands the player
//! gives go to the server as orders, and a turn runs only when the server
//! has sent it, with every player's orders, so that every client runs the
//! same game.

use soundrts_protocol::{ClientMsg, SlotIndex};
use soundrts_world::PlayerId;

use super::GameInterface;
use crate::client::Client;
use crate::record::{GameRecord, RecordedOrder, RecordedTurn};

/// What runs the turns of a game on a server.
pub struct NetDriver {
    /// The player's slot.
    pub you: SlotIndex,
    /// The world's player of each slot.
    pub slots: Vec<PlayerId>,
    /// The real time between two turns, in seconds.
    pub interval: f64,
    /// The game so far: each turn run, with the orders queued for it and
    /// the digest told to the server.
    pub record: GameRecord,
}

impl GameInterface {
    /// Sends the commands written, then runs the next turn received if its
    /// time has come. Turns that piled up run closer together until the
    /// game has caught up. The game ends if the server ended it or the
    /// connection is lost. When the server has said the game is out of
    /// sync, the record is saved first.
    pub(crate) fn net_update(&mut self, cx: &mut Client) {
        self.save_record_if_out_of_sync(cx);
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
        self.send_pending(cx);
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
        let mut queued = Vec::new();
        for so in orders {
            let Some(&p) = driver.slots.get(so.slot) else {
                cx.log
                    .warning(format!("orders for an unknown slot: {}", so.slot));
                continue;
            };
            // As the original: a player out of the game gets no order.
            if !self.view().players().contains(&p) {
                continue;
            }
            for order in so.orders.as_slice() {
                self.host.queue_line(p, order.as_str());
                queued.push(RecordedOrder {
                    slot: so.slot,
                    order: order.as_str().to_owned(),
                });
            }
        }
        self.next_update = now + interval / (1 + backlog) as f64;
        self.run_turn(cx);
        let digest = self.host.digest();
        if let Some(driver) = &mut self.net {
            driver.record.turns.push(RecordedTurn {
                turn,
                orders: queued,
                digest,
            });
        }
        crate::net::send(cx, ClientMsg::TurnDone { turn, digest });
    }

    /// Sends the commands given since the last call as orders, as many as
    /// the server takes for one turn; the others wait.
    fn send_pending(&mut self, cx: &mut Client) {
        // Every pending command has a line: `send_command` keeps no other
        // in a game on a server.
        let mut lines: Vec<String> = self
            .pending_commands
            .iter()
            .filter_map(|c| self.host.line(c))
            .collect();
        let before = lines.len();
        crate::net::send_orders(cx, &mut lines);
        self.pending_commands.drain(..before - lines.len());
    }

    /// Writes the record to the replays folder if the server has said the
    /// game is out of sync since the last call: in
    /// `out-of-sync-<login>-seed<seed>-turn<turn>.jsonl`, with the
    /// player's login on the server and the turn the server named.
    fn save_record_if_out_of_sync(&self, cx: &mut Client) {
        let Some(net) = &mut cx.net else {
            return;
        };
        let Some(turn) = net.game.as_mut().and_then(|g| g.take_out_of_sync()) else {
            return;
        };
        let Some(driver) = &self.net else {
            return;
        };
        let name = format!(
            "out-of-sync-{}-seed{}-turn{turn}.jsonl",
            net.lobby.you.as_str(),
            driver.record.setup.seed
        );
        let path = cx.paths.replays().join(name);
        match driver.record.write(&path) {
            Ok(()) => cx
                .log
                .warning(format!("game record written to {}", path.display())),
            Err(e) => cx
                .log
                .warning(format!("couldn't write {}: {e}", path.display())),
        }
    }
}
