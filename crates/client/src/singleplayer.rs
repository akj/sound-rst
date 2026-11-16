//! The single player menus (`clientmain.py`): the campaigns, and a
//! training game on a multiplayer map against computers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_protocol::{Computer, Faction, Income, Login, Slot, SlotPlayer};
use soundrts_world::Rules;
use soundrts_world::msg::{Phrase, msg};

use crate::campaign::run_campaign;
use crate::client::Client;
use crate::mapfile::Map;
use crate::menu::{Action, Choice, Exit, Flow, Menu};
use crate::net::lobby::{computer_name, seat_name_in_sentence, seat_settings};
use crate::session::Session;

/// The campaigns, a training game, loading a saved game if there is one,
/// listening to a game if there is one to listen to, and back. The menu is
/// made again each time it comes back, so that a game saved meanwhile can
/// be loaded.
pub fn single_player_menu(cx: &mut Client) -> Result<(), Exit> {
    loop {
        let mut menu = build_single_player_menu(cx);
        menu.run(cx)?;
        if menu.ended() {
            return Ok(());
        }
    }
}

pub(crate) fn build_single_player_menu(cx: &Client) -> Menu {
    let mut choices = Vec::new();
    for c in cx.res.campaigns() {
        let title = c.title.clone();
        let c = Rc::new(c);
        choices.push(Choice::new(
            title,
            Action::run(move |cx| run_campaign(cx, c.clone()).map(|()| Flow::Stay)),
        ));
    }
    choices.push(Choice::new(
        msg(Phrase::StartAGameOn),
        Action::run(|cx| training_menu(cx).map(|()| Flow::Stay)),
    ));
    if crate::saves::any_save(&cx.paths.saves()) {
        choices.push(Choice::new(
            msg(Phrase::LoadAGame),
            Action::run(|cx| crate::saves::load::load_menu(cx).map(|()| Flow::Stay)),
        ));
    }
    if crate::saves::listen::any_to_listen(cx) {
        choices.push(Choice::new(
            msg(Phrase::ListenToAGame),
            Action::run(|cx| crate::saves::listen::listen_menu(cx).map(|()| Flow::Stay)),
        ));
    }
    choices.push(Choice::new(msg(Phrase::Back), Action::CloseMenu));
    Menu::new(msg(Phrase::MakeASelection), choices)
}

/// The maps; the choice is remembered for next time.
pub fn training_menu(cx: &mut Client) -> Result<(), Exit> {
    let mut menu = Menu::remembering(cx, msg(Phrase::StartAGameOn), "mapmenu");
    for m in cx.res.multiplayer_maps() {
        let title = m.title.clone();
        menu.append(
            title,
            Action::run(move |cx| players_menu(cx, m.clone()).map(|()| Flow::Stay)),
            None,
        );
    }
    menu.append(msg(Phrase::QuitMenuItem), Action::Nothing, None);
    menu.run(cx)
}

/// The players of the training game being prepared: the local player in
/// alliance 1, then the computers invited, in alliance 2. They are named,
/// and their level and income chosen, as in a game room on a server.
struct Training {
    map: Arc<Map>,
    players: Vec<Slot>,
    /// An action changed the players: the menu must be built again.
    changed: bool,
}

type TrainingRef = Rc<RefCell<Training>>;

/// A player of the training game, with a random faction.
fn seat(player: SlotPlayer, alliance: u8) -> Slot {
    Slot {
        player,
        faction: Faction::random(),
        alliance,
        ready: true,
        income: Income::FULL,
    }
}

/// Changes the player `pn` of the training game.
fn change(t: &TrainingRef, pn: usize, f: impl Fn(&mut Slot) + 'static) -> Action {
    let t = t.clone();
    Action::run(move |_| {
        let mut t = t.borrow_mut();
        if let Some(p) = t.players.get_mut(pn) {
            f(p);
        }
        t.changed = true;
        Ok(Flow::Stay)
    })
}

fn add_ai(t: &TrainingRef, computer: Computer) -> Action {
    let t = t.clone();
    Action::run(move |_| {
        let mut t = t.borrow_mut();
        t.players.push(seat(SlotPlayer::Computer(computer), 2));
        t.changed = true;
        Ok(Flow::Stay)
    })
}

fn run_game(t: &TrainingRef) -> Action {
    let t = t.clone();
    Action::run(move |cx| {
        let (map, players) = {
            let t = t.borrow();
            let players = t.players.iter().map(Slot::controller).collect();
            (t.map.clone(), players)
        };
        Session::training(cx, map, players).run(cx)?;
        Ok(Flow::CloseMenu)
    })
}

/// Inviting computers, choosing factions, levels and incomes, starting, or
/// cancelling.
fn build_players_menu(cx: &Client, t: &TrainingRef) -> Menu {
    let mut menu = Menu::new(Vec::new(), Vec::new());
    let (n, map, players) = {
        let tr = t.borrow();
        (tr.players.len() as i64, tr.map.clone(), tr.players.clone())
    };
    if n < map.nb_players_max {
        for computer in Computer::NORMAL {
            menu.append(
                [msg(Phrase::Invite), computer_name(computer)].concat(),
                add_ai(t, computer),
                None,
            );
        }
    }
    if n >= map.nb_players_min {
        menu.append(msg(Phrase::Start), run_game(t), None);
    }
    let rule_factions = Rules::load(&[cx.res.rules_text()]).factions();
    if rule_factions.len() > 1 {
        for (pn, p) in players.iter().enumerate() {
            for r in std::iter::once("random_faction".to_owned()).chain(rule_factions.clone()) {
                let Ok(faction) = Faction::new(r.as_str()) else {
                    continue;
                };
                if faction != p.faction {
                    let label = [
                        seat_name_in_sentence(p),
                        cx.res.style().get(&r).title_msg(),
                    ]
                    .concat();
                    let action = change(t, pn, move |p| p.faction = faction.clone());
                    menu.append(label, action, None);
                }
            }
        }
    }
    for (pn, p) in players.iter().enumerate() {
        for (label, setting) in seat_settings(p) {
            menu.append(label, change(t, pn, move |p| setting.apply(p)), None);
        }
    }
    menu.append(
        msg(Phrase::Cancel),
        Action::CloseMenu,
        Some(msg(Phrase::CancelThisGame)),
    );
    menu
}

/// The players menu of a map, until the game is played or cancelled.
fn players_menu(cx: &mut Client, map: Arc<Map>) -> Result<(), Exit> {
    let Ok(login) = Login::new(cx.config.login.as_str()) else {
        // A login of the settings is always one.
        return Ok(());
    };
    let t = Rc::new(RefCell::new(Training {
        map: map.clone(),
        players: vec![seat(SlotPlayer::Human(login), 1)],
        changed: false,
    }));
    cx.res.set_map(Some(&map));
    let result = (|| {
        let mut menu = build_players_menu(cx, &t);
        loop {
            menu.run(cx)?;
            if menu.ended() {
                return Ok(());
            }
            if std::mem::take(&mut t.borrow_mut().changed) {
                let new = build_players_menu(cx, &t);
                menu.update_menu(cx, new);
            }
        }
    })();
    cx.res.set_map(None);
    result
}
