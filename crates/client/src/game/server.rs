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
//! The start names the rules the game was built from by their hash. The
//! stream names unit types, upgrades and factions by their place in those
//! rules, so a client whose rules differ does not play the game, and says
//! why.
//!
//! Everything received is checked before it is trusted: the interface
//! reads the stream only through [`JoinStream`], [`SeatStream`] and
//! [`ReturnStream`], and checks the news of the game against the room. A
//! frame that cannot be right ends the game, with a short reason said, and
//! the player goes back to the server's lobby.
//!
//! A watcher's seat plays the same way, from the stream of the player it
//! follows: the interface is that player's, and only the orders are not
//! given. When the watcher follows another player, the server sends that
//! player's whole message, and the interface starts again from it
//! ([`GameInterface::follow`]), keeping only its settings.
//!
//! When the connection is lost during the game, the game goes on without
//! it ([`Link::Returning`]): the player hears it, asks the questions of
//! the last view and reviews the log, and the orders given, the chat lines
//! and the flares are not taken.
//! Meanwhile the client opens a new connection to the same server, says
//! hello, and presents the seat's token, again after a pause, for as long as
//! the server holds the seat. The server sends the seat, the start and
//! the whole message again, which replace the seat's view once checked
//! ([`ReturnStream`]): the same slot, the same game, the same player, not
//! back in time. A seat the server no longer holds ends the game with the
//! reason said, as a lost connection did. Each step polls; nothing waits.

use std::sync::Arc;
use std::time::Duration;

use soundrts_protocol::game::{
    self, Checked, JoinRefused, JoinStream, Joined, Joining, ReturnStream, Role, SeatFrame,
    SeatStream, encode_command,
};
use soundrts_protocol::{
    Channel, ClientMsg, ClockChange, Login, MAX_TURN_ORDER_BYTES, Refusal, RoomId, RoomState,
    SeatToken, ServerMsg, SlotPlayer, Speed, Step, StopReason, Whereabouts,
};
use soundrts_world::msg::{Cue, Phrase, cue, msg};
use soundrts_world::{Command, GameEnd, GameStart, SeatView, TurnMessage};

use super::GameInterface;
use super::watching::Orders;
use crate::client::Client;
use crate::message::{Category, News, RoomTopic, Subject};
use crate::net::conn::{Connection, NetEvent, Opening};
use crate::net::lobby::{LobbyModel, check_watchers, player_name};
use crate::net::words;
use crate::net::{Incoming, PUMP_LIMIT, Seated};

/// How long the start of the game may take to come after the seat.
const JOIN_WAIT: f64 = 30.0;

/// The pause between two attempts to return to the seat.
const RETRY_PAUSE: f64 = 1.0;

/// How long the server may take to give the seat back once its token is
/// presented.
const ANSWER_WAIT: f64 = 10.0;

/// The longest a client tries to return to a seat, whatever the server
/// says it holds it for.
const MAX_HOLD: Duration = Duration::from_secs(3600);

/// The most objectives a game on a server gives the player, over the
/// whole game: the interface remembers each.
const MAX_OBJECTIVES_SEEN: usize = 4 * soundrts_world::MAX_OBJECTIVES;

/// The most events waiting to be told, with their ends of turn and the
/// units replaced that the interface keeps until those ends are handled. The interface
/// tells one event at each pass of its loop, about a thousand a second,
/// and a busy turn has a few dozen: a server that sends more is not
/// playing a game.
const MAX_WAITING_EVENTS: usize = 65_536;

/// How a turn the server sent enters the interface
/// ([`GameInterface::take_server_turn`]).
pub(super) enum ServerTurn {
    /// The end of a turn, or a whole message of the seat taken as one: its
    /// events are told, then its end.
    End,
    /// A change that ends no turn: only its events are told.
    Change,
    /// A watcher follows the player of a whole message, in the game of
    /// this start.
    Follow(Arc<GameStart>),
}

/// What a seat of a game on a server keeps of the server's stream, beside
/// what its view holds.
pub struct ServerGame {
    speed: Speed,
    paused: bool,
    /// When the server's next turn is due, as of the last turn it sent.
    next_turn: f64,
    end: End,
    /// The game told the player it is over for them, and the end has not
    /// come yet: the game ends when it comes.
    quit: bool,
    /// The objectives the turns taken brought, each counted when it came.
    objectives_given: usize,
    seat: SeatKey,
    /// The seat's stream, checked, as of the last frame taken.
    stream: SeatStream,
    link: Link,
    /// Whether a watcher gave orders, and heard that it gives none.
    orders: Orders,
}

