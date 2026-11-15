//! SoundRTS: starts the client and runs the main menu.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use soundrts::audio::{AudioOutput, NullOutput};
use soundrts::client::{Client, Devices, app_title, open_resources};
use soundrts::clock::SystemClock;
use soundrts::config::Config;
use soundrts::log::Log;
use soundrts::options::Options;
use soundrts::parameters::Parameters;
use soundrts::paths::{self, Paths};
use soundrts::resource::preferred_language;
use soundrts::rodio_output::RodioOutput;
use soundrts::tts::screen_reader_speech;
use soundrts::upstream::{self, MissingData};
use soundrts::window::{self, WindowCommand, WindowLink};
use soundrts::{mainmenu, notice, probe};

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args_os().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            notice::fatal(&e.to_string());
            return ExitCode::from(2);
        }
    };
    if options.probe {
        return match probe::report(&options) {
            Ok(report) => {
                notice::print(&report);
                ExitCode::SUCCESS
            }
            Err(e) => {
                notice::fatal(&e.to_string());
                ExitCode::FAILURE
            }
        };
    }
    if options.fetch_data {
        return if upstream::fetch_data(options.root.as_deref()) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let paths = match find_or_fetch_data(&options) {
        Ok(p) => p,
        Err(code) => return code,
    };
    paths.create_user_folders(&Log::stderr());
    let log = Log::to_file(&paths.client_log());
    let panic_log = log.clone();
    std::panic::set_hook(Box::new(move |info| {
        panic_log.error(format!("error: {info}"))
    }));

    let config = Config::load(&paths.config_file(), &log);
    let params = match Parameters::load(&paths.parameters_file(), &log) {
        Ok(p) => p,
        Err(e) => {
            let reason = format!("cannot read {}: {e}", paths.parameters_file().display());
            log.error(&reason);
            notice::fatal(&reason);
            return ExitCode::FAILURE;
        }
    };
    let preferred = preferred_language(&paths.language_file(), &log);
    let mods = options.mods.clone().unwrap_or_else(|| config.mods.clone());
    let soundpacks = options
        .soundpacks
        .clone()
        .unwrap_or_else(|| config.soundpacks.clone());
    let title = app_title(&mods, &soundpacks);

    let game_log = log.clone();
    let log_file = paths.client_log();
    let crashed = Arc::new(AtomicBool::new(false));
    let game_crashed = crashed.clone();
    let result = window::run(title, log.clone(), move |link| {
        let WindowLink {
            input,
            desktop,
            start,
            proxy,
        } = link;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let clock = Rc::new(SystemClock::starting_at(start));
            let res = open_resources(
                &paths,
                Rc::new(params),
                &mods,
                &soundpacks,
                preferred,
                &game_log,
            );
            let channels = usize::try_from(config.num_channels).unwrap_or(1).max(1);
            let sound_log = game_log.clone();
            let on_sound_error = move |e| sound_log.warning(format!("sound stopped: {e}"));
            let audio: Box<dyn AudioOutput> = match RodioOutput::open(channels, on_sound_error) {
                Ok(o) => Box::new(o),
                Err(e) => {
                    game_log.warning(format!("no sound: {e}"));
                    Box::new(NullOutput)
                }
            };
            let devices = Devices {
                speech: Box::new(screen_reader_speech(
                    config.nvda_direct != 0,
                    game_log.clone(),
                )),
                audio,
                input: Box::new(input),
                clock,
                desktop: Rc::new(desktop),
                random_seed: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64),
                utterances: None,
            };
            let mut client = Client::new(paths, config, res, game_log.clone(), devices);
            // An exit request ends the menus the same way as quitting.
            let _ = mainmenu::main_menu(&mut client);
            client.close();
        }));
        if outcome.is_err() {
            game_log.error("the game stopped after an error");
            game_crashed.store(true, Ordering::Relaxed);
        }
        let _ = proxy.send_event(WindowCommand::Exit);
    });
    if crashed.load(Ordering::Relaxed) {
        notice::fatal(&format!(
            "SoundRTS stopped after an error. The details are in {}",
            log_file.display()
        ));
        return ExitCode::FAILURE;
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let reason = format!("cannot open the game window: {e}");
            log.error(&reason);
            notice::fatal(&reason);
            ExitCode::FAILURE
        }
    }
}

/// The game's folders. Without game data, a game started from the desktop
/// offers to download it; any other start fails with the message that says
/// how to get it.
fn find_or_fetch_data(options: &Options) -> Result<Paths, ExitCode> {
    let cli_root = options.root.as_deref();
    let error = match Paths::from_environment(cli_root) {
        Ok(p) => return Ok(p),
        Err(e) => e,
    };
    let fail = |e: &dyn std::fmt::Display| {
        notice::fatal(&e.to_string());
        ExitCode::FAILURE
    };
    let missing = upstream::on_missing_data(
        paths::root_is_explicit(cli_root),
        paths::player_dir_from_environment(),
        notice::can_ask(),
    );
    match missing {
        MissingData::Fail => Err(fail(&error)),
        MissingData::Ask(player) => match upstream::ask_and_fetch(&player) {
            Some(_) => Paths::from_environment(cli_root).map_err(|e| fail(&e)),
            // The player said no: nothing went wrong.
            None => Err(ExitCode::SUCCESS),
        },
    }
}
