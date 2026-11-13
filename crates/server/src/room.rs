//! A room: its slots, its host, and whether its game has started.

use std::collections::BTreeSet;

use soundrts_protocol::{
    AiLevel, ContentHash, Faction, Login, MapInfo, RoomChange, RoomId, RoomState, RoomSummary,
    ServerMsg, Slot, SlotIndex, SlotPlayer, Speed,
};
use tokio::sync::mpsc;

use crate::connection::{ConnId, Outbox, encode};
use crate::game::GameInput;

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
}

pub(crate) enum Phase {
    Gathering {
        /// Who may join a closed room.
        invited: BTreeSet<ConnId>,
    },
    /// The game runs; the slots no longer change.
    Playing {
        game: mpsc::UnboundedSender<GameInput>,
    },
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

impl Room {
    pub fn new(
        id: RoomId,
        host: Member,
        map: MapInfo,
        speed: Speed,
        open: bool,
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
        }
    }

    /// Tells every member what changed.
    pub fn notify(&self, change: RoomChange) {
        let text = encode(&ServerMsg::Room {
            change,
            room: self.state(),
        });
        for (_, m) in self.members() {
            m.outbox.send_text(text.clone());
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