/// What takes the seat back after a lost connection. No `Debug`: the
/// token is a secret for the length of the game.
struct SeatKey {
    /// How long the server holds the seat once it notices the loss, in
    /// seconds.
    hold: f64,
    token: SeatToken,
}

impl SeatKey {
    fn of(hold: Duration, token: SeatToken) -> SeatKey {
        SeatKey {
            hold: hold.min(MAX_HOLD).as_secs_f64(),
            token,
        }
    }
}

/// Whether the seat's stream comes.
enum Link {
    /// It comes on the connection: the bytes of commands sent since the
    /// last turn came.
    Playing { sent_since_turn: usize },
    /// The connection is lost, and the client returns to the seat.
    Returning(Box<Return>),
}

/// A return to the seat under way.
struct Return {
    room: RoomId,
    /// The player's login when the connection was lost: while the server
    /// still has it playing, it has not noticed the loss yet.
    login: Login,
    /// When the server no longer holds the seat, as far as the client can
    /// tell: the hold from when the client noticed the loss.
    until: f64,
    attempt: Attempt,
    orders: Orders,
    /// Whether the player wrote a chat line or sent a flare while away,
    /// and heard that it was not sent.
    unsent: Orders,
}

/// One attempt to return, in the order it goes.
enum Attempt {
    /// The next attempt starts at `at`.
    Pause { at: f64 },
    /// A new connection is opened and says hello.
    Opening(Opening),
    /// The token is presented on `conn`, at `since`.
    Presented(Box<Presented>),
}

/// The token presented: the server gives the seat back, or refuses.
struct Presented {
    conn: Connection,
    /// The lobby as the new connection is told of it.
    lobby: LobbyModel,
    /// The room of the game as the new connection is told of it: who
    /// watches may have changed while the connection was lost.
    room: Option<RoomState>,
    since: f64,
    /// The seat's stream on the new connection.
    stream: ReturnStream,
}

/// What an attempt to return came to.
enum Outcome {
    /// It goes on.
    Waiting,
    /// The seat is back, checked: the game plays on from the new
    /// connection.
    Back(Box<Reconnected>),
    /// The server no longer holds the seat.
    Failed(Failure),
    /// What came cannot be right.
    CannotBeRight(String),
    /// The server stopped the game.
    Stopped(StopReason),
}

/// What returns the player to the game.
struct Reconnected {
    conn: Connection,
    lobby: LobbyModel,
    room: Option<RoomState>,
    back: game::Back,
}

/// Why the player cannot return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The game is over.
    GameOver,
    /// The server refused the token: the seat was given up.
    SeatGivenUp,
    /// The server could not be reached while it held the seat.
    TimeUp,
}

/// Whether the server has sent the end of the game for the seat.
enum End {
    Waiting,
    Came(GameEnd),
    /// The score was said from it.
    Taken,
}

impl ServerGame {
    fn new(seated: &Seated, stream: SeatStream, now: f64) -> ServerGame {
        let mut game = ServerGame {
            speed: seated.speed,
            paused: seated.paused,
            next_turn: now,
            end: End::Waiting,
            quit: false,
            objectives_given: 0,
            seat: SeatKey::of(seated.hold, seated.token.clone()),
            stream,
            link: Link::Playing { sent_since_turn: 0 },
            orders: Orders::NoneGiven,
        };
        game.next_turn = game.after_a_turn(now);
        game
    }

    /// Whether the player only watches.
    pub(crate) fn watching(&self) -> bool {
        self.stream.role() == Role::Watcher
    }

    /// Whether a watcher gave orders, and heard that it gives none.
    pub(crate) fn watcher_orders(&mut self) -> &mut Orders {
        &mut self.orders
    }

