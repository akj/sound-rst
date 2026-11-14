//! The multiplayer menus (`clientmain.multiplayer_menu`,
//! `clientservermenu.py`): finding a server or hosting one, the server's
//! lobby, the room of a game before it starts, and the game.
//!
//! The lobby and room menus are live: they are built again whenever the
//! server changes what they show, while the player moves through them.
//! The other menus are static and give up, as Escape would, when the
//! server interrupts them (see [`crate::net`]).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use soundrts_protocol::{
    AiLevel, ClientMsg, Faction, Login, PROTOCOL_VERSION, Rejection, SlotIndex, SlotPlayer, Speed,
};
use soundrts_server::{ServerConfig, ServerHandle, UpnpOutcome};
use soundrts_world::Rules;
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::client::Client;
use crate::config::DEFAULT_LOGIN;
use crate::game::style_msg;
use crate::mainmenu::modify_login;
use crate::mapfile::{Map, title_of};
use crate::menu::{Action, Choice, Exit, Flow, Menu, input_string, string_to_msg};
use crate::net::conn::{self, ConnectError, Connection};
use crate::net::lobby::{MapState, computer_name, player_name};
use crate::net::servers::{self, ListedServer};
use crate::net::words::{self, text};
use crate::net::{self, Net, discovery};
use crate::session::Session;
use crate::sound_cache::literal;

/// How long to wait for a server's welcome.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait for servers of the local network to answer.
const DISCOVERY_WAIT: Duration = Duration::from_millis(500);

/// The multiplayer menu of the main menu: a server from a list, a server
/// by its address, or a server of one's own.
pub fn multiplayer_menu(cx: &mut Client) -> Result<(), Exit> {
    if cx.config.login.as_str() == DEFAULT_LOGIN {
        cx.voice.alert(&msg(mp::ENTER_NEW_LOGIN));
        modify_login(cx)?;
    }
    loop {
        let mut menu = Menu::new(
            msg(mp::MAKE_A_SELECTION),
            vec![
                Choice::new(
                    msg(mp::CHOOSE_SERVER_IN_LIST),
                    Action::run(|cx| server_list(cx).map(|()| Flow::CloseMenu)),
                ),
                Choice::new(
                    msg(mp::ENTER_SERVER_IP),
                    Action::run(|cx| enter_address(cx).map(|()| Flow::CloseMenu)),
                ),
                Choice::explained(
                    msg(mp::SERVER),
                    Action::run(|cx| host(cx).map(|()| Flow::CloseMenu)),
                    msg(mp::SERVER_EXPLANATION),
                ),
                Choice::new(
                    arrivals_label(cx),
                    Action::run(|cx| {
                        cx.config.announce_arrivals = i64::from(cx.config.announce_arrivals == 0);
                        cx.save_config();
                        Ok(Flow::Stay)
                    }),
                ),
                Choice::new(msg(mp::CANCEL), Action::CloseMenu),
            ],
        );
        menu.run(cx)?;
        if menu.ended() {
            return Ok(());
        }
    }
}

/// The arrivals option and its state.
fn arrivals_label(cx: &Client) -> Msg {
    let state = if cx.config.announce_arrivals != 0 {
        words::ON
    } else {
        words::OFF
    };
    [text(words::ANNOUNCE_ARRIVALS), msg(mp::COMMA), text(state)].concat()
}

/// The servers of the local network, of the public list, and the recent
/// and favorite ones.
fn server_list(cx: &mut Client) -> Result<(), Exit> {
    cx.voice.item(&msg(words::PROCESSING));
    let config = cx.net_config.clone();
    let mut listed: Vec<(ListedServer, &[u64], &str)> = Vec::new();
    for f in discovery::discover(&config.discovery_targets, DISCOVERY_WAIT) {
        let s = ListedServer {
            address: f.addr.to_string(),
            name: f.name,
        };
        listed.push((s, words::LOCAL, ""));
    }
    for s in servers::favorites(&cx.paths) {
        listed.push((s, &[], words::FAVORITE));
    }
    for s in servers::recent(&cx.paths) {
        listed.push((s, &[], words::RECENT));
    }
    if let Some(url) = &config.public_list_url {
        match servers::public(url) {
            Ok(list) => listed.extend(list.into_iter().map(|s| (s, words::INTERNET, ""))),
            Err(e) => cx.log.warning(format!("no public list of servers: {e}")),
        }
    }
    let mut menu = Menu::new(msg(mp::CHOOSE_SERVER_IN_LIST), Vec::new());
    let mut seen: Vec<String> = Vec::new();
    for (s, number, phrase) in listed {
        if seen.contains(&s.address) {
            continue;
        }
        seen.push(s.address.clone());
        let explanation = if phrase.is_empty() {
            msg(number)
        } else {
            text(phrase)
        };
        let label = vec![literal(&s.name)];
        menu.append(
            label,
            Action::run(move |cx| connect(cx, &s.address, None).map(|()| Flow::Stay)),
            Some(explanation),
        );
    }
    if seen.is_empty() {
        cx.voice.item(&text(words::NO_SERVER_FOUND));
    }
    menu.append(
        msg(mp::CANCEL2),
        Action::Nothing,
        Some(msg(mp::GO_BACK_TO_PREVIOUS_MENU)),
    );
    menu.run(cx)
}

