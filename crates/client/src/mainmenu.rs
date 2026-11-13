//! The main menu and the options menu.

use soundrts_world::msg::{Msg, MsgPart, msg};
use soundrts_world::msgparts as mp;

use crate::client::Client;
use crate::config::Login;
use crate::menu::{Action, Choice, Exit, Flow, Menu, input_string, letters_and_digits};

fn text(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}

/// The main menu, until the player quits.
pub fn main_menu(cx: &mut Client) -> Result<(), Exit> {
    let mut title = text(&format!("{},", cx.app_title()));
    title.extend(msg(mp::MAKE_A_SELECTION));
    Menu::new(
        title,
        vec![
            Choice::explained(
                msg(mp::SINGLE_PLAYER),
                Action::run(|cx| crate::singleplayer::single_player_menu(cx).map(|()| Flow::Stay)),
                msg(mp::SINGLE_PLAYER_EXPLANATION),
            ),
            Choice::explained(
                msg(mp::MULTIPLAYER2),
                Action::run(|cx| crate::multiplayer::multiplayer_menu(cx).map(|()| Flow::Stay)),
                msg(mp::MULTIPLAYER2_EXPLANATION),
            ),
            Choice::explained(
                msg(mp::OPTIONS),
                Action::run(|cx| options_menu(cx).map(|()| Flow::Stay)),
                msg(mp::OPTIONS_EXPLANATION),
            ),
            Choice::explained(
                msg(mp::QUIT2),
                Action::CloseMenu,
                msg(mp::QUIT2_EXPLANATION),
            ),
        ],
    )
    .run_loop(cx)
}

/// Login, mods, soundpacks, and the user folder.
pub fn options_menu(cx: &mut Client) -> Result<(), Exit> {
    Menu::new(
        msg(mp::OPTIONS_MENU),
        vec![
            Choice::new(
                msg(mp::MODIFY_LOGIN),
                Action::run(|cx| modify_login(cx).map(|()| Flow::Stay)),
            ),
            Choice::new(msg(mp::MODS), Action::run(mods_menu)),
            Choice::new(msg(mp::SOUNDPACKS), Action::run(soundpacks_menu)),
            Choice::new(
                msg(mp::OPEN_USER_FOLDER),
                Action::run(|cx| {
                    let user = cx.paths.user.clone();
                    cx.desktop.open_folder(&user);
                    Ok(Flow::Stay)
                }),
            ),
            Choice::new(msg(mp::BACK), Action::CloseMenu),
        ],
    )
    .run_loop(cx)
}

/// Asks for a new login and saves it.
pub fn modify_login(cx: &mut Client) -> Result<(), Exit> {
    let prompt = [
        msg(mp::ENTER_NEW_LOGIN),
        msg(mp::USE_LETTERS_AND_NUMBERS_ONLY),
    ]
    .concat();
    match input_string(cx, &prompt, letters_and_digits, "", true)? {
        None => cx.voice.alert(&msg(mp::CURRENT_LOGIN_KEPT)),
        Some(login) => match Login::parse(&login) {
            None => cx
                .voice
                .alert(&[msg(mp::BAD_LOGIN), msg(mp::CURRENT_LOGIN_KEPT)].concat()),
            Some(valid) => {
                cx.voice.alert(&[msg(mp::NEW_LOGIN), text(&login)].concat());
                cx.config.login = valid;
                cx.save_config();
            }
        },
    }
    Ok(())
}

/// Saves the mods, applies them, and opens the main menu again; the game
/// ends when that menu does.
fn set_and_launch_mod(cx: &mut Client, mods: &str) -> Result<Flow, Exit> {
    cx.config.mods = mods.to_owned();
    cx.save_config();
    let mods = cx.config.mods.clone();
    cx.res.set_mods(&mods);
    main_menu(cx)?;
    Err(Exit)
}

fn set_and_launch_soundpack(cx: &mut Client, soundpacks: &str) -> Result<Flow, Exit> {
    cx.config.soundpacks = soundpacks.to_owned();
    cx.save_config();
    let soundpacks = cx.config.soundpacks.clone();
    cx.res.set_soundpacks(&soundpacks);
    main_menu(cx)?;
    Err(Exit)
}

pub fn mods_menu(cx: &mut Client) -> Result<Flow, Exit> {
    cx.res.update_packages();
    let mut menu = Menu::new(msg(mp::MODS), Vec::new());
    menu.append(
        msg(&[0]),
        Action::run(|cx| set_and_launch_mod(cx, "")),
        None,
    );
    for m in cx.res.available_mods() {
        let name = m.clone();
        menu.append(
            text(&m),
            Action::run(move |cx| set_and_launch_mod(cx, &name)),
            None,
        );
    }
    menu.append(msg(mp::BACK), Action::CloseMenu, None);
    menu.run(cx)?;
    Ok(Flow::CloseMenu)
}

pub fn soundpacks_menu(cx: &mut Client) -> Result<Flow, Exit> {
    cx.res.update_packages();
    let mut menu = Menu::new(msg(mp::SOUNDPACKS), Vec::new());
    menu.append(
        msg(mp::NOTHING),
        Action::run(|cx| set_and_launch_soundpack(cx, "")),
        None,
    );
    for s in cx.res.available_soundpacks() {
        let name = s.clone();
        menu.append(
            text(&s),
            Action::run(move |cx| set_and_launch_soundpack(cx, &name)),
            None,
        );
    }
    menu.append(msg(mp::BACK), Action::CloseMenu, None);
    menu.run(cx)?;
    Ok(Flow::CloseMenu)
}