    /// The watcher now follows another player, whose message brought
    /// `objectives`.
    pub(crate) fn followed(&mut self, objectives: usize) {
        self.objectives_given = objectives;
        self.quit = false;
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

    /// The game told the player it is over for them: whether the game
    /// ends now. It ends once the end has come, with the statistics of the
    /// score, and a watcher's when its own end comes: the server moves it on
    /// to another player meanwhile.
    pub(crate) fn told_over(&mut self) -> bool {
        if self.watching() {
            return false;
        }
        match self.end {
            End::Waiting => {
                self.quit = true;
                false
            }
            End::Came(_) | End::Taken => true,
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

/// Whether the player is the host of the room whose game is played: the
/// host is the player of their slot, whatever the login of the connection
/// that took the seat back. A watcher is not.
fn is_room_host(cx: &Client) -> bool {
    let Some(net) = &cx.net else {
        return false;
    };
    let (Some(room), Some(link)) = (&net.room, &net.game) else {
        return false;
    };
    let Role::Player(slot) = link.you else {
        return false;
    };
    room.state
        .slots
        .get(slot)
        .is_some_and(|s| s.player == SlotPlayer::Human(room.state.host.clone()))
}

/// Builds the interface of the seat a server gave the player, from the
/// start of the game and the seat's whole message, which the server sends
/// with the seat. `Err` says why the game cannot be played: `None` when
/// the player was told why, or the connection or the game ended
/// meanwhile.
pub(crate) fn join(cx: &mut Client, seated: &Seated) -> Result<GameInterface, Option<String>> {
    let deadline = cx.clock.now() + JOIN_WAIT;
    let mut stream = JoinStream::new(seated.you, seated.map.hash.clone(), &cx.res.rules_text());
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
        let Joined { stream, message: m } = match stream.take(*frame) {
            Ok(Joining::Started(next)) => {
                stream = next;
                continue;
            }
            Ok(Joining::Joined(joined)) => joined,
            Err(why @ JoinRefused::RulesDiffer { .. }) => {
                cx.log.warning(format!("cannot play the game: {why}"));
                cx.news(News::new(Category::System, msg(Phrase::RulesDiffer)));
                return Err(None);
            }
            Err(JoinRefused::Invalid(why)) => return Err(Some(why.0)),
        };
        let seat = SeatView::new(Arc::clone(stream.start()), Arc::clone(&m));
        let server = ServerGame::new(seated, stream, cx.clock.now());
        let mut game = GameInterface::on_server(cx, seat, server);
        if seated.you == Role::Watcher {
            // A watcher may join a game that is paused: the entities and
            // squares of the first message are taken at once, as at a
            // return.
            game.take_server_turn(cx, m, ServerTurn::End)
                .map_err(|why| Some(format!("the first message: {why}")))?;
            game.now_following(cx);
        }
        return Ok(game);
    }
}

impl Return {
    /// Moves the attempt on, without waiting: starts one once the pause is
    /// over, takes the new connection once it is open, then what the
    /// server answers to the token. `key` is the seat's, and `stream` its
    /// stream as of the last frame taken.
    fn attempt(&mut self, key: &SeatKey, stream: &SeatStream, cx: &mut Client) -> Outcome {
        let now = cx.clock.now();
        let retry = Attempt::Pause {
            at: now + RETRY_PAUSE,
        };
        let (next, outcome) = match std::mem::replace(&mut self.attempt, Attempt::Pause { at: now })
        {
            Attempt::Pause { at } if now < at => (Attempt::Pause { at }, Outcome::Waiting),
            Attempt::Pause { .. } => match &mut cx.net {
                Some(net) if now < self.until => (Attempt::Opening(net.redial()), Outcome::Waiting),
                _ => (retry, Outcome::Failed(Failure::TimeUp)),
            },
            Attempt::Opening(mut opening) => match opening.poll() {
                None => (Attempt::Opening(opening), Outcome::Waiting),
                Some(Err(e)) => {
                    cx.log.info(format!("cannot reach the server: {e:?}"));
                    (retry, Outcome::Waiting)
                }
                Some(Ok((conn, snapshot))) => self.welcomed(key, stream, conn, &snapshot, now),
            },
            Attempt::Presented(p) => p.answers(cx, now),
        };
        self.attempt = next;
        outcome
    }

    /// A new connection is open: presents the token, unless the game is
    /// over, on the server or for the seat's stream, or the server still has
    /// the player playing and has not noticed the old connection is lost:
    /// then the server's hold has not started yet, and the next attempt
    /// follows a pause.
    fn welcomed(
        &mut self,
        key: &SeatKey,
        stream: &SeatStream,
        mut conn: Connection,
        snapshot: &soundrts_protocol::LobbySnapshot,
        now: f64,
    ) -> (Attempt, Outcome) {
        let running = snapshot
            .rooms
            .iter()
            .any(|r| r.id == self.room && r.started);
        let returning = stream.returning();
        let (true, Some(returning)) = (running, returning) else {
            return (
                Attempt::Pause { at: now },
                Outcome::Failed(Failure::GameOver),
            );
        };
        let still_playing = snapshot
            .players
            .iter()
            .any(|p| p.login == self.login && p.whereabouts == Whereabouts::Playing(self.room));
        if still_playing {
            self.until = now + key.hold;
            let retry = Attempt::Pause {
                at: now + RETRY_PAUSE,
            };
            return (retry, Outcome::Waiting);
        }
        conn.send(ClientMsg::ReturnToGame {
            room: self.room,
            token: key.token.clone(),
        });
        let presented = Presented {
            conn,
            lobby: LobbyModel::new(snapshot),
            room: None,
            since: now,
            stream: returning,
        };
        (Attempt::Presented(Box::new(presented)), Outcome::Waiting)
    }
}

/// What came of one message the server sent after the token.
enum Answer {
    GoOn,
    /// The connection is lost: another attempt may follow.
    Lost,
    /// The return is over, without the seat.
    Over(Outcome),
    /// A frame of the seat's stream, for the stream to check.
    Frame(Box<SeatFrame>),
}

impl Presented {
    /// Takes what the server sent after the token, as much as has come:
    /// the seat's stream, and the lobby's messages meanwhile.
    fn answers(self: Box<Self>, cx: &mut Client, now: f64) -> (Attempt, Outcome) {
        let retry = Attempt::Pause {
            at: now + RETRY_PAUSE,
        };
        if now > self.since + ANSWER_WAIT {
            cx.log.info("the server did not answer the return in time");
            return (retry, Outcome::Waiting);
        }
        let mut p = *self;
        for _ in 0..PUMP_LIMIT {
            let Some(event) = p.conn.poll() else {
                break;
            };
            let frame = match p.answer(cx, event) {
                Answer::GoOn => continue,
                Answer::Lost => return (retry, Outcome::Waiting),
                Answer::Over(outcome) => return (retry, outcome),
                Answer::Frame(frame) => frame,
            };
            match p.stream.take(*frame) {
                Ok(game::Returning::More(next)) => p.stream = next,
                Ok(game::Returning::Back(back)) => {
                    let back = Reconnected {
                        conn: p.conn,
                        lobby: p.lobby,
                        room: p.room,
                        back,
                    };
                    return (retry, Outcome::Back(Box::new(back)));
                }
                Ok(game::Returning::Stopped(reason)) => return (retry, Outcome::Stopped(reason)),
                Err(why) => return (retry, Outcome::CannotBeRight(why.0)),
            }
        }
        (Attempt::Presented(Box::new(p)), Outcome::Waiting)
    }

    /// Takes one message the server sent after the token. A frame of the
    /// seat's stream is left for the stream, which checks it.
    fn answer(&mut self, cx: &mut Client, event: NetEvent) -> Answer {
        let wrong = |what: String| Answer::Over(Outcome::CannotBeRight(what));
        match event {
            NetEvent::Closed(reason) => {
                cx.log
                    .info(format!("the connection to return on closed: {reason}"));
                Answer::Lost
            }
            NetEvent::Msg(ServerMsg::Refused {
                reason: Refusal::NoSuchRoom,
            }) => Answer::Over(Outcome::Failed(Failure::GameOver)),
            NetEvent::Msg(ServerMsg::Refused { .. }) => {
                Answer::Over(Outcome::Failed(Failure::SeatGivenUp))
            }
            NetEvent::Msg(ServerMsg::Lobby { change }) => match self.lobby.apply(change, false) {
                Ok(_) => Answer::GoOn,
                Err(too_many) => wrong(too_many.to_string()),
            },
            NetEvent::Msg(ServerMsg::Room { room, .. }) => {
                if let Err(too_many) = check_watchers(room.watchers.len()) {
                    return wrong(too_many.to_string());
                }
                self.room = Some(room);
                Answer::GoOn
            }
            // Of the lobby, while the seat is not back yet.
            NetEvent::Msg(_) => Answer::GoOn,
            NetEvent::Frame(Err(e)) => wrong(format!("a frame: {e}")),
            NetEvent::Frame(Ok(frame)) => Answer::Frame(frame),
        }
    }
}

impl GameInterface {
    fn server(&mut self) -> Option<&mut ServerGame> {
        self.source.server_mut()
    }

    /// Whether the connection is lost and the client is returning to the
    /// seat.
    pub fn returning(&self) -> bool {
        self.source
            .server()
            .is_some_and(|s| matches!(s.link, Link::Returning(_)))
    }

    /// Plays from the stream, or goes on returning to the seat.
    pub(crate) fn server_update(&mut self, cx: &mut Client) {
        if self.returning() {
            self.return_update(cx);
        } else {
            self.stream_update(cx);
        }
    }

    /// Sends the commands given, as many as the server takes for one turn,
    /// then takes every frame the server sent, in order. The game ends if
    /// the server ended it, or a frame cannot be right; once the frames
    /// that came before a lost connection are taken, the client returns to
    /// the seat, if it can.
    fn stream_update(&mut self, cx: &mut Client) {
        let gone = cx
            .net
            .as_ref()
            .is_none_or(|n| n.closed.is_some() || n.game.is_none());
        if !gone {
            self.send_pending(cx);
        }
        self.tell_watcher_no_orders(cx);
        // What came before a lost connection is taken first.
        while let Some(frame) = crate::net::game_link(cx).and_then(|l| l.frames.pop_front()) {
            if let Err(why) = self.take_frame(cx, frame) {
                self.cannot_be_right(cx, &why);
                return;
            }
            if self.end_loop {
                return;
            }
        }
        let ended = gone || crate::net::game_link(cx).is_some_and(|l| l.ended);
        if ended && (gone || self.srv_queue.is_empty()) && !self.end_loop {
            match self.may_return(cx) {
                Some((room, login)) => self.start_return(cx, room, login),
                None => self.srv_quit(cx),
            }
        }
    }

    /// The room and the player's login, if the connection is lost while
    /// the player is still in a game the server has not ended for them, or
    /// still watches one.
    fn may_return(&mut self, cx: &mut Client) -> Option<(RoomId, Login)> {
        let playing = self.view().players().contains(&self.me);
        let server = self.server()?;
        let in_it = playing || server.watching();
        if !in_it || server.quit || !matches!(server.end, End::Waiting) {
            return None;
        }
        let net = cx.net.as_ref()?;
        let link = net.game.as_ref()?;
        if net.closed.is_none() || link.ended {
            return None;
        }
        let room = net.room.as_ref()?.state.id;
        Some((room, net.lobby.you.clone()))
    }

    /// The connection is lost: says so, stops the sounds that loop, gives
    /// up the orders not sent, and starts returning to the seat at once.
    fn start_return(&mut self, cx: &mut Client, room: RoomId, login: Login) {
        cx.log.info("the connection is lost: returning to the game");
        cx.news(News::new(Category::System, msg(Phrase::ConnectionLost)));
        self.silence_noises(cx);
        self.pending_commands.clear();
        let now = cx.clock.now();
        if let Some(server) = self.server() {
            server.link = Link::Returning(Box::new(Return {
                room,
                login,
                until: now + server.seat.hold,
                attempt: Attempt::Pause { at: now },
                orders: Orders::NoneGiven,
                unsent: Orders::NoneGiven,
            }));
        }
    }

    /// A command given while the client returns to the seat: not taken.
    /// Whether it was given then.
    pub(crate) fn refuse_while_returning(&mut self) -> bool {
        match self.server().map(|s| &mut s.link) {
            Some(Link::Returning(r)) => {
                if r.orders == Orders::NoneGiven {
                    r.orders = Orders::Given;
                }
                true
            }
            _ => false,
        }
    }

    /// A chat line or a flare while the client returns to the seat: not
    /// sent. Whether it was given then.
    fn refuse_unsent_while_returning(&mut self) -> bool {
        match self.server().map(|s| &mut s.link) {
            Some(Link::Returning(r)) => {
                if r.unsent == Orders::NoneGiven {
                    r.unsent = Orders::Given;
                }
                true
            }
            _ => false,
        }
    }

    /// Sends a chat line typed in the game, unless the connection is lost:
    /// then the player hears once that it was not sent.
    pub(crate) fn send_chat(&mut self, cx: &mut Client, channel: Channel, line: &str) {
        if !self.refuse_unsent_while_returning() {
            crate::net::send_chat(cx, channel, line);
        }
    }

    /// Leaves the game at once while it is paused on a server: the world
    /// takes a quit only at a turn, and none comes. Whether it is paused.
    pub(crate) fn leave_while_paused(&mut self, cx: &mut Client) -> bool {
        if !self.server().is_some_and(|s| s.paused) {
            return false;
        }
        cx.log.info("left the game while it was paused");
        self.srv_quit(cx);
        self.forced_quit = true;
        true
    }

    /// Leaves the game at once while the client returns to the seat.
    /// Whether it was returning.
    pub(crate) fn leave_while_returning(&mut self, cx: &mut Client) -> bool {
        if !self.returning() {
            return false;
        }
        cx.log.info("left the game while returning to it");
        self.srv_quit(cx);
        self.forced_quit = true;
        true
    }

    /// One step of the return: says once that orders are not taken, and
    /// once that a chat line or a flare was not sent, then moves the
    /// attempt on.
    fn return_update(&mut self, cx: &mut Client) {
        let Some(server) = self.source.server_mut() else {
            return;
        };
        let Link::Returning(r) = &mut server.link else {
            return;
        };
        if r.orders == Orders::Given {
            r.orders = Orders::Told;
            cx.voice.item(&msg(Phrase::OrderNotTaken));
        }
        if r.unsent == Orders::Given {
            r.unsent = Orders::Told;
            cx.voice.item(&msg(Phrase::NotSentNotConnected));
        }
        match r.attempt(&server.seat, &server.stream, cx) {
            Outcome::Waiting => {}
            Outcome::Back(back) => self.back(cx, *back),
            Outcome::Failed(why) => {
                cx.log.info(format!("cannot return to the game: {why:?}"));
                cx.news(News::new(Category::System, words::return_failed(why)));
                self.srv_quit(cx);
            }
            Outcome::CannotBeRight(why) => self.cannot_be_right(cx, &format!("on return: {why}")),
            Outcome::Stopped(reason) => self.stopped(cx, reason),
        }
    }

    /// The seat is back: the whole message the server sent replaces the
    /// view, the clock is the server's, and the new connection carries the
    /// stream from now on.
    fn back(&mut self, cx: &mut Client, back: Reconnected) {
        let Reconnected {
            conn,
            lobby,
            room,
            back:
                game::Back {
                    stream,
                    message,
                    seated,
                },
        } = back;
        let mut message = Arc::unwrap_or_clone(message);
        let last = self.view().message();
        // The events of a turn already taken were told then.
        if message.time == last.time {
            message.events.clear();
        }
        let new = message
            .state
            .objectives
            .iter()
            .filter(|o| !last.state.objectives.contains(o))
            .count();
        let now = cx.clock.now();
        let Some(server) = self.server() else {
            return;
        };
        server.objectives_given = server.objectives_given.saturating_add(new);
        if server.objectives_given > MAX_OBJECTIVES_SEEN {
            return self.cannot_be_right(cx, "on return: more objectives than a game gives");
        }
        server.speed = seated.speed;
        server.paused = seated.paused;
        server.seat = SeatKey::of(seated.hold, seated.token);
        server.stream = stream;
        server.next_turn = server.after_a_turn(now);
        server.link = Link::Playing { sent_since_turn: 0 };
        let you = server.stream.role();
        let start = Arc::clone(server.stream.start());
        self.speed = server.speed_factor();
        // The time away is no turn's.
        self.previous_update_time = None;
        if let Some(net) = &mut cx.net {
            net.returned(conn, lobby, you);
            if let (Some(state), Some(r)) = (room, &mut net.room)
                && state.id == r.state.id
            {
                r.state = state;
            }
        }
        cx.log.info("back in the game");
        cx.news(News::new(Category::System, msg(Phrase::BackInTheGame)));
        let m = Arc::new(message);
        // A watcher moved on to another player meanwhile follows that one.
        let how = if m.seat == self.me {
            ServerTurn::End
        } else {
            ServerTurn::Follow(start)
        };
        if let Err(why) = self.take_server_turn(cx, m, how) {
            self.cannot_be_right(cx, &format!("on return: {why}"));
        }
    }

    /// Takes a turn the server sent: the only way one enters the
    /// interface. It is refused, before anything of it is taken, when its
    /// events and its end, with the events waiting and the units replaced
    /// that the interface keeps, would be more than [`MAX_WAITING_EVENTS`]:
    /// so the interface never keeps more than that.
    pub(super) fn take_server_turn(
        &mut self,
        cx: &mut Client,
        m: Arc<TurnMessage>,
        how: ServerTurn,
    ) -> Result<(), String> {
        let kept = match how {
            // The watcher starts again from the message.
            ServerTurn::Follow(_) => 0,
            ServerTurn::End | ServerTurn::Change => {
                self.srv_queue.len() + self.known.replacements()
            }
        };
        if kept.saturating_add(m.events.len()) >= MAX_WAITING_EVENTS {
            return Err("more events than the game can tell".into());
        }
        match how {
            ServerTurn::End => self.take_turn(Arc::clone(&m), Some(m), Vec::new()),
            ServerTurn::Change => {
                let mut m = Arc::unwrap_or_clone(m);
                let then = std::mem::take(&mut m.events);
                self.take_turn(Arc::new(m), None, then);
            }
            ServerTurn::Follow(start) => self.follow(cx, start, m),
        }
        Ok(())
    }

    /// The server stopped the game: says why, and leaves it.
    fn stopped(&mut self, cx: &mut Client, reason: StopReason) {
        cx.log
            .warning(format!("the server stopped the game: {reason:?}"));
        cx.news(News::new(Category::System, words::stopped(reason)));
        if let Some(link) = crate::net::game_link(cx) {
            link.ended = true;
        }
        self.srv_quit(cx);
    }

    /// Ends the game on a frame that cannot be right: says so, and leaves
    /// the game.
    fn cannot_be_right(&mut self, cx: &mut Client, why: &str) {
        cx.log
            .warning(format!("the server's stream cannot be right: {why}"));
        cx.news(News::new(
            Category::System,
            msg(Phrase::ServerSentWhatCannotBeRight),
        ));
        self.srv_quit(cx);
    }

    /// Sends the commands given since the last call, as many as fit in
    /// what the server takes for one turn; the others wait for the next
    /// turn. A command that cannot be sent is dropped. A speed is the room
    /// host's to ask for ([`GameInterface::set_server_speed`]), not a
    /// command.
    fn send_pending(&mut self, cx: &mut Client) {
        let Some(&mut ServerGame {
            link: Link::Playing { sent_since_turn },
            ..
        }) = self.server()
        else {
            return;
        };
        // Commands sent just before a turn came may still wait at the server
        // for the next one: half of what it takes leaves room for them.
        let budget = MAX_TURN_ORDER_BYTES / 2;
        let mut sent = sent_since_turn;
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
        if let Some(ServerGame {
            link: Link::Playing { sent_since_turn },
            ..
        }) = self.server()
        {
            *sent_since_turn = sent;
        }
        for bytes in out {
            crate::net::send_command(cx, bytes);
        }
        for s in speeds {
            self.set_server_speed(cx, s);
        }
    }

    /// Takes one frame of the stream, once the seat's stream has checked
    /// it. `Err` says why it cannot be right.
    fn take_frame(&mut self, cx: &mut Client, frame: Incoming) -> Result<(), String> {
        let frame = frame.map_err(|e| format!("a frame cannot be read: {e}"))?;
        let Some(server) = self.server() else {
            return Err("a frame of a game played here".into());
        };
        let start = Arc::clone(server.stream.start());
        match server.stream.take(*frame).map_err(|e| e.0)? {
            Checked::Turn(m) => self.take_change(cx, m),
            Checked::Follow(m) => self.take_server_turn(cx, m, ServerTurn::Follow(start)),
            Checked::End(end) => {
                let mut quit = false;
                if let Some(server) = self.server() {
                    server.end = End::Came(end);
                    // A watcher's game ends when its end comes.
                    quit = server.quit || server.watching();
                }
                // The server has taken the player out of the game.
                if let Some(link) = crate::net::game_link(cx) {
                    link.ended = true;
                }
                if quit {
                    self.srv_quit(cx);
                }
                Ok(())
            }
            Checked::Seat { slot, change } => {
                let who = cx
                    .net
                    .as_ref()
                    .and_then(|n| n.room.as_ref())
                    .and_then(|r| r.state.slots.get(slot))
                    .map(|s| player_name(&s.player))
                    .ok_or_else(|| format!("news of slot {slot}, not in the room"))?;
                cx.news(News::new(Category::System, words::seat_news(who, &change)));
                Ok(())
            }
            Checked::Host { slot } => {
                let room = cx
                    .net
                    .as_mut()
                    .and_then(|n| n.room.as_mut())
                    .ok_or("news of the host, not in a room")?;
                let Some(SlotPlayer::Human(host)) = room.state.slots.get(slot).map(|s| &s.player)
                else {
                    return Err(format!("the host's powers to slot {slot}, no player's"));
                };
                room.state.host = host.clone();
                let m = words::new_host(host);
                let topic = Subject::Room(RoomTopic::Host);
                cx.news(News::new(Category::System, m).about(topic));
                Ok(())
            }
            Checked::Flare { from, square } => {
                self.flare_alert(cx, from, square);
                Ok(())
            }
            Checked::Clock { by, change } => {
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
                let m = words::clock_news(&by, change);
                cx.news(News::new(Category::System, m).about(Subject::Clock));
                Ok(())
            }
            Checked::Stopped(reason) => {
                self.stopped(cx, reason);
                Ok(())
            }
        }
    }

    /// Takes a turn's change, applied and checked: the seat's new message,
    /// current from now on. A change of a later time while the player is in
    /// the game is the end of a turn; one of the same time as the last, or
    /// after the player left the game, is what followed the end of their
    /// turn.
    fn take_change(&mut self, cx: &mut Client, m: Arc<TurnMessage>) -> Result<(), String> {
        let last = self.view().message();
        let before = &last.state.objectives;
        let new = m
            .state
            .objectives
            .iter()
            .filter(|o| !before.contains(o))
            .count();
        let end_of_turn = m.time > last.time && m.state.players.contains(&self.me);
        if let Some(server) = self.server() {
            server.objectives_given = server.objectives_given.saturating_add(new);
            if server.objectives_given > MAX_OBJECTIVES_SEEN {
                return Err("more objectives than a game gives".into());
            }
        }
        if end_of_turn {
            let now = cx.clock.now();
            if let Some(server) = self.server() {
                if let Link::Playing { sent_since_turn } = &mut server.link {
                    *sent_since_turn = 0;
                }
                server.next_turn = server.after_a_turn(now);
            }
            self.take_server_turn(cx, m, ServerTurn::End)
        } else {
            self.take_server_turn(cx, m, ServerTurn::Change)
        }
    }

    /// Asks the server to follow the next player still in the game, or
    /// the previous one, for a watcher; the server sends that player's
    /// whole message.
    pub(crate) fn ask_server_to_follow(&mut self, cx: &mut Client, step: Step) {
        if self.refuse_while_returning() {
            return;
        }
        crate::net::send(cx, ClientMsg::Follow { step });
    }

    /// Points the allies at the square under the cursor: the server tells
    /// them, and tells the player once it has. Off a server, or with no ally
    /// still in the game, says why and sends nothing.
    pub(crate) fn flare(&mut self, cx: &mut Client) {
        if !self.on_a_server() {
            cx.voice.item(&msg(Phrase::NobodyToTell));
            return;
        }
        if self.refuse_while_watching() || self.refuse_unsent_while_returning() {
            return;
        }
        let Some(square) = self.place else {
            cx.voice.item(&cue(Cue::Beep));
            return;
        };
        let (me, view) = (self.me, self.view());
        if !view.players().iter().any(|&p| p != me && view.is_ally(p)) {
            cx.voice.item(&msg(Phrase::NoAlly));
            return;
        }
        crate::net::send(cx, ClientMsg::Flare { square });
    }

    /// Whether the game is paused, for the game menu of the room's host in
    /// a game on a server; `None` for anyone else.
    pub(crate) fn paused_for_the_host(&mut self, cx: &Client) -> Option<bool> {
        let paused = self.server()?.paused;
        is_room_host(cx).then_some(paused)
    }

    /// Asks the server to pause the game, or to resume it if it is
    /// paused: the room's host only. The server tells every player.
    pub(crate) fn pause_or_resume(&mut self, cx: &mut Client) {
        if self.refuse_while_returning() {
            return;
        }
        let Some(server) = self.server() else {
            return;
        };
        let m = if server.paused {
            ClientMsg::ResumeGame
        } else {
            ClientMsg::PauseGame
        };
        crate::net::send(cx, m);
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
            cx.voice.item(&msg(Phrase::OnlyTheOrganizerSetsTheSpeed));
        }
    }
}
