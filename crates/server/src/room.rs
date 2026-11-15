//! A room: its slots, its host, its watchers, and whether its game has
//! started.

use std::collections::BTreeSet;

use crate::connection::{ConnId, Outbox, encode};
use crate::game::{GameTx, SeatId, WatcherId};
use soundrts_protocol::{
    AiLevel, ContentHash, Faction, Login, MapInfo, RoomChange, RoomId, RoomState, RoomSummary,
    ServerMsg, Slot, SlotIndex, SlotPlayer, Speed,
};

pub(crate) struct Room {
    pub id: RoomId,
    /// Always the connection of a human in `slots`.
    pub host: ConnId,
    pub map: MapInfo,
    pub speed: Speed,
    pub open: bool,
    pub content: ContentHash,
    pub slots: Vec<RoomSlot>,
    pub phase: Phase,
    /// Whether people may join only to watch.
    pub takes_watchers: bool,
    /// Who watches, in the order they came.
    pub watchers: Vec<Watcher>,
    next_watcher: u32,
}

pub(crate) enum Phase {
    Gathering {
        /// Who may join a closed room.
        invited: BTreeSet<ConnId>,
    },
    /// The server runs the game; the slots no longer change, but who holds
    /// a human slot does when a player returns.
    Playing { game: GameTx },
}

pub(crate) struct RoomSlot {
    pub occupant: Occupant,
    pub faction: Faction,
    pub alliance: u8,
}

pub(crate) enum Occupant {
    Human(Member),
    Computer(AiLevel),
}

pub(crate) struct Member {
    pub conn: ConnId,
    pub login: Login,
    pub outbox: Outbox,
    /// What the player said. The host is ready whatever this says.
    pub ready: bool,
}

/// Someone in the room, or its game, only to watch: in no slot.
pub(crate) struct Watcher {
    pub id: WatcherId,
    pub conn: ConnId,
    pub login: Login,
    pub outbox: Outbox,
}

impl Room {
    pub fn new(
        id: RoomId,
        host: Member,
        map: MapInfo,
        speed: Speed,
        open: bool,
        takes_watchers: bool,
        content: ContentHash,
    ) -> Room {
        let mut room = Room {
            id,
            host: host.conn,
            map,
            speed,
            open,
            content,
            slots: Vec::new(),
            phase: Phase::Gathering {
                invited: BTreeSet::new(),
            },
            takes_watchers,
            watchers: Vec::new(),
            next_watcher: 0,
        };
        room.add(Occupant::Human(host));
        room
    }

    /// Adds a slot with the lowest alliance no other slot uses and a
    /// random faction.
    pub fn add(&mut self, occupant: Occupant) -> SlotIndex {
        let alliance = (1..)
            .find(|a| self.slots.iter().all(|s| s.alliance != *a))
            .expect("fewer than 255 slots");
        self.slots.push(RoomSlot {
            occupant,
            faction: Faction::random(),
            alliance,
        });
        self.slots.len() - 1
    }

    pub fn is_full(&self) -> bool {
        self.slots.len() >= usize::from(self.map.nb_players_max)
    }

    pub fn is_started(&self) -> bool {
        matches!(self.phase, Phase::Playing { .. })
    }

