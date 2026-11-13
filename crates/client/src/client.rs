//! The client context: everything the menus and the game interface use,
//! built once from the devices it speaks, plays and reads keys through.

use std::path::Path;
use std::rc::Rc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::audio::AudioOutput;
use crate::clock::ClockRef;
use crate::config::Config;
use crate::input::{Input, InputSource};
use crate::log::Log;
use crate::parameters::Parameters;
use crate::paths::Paths;
use crate::resource::Resources;
use crate::rng::ClientRng;
use crate::sound::Audio;
use crate::sound_cache::Sounds;
use crate::tts::{SpeechBackend, Tts};
use crate::voice::{UtteranceLog, Voice, VoiceChannel};

/// The version of the game this client ports, said in the window title.
pub const VERSION: &str = "1.3.8.1";

/// The window title: the version, the mods and the soundpacks.
pub fn app_title(mods: &str, soundpacks: &str) -> String {
    format!("SoundRTS {VERSION} {mods} {soundpacks}")
}

/// What the client asks of the desktop besides sound and keys.
pub trait Desktop {
    fn set_title(&self, title: &str);
    /// Shows a folder to the player (in Explorer on Windows).
    fn open_folder(&self, path: &Path);
    /// Puts text on the clipboard, for the player to paste elsewhere.
    fn set_clipboard(&self, text: &str);
}

/// The devices the client talks through: real ones, or recording fakes.
pub struct Devices {
    pub speech: Box<dyn SpeechBackend>,
    pub audio: Box<dyn AudioOutput>,
    pub input: Box<dyn InputSource>,
    pub clock: ClockRef,
    pub desktop: Rc<dyn Desktop>,
    /// The seed of the client's own randomness (see [`ClientRng`]).
    pub random_seed: u64,
    /// Where to record each utterance the voice starts, if anywhere.
    pub utterances: Option<UtteranceLog>,
}

/// Called once per pass of the game loop with the running game: a test
/// reads the world there, or acts as a scripted player would.
pub type GameHook = Rc<dyn Fn(&mut crate::game::GameInterface, &mut Client)>;

/// How the next games are set up, beyond what the menus choose.
#[derive(Clone, Default)]
pub struct GameSettings {
    /// The world's seed; `None` draws one at random, as upstream does.
    pub seed: Option<i64>,
    pub hook: Option<GameHook>,
}

/// The help messages of `ui/menu_help.txt` and `ui/game_help.txt`, said one
/// after the other.
struct Help {
    game: Vec<Msg>,
    menu: Vec<Msg>,
    context: String,
    index: i64,
}

fn read_help_table(res: &Resources, name: &str) -> Vec<Msg> {
    res.text(name, false)
        .unwrap_or_default()
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_whitespace().map(MsgPart::from).collect())
        .collect()
}

/// The client: settings, resources, sound, voice and keyboard.
pub struct Client {
    pub paths: Paths,
    pub config: Config,
    pub params: Rc<Parameters>,
    pub log: Log,
    pub clock: ClockRef,
    pub res: Resources,
    pub sounds: Sounds,
    pub audio: Audio,
    pub voice: Voice,
    pub input: Input,
    pub desktop: Rc<dyn Desktop>,
    /// The client's randomness, apart from the simulation's.
    pub rng: ClientRng,
    pub game: GameSettings,
    /// The server the player is on, if any.
    pub net: Option<crate::net::Net>,
    pub net_config: crate::net::NetConfig,
    fullscreen: bool,
    help: Help,
}

/// Opens the resource stack for these paths and settings. The mods and
/// soundpacks given on the command line win over the configuration's.
pub fn open_resources(
    paths: &Paths,
    params: Rc<Parameters>,
    mods: &str,
    soundpacks: &str,
    preferred_language: Option<String>,
    log: &Log,
) -> Resources {
    let (base, additional) = params.packages();
    let mut package_paths = vec![paths.root.join(&base)];
    package_paths.extend(paths.additional_package_paths(&base, &additional));
    let mut res = Resources::new(
        package_paths,
        paths.downloaded(),
        preferred_language,
        params,
        log.clone(),
    );
    res.set_mods(mods);
    res.set_soundpacks(soundpacks);
    res
}

