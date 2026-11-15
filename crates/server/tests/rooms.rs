//! Creating rooms and maps, joining, the host's settings, and starting.

mod common;

use common::*;
use soundrts_protocol::game::{SeatFrame, decode_frame};
use soundrts_protocol::*;

fn slot(player: SlotPlayer, alliance: u8, ready: bool) -> Slot {
    Slot {
        player,
        faction: Faction::random(),
        alliance,
        ready,
    }
}

#[tokio::test]
async fn creating_a_room_asks_for_the_map_once() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let (info, data) = map(2, 4);
    let (andrew, bob) = (all[0].login.clone(), all[1].login.clone());
    let create = ClientMsg::CreateRoom {
        map: info.clone(),
        speed: Speed::Normal,
        open: true,
    };

    // The wrong file is refused, and the room is not created.
    all[0].send(create.clone()).await;
    all[0]
        .expect(ServerMsg::SendMap {
            hash: info.hash.clone(),
        })
        .await;
    let (_, other) = map(3, 4);
    all[0].send(ClientMsg::Map { map: other }).await;
    all[0].expect(refused(Refusal::WrongMap)).await;
    all[0].send(ClientMsg::Map { map: data.clone() }).await;
    all[0].expect(refused(Refusal::NotNow)).await;

    all[0].send(create).await;
    all[0]
        .expect(ServerMsg::SendMap {
            hash: info.hash.clone(),
        })
        .await;
    all[0].send(ClientMsg::Map { map: data.clone() }).await;
    let state = RoomState {
        id: RoomId(1),
        host: andrew.clone(),
        map: info.clone(),
        speed: Speed::Normal,
        open: true,
        content: content(),
        slots: vec![slot(human(&andrew), 1, true)],
    };
    all[0]
        .expect(ServerMsg::Room {
            change: RoomChange::Entered,
            room: state,
        })
        .await;
    let summary = RoomSummary {
        id: RoomId(1),
        host: andrew.clone(),
        map: info.clone(),
        speed: Speed::Normal,
        open: true,
        content: content(),
        players: vec![human(&andrew)],
        started: false,
    };
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomOpened {
            room: summary.clone(),
        }))
        .await;
        c.expect(moved(&andrew, Whereabouts::InRoom(RoomId(1))))
            .await;
    }

    // The server has the map now: Bob's room on it opens at once.
    all[1]
        .send(ClientMsg::CreateRoom {
            map: info.clone(),
            speed: Speed::Fast2,
            open: false,
        })
        .await;
    let msg = all[1].recv().await;
    assert!(
        matches!(
            &msg,
            ServerMsg::Room { change: RoomChange::Entered, room } if room.id == RoomId(2) && room.host == bob
        ),
        "{msg:?}"
    );
}

#[tokio::test]
async fn a_map_nobody_asked_for_is_refused() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    c.send(ClientMsg::Map { map: map(2, 4).1 }).await;
    c.expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_map_that_does_not_suit_any_number_of_players_is_refused() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    for (min, max) in [(0, 2), (3, 2)] {
        let (mut info, _) = map(2, 4);
        info.nb_players_min = min;
        info.nb_players_max = max;
        c.send(ClientMsg::CreateRoom {
            map: info,
            speed: Speed::Normal,
            open: true,
        })
        .await;
        c.expect(refused(Refusal::Invalid)).await;
    }
}

#[tokio::test]
async fn a_map_title_with_too_many_sounds_is_refused() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    let (mut info, _) = map(2, 4);
    info.title_sounds = vec![1; MAX_TITLE_SOUNDS + 1];
    c.send(ClientMsg::CreateRoom {
        map: info.clone(),
        speed: Speed::Normal,
        open: true,
    })
    .await;
    c.expect(refused(Refusal::Invalid)).await;

    info.title_sounds = vec![1; MAX_TITLE_SOUNDS];
    c.send(ClientMsg::CreateRoom {
        map: info.clone(),
        speed: Speed::Normal,
        open: true,
    })
    .await;
    c.expect(ServerMsg::SendMap { hash: info.hash }).await;
}