/// The characters of a server address.
fn address_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '[' | ']')
}

fn enter_address(cx: &mut Client) -> Result<(), Exit> {
    if let Some(address) = input_string(cx, &msg(mp::ENTER_SERVER_IP), address_char, "", true)? {
        let address = if address.is_empty() {
            "localhost".to_owned()
        } else {
            address
        };
        connect(cx, &address, None)?;
    }
    Ok(())
}

/// Starts a server on this machine, says the address to give to the
/// other players and copies it, then joins it. The server stops when the
/// player leaves it.
fn host(cx: &mut Client) -> Result<(), Exit> {
    cx.voice.item(&msg(words::PROCESSING));
    let config = ServerConfig {
        name: cx.config.login.as_str().to_owned(),
        ip: cx.net_config.host_ip,
        port: cx.net_config.host_port,
        upnp: cx.net_config.upnp,
        ..ServerConfig::default()
    };
    let server = match soundrts_server::start(config) {
        Ok(s) => s,
        Err(e) => {
            cx.log.warning(format!("cannot start the server: {e}"));
            cx.voice.alert(&msg(words::SERVER_COULD_NOT_START));
            return Ok(());
        }
    };
    let local = server.local_addr();
    let shared = shared_address(&server);
    cx.log
        .info(format!("hosting on {local}, shared as {shared}"));
    let address = shared.to_string();
    cx.desktop.set_clipboard(&address);
    let mut said = [
        text(words::SHARE_ADDRESS),
        string_to_msg(&address, true),
        msg(mp::PERIOD),
        text(words::ADDRESS_COPIED),
    ]
    .concat();
    if let UpnpOutcome::Failed { reason } = server.upnp() {
        cx.log.warning(format!("no port forwarding: {reason}"));
        said.extend(msg(mp::PERIOD));
        said.extend(text(words::NO_PORT_FORWARDING));
    }
    cx.voice.important(&said);
    let to = SocketAddr::new(
        if local.ip().is_unspecified() {
            IpAddr::from([127, 0, 0, 1])
        } else {
            local.ip()
        },
        local.port(),
    );
    connect(cx, &to.to_string(), Some(server))
}

/// The address the other players connect to: the router's if it forwards
/// the port, else this machine's on the local network.
fn shared_address(server: &ServerHandle) -> SocketAddr {
    let local = server.local_addr();
    match server.upnp() {
        UpnpOutcome::Mapped { external } => *external,
        _ if local.ip().is_unspecified() => {
            SocketAddr::new(lan_ip().unwrap_or(local.ip()), local.port())
        }
        _ => local,
    }
}

/// This machine's address on the local network: the one it would use to
/// reach the Internet. Nothing is sent.
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Connects to a server and runs its menus until the player leaves it or
/// the connection is lost. `hosted` is the player's own server, stopped
/// at the end.
pub fn connect(cx: &mut Client, address: &str, hosted: Option<ServerHandle>) -> Result<(), Exit> {
    let content = cx.res.content_hash();
    let login = Login::new(cx.config.login.as_str()).expect("a valid login");
    let hello = ClientMsg::Hello {
        protocol: PROTOCOL_VERSION,
        login,
        content: content.clone(),
    };
    let (conn, snapshot) = match conn::open(address, hello, CONNECT_TIMEOUT) {
        Ok(opened) => opened,
        Err(e) => {
            cx.log
                .warning(format!("cannot connect to {address}: {e:?}"));
            cx.voice.alert(&connect_error_msg(&e));
            return Ok(());
        }
    };
    let favorite = hosted.is_none().then(|| ListedServer {
        address: address.to_owned(),
        name: snapshot.server_name.clone(),
    });
    if let Some(s) = &favorite
        && let Err(e) = servers::add_recent(&cx.paths, s.clone())
    {
        cx.log
            .warning(format!("cannot save the recent servers: {e}"));
    }
    let result = play_on(cx, conn, snapshot, content, favorite);
    if let Some(server) = hosted {
        server.shutdown();
    }
    result
}