    pub fn members(&self) -> impl Iterator<Item = (SlotIndex, &Member)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match &s.occupant {
                Occupant::Human(m) => Some((i, m)),
                Occupant::Computer(_) => None,
            })
    }

    pub fn slot_of(&self, conn: ConnId) -> Option<SlotIndex> {
        self.members().find(|(_, m)| m.conn == conn).map(|(i, _)| i)
    }

    /// Adds a watcher, who comes last.
    pub fn add_watcher(&mut self, conn: ConnId, login: Login, outbox: Outbox) -> WatcherId {
        let id = WatcherId(self.next_watcher);
        self.next_watcher = self.next_watcher.wrapping_add(1);
        self.watchers.push(Watcher {
            id,
            conn,
            login,
            outbox,
        });
        id
    }

    pub fn watcher_of(&self, conn: ConnId) -> Option<WatcherId> {
        self.watchers.iter().find(|w| w.conn == conn).map(|w| w.id)
    }

    pub fn watcher(&mut self, id: WatcherId) -> Option<&mut Watcher> {
        self.watchers.iter_mut().find(|w| w.id == id)
    }

    pub fn remove_watcher(&mut self, id: WatcherId) -> Option<Watcher> {
        let i = self.watchers.iter().position(|w| w.id == id)?;
        Some(self.watchers.remove(i))
    }

    /// The seat of this connection: a watcher's, or a player's slot. A
    /// player whose game is over keeps the slot in a room that plays, and
    /// may come back to watch: then the watcher's seat is theirs.
    pub fn seat_of(&self, conn: ConnId) -> Option<SeatId> {
        self.watcher_of(conn)
            .map(SeatId::Watcher)
            .or_else(|| self.slot_of(conn).map(SeatId::Player))
    }

    /// Everyone in the room, players and watchers, each connection once:
    /// a player whose game is over may also watch it.
    pub fn everyone(&self) -> impl Iterator<Item = (ConnId, &Outbox)> {
        let members = self
            .members()
            .map(|(_, m)| (m.conn, &m.outbox))
            .filter(|(c, _)| self.watcher_of(*c).is_none());
        let watchers = self.watchers.iter().map(|w| (w.conn, &w.outbox));
        members.chain(watchers)
    }

    /// Everyone in the room but `but`.
    fn everyone_but(&self, but: Option<ConnId>) -> impl Iterator<Item = &Outbox> {
        self.everyone()
            .filter(move |(c, _)| Some(*c) != but)
            .map(|(_, o)| o)
    }

    pub fn host_login(&self) -> Login {
        self.members()
            .find(|(_, m)| m.conn == self.host)
            .map(|(_, m)| m.login.clone())
            .expect("the host is a member")
    }

    pub fn is_ready(&self, slot: &RoomSlot) -> bool {
        match &slot.occupant {
            Occupant::Human(m) => m.conn == self.host || m.ready,
            Occupant::Computer(_) => true,
        }
    }

    pub fn slots(&self) -> Vec<Slot> {
        self.slots
            .iter()
            .map(|s| Slot {
                player: s.player(),
                faction: s.faction.clone(),
                alliance: s.alliance,
                ready: self.is_ready(s),
            })
            .collect()
    }

    pub fn state(&self) -> RoomState {
        RoomState {
            id: self.id,
            host: self.host_login(),
            map: self.map.clone(),
            speed: self.speed,
            open: self.open,
            content: self.content.clone(),
            slots: self.slots(),
            takes_watchers: self.takes_watchers,
            watchers: self.watchers.iter().map(|w| w.login.clone()).collect(),
        }
    }

    pub fn summary(&self) -> RoomSummary {
        RoomSummary {
            id: self.id,
            host: self.host_login(),
            map: self.map.clone(),
            speed: self.speed,
            open: self.open,
            content: self.content.clone(),
            players: self.slots.iter().map(RoomSlot::player).collect(),
            started: self.is_started(),
            takes_watchers: self.takes_watchers,
        }
    }

    /// Tells everyone in the room what changed, players and watchers.
    pub fn notify(&self, change: RoomChange) {
        self.notify_but(None, change);
    }

    /// Tells everyone in the room but `but` what changed.
    pub fn notify_but(&self, but: Option<ConnId>, change: RoomChange) {
        let text = encode(&ServerMsg::Room {
            change,
            room: self.state(),
        });
        for outbox in self.everyone_but(but) {
            outbox.send_text(text.clone());
        }
    }
}

impl RoomSlot {
    pub fn player(&self) -> SlotPlayer {
        match &self.occupant {
            Occupant::Human(m) => SlotPlayer::Human(m.login.clone()),
            Occupant::Computer(level) => SlotPlayer::Computer(*level),
        }
    }
}