#[tokio::test]
async fn a_rooms_map_goes_to_its_players_and_those_invited_while_the_room_lasts() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let (info, data) = map(2, 4);
    let room = open_room(&mut all, 0, &(info.clone(), data.clone()), false).await;
    let get = || ClientMsg::GetMap {
        hash: info.hash.clone(),
    };

    // Not invited yet: as if the server had no such map.
    all[1].send(get()).await;
    all[1].expect(refused(Refusal::WrongMap)).await;
    all[0]
        .send(ClientMsg::Invite {
            login: login("bob"),
        })
        .await;
    all[1]
        .expect(ServerMsg::Invited {
            room,
            by: login("andrew"),
        })
        .await;
    all[1].send(get()).await;
    all[1].expect(ServerMsg::Map { map: data.clone() }).await;
    all[0].send(get()).await;
    all[0].expect(ServerMsg::Map { map: data }).await;
    all[2].send(get()).await;
    all[2].expect(refused(Refusal::WrongMap)).await;
    all[1]
        .send(ClientMsg::GetMap {
            hash: map(3, 4).0.hash,
        })
        .await;
    all[1].expect(refused(Refusal::WrongMap)).await;

    all[0].send(ClientMsg::LeaveRoom).await;
    all[0]
        .expect(ServerMsg::RoomExited {
            reason: RoomExit::Left,
        })
        .await;
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomClosed { id: room })).await;
        c.expect(moved(&login("andrew"), Whereabouts::Lobby)).await;
    }
    all[1].send(get()).await;
    all[1].expect(refused(Refusal::WrongMap)).await;
}

#[tokio::test]
async fn maps_asked_for_are_rate_limited() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    let hash = map(2, 4).0.hash;
    // The server's MAP_TRANSFERS in a minute.
    for _ in 0..10 {
        c.send(ClientMsg::GetMap { hash: hash.clone() }).await;
        c.expect(refused(Refusal::WrongMap)).await;
    }
    c.send(ClientMsg::GetMap { hash }).await;
    c.expect(refused(Refusal::SlowDown)).await;
}

#[tokio::test]
async fn a_map_larger_than_any_other_message_is_taken() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew"]).await;
    // Twice what a connection may have waiting in other messages.
    open_room(&mut all, 0, &large_map(2 * 1024 * 1024), true).await;
}

#[tokio::test]
async fn the_number_of_rooms_is_capped() {
    let server = server(|c| c.max_rooms = 1);
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let m = map(2, 4);
    open_room(&mut all, 0, &m, true).await;
    all[1]
        .send(ClientMsg::CreateRoom {
            map: m.0.clone(),
            speed: Speed::Normal,
            open: true,
        })
        .await;
    all[1].expect(refused(Refusal::TooManyRooms)).await;
}