fn connect_error_msg(e: &ConnectError) -> Msg {
    match e {
        ConnectError::BadAddress | ConnectError::Unreachable(_) => msg(mp::SERVER_UNREACHABLE),
        ConnectError::NotAServer(_) => msg(mp::UNEXPECTED_REPLY),
        ConnectError::Rejected(Rejection::ProtocolMismatch { .. }) => msg(mp::VERSION_ERROR),
        ConnectError::Rejected(Rejection::ServerFull) => text(words::SERVER_FULL),
        ConnectError::Rejected(Rejection::NoHello) => {
            [msg(mp::CONNECTION_REJECTED), msg(mp::OR_LOGIN_REJECTED)].concat()
        }
    }
}

/// The server's menus on an open connection, until the player leaves the
/// server or the connection is lost. `favorite` is the server as the
/// player can keep it among the favorites.
pub fn play_on(
    cx: &mut Client,
    conn: Connection,
    snapshot: soundrts_protocol::LobbySnapshot,
    content: soundrts_protocol::ContentHash,
    favorite: Option<ListedServer>,
) -> Result<(), Exit> {
    let welcome = [
        msg(mp::WELCOME),
        vec![literal(snapshot.you.as_str())],
        msg(mp::ON_THE_SERVER_OF),
        vec![literal(&snapshot.server_name)],
    ]
    .concat();
    cx.net = Some(Net::new(conn, snapshot, content));
    cx.voice.important(&welcome);
    let result = lobby(cx, favorite.as_ref());
    // Closes the connection.
    cx.net = None;
    result
}

fn net_ref(cx: &Client) -> &Net {
    cx.net.as_ref().expect("on a server")
}

fn net_mut(cx: &mut Client) -> &mut Net {
    cx.net.as_mut().expect("on a server")
}

fn closed(cx: &Client) -> bool {
    net_ref(cx).closed.is_some()
}

/// Runs a live menu until an item closes it (`true`) or `stop` holds
/// (`false`). The menu is built again each time the server changes what
/// it shows; the selected item stays selected if it is still there.
fn run_live(
    cx: &mut Client,
    build: &dyn Fn(&mut Client) -> Menu,
    stop: &dyn Fn(&Client) -> bool,
) -> Result<bool, Exit> {
    let live = |cx: &mut Client| {
        let mut m = build(cx);
        m.escape_on_interrupt = false;
        m
    };
    let mut menu = live(cx);
    let mut seen = net_ref(cx).version;
    loop {
        if stop(cx) {
            return Ok(false);
        }
        menu.step(cx)?;
        if menu.ended() {
            return Ok(true);
        }
        if stop(cx) {
            return Ok(false);
        }
        let version = net_ref(cx).version;
        if version != seen {
            seen = version;
            let new = live(cx);
            menu.update_menu(cx, new);
        }
        cx.clock.sleep(0.01);
    }
}

/// The lobby, until the player leaves the server or the connection is
/// lost.
fn lobby(cx: &mut Client, favorite: Option<&ListedServer>) -> Result<(), Exit> {
    let mut say_title = true;
    loop {
        if say_title {
            cx.voice.menu(&msg(mp::MAKE_A_SELECTION2));
        }
        let build = |cx: &mut Client| build_lobby(cx, favorite);
        let stop = |cx: &Client| {
            let n = net_ref(cx);
            n.closed.is_some() || n.room.is_some() || n.started.is_some()
        };
        let left = run_live(cx, &build, &stop)?;
        if closed(cx) {
            cx.voice.alert(&msg(mp::CONNECTION_INTERRUPTED));
            return Ok(());
        }
        // A game the server started while the player was leaving its room:
        // the server did not take the player out, so the game is theirs.
        if let Some(start) = net_mut(cx).started.take() {
            play(cx, start)?;
            say_title = false;
            continue;
        }
        if left {
            return Ok(());
        }
        say_title = !room(cx)?;
        if closed(cx) {
            cx.voice.alert(&msg(mp::CONNECTION_INTERRUPTED));
            return Ok(());
        }
    }
}

