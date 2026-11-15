//! A game a server runs: the interface plays from its seat's stream.
//!
//! When the seat joins, the server sends the start of the game and the
//! seat's whole message; then, at each turn of its clock, the change from
//! the last message it sent; then the end, once. When a turn ends the
//! game, two changes of the same game time come: the end of the turn, then
//! the players leaving. The interface takes every frame as it comes, in
//! order, and treats each as a local game treats the parts of a turn it
//! runs. The server's clock is the game's: the interface keeps none of its
//! own.
//!
//! Everything received is checked before it is trusted. A frame that
//! cannot be right ends the game, with a short reason said, and the player
//! goes back to the server's lobby.

use std::sync::Arc;

use soundrts_protocol::game::{SeatFrame, encode_command, encode_turn};
use soundrts_protocol::{ClientMsg, ClockChange, MAX_TURN_ORDER_BYTES, Speed};
use soundrts_world::{Command, GameEnd, GameStart, Rules, SeatView};

use super::GameInterface;
use crate::client::Client;
use crate::message::{Category, News};
use crate::net::words::{self, text};
use crate::net::{Incoming, Seated};

/// How long the start of the game may take to come after the seat.
const JOIN_WAIT: f64 = 30.0;

/// The most events waiting to be told. The interface tells one at each
/// pass of its loop, about a thousand a second, and a busy turn has a few
/// dozen: a server that sends more is not playing a game.
const MAX_WAITING_EVENTS: usize = 65_536;

/// What a seat of a game on a server keeps of the server's stream, beside
/// what its view holds.
pub struct ServerGame {
    speed: Speed,
    paused: bool,
    /// When the server's next turn is due, as of the last turn it sent.
    next_turn: f64,
    /// The bytes of commands sent since the last turn came.
    sent_since_turn: usize,
    end: End,
}

/// Whether the server has sent the end of the game for the seat.
enum End {
    Waiting,
    Came(GameEnd),
    /// The score was said from it.
    Taken,
}

impl ServerGame {
    fn new(seated: &Seated, now: f64) -> ServerGame {
        let mut game = ServerGame {
            speed: seated.speed,
            paused: seated.paused,
            next_turn: now,
            sent_since_turn: 0,
            end: End::Waiting,
        };
        game.next_turn = game.after_a_turn(now);
        game
    }

    /// How many times faster than real time the server plays.
    pub(crate) fn speed_factor(&self) -> f64 {
        self.speed.factor()
    }

    pub(crate) fn next_turn(&self) -> f64 {
        self.next_turn
    }

    /// When the turn after `now` is due.
    fn after_a_turn(&self, now: f64) -> f64 {
        if self.paused {
            f64::INFINITY
        } else {
            now + self.speed.turn_interval().as_secs_f64()
        }
    }

    /// The end the server sent, once.
    pub(crate) fn take_end(&mut self) -> Option<GameEnd> {
        match std::mem::replace(&mut self.end, End::Taken) {
            End::Came(end) => Some(end),
            End::Waiting => {
                self.end = End::Waiting;
                None
            }
            End::Taken => None,
        }
    }
}

/// The speed of a server's game that a speed of the game menu asks for.
fn server_speed(factor: f64) -> Option<Speed> {
    [Speed::Slow, Speed::Normal, Speed::Fast2, Speed::Fast4]
        .into_iter()
        .find(|s| s.factor() == factor)
}

/// Whether the player is the host of the room whose game is played.
fn is_room_host(cx: &Client) -> bool {
    cx.net
        .as_ref()
        .is_some_and(|n| n.room.as_ref().is_some_and(|r| r.is_host(&n.lobby.you)))
}

/// Builds the interface of the seat a server gave the player, from the
/// start of the game and the seat's whole message, which the server sends
/// with the seat. `Err` says why the game cannot be played: `None` when the
/// connection or the game ended meanwhile.
pub(crate) fn join(cx: &mut Client, seated: &Seated) -> Result<GameInterface, Option<String>> {
    let deadline = cx.clock.now() + JOIN_WAIT;
    let mut start: Option<Arc<GameStart>> = None;
    loop {
        crate::net::pump(cx);
        let gone = cx.net.as_ref().is_none_or(|n| n.closed.is_some());
        let frame = match crate::net::game_link(cx) {
            Some(link) if !gone => link.frames.pop_front(),
            _ => return Err(None),
        };
        let Some(frame) = frame else {
            if cx.clock.now() > deadline {
                return Err(Some("the start of the game did not come".into()));
            }
            cx.voice.update();
            cx.clock.sleep(0.01);
            continue;
        };
        let frame = frame.map_err(|e| Some(format!("a frame cannot be read: {e}")))?;
        match (*frame, &start) {
            (SeatFrame::Start(s), None) => {
                s.check().map_err(|e| Some(format!("the start: {e}")))?;
                let rules = Rules::load(&[cx.res.rules_text()]);
                start = Some(Arc::new(GameStart::from_message(s, rules)));
            }
            (SeatFrame::Whole(m), Some(start)) => {
                start
                    .check(&m)
                    .map_err(|e| Some(format!("the first message: {e}")))?;
                let seat = SeatView::new(Arc::clone(start), m);
                let server = ServerGame::new(seated, cx.clock.now());
                return Ok(GameInterface::on_server(cx, seat, server));
            }
            (other, _) => return Err(Some(format!("before the game: {}", kind(&other)))),
        }
    }
}