#[tokio::test]
async fn joining_an_open_room() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let (info, data) = map(2, 4);
    let room = open_room(&mut all, 0, &(info.clone(), data), true).await;
    let (andrew, bob) = (all[0].login.clone(), all[1].login.clone());

    all[1].send(ClientMsg::JoinRoom { room }).await;
    let state = RoomState {
        id: room,
        host: andrew.clone(),
        map: info.clone(),
        speed: Speed::Fast4,
        open: true,
        content: content(),
        slots: vec![slot(human(&andrew), 1, true), slot(human(&bob), 2, false)],
    };
    all[1]
        .expect(ServerMsg::Room {
            change: RoomChange::Entered,
            room: state.clone(),
        })
        .await;
    all[0]
        .expect(ServerMsg::Room {
            change: RoomChange::Joined {
                player: human(&bob),
            },
            room: state,
        })
        .await;
    let summary = RoomSummary {
        id: room,
        host: andrew.clone(),
        map: info,
        speed: Speed::Fast4,
        open: true,
        content: content(),
        players: vec![human(&andrew), human(&bob)],
        started: false,
    };
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomChanged {
            room: summary.clone(),
        }))
        .await;
        c.expect(moved(&bob, Whereabouts::InRoom(room))).await;
    }
    // A player in a room cannot join or create another.
    all[1].send(ClientMsg::JoinRoom { room }).await;
    all[1].expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_closed_room_takes_only_invited_players() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), false).await;
    let andrew = all[0].login.clone();

    all[1].send(ClientMsg::JoinRoom { room }).await;
    all[1].expect(refused(Refusal::NotNow)).await;

    // Only the host invites, and only players on the server.
    all[2]
        .send(ClientMsg::Invite {
            login: login("bob"),
        })
        .await;
    all[2].expect(refused(Refusal::NotNow)).await;
    all[0]
        .send(ClientMsg::Invite {
            login: login("nobody"),
        })
        .await;
    all[0].expect(refused(Refusal::NoSuchPlayer)).await;

    all[0]
        .send(ClientMsg::Invite {
            login: login("bob"),
        })
        .await;
    all[1].expect(ServerMsg::Invited { room, by: andrew }).await;
    join_room(&mut all, 1, room).await;
    // Carol was not invited.
    all[2].send(ClientMsg::JoinRoom { room }).await;
    all[2].expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_player_is_invited_to_a_room_once() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), false).await;
    let invite = || ClientMsg::Invite {
        login: login("bob"),
    };
    all[0].send(invite()).await;
    all[1]
        .expect(ServerMsg::Invited {
            room,
            by: login("andrew"),
        })
        .await;
    all[0].send(invite()).await;
    all[0].expect(refused(Refusal::NotNow)).await;
    // Bob hears the next chat line, not a second invitation.
    all[0]
        .send(ClientMsg::Chat {
            channel: Channel::Lobby,
            text: chat("after"),
        })
        .await;
    all[1]
        .expect(ServerMsg::Chat {
            line: ChatLine {
                channel: Channel::Lobby,
                from: login("andrew"),
                text: chat("after"),
            },
        })
        .await;
}

#[tokio::test]
async fn invitations_are_rate_limited() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew"]).await;
    open_room(&mut all, 0, &map(2, 4), false).await;
    let invite = |i: usize| ClientMsg::Invite {
        login: login(&format!("nobody{i}")),
    };
    // The server's INVITATIONS in a minute, here of players not on it.
    for i in 0..20 {
        all[0].send(invite(i)).await;
        all[0].expect(refused(Refusal::NoSuchPlayer)).await;
    }
    all[0].send(invite(20)).await;
    all[0].expect(refused(Refusal::SlowDown)).await;
}

#[tokio::test]
async fn joining_needs_the_same_content_a_free_slot_and_a_room() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let mut all = clients(addr, &["andrew", "bob", "carol"]).await;
    let room = open_room(&mut all, 0, &map(2, 2), true).await;

    let (mut other, _) =
        Client::hello_with(addr, "dave", ContentHash::new("modded").unwrap()).await;
    other.send(ClientMsg::JoinRoom { room }).await;
    other.expect(refused(Refusal::Incompatible)).await;

    join_room(&mut all, 1, room).await;
    all[2].send(ClientMsg::JoinRoom { room }).await;
    all[2].expect(refused(Refusal::RoomFull)).await;
    all[0]
        .send(ClientMsg::AddComputer {
            level: AiLevel::Easy,
        })
        .await;
    all[0].expect(refused(Refusal::RoomFull)).await;

    all[2].send(ClientMsg::JoinRoom { room: RoomId(99) }).await;
    all[2].expect(refused(Refusal::NoSuchRoom)).await;
}