fn join(room: soundrts_protocol::RoomId) -> Action {
    Action::run(move |cx| {
        net::send(cx, ClientMsg::JoinRoom { room });
        Ok(Flow::Stay)
    })
}

fn build_lobby(cx: &mut Client, favorite: Option<&ListedServer>) -> Menu {
    let net = net_ref(cx);
    let mut choices = Vec::new();
    for i in &net.invitations {
        if let Some(r) = net.lobby.room(i.room)
            && !r.started
        {
            let label = [
                msg(mp::ACCEPT_INVITATION_FROM),
                vec![literal(i.by.as_str())],
                title_of(&r.map),
            ]
            .concat();
            choices.push(Choice::new(label, join(i.room)));
        }
    }
    for r in &net.lobby.rooms {
        let can_join = r.open
            && !r.started
            && r.content == net.content
            && r.host != net.lobby.you
            && r.players.len() < usize::from(r.map.nb_players_max)
            && !net.invitations.iter().any(|i| i.room == r.id);
        if can_join {
            let label = [
                msg(words::JOIN),
                vec![literal(r.host.as_str())],
                title_of(&r.map),
            ]
            .concat();
            let explanation = [
                nb2msg(r.players.len() as i64),
                msg(mp::PLAYERS_ON),
                nb2msg(i64::from(r.map.nb_players_max)),
            ]
            .concat();
            choices.push(Choice::explained(label, join(r.id), explanation));
        }
    }
    choices.push(Choice::new(
        msg(mp::START_A_GAME_ON),
        Action::run(|cx| create_menu(cx, false).map(|()| Flow::Stay)),
    ));
    choices.push(Choice::new(
        msg(mp::START_A_PUBLIC_GAME_ON),
        Action::run(|cx| create_menu(cx, true).map(|()| Flow::Stay)),
    ));
    if let Some(s) = favorite {
        let label = if servers::is_favorite(&cx.paths, &s.address) {
            words::REMOVE_FROM_FAVORITES
        } else {
            words::ADD_TO_FAVORITES
        };
        let s = s.clone();
        choices.push(Choice::new(
            text(label),
            Action::run(move |cx| {
                if let Err(e) = servers::toggle_favorite(&cx.paths, s.clone()) {
                    cx.log.warning(format!("cannot save the favorites: {e}"));
                }
                net_mut(cx).version += 1;
                Ok(Flow::Stay)
            }),
        ));
    }
    choices.push(Choice::new(msg(mp::QUIT2), Action::CloseMenu));
    Menu::new(Vec::new(), choices)
}

/// The maps a game can be created on (map files only), then the speed.
/// `open`: anyone can join, else only the invited.
fn create_menu(cx: &mut Client, open: bool) -> Result<(), Exit> {
    let title = msg(if open {
        mp::START_A_PUBLIC_GAME_ON
    } else {
        mp::START_A_GAME_ON
    });
    let mut menu = Menu::remembering(cx, title.clone(), "mapmenu");
    for m in cx.res.multiplayer_maps() {
        if m.info().is_none() {
            continue;
        }
        let speed_title = [title.clone(), m.title.clone()].concat();
        menu.append(
            m.title.clone(),
            Action::run(move |cx| speed_menu(cx, m.clone(), open, &speed_title)),
            None,
        );
    }
    menu.append(msg(mp::CANCEL), Action::Nothing, None);
    menu.run(cx)
}

