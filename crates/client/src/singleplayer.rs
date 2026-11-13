//! The single player menus (`clientmain.py`): the campaigns, and a
//! training game on a multiplayer map against computers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::Rules;
use soundrts_world::msg::{MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::campaign::run_campaign;
use crate::client::Client;
use crate::game::style_msg;
use crate::mapfile::Map;
use crate::menu::{Action, Choice, Exit, Flow, Menu};
use crate::session::Session;

/// The campaigns, a training game, and back.
pub fn single_player_menu(cx: &mut Client) -> Result<(), Exit> {
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
        msg(mp::START_A_GAME_ON),
        Action::run(|cx| training_menu(cx).map(|()| Flow::Stay)),
    ));
    choices.push(Choice::new(msg(mp::BACK), Action::CloseMenu));
    Menu::new(msg(mp::MAKE_A_SELECTION), choices).run_loop(cx)
}

/// The maps; the choice is remembered for next time.
pub fn training_menu(cx: &mut Client) -> Result<(), Exit> {
    let mut menu = Menu::remembering(cx, msg(mp::START_A_GAME_ON), "mapmenu");
    for m in cx.res.multiplayer_maps() {
        let title = m.title.clone();
        menu.append(
            title,
            Action::run(move |cx| players_menu(cx, m.clone()).map(|()| Flow::Stay)),
            None,
        );
    }
    menu.append(msg(mp::QUIT2), Action::Nothing, None);
    menu.run(cx)
}

/// The players of the training game being prepared.
struct Training {
    map: Arc<Map>,
    players: Vec<String>,
    factions: Vec<String>,
    /// An action changed the players: the menu must be built again.
    changed: bool,
}

type TrainingRef = Rc<RefCell<Training>>;

fn add_ai(t: &TrainingRef, ai_type: &'static str) -> Action {
    let t = t.clone();
    Action::run(move |_| {
        let mut t = t.borrow_mut();
        t.players.push(ai_type.to_owned());
        t.factions.push("random_faction".to_owned());
        t.changed = true;
        Ok(Flow::Stay)
    })
}

fn set_faction(t: &TrainingRef, pn: usize, faction: String) -> Action {
    let t = t.clone();
    Action::run(move |_| {
        let mut t = t.borrow_mut();
        t.factions[pn] = faction.clone();
        t.changed = true;
        Ok(Flow::Stay)
    })
}

fn run_game(t: &TrainingRef) -> Action {
    let t = t.clone();
    Action::run(move |cx| {
        let (map, players, factions) = {
            let t = t.borrow();
            (t.map.clone(), t.players.clone(), t.factions.clone())
        };
        let mut alliances = vec!["1".to_owned()];
        alliances.extend(std::iter::repeat_n("2".to_owned(), players.len() - 1));
        Session::training(cx, map, &players, &factions, &alliances).run(cx)?;
        Ok(Flow::CloseMenu)
    })
}

/// Inviting computers, choosing factions, starting, or cancelling.
fn build_players_menu(cx: &Client, t: &TrainingRef) -> Menu {
    let mut menu = Menu::new(Vec::new(), Vec::new());
    let (n, map, players, factions) = {
        let tr = t.borrow();
        (
            tr.players.len() as i64,
            tr.map.clone(),
            tr.players.clone(),
            tr.factions.clone(),
        )
    };
    if n < map.nb_players_max {
        menu.append(
            [msg(mp::INVITE), msg(mp::QUIET_COMPUTER)].concat(),
            add_ai(t, "easy"),
            None,
        );
        menu.append(
            [msg(mp::INVITE), msg(mp::AGGRESSIVE_COMPUTER)].concat(),
            add_ai(t, "aggressive"),
            None,
        );
        menu.append(
            [msg(mp::INVITE), msg(mp::AGGRESSIVE_COMPUTER), nb2msg(2)].concat(),
            add_ai(t, "ai2"),
            None,
        );
    }
    if n >= map.nb_players_min {
        menu.append(msg(mp::START), run_game(t), None);
    }
    let rule_factions = Rules::load(&[cx.res.rules_text()]).factions();
    if rule_factions.len() > 1 {
        for (pn, (p, pr)) in players.iter().zip(&factions).enumerate() {
            for r in std::iter::once("random_faction".to_owned()).chain(rule_factions.clone()) {
                if r != *pr {
                    let label = [
                        vec![MsgPart::Text(p.clone())],
                        style_msg(cx.res.style(), &r, "title"),
                    ]
                    .concat();
                    menu.append(label, set_faction(t, pn, r), None);
                }
            }
        }
    }
    menu.append(
        msg(mp::CANCEL),
        Action::CloseMenu,
        Some(msg(mp::CANCEL_THIS_GAME)),
    );
    menu
}

/// The players menu of a map, until the game is played or cancelled.
fn players_menu(cx: &mut Client, map: Arc<Map>) -> Result<(), Exit> {
    let t = Rc::new(RefCell::new(Training {
        map: map.clone(),
        players: vec![cx.config.login.as_str().to_owned()],
        factions: vec!["random_faction".to_owned()],
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