#[tokio::test]
async fn ready_factions_and_alliances() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;
    add_computer(&mut all, room).await;
    let (andrew, bob) = (all[0].login.clone(), all[1].login.clone());
    let mut slots = vec![
        slot(human(&andrew), 1, true),
        slot(human(&bob), 2, false),
        slot(SlotPlayer::Computer(AiLevel::Easy), 3, true),
    ];
    let state = |slots: &Vec<Slot>| RoomState {
        id: room,
        host: andrew.clone(),
        map: map(2, 4).0,
        speed: Speed::Fast4,
        open: true,
        content: content(),
        slots: slots.clone(),
    };

    all[1].send(ClientMsg::SetReady { ready: true }).await;
    slots[1].ready = true;
    for c in &mut all {
        c.expect(ServerMsg::Room {
            change: RoomChange::Ready { slot: 1 },
            room: state(&slots),
        })
        .await;
    }
    // The host is always ready.
    all[0].send(ClientMsg::SetReady { ready: false }).await;
    all[0].expect(refused(Refusal::NotNow)).await;

    let orc = Faction::new("orc").unwrap();
    all[1]
        .send(ClientMsg::SetFaction {
            slot: 1,
            faction: orc.clone(),
        })
        .await;
    slots[1].faction = orc.clone();
    for c in &mut all {
        c.expect(ServerMsg::Room {
            change: RoomChange::Faction { slot: 1 },
            room: state(&slots),
        })
        .await;
    }
    // Not someone else's slot, nor a computer's unless host.
    for slot in [0, 2] {
        all[1]
            .send(ClientMsg::SetFaction {
                slot,
                faction: orc.clone(),
            })
            .await;
        all[1].expect(refused(Refusal::NotNow)).await;
    }
    all[0]
        .send(ClientMsg::SetFaction {
            slot: 2,
            faction: orc.clone(),
        })
        .await;
    slots[2].faction = orc.clone();
    for c in &mut all {
        c.expect(ServerMsg::Room {
            change: RoomChange::Faction { slot: 2 },
            room: state(&slots),
        })
        .await;
    }

    all[1]
        .send(ClientMsg::SetAlliance {
            slot: 1,
            alliance: 1,
        })
        .await;
    all[1].expect(refused(Refusal::NotNow)).await;
    for alliance in [0, 9] {
        all[0]
            .send(ClientMsg::SetAlliance { slot: 1, alliance })
            .await;
        all[0].expect(refused(Refusal::Invalid)).await;
    }
    all[0]
        .send(ClientMsg::SetAlliance {
            slot: 3,
            alliance: 1,
        })
        .await;
    all[0].expect(refused(Refusal::NoSuchPlayer)).await;
    all[0]
        .send(ClientMsg::SetAlliance {
            slot: 2,
            alliance: 8,
        })
        .await;
    slots[2].alliance = 8;
    for c in &mut all {
        c.expect(ServerMsg::Room {
            change: RoomChange::Alliance { slot: 2 },
            room: state(&slots),
        })
        .await;
    }
}

#[tokio::test]
async fn the_host_adds_and_removes_computers_and_players() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let (info, data) = map(2, 4);
    let room = open_room(&mut all, 0, &(info.clone(), data), true).await;
    join_room(&mut all, 1, room).await;
    let (andrew, bob) = (all[0].login.clone(), all[1].login.clone());

    // Only the host.
    all[1]
        .send(ClientMsg::AddComputer {
            level: AiLevel::Ai2,
        })
        .await;
    all[1].expect(refused(Refusal::NotNow)).await;

    all[0]
        .send(ClientMsg::AddComputer {
            level: AiLevel::Ai2,
        })
        .await;
    let ai2 = SlotPlayer::Computer(AiLevel::Ai2);
    let state = RoomState {
        id: room,
        host: andrew.clone(),
        map: info.clone(),
        speed: Speed::Fast4,
        open: true,
        content: content(),
        slots: vec![
            slot(human(&andrew), 1, true),
            slot(human(&bob), 2, false),
            slot(ai2.clone(), 3, true),
        ],
    };
    for c in &mut all[..2] {
        c.expect(ServerMsg::Room {
            change: RoomChange::Joined {
                player: ai2.clone(),
            },
            room: state.clone(),
        })
        .await;
    }
    for c in &mut all {
        let msg = c.recv().await;
        assert!(
            matches!(&msg, ServerMsg::Lobby { change: LobbyChange::RoomChanged { room: r } } if r.players.len() == 3),
            "{msg:?}"
        );
    }

    // Removing Bob sends him back to the lobby.
    all[0].send(ClientMsg::RemoveSlot { slot: 1 }).await;
    all[1]
        .expect(ServerMsg::RoomExited {
            reason: RoomExit::Removed,
        })
        .await;
    let state = RoomState {
        slots: vec![slot(human(&andrew), 1, true), slot(ai2.clone(), 3, true)],
        ..state
    };
    all[0]
        .expect(ServerMsg::Room {
            change: RoomChange::Left {
                player: human(&bob),
            },
            room: state.clone(),
        })
        .await;
    for c in &mut all {
        let msg = c.recv().await;
        assert!(
            matches!(&msg, ServerMsg::Lobby { change: LobbyChange::RoomChanged { room: r } } if r.players.len() == 2),
            "{msg:?}"
        );
        c.expect(moved(&bob, Whereabouts::Lobby)).await;
    }

    // A new slot takes the lowest free alliance.
    join_room(&mut all, 2, room).await;
    let carol = all[2].login.clone();
    all[0].send(ClientMsg::RemoveSlot { slot: 1 }).await;
    let state = RoomState {
        slots: vec![slot(human(&andrew), 1, true), slot(human(&carol), 2, false)],
        ..state
    };
    for i in [0, 2] {
        all[i]
            .expect(ServerMsg::Room {
                change: RoomChange::Left {
                    player: ai2.clone(),
                },
                room: state.clone(),
            })
            .await;
    }
    for c in &mut all {
        let msg = c.recv().await;
        assert!(
            matches!(&msg, ServerMsg::Lobby { change: LobbyChange::RoomChanged { room: r } } if r.players == [human(&andrew), human(&carol)]),
            "{msg:?}"
        );
    }

    // Not yourself, and not a slot that does not exist.
    all[0].send(ClientMsg::RemoveSlot { slot: 0 }).await;
    all[0].expect(refused(Refusal::NotNow)).await;
    all[0].send(ClientMsg::RemoveSlot { slot: 5 }).await;
    all[0].expect(refused(Refusal::NoSuchPlayer)).await;
}