fn speed_menu(cx: &mut Client, map: Arc<Map>, open: bool, title: &Msg) -> Result<Flow, Exit> {
    let create = |speed: Speed| {
        let map = map.clone();
        Action::run(move |cx| {
            if let Some(info) = map.info() {
                net_mut(cx).creating = Some(map.clone());
                net::send(
                    cx,
                    ClientMsg::CreateRoom {
                        map: info,
                        speed,
                        open,
                    },
                );
            }
            Ok(Flow::Stay)
        })
    };
    let mut menu = Menu::new(
        title.clone(),
        vec![
            Choice::new(msg(mp::SET_SPEED_TO_SLOW), create(Speed::Slow)),
            Choice::new(msg(mp::SET_SPEED_TO_NORMAL), create(Speed::Normal)),
            Choice::new(
                [msg(mp::SET_SPEED_TO_FAST), nb2msg(2)].concat(),
                create(Speed::Fast2),
            ),
            Choice::new(
                [msg(mp::SET_SPEED_TO_FAST), nb2msg(4)].concat(),
                create(Speed::Fast4),
            ),
            Choice::new(msg(mp::CANCEL), Action::Nothing),
        ],
    );
    menu.default_choice_index = 1;
    menu.run(cx)?;
    Ok(Flow::Stay)
}

