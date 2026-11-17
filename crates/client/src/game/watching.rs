//! Watching a game: hearing one player's side at a time, without giving
//! orders. A watcher of a game on a server ([`super::server`]) and a
//! listener to a recorded game ([`super::replay`]) are the same watcher;
//! only where the turns come from differs, and what changes the player
//! followed: the server sends that player's whole message, or the replay
//! gives it.
//!
//! The interface is the followed player's, as if it had just joined the
//! game: every question key, the cursor, the log and the news work on
//! what that player is told. An order is not given, and the watcher hears
//! it once.

use soundrts_world::msg::{Phrase, msg};
use std::sync::Arc;

use soundrts_world::{GameStart, SeatView, TurnMessage};

use super::GameInterface;
use crate::client::Client;
use crate::message::{Category, News};

/// Whether the player gave orders (or sent something) where none is
/// taken, and heard that they are not taken.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orders {
    NoneGiven,
    Given,
    Told,
}

impl GameInterface {
    /// A command given by a watcher: not taken, and it hears so once.
    /// Whether the player watches.
    pub(crate) fn refuse_while_watching(&mut self) -> bool {
        match self.watcher_orders() {
            Some(orders) => {
                if *orders == Orders::NoneGiven {
                    *orders = Orders::Given;
                }
                true
            }
            None => false,
        }
    }

    /// Says once that a watcher gives no orders, after the first one given.
    pub(crate) fn tell_watcher_no_orders(&mut self, cx: &mut Client) {
        if let Some(orders) = self.watcher_orders()
            && *orders == Orders::Given
        {
            *orders = Orders::Told;
            cx.voice.item(&msg(Phrase::WatchingNoOrders));
        }
    }

    /// Leaves the game at once while the player watches: a watcher has
    /// no orders to quit with. Whether it watches.
    pub(crate) fn leave_while_watching(&mut self, cx: &mut Client) -> bool {
        if !self.is_watching() {
            return false;
        }
        cx.log.info("stopped watching the game");
        self.srv_quit(cx);
        self.forced_quit = true;
        true
    }

    /// The watcher follows the player of `m`, that player's whole message
    /// in the game of `start`: from now on the interface is that player's,
    /// as if it had just joined the game, and the watcher hears whose side
    /// it is.
    pub(crate) fn follow(&mut self, cx: &mut Client, start: Arc<GameStart>, m: Arc<TurnMessage>) {
        self.restart_from(cx, start, m);
        self.now_following(cx);
    }

    /// The interface starts again from `m`, a whole message of the player
    /// it follows from now on in the game of `start`, as if it had just
    /// joined the game. It keeps only what is its own: the keys, the mode,
    /// the log, the clock and the settings of the listener; what it knew of
    /// the game, it learns again from `m`, whose end of turn waits in the
    /// queue.
    ///
    /// `m` is trusted: on a server, the seat's stream checked it
    /// ([`soundrts_protocol::game::SeatStream`]); in a recorded game, the
    /// world replayed here made it, and a listener may go back in time.
    pub(crate) fn restart_from(
        &mut self,
        cx: &mut Client,
        start: Arc<GameStart>,
        m: Arc<TurnMessage>,
    ) {
        let objectives = m.state.objectives.len();
        self.silence_noises(cx);
        // Every field is named, so that a new one is given a thought: what
        // the interface knew of the last player's side goes.
        let GameInterface {
            source,
            me,
            seat,
            speed: _,
            dobjets,
            scouted_squares,
            scouted_before_squares,
            last_virtual_time: _,
            turns_handled: _,
            place,
            target,
            zoom,
            order,
            group,
            immersion: _,
            follow_mode: _,
            x: _,
            y: _,
            o: _,
            mode,
            previous_order,
            previous_group,
            previous_compass,
            previous_menus,
            attacks,
            scout_info,
            known_resource_places,
            log_start: _,
            alert_jump,
            objectives_seen,
            objective_at,
            previous_resources,
            previous_available_food,
            previous_used_food,
            bell_enabled: _,
            previous_nb_minutes: _,
            must_play_tick: _,
            average_turn_duration: _,
            previous_update_time: _,
            end_loop: _,
            already_asked_to_quit: _,
            forced_quit: _,
            terrain_noises,
            previous_animation: _,
            bindings: _,
            srv_queue,
            pending_commands,
            orders_list: _,
        } = self;
        *me = m.seat;
        *seat = SeatView::new(start, Arc::clone(&m));
        dobjets.clear();
        scouted_squares.clear();
        scouted_before_squares.clear();
        *place = None;
        *target = None;
        *zoom = None;
        *order = None;
        group.clear();
        if matches!(mode, super::Mode::OrderShortcut) {
            *mode = super::Mode::Playing;
        }
        *previous_order = None;
        *previous_group = None;
        *previous_compass = None;
        previous_menus.clear();
        attacks.clear();
        scout_info.clear();
        known_resource_places.clear();
        cx.voice.new_side();
        *alert_jump = None;
        objectives_seen.clear();
        *objective_at = None;
        *previous_resources = None;
        *previous_available_food = 0;
        *previous_used_food = 0;
        terrain_noises.clear();
        srv_queue.clear();
        pending_commands.clear();
        source.followed(objectives);
        // Its end of turn brings the interface's copies of the entities
        // and squares to the new player's.
        self.take_turn(Arc::clone(&m), Some(m), Vec::new());
    }

    /// Says whose side the watcher now hears, and puts the cursor on the
    /// square of that player's first unit, without saying it.
    pub(crate) fn now_following(&mut self, cx: &mut Client) {
        let me = self.me;
        let square = self
            .view()
            .own_units()
            .find_map(|u| self.outer_square(u.place));
        if square.is_some() {
            self.place = square;
        }
        let name = self.view().player_name(me);
        cx.log.info(format!("watching player {}", me.0));
        cx.news(News::new(
            Category::System,
            [msg(Phrase::Watching), name].concat(),
        ));
    }
}