#[tokio::test]
async fn the_host_leaving_hands_the_room_over_and_the_last_human_closes_it() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let (info, data) = map(2, 4);
    let room = open_room(&mut all, 0, &(info.clone(), data), true).await;
    join_room(&mut all, 1, room).await;
    join_room(&mut all, 2, room).await;
    add_computer(&mut all, room).await;
    let (andrew, bob, carol) = (
        all[0].login.clone(),
        all[1].login.clone(),
        all[2].login.clone(),
    );

    all[0].send(ClientMsg::LeaveRoom).await;
    all[0]
        .expect(ServerMsg::RoomExited {
            reason: RoomExit::Left,
        })
        .await;
    let easy = SlotPlayer::Computer(AiLevel::Easy);
    // Bob is host now, so ready.
    let slots_after = vec![
        slot(human(&bob), 2, true),
        slot(human(&carol), 3, false),
        slot(easy.clone(), 4, true),
    ];
    let after_handover = RoomState {
        id: room,
        host: bob.clone(),
        map: info.clone(),
        speed: Speed::Fast4,
        open: true,
        content: content(),
        slots: slots_after,
    };
    for i in [1, 2] {
        all[i]
            .expect(ServerMsg::Room {
                change: RoomChange::Left {
                    player: human(&andrew),
                },
                room: after_handover.clone(),
            })
            .await;
        all[i]
            .expect(ServerMsg::Room {
                change: RoomChange::Host { host: bob.clone() },
                room: after_handover.clone(),
            })
            .await;
    }
    let summary = RoomSummary {
        id: room,
        host: bob.clone(),
        map: info,
        speed: Speed::Fast4,
        open: true,
        content: content(),
        players: vec![human(&bob), human(&carol), easy],
        started: false,
    };
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomChanged {
            room: summary.clone(),
        }))
        .await;
        c.expect(moved(&andrew, Whereabouts::Lobby)).await;
    }

    // Bob, now host, can add computers; Carol leaves, then Bob.
    all[2].send(ClientMsg::LeaveRoom).await;
    sync(&mut all, &moved(&carol, Whereabouts::Lobby)).await;
    all[1].send(ClientMsg::LeaveRoom).await;
    all[1]
        .expect(ServerMsg::RoomExited {
            reason: RoomExit::Left,
        })
        .await;
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomClosed { id: room })).await;
        c.expect(moved(&bob, Whereabouts::Lobby)).await;
    }
}

