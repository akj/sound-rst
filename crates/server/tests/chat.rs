//! Chat on each channel, the lobby's history, and the rate limit.

mod common;

use common::*;
use soundrts_protocol::*;

fn line(channel: Channel, from: &Login, text: &str) -> ServerMsg {
    ServerMsg::Chat {
        line: ChatLine {
            channel,
            from: from.clone(),
            text: chat(text),
        },
    }
}

fn say(channel: Channel, text: &str) -> ClientMsg {
    ClientMsg::Chat {
        channel,
        text: chat(text),
    }
}

#[tokio::test]
async fn lobby_chat_reaches_everyone_out_of_a_game_and_is_kept() {
    let server = server(|_| {});
    let addr = server.local_addr();
    let mut all = clients(addr, &["andrew", "bob", "carol"]).await;
    // Bob is in a room: he still hears the lobby.
    open_room(&mut all, 1, &map(2, 4), true).await;
    all[0].send(say(Channel::Lobby, "hello all")).await;
    let andrew = all[0].login.clone();
    for c in &mut all {
        c.expect(line(Channel::Lobby, &andrew, "hello all")).await;
    }
    all[1].send(say(Channel::Lobby, "à bientôt")).await;
    let bob = all[1].login.clone();
    for c in &mut all {
        c.expect(line(Channel::Lobby, &bob, "à bientôt")).await;
    }

    let (_d, lobby) = Client::hello(addr, "dave").await;
    assert_eq!(
        lobby.chat,
        vec![
            ChatLine {
                channel: Channel::Lobby,
                from: andrew,
                text: chat("hello all"),
            },
            ChatLine {
                channel: Channel::Lobby,
                from: bob,
                text: chat("à bientôt"),
            },
        ]
    );
}

#[tokio::test]
async fn room_chat_reaches_only_the_room() {
    let server = server(|_| {});
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;

    all[1].send(say(Channel::Room, "ready?")).await;
    let bob = all[1].login.clone();
    all[0].expect(line(Channel::Room, &bob, "ready?")).await;
    all[1].expect(line(Channel::Room, &bob, "ready?")).await;

    // Carol is not in a room; she does not hear it and cannot use it.
    all[2].send(say(Channel::Room, "me too")).await;
    all[2].expect(refused(Refusal::NotNow)).await;
    // Game channels need a game.
    all[0].send(say(Channel::Game, "gg")).await;
    all[0].expect(refused(Refusal::NotNow)).await;
    all[0].send(say(Channel::Allies, "gg")).await;
    all[0].expect(refused(Refusal::NotNow)).await;
}

#[tokio::test]
async fn game_chat_reaches_the_game_and_allies_chat_the_alliance() {
    // Nobody reports turns; the game must not wait meanwhile.
    let server = server(|c| c.max_lead = 1_000_000);
    let mut all = clients(server.local_addr(), &["andrew", "bob", "carol", "dave"]).await;
    let room = open_room(&mut all, 0, &map(2, 4), true).await;
    join_room(&mut all, 1, room).await;
    join_room(&mut all, 2, room).await;
    // Andrew (1) and Carol allied; Bob alone (2).
    all[0]
        .send(ClientMsg::SetAlliance {
            slot: 2,
            alliance: 1,
        })
        .await;
    for c in &mut all[..3] {
        c.skip_until(|m| {
            matches!(
                m,
                ServerMsg::Room {
                    change: RoomChange::Alliance { slot: 2 },
                    ..
                }
            )
        })
        .await;
    }
    ready(&mut all, 1, &[0, 1, 2]).await;
    ready(&mut all, 2, &[0, 1, 2]).await;
    start_game(&mut all, room, &[0, 1, 2]).await;

    let (bob, carol) = (all[1].login.clone(), all[2].login.clone());
    all[1].send(say(Channel::Game, "gl hf")).await;
    for c in &mut all[..3] {
        c.expect_event(false, line(Channel::Game, &bob, "gl hf"))
            .await;
    }
    all[2].send(say(Channel::Allies, "rush him")).await;
    all[0]
        .expect_event(false, line(Channel::Allies, &carol, "rush him"))
        .await;
    all[2]
        .expect_event(false, line(Channel::Allies, &carol, "rush him"))
        .await;
    // Players in a game cannot use the lobby or room channels.
    all[0].send(say(Channel::Lobby, "anyone?")).await;
    all[0].expect_event(false, refused(Refusal::NotNow)).await;
    all[0].send(say(Channel::Room, "anyone?")).await;
    all[0].expect_event(false, refused(Refusal::NotNow)).await;
    // Dave, in the lobby, heard none of it; Bob did not hear the allies.
    let dave = all[3].login.clone();
    all[3].send(say(Channel::Lobby, "lonely")).await;
    all[3].expect(line(Channel::Lobby, &dave, "lonely")).await;
    all[1].send(say(Channel::Game, "?")).await;
    all[1]
        .expect_event(false, line(Channel::Game, &bob, "?"))
        .await;
}

#[tokio::test]
async fn chat_is_rate_limited() {
    let server = server(|_| {});
    let (mut c, _) = Client::hello(server.local_addr(), "andrew").await;
    let andrew = c.login.clone();
    for i in 0..5 {
        c.send(say(Channel::Lobby, &format!("line {i}"))).await;
        c.expect(line(Channel::Lobby, &andrew, &format!("line {i}")))
            .await;
    }
    c.send(say(Channel::Lobby, "one too many")).await;
    c.expect(refused(Refusal::SlowDown)).await;
}