/// What a frame is, for the log.
fn kind(frame: &SeatFrame) -> &'static str {
    match frame {
        SeatFrame::Start(_) => "a start",
        SeatFrame::Whole(_) => "a whole message",
        SeatFrame::Change(_) => "a change",
        SeatFrame::End(_) => "an end",
        SeatFrame::Seated { .. } => "a seat",
        SeatFrame::Seat { .. } => "news of a seat",
        SeatFrame::Clock { .. } => "news of the clock",
        SeatFrame::Stopped(_) => "a stop",
        SeatFrame::NotStarted(_) => "a refused start",
    }
}

impl GameInterface {
    fn server(&mut self) -> Option<&mut ServerGame> {
        match &mut self.source {
            super::Source::Server(s) => Some(s),
            super::Source::Local(_) => None,
        }
    }

    /// Sends the commands given, as many as the server takes for one turn,
    /// then takes every frame the server sent, in order. The game ends if
    /// the server ended it, the connection is lost, or a frame cannot be
    /// right.
    pub(crate) fn server_update(&mut self, cx: &mut Client) {
        let (gone, ended) = match &cx.net {
            Some(n) if n.closed.is_none() => match &n.game {
                Some(g) => (false, g.ended),
                None => (true, true),
            },
            _ => (true, true),
        };
        if gone {
            if !self.end_loop {
                self.srv_quit(cx);
            }
            return;
        }
        self.send_pending(cx);
        while let Some(frame) = crate::net::game_link(cx).and_then(|l| l.frames.pop_front()) {
            if let Err(why) = self.take_frame(cx, frame) {
                self.cannot_be_right(cx, &why);
                return;
            }
            if self.end_loop {
                return;
            }
        }
        if ended && self.srv_queue.is_empty() && !self.end_loop {
            self.srv_quit(cx);
        }
    }

    /// Ends the game on a frame that cannot be right: says so, and leaves
    /// the game.
    fn cannot_be_right(&mut self, cx: &mut Client, why: &str) {
        cx.log
            .warning(format!("the server's stream cannot be right: {why}"));
        cx.news(News::new(Category::System, text(words::CANNOT_BE_RIGHT)));
        self.srv_quit(cx);
    }

    /// Sends the commands given since the last call, as many as fit in
    /// what the server takes for one turn; the others wait for the next
    /// turn. A command that cannot be sent is dropped. A speed is the room
    /// host's to ask for ([`GameInterface::set_server_speed`]), not a
    /// command.
    fn send_pending(&mut self, cx: &mut Client) {
        let Some(server) = self.server() else {
            return;
        };
        // Commands sent just before a turn came may still wait at the server
        // for the next one: half of what it takes leaves room for them.
        let budget = MAX_TURN_ORDER_BYTES / 2;
        let mut sent = server.sent_since_turn;
        let mut taken = 0;
        let mut out = Vec::new();
        let mut speeds = Vec::new();
        for command in &self.pending_commands {
            if let Command::Speed(s) = command {
                speeds.push(*s);
                taken += 1;
                continue;
            }
            match encode_command(command) {
                Ok(bytes) if sent + bytes.len() > budget => break,
                Ok(bytes) => {
                    sent += bytes.len();
                    out.push(bytes);
                }
                Err(e) => cx.log.warning(format!("cannot send {command:?}: {e}")),
            }
            taken += 1;
        }
        self.pending_commands.drain(..taken);
        if let Some(server) = self.server() {
            server.sent_since_turn = sent;
        }
        for bytes in out {
            crate::net::send_command(cx, bytes);
        }
        for s in speeds {
            self.set_server_speed(cx, s);
        }
    }