#[tokio::test]
async fn a_host_who_disconnects_hands_the_room_over() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;
    let andrew = all.remove(0);
    andrew.close().await;
    let bob = all[0].login.clone();
    all[0]
        .expect(lobby(LobbyChange::PlayerLeft {
            login: login("andrew"),
        }))
        .await;
    let msg = all[0].recv().await;
    assert!(
        matches!(&msg, ServerMsg::Room { change: RoomChange::Left { player }, .. } if *player == human(&login("andrew"))),
        "{msg:?}"
    );
    let msg = all[0].recv().await;
    assert!(
        matches!(&msg, ServerMsg::Room { change: RoomChange::Host { host }, room } if *host == bob && room.host == bob),
        "{msg:?}"
    );
    let msg = all[0].recv().await;
    assert!(
        matches!(&msg, ServerMsg::Lobby { change: LobbyChange::RoomChanged { room: r } } if r.host == bob),
        "{msg:?}"
    );
}

#[tokio::test]
async fn starting_needs_the_host_everyone_ready_and_enough_players() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let room = open_room(&mut all, 0, &map(3, 4), true).await;

    // Too few players.
    all[0].send(ClientMsg::StartHosted { texts: texts() }).await;
    all[0].expect(refused(Refusal::CannotStart)).await;
    join_room(&mut all, 1, room).await;
    add_computer(&mut all, room).await;
    // Bob is not ready.
    all[0].send(ClientMsg::StartHosted { texts: texts() }).await;
    all[0].expect(refused(Refusal::CannotStart)).await;
    // Only the host starts.
    all[1].send(ClientMsg::StartHosted { texts: texts() }).await;
    all[1].expect(refused(Refusal::NotNow)).await;
    // Nor can someone in the lobby.
    all[2].send(ClientMsg::StartHosted { texts: texts() }).await;
    all[2].expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn a_good_start() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let (info, data) = map(2, 4);
    let room = open_room(&mut all, 0, &(info.clone(), data), true).await;
    join_room(&mut all, 1, room).await;
    add_computer(&mut all, room).await;
    ready(&mut all, 1, &[0, 1]).await;
    let (andrew, bob) = (all[0].login.clone(), all[1].login.clone());

    all[0].send(ClientMsg::StartHosted { texts: texts() }).await;
    let slots = [
        slot(human(&andrew), 1, true),
        slot(human(&bob), 2, true),
        slot(SlotPlayer::Computer(AiLevel::Easy), 3, true),
    ];
    let summary = RoomSummary {
        id: room,
        host: andrew.clone(),
        map: info.clone(),
        speed: Speed::Fast4,
        open: true,
        content: content(),
        players: slots.iter().map(|s| s.player.clone()).collect(),
        started: true,
    };
    for c in &mut all {
        c.expect(lobby(LobbyChange::RoomChanged {
            room: summary.clone(),
        }))
        .await;
        c.expect(moved(&andrew, Whereabouts::Playing(room))).await;
        c.expect(moved(&bob, Whereabouts::Playing(room))).await;
    }
    // Each human has a seat in the game.
    for (you, c) in all[..2].iter_mut().enumerate() {
        let Frame::Binary(bytes) = c.frame().await else {
            panic!("{}: expected a seat", c.login);
        };
        let SeatFrame::Seated {
            you: y,
            map: m,
            speed,
            paused,
            ..
        } = decode_frame(&bytes).unwrap()
        else {
            panic!("{}: expected a seat", c.login);
        };
        assert_eq!(
            (y, m, speed, paused),
            (you, info.clone(), Speed::Fast4, false)
        );
    }
    // A started room takes nobody, and the room's settings are fixed.
    all[2].send(ClientMsg::JoinRoom { room }).await;
    all[2].expect(refused(Refusal::NotNow)).await;
    all[0]
        .send(ClientMsg::AddComputer {
            level: AiLevel::Easy,
        })
        .await;
    all[0].expect(refused(Refusal::NotNow)).await;
    all[1].send(ClientMsg::LeaveRoom).await;
    all[1].expect(refused(Refusal::NotNow)).await;
}
