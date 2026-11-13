//! SoundRTS: starts the client and runs the main menu.

use std::process::ExitCode;
use std::rc::Rc;

use soundrts::audio::{AudioOutput, NullOutput};
use soundrts::client::{Client, Devices, app_title, open_resources};
use soundrts::clock::SystemClock;
use soundrts::config::Config;
use soundrts::log::Log;
use soundrts::options::Options;
use soundrts::parameters::Parameters;
use soundrts::paths::Paths;
use soundrts::resource::preferred_language;
use soundrts::rodio_output::RodioOutput;
use soundrts::tts::ScreenReaderSpeech;
use soundrts::window::{self, WindowCommand, WindowLink};
use soundrts::{mainmenu, probe};

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args_os().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if options.probe {
        return match probe::report(&options) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    let paths = match Paths::from_environment(options.root.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
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
            log.error(format!(
                "cannot read {}: {e}",
                paths.parameters_file().display()
            ));
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
            let audio: Box<dyn AudioOutput> = match RodioOutput::open(channels) {
                Ok(o) => Box::new(o),
                Err(e) => {
                    game_log.warning(format!("no sound: {e}"));
                    Box::new(NullOutput)
                }
            };
            let devices = Devices {
                speech: Box::new(ScreenReaderSpeech::new()),
                audio,
                input: Box::new(input),
                clock,
                desktop: Rc::new(desktop),
            };
            let mut client = Client::new(paths, config, res, game_log.clone(), devices);
            // An exit request ends the menus the same way as quitting.
            let _ = mainmenu::main_menu(&mut client);
            client.close();
        }));
        if outcome.is_err() {
            game_log.error("the game stopped after an error");
        }
        let _ = proxy.send_event(WindowCommand::Exit);
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log.error(format!("window: {e}"));
            ExitCode::FAILURE
        }
    }
}