    /// Takes one frame of the stream. `Err` says why it cannot be right.
    fn take_frame(&mut self, cx: &mut Client, frame: Incoming) -> Result<(), String> {
        let frame = frame.map_err(|e| format!("a frame cannot be read: {e}"))?;
        if let Some(ServerGame {
            end: End::Came(_) | End::Taken,
            ..
        }) = self.server()
        {
            return Err(format!("{} after the end", kind(&frame)));
        }
        match *frame {
            SeatFrame::Change(change) => self.take_change(cx, change),
            SeatFrame::End(end) => {
                if let Some(server) = self.server() {
                    server.end = End::Came(end);
                }
                // The server has taken the player out of the game.
                if let Some(link) = crate::net::game_link(cx) {
                    link.ended = true;
                }
                Ok(())
            }
            SeatFrame::Seat { slot, change } => {
                let known = cx
                    .net
                    .as_ref()
                    .and_then(|n| n.room.as_ref())
                    .is_some_and(|r| r.state.slots.get(slot).is_some());
                if !known {
                    return Err(format!("news of slot {slot}, not in the room"));
                }
                cx.log.info(format!("slot {slot}: {change:?}"));
                Ok(())
            }
            SeatFrame::Clock { by, change } => {
                let now = cx.clock.now();
                if let Some(server) = self.server() {
                    match change {
                        ClockChange::Paused => server.paused = true,
                        ClockChange::Resumed => server.paused = false,
                        ClockChange::Speed { speed } => server.speed = speed,
                    }
                    server.next_turn = server.after_a_turn(now);
                    let factor = server.speed_factor();
                    self.speed = factor;
                }
                cx.log.info(format!("{by}: {change:?}"));
                Ok(())
            }
            SeatFrame::Stopped(reason) => {
                cx.log
                    .warning(format!("the server stopped the game: {reason:?}"));
                if let Some(link) = crate::net::game_link(cx) {
                    link.ended = true;
                }
                self.srv_quit(cx);
                Ok(())
            }
            other @ (SeatFrame::Start(_)
            | SeatFrame::Whole(_)
            | SeatFrame::Seated { .. }
            | SeatFrame::NotStarted(_)) => Err(format!("{} during the game", kind(&other))),
        }
    }

    /// Takes a change: the seat's new message, current from now on. A
    /// change of a later time while the player is in the game is the end of
    /// a turn; one of the same time as the last, or after the player left
    /// the game, is what followed the end of their turn.
    fn take_change(
        &mut self,
        cx: &mut Client,
        change: soundrts_world::TurnChange,
    ) -> Result<(), String> {
        let last = self.view().message();
        let mut m = change.apply(last).map_err(|e| format!("a change: {e}"))?;
        self.view()
            .game_start()
            .check(&m)
            .map_err(|e| format!("a turn: {e}"))?;
        if m.seat != self.me {
            return Err(format!("a turn of player {}", m.seat.0));
        }
        if m.time < last.time {
            return Err("a turn back in time".into());
        }
        // A message the server could not send whole, as it does to a seat
        // that joins, cannot be right: so a stream cannot grow it without
        // bound.
        encode_turn(&m).map_err(|e| format!("a turn: {e}"))?;
        let end_of_turn = m.time > last.time && m.state.players.contains(&self.me);
        if end_of_turn {
            let now = cx.clock.now();
            if let Some(server) = self.server() {
                server.sent_since_turn = 0;
                server.next_turn = server.after_a_turn(now);
            }
            let m = Arc::new(m);
            self.take_turn(Arc::clone(&m), Some(m), Vec::new());
        } else {
            let then = std::mem::take(&mut m.events);
            self.take_turn(Arc::new(m), None, then);
        }
        if self.srv_queue.len() > MAX_WAITING_EVENTS {
            return Err("more events than the game can tell".into());
        }
        Ok(())
    }

    /// Whether the game menu offers the speeds: in a game run here, to its
    /// first player; on a server, to every player, and only the room's
    /// host's choice goes to the server.
    pub(crate) fn offers_speeds(&self) -> bool {
        self.on_a_server() || self.view().players().first() == Some(&self.me)
    }

    /// Asks the server to play at the speed `factor` of the game menu: the
    /// room's host only. Anyone else hears that only the host can.
    fn set_server_speed(&mut self, cx: &mut Client, factor: f64) {
        let Some(speed) = server_speed(factor) else {
            cx.log.warning(format!("no server speed for {factor}"));
            return;
        };
        if is_room_host(cx) {
            crate::net::send(cx, ClientMsg::SetGameSpeed { speed });
        } else {
            cx.voice.item(&text(words::ONLY_THE_HOST_SETS_THE_SPEED));
        }
    }
}