impl Client {
    pub fn new(
        paths: Paths,
        config: Config,
        mut res: Resources,
        log: Log,
        devices: Devices,
    ) -> Client {
        let clock = devices.clock;
        let rng = ClientRng::new(devices.random_seed);
        let params = res.params();
        let channels = usize::try_from(config.num_channels).unwrap_or(1).max(1);
        let audio = Audio::new(
            devices.audio,
            channels,
            clock.clone(),
            params.flattening_factor(),
            rng.clone(),
        );
        let input = Input::new(devices.input, clock.clone());
        let sounds = res.sounds();
        let tts = Tts::new(
            devices.speech,
            clock.clone(),
            log.clone(),
            config.wait_delay_per_character,
            params.tts_digit_coefficient(),
        );
        let channel = VoiceChannel::new(
            tts,
            audio.clone(),
            sounds.clone(),
            clock.clone(),
            log.clone(),
            config.debug_mode != 0,
            devices.utterances,
        );
        let voice = Voice::new(channel, input.clone(), clock.clone());
        let desktop = devices.desktop;
        let title_desktop = desktop.clone();
        res.register(Box::new(move |mods, soundpacks| {
            title_desktop.set_title(&app_title(mods, soundpacks));
        }));
        let help = Help {
            game: read_help_table(&res, "ui/game_help"),
            menu: read_help_table(&res, "ui/menu_help"),
            context: String::new(),
            index: -1,
        };
        Client {
            paths,
            config,
            params,
            log,
            clock,
            res,
            sounds,
            audio,
            voice,
            input,
            desktop,
            rng,
            game: GameSettings::default(),
            net: None,
            net_config: crate::net::NetConfig::default(),
            fullscreen: false,
            help,
        }
    }

    /// The window title for the active mods and soundpacks.
    pub fn app_title(&self) -> String {
        app_title(&self.res.mods, &self.res.soundpacks)
    }

    pub fn save_config(&self) {
        self.config.save(&self.paths.config_file(), &self.log);
    }

    /// Stops every sound, and speech too if `voice_too`.
    pub fn stop_sound(&mut self, voice_too: bool) {
        if voice_too {
            self.voice.stop_speech();
        }
        self.audio.stop(voice_too);
    }

    /// Raises (1) or lowers (-1) the main volume by a tenth, and says it.
    pub fn modify_volume(&mut self, incr: i32) {
        let v = (self.audio.main_volume() + 0.1 * f64::from(incr)).clamp(0.0, 1.0);
        self.audio.set_main_volume(v);
        self.stop_sound(true);
        let percent = (v * 100.0).round_ties_even() as i64;
        self.voice
            .item(&[nb2msg(percent), msg(mp::PERCENT_VOLUME)].concat());
    }

    /// Switches the full screen flag and says it. The window shows nothing,
    /// so only the flag changes.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        let m = if self.fullscreen {
            mp::DISPLAY_ON
        } else {
            mp::DISPLAY_OFF
        };
        self.voice.item(&msg(m));
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Says sounds or texts in order, each one interruptible.
    pub fn play_sequence(&mut self, names: &[String]) {
        self.stop_sound(true);
        for name in names {
            self.voice.important(&vec![MsgPart::from(name.as_str())]);
        }
    }

    /// The next (`incr` 1) or previous (-1) help message of "menu" or
    /// "game". Changing context starts again from the first.
    pub fn help_msg(&mut self, context: &str, incr: i64) -> Msg {
        let h = &mut self.help;
        if context != h.context {
            h.index = -1;
            h.context = context.to_owned();
        }
        let table = if context == "game" { &h.game } else { &h.menu };
        if table.is_empty() {
            return Vec::new();
        }
        let len = table.len() as i64;
        h.index += incr;
        if h.index >= len {
            h.index = 0;
        }
        if h.index < 0 {
            h.index = len - 1;
        }
        table[h.index as usize].clone()
    }

    /// Reads `cfg/parameters.toml` again and applies the sound volumes. A
    /// file that cannot be read keeps the current parameters.
    pub fn reload_parameters(&mut self) {
        match Parameters::load(&self.paths.parameters_file(), &self.log) {
            Ok(p) => {
                let p = Rc::new(p);
                self.params = p.clone();
                self.res.set_params(p.clone());
                self.sounds.update_volumes(p);
            }
            Err(e) => self.log.warning(format!(
                "cannot read {}: {e}",
                self.paths.parameters_file().display()
            )),
        }
    }

    /// Stops sounds before the program ends.
    pub fn close(&mut self) {
        self.stop_sound(true);
    }
}