/// The room the player entered, until they leave it, are removed, or its
/// game is played. `true` if a game was played (its end said the menu).
fn room(cx: &mut Client) -> Result<bool, Exit> {
    let title = net_ref(cx)
        .room
        .as_ref()
        .map(|r| r.title())
        .unwrap_or_default();
    cx.voice.menu(&title);
    let factions = Rules::load(&[cx.res.rules_text()]).factions();
    let build = |cx: &mut Client| build_room(cx, &factions);
    let stop = |cx: &Client| {
        let n = net_ref(cx);
        n.closed.is_some() || n.room.is_none() || n.started.is_some()
    };
    let left = run_live(cx, &build, &stop)?;
    // A game that starts wins over a leave the server has not heard of.
    net::pump(cx);
    if left && net_ref(cx).started.is_none() {
        net::send(cx, ClientMsg::LeaveRoom);
        let n = net_mut(cx);
        n.room = None;
        n.version += 1;
        return Ok(false);
    }
    match net_mut(cx).started.take() {
        Some(start) => {
            play(cx, start)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn send_action(m: ClientMsg) -> Action {
    Action::run(move |cx| {
        net::send(cx, m.clone());
        Ok(Flow::Stay)
    })
}

/// The slot of the player an item was built for, now: the room may have
/// changed between the menu and the choice. A human is found wherever
/// they are; a computer only where it was, if it is the same kind.
fn slot_now(cx: &Client, index: SlotIndex, player: &SlotPlayer) -> Option<SlotIndex> {
    let slots = &net_ref(cx).room.as_ref()?.state.slots;
    match player {
        SlotPlayer::Human(_) => slots.iter().position(|s| s.player == *player),
        SlotPlayer::Computer(_) => (slots.get(index)?.player == *player).then_some(index),
    }
}

/// An item acting on the slot of `player` (at `index` when the menu was
/// built); nothing happens if that player is no longer there.
fn slot_action(
    index: SlotIndex,
    player: &SlotPlayer,
    make: impl Fn(SlotIndex) -> ClientMsg + 'static,
) -> Action {
    let player = player.clone();
    Action::run(move |cx| {
        if let Some(slot) = slot_now(cx, index, &player) {
            net::send(cx, make(slot));
        }
        Ok(Flow::Stay)
    })
}

fn faction_choices(
    cx: &Client,
    slot: usize,
    player: &SlotPlayer,
    current: &str,
    factions: &[String],
) -> Vec<Choice> {
    let name = player_name(player);
    if factions.len() <= 1 {
        return Vec::new();
    }
    std::iter::once("random_faction".to_owned())
        .chain(factions.iter().cloned())
        .filter(|f| f != current)
        .filter_map(|f| {
            let faction = Faction::new(f.as_str()).ok()?;
            let label = [name.clone(), style_msg(cx.res.style(), &f, "title")].concat();
            Some(Choice::new(
                label,
                slot_action(slot, player, move |slot| ClientMsg::SetFaction {
                    slot,
                    faction: faction.clone(),
                }),
            ))
        })
        .collect()
}

fn build_room(cx: &mut Client, factions: &[String]) -> Menu {
    let net = net_ref(cx);
    let Some(room) = &net.room else {
        return Menu::new(
            Vec::new(),
            vec![Choice::new(msg(mp::QUIT2), Action::CloseMenu)],
        );
    };
    let you = net.lobby.you.clone();
    let slots = &room.state.slots;
    let map = &room.state.map;
    let mut choices = Vec::new();
    if room.is_host(&you) {
        if slots.len() < usize::from(map.nb_players_max) {
            for p in net.lobby.available_players() {
                let label = [msg(mp::INVITE), vec![literal(p.as_str())]].concat();
                choices.push(Choice::new(
                    label,
                    send_action(ClientMsg::Invite { login: p }),
                ));
            }
            for level in [AiLevel::Easy, AiLevel::Aggressive, AiLevel::Ai2] {
                let label = [msg(mp::INVITE), computer_name(level)].concat();
                choices.push(Choice::new(
                    label,
                    send_action(ClientMsg::AddComputer { level }),
                ));
            }
        }
        if slots.len() >= usize::from(map.nb_players_min) {
            choices.push(Choice::new(msg(mp::START), send_action(ClientMsg::Start)));
        }
        for (i, slot) in slots.iter().enumerate() {
            let name = player_name(&slot.player);
            for a in 1..=slots.len() {
                let a = a as u8;
                if a != slot.alliance {
                    let label = [
                        msg(mp::MOVE),
                        name.clone(),
                        msg(mp::TO_ALLIANCE),
                        nb2msg(i64::from(a)),
                    ]
                    .concat();
                    choices.push(Choice::new(
                        label,
                        slot_action(i, &slot.player, move |slot| ClientMsg::SetAlliance {
                            slot,
                            alliance: a,
                        }),
                    ));
                }
            }
            let mine = slot.player == SlotPlayer::Human(you.clone());
            if mine || matches!(slot.player, SlotPlayer::Computer(_)) {
                choices.extend(faction_choices(
                    cx,
                    i,
                    &slot.player,
                    slot.faction.as_str(),
                    factions,
                ));
            }
        }
        for (i, slot) in slots.iter().enumerate() {
            if slot.player != SlotPlayer::Human(you.clone()) {
                let label = [text(words::REMOVE), player_name(&slot.player)].concat();
                choices.push(Choice::new(
                    label,
                    slot_action(i, &slot.player, |slot| ClientMsg::RemoveSlot { slot }),
                ));
            }
        }
    } else {
        if let Some(i) = room.slot_of(&you) {
            let slot = &slots[i];
            choices.extend(faction_choices(
                cx,
                i,
                &slot.player,
                slot.faction.as_str(),
                factions,
            ));
            choices.push(match room.map {
                MapState::Downloading => Choice::new(msg(words::DOWNLOADING), Action::Nothing),
                MapState::Refused => Choice::new(text(words::GAME_CANNOT_START), Action::Nothing),
                MapState::Ready(_) if slot.ready => Choice::new(
                    text(words::I_AM_NOT_READY),
                    send_action(ClientMsg::SetReady { ready: false }),
                ),
                MapState::Ready(_) => Choice::new(
                    text(words::I_AM_READY),
                    send_action(ClientMsg::SetReady { ready: true }),
                ),
            });
        }
    }
    choices.push(Choice::new(
        [msg(mp::QUIT2), msg(mp::LEAVE_THIS_GAME)].concat(),
        Action::CloseMenu,
    ));
    Menu::new(room.title(), choices)
}

/// Plays the game the server started, from the room's map.
/// Plays the game the server started, from the room's map. A game whose
/// map is missing, or where the player's slot is not theirs, is left.
fn play(cx: &mut Client, start: net::GameStart) -> Result<(), Exit> {
    let you = net_ref(cx).lobby.you.clone();
    let mine = start
        .slots
        .get(start.you)
        .is_some_and(|s| s.player == SlotPlayer::Human(you));
    let room_map = net_ref(cx)
        .room
        .as_ref()
        .and_then(|r| r.map())
        .filter(|m| m.hash().as_ref() == Some(&start.map.hash));
    let map = room_map.or_else(|| cx.res.find_multiplayer_map(&start.map.hash));
    let result = match map {
        Some(map) if mine => {
            Session::multiplayer(map, &start.slots, start.you, start.seed, start.speed).run(cx)
        }
        _ => {
            cx.log.warning(format!(
                "cannot play the game: slot {} of {:?}, map found: {}",
                start.you,
                start.slots,
                map.is_some()
            ));
            net::send(cx, ClientMsg::LeaveGame);
            cx.voice
                .alert(&[msg(mp::BEEP), text(words::GAME_CANNOT_START)].concat());
            Ok(())
        }
    };
    let n = net_mut(cx);
    n.room = None;
    n.game = None;
    n.version += 1;
    result
}
