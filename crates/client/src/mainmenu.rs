//! The main menu and the options menu.

use soundrts_world::msg::{Msg, MsgPart, Phrase, msg};

use crate::client::Client;
use crate::config::Login;
use crate::menu::{Action, Choice, Exit, Flow, Menu, input_string, letters_and_digits};
use crate::sound_and_speech::sound_and_speech_menu;

/// Words said as written: a name, a login.
fn literal(s: &str) -> Msg {
    vec![MsgPart::literal(s)]
}

/// The main menu, until the player quits.
pub fn main_menu(cx: &mut Client) -> Result<(), Exit> {
    let mut title = literal(&format!("{},", cx.app_title()));
    title.extend(msg(Phrase::MakeASelection));
    Menu::new(
        title,
        vec![
            Choice::explained(
                msg(Phrase::SinglePlayer),
                Action::run(|cx| crate::singleplayer::single_player_menu(cx).map(|()| Flow::Stay)),
                msg(Phrase::SinglePlayerExplanation),
            ),
            Choice::explained(
                msg(Phrase::MultiplayerMenuItem),
                Action::run(|cx| crate::multiplayer::multiplayer_menu(cx).map(|()| Flow::Stay)),
                msg(Phrase::MultiplayerExplanation),
            ),
            Choice::explained(
                msg(Phrase::Options),
                Action::run(|cx| options_menu(cx).map(|()| Flow::Stay)),
                msg(Phrase::OptionsExplanation),
            ),
            Choice::explained(
                msg(Phrase::QuitMenuItem),
                Action::CloseMenu,
                msg(Phrase::QuitExplanation),
            ),
        ],
    )
    .run_loop(cx)
}

/// Login, sound and speech, mods, soundpacks, and the user folder.
pub fn options_menu(cx: &mut Client) -> Result<(), Exit> {
    Menu::new(
        msg(Phrase::OptionsMenu),
        vec![
            Choice::new(
                msg(Phrase::ModifyLogin),
                Action::run(|cx| modify_login(cx).map(|()| Flow::Stay)),
            ),
            Choice::new(
                msg(Phrase::SoundAndSpeech),
                Action::run(|cx| sound_and_speech_menu(cx).map(|()| Flow::Stay)),
            ),
            Choice::new(msg(Phrase::Mods), Action::run(mods_menu)),
            Choice::new(msg(Phrase::Soundpacks), Action::run(soundpacks_menu)),
            Choice::new(
                msg(Phrase::OpenUserFolder),
                Action::run(|cx| {
                    let user = cx.paths.user.clone();
                    cx.desktop.open_folder(&user);
                    Ok(Flow::Stay)
                }),
            ),
            Choice::new(msg(Phrase::Back), Action::CloseMenu),
        ],
    )
    .run_loop(cx)
}

/// Asks for a new login and saves it.
pub fn modify_login(cx: &mut Client) -> Result<(), Exit> {
    let prompt = [
        msg(Phrase::EnterNewLogin),
        msg(Phrase::UseLettersAndNumbersOnly),
    ]
    .concat();
    match input_string(cx, &prompt, letters_and_digits, "", true)? {
        None => cx.voice.alert(&msg(Phrase::CurrentLoginKept)),
        Some(login) => match Login::parse(&login) {
            None => cx
                .voice
                .alert(&[msg(Phrase::BadLogin), msg(Phrase::CurrentLoginKept)].concat()),
            Some(valid) => {
                cx.voice
                    .alert(&[msg(Phrase::NewLogin), literal(&login)].concat());
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
    let mut menu = Menu::new(msg(Phrase::Mods), Vec::new());
    menu.append(
        msg(Phrase::Nothing),
        Action::run(|cx| set_and_launch_mod(cx, "")),
        None,
    );
    for m in cx.res.available_mods() {
        let name = m.clone();
        menu.append(
            literal(&m),
            Action::run(move |cx| set_and_launch_mod(cx, &name)),
            None,
        );
    }
    menu.append(msg(Phrase::Back), Action::CloseMenu, None);
    menu.run(cx)?;
    Ok(Flow::CloseMenu)
}

pub fn soundpacks_menu(cx: &mut Client) -> Result<Flow, Exit> {
    cx.res.update_packages();
    let mut menu = Menu::new(msg(Phrase::Soundpacks), Vec::new());
    menu.append(
        msg(Phrase::Nothing),
        Action::run(|cx| set_and_launch_soundpack(cx, "")),
        None,
    );
    for s in cx.res.available_soundpacks() {
        let name = s.clone();
        menu.append(
            literal(&s),
            Action::run(move |cx| set_and_launch_soundpack(cx, &name)),
            None,
        );
    }
    menu.append(msg(Phrase::Back), Action::CloseMenu, None);
    menu.run(cx)?;
    Ok(Flow::CloseMenu)
}
