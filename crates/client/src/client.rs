//! The client context: everything the menus and the game interface use,
//! built once from the devices it speaks, plays and reads keys through.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Msg, MsgPart, Phrase, msg, nb2msg};

use crate::audio::AudioOutput;
use crate::clock::ClockRef;
use crate::config::Config;
use crate::input::{Input, InputSource};
use crate::layers::InterfaceFile;
use crate::log::Log;
use crate::message::{Category, News, Policy};
use crate::parameters::Parameters;
use crate::paths::Paths;
use crate::resource::Resources;
use crate::rng::ClientRng;
use crate::sound::Audio;
use crate::sound_cache::Sounds;
use crate::tts::{CharDelay, SpeechBackend, Tts};
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
    /// The clipboard's text, for the player to paste here; `None` when it
    /// holds none. At most [`crate::clipboard::MAX_READ_UNITS`] long.
    fn clipboard_text(&self) -> Option<String>;
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

/// The lines of a help file: each word names a phrase or a sound, or is a
/// count. A word that names nothing is refused, with its file and its line.
fn read_help_table(res: &Resources, help: InterfaceFile, log: &Log) -> Vec<Msg> {
    let Some(file) = res.interface_files(help).pop() else {
        return Vec::new();
    };
    let sounds = res.sounds();
    let mut lines = Vec::new();
    for (i, line) in file.text.split('\n').enumerate() {
        let refuse = |what: String| {
            log.error(format!(
                "{} line {}: {what}",
                file.path,
                i.saturating_add(1)
            ));
        };
        let mut msg = Vec::new();
        for word in line.split_whitespace() {
            match MsgPart::named(word) {
                Some(MsgPart::Name(n)) if !sounds.has(&n) => {
                    refuse(format!("{n} names no phrase and no sound"))
                }
                Some(part) => msg.push(part),
                None => refuse(format!("{word} is not a name or a count")),
            }
        }
        if !line.trim().is_empty() {
            lines.push(msg);
        }
    }
    lines
}

/// The client: settings, resources, sound, voice and keyboard.
pub struct Client {
    pub paths: Paths,
    pub config: Config,
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
    /// The speech's seconds per character, shared with the speech.
    char_delay: CharDelay,
    fullscreen: bool,
    help: Help,
}

/// Opens the resource stack for these paths and settings. The mods and
/// soundpacks given on the command line win over the configuration's.
pub fn open_resources(
    paths: &Paths,
    params: Arc<Parameters>,
    mods: &str,
    soundpacks: &str,
    preferred_language: Option<String>,
    log: &Log,
) -> Resources {
    let packages = &params.packages;
    let package_paths = paths.package_paths(&packages.base, &packages.additional);
    let mut res = Resources::new(
        package_paths,
        paths.user.clone(),
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
        let params = config.parameters.clone();
        let audio = Audio::new(
            devices.audio,
            usize::from(config.num_channels.get()),
            clock.clone(),
            params.flattening_factor,
            rng.clone(),
            f64::from(config.main_volume) / 100.0,
        );
        let input = Input::new(devices.input, clock.clone());
        let sounds = res.sounds();
        let char_delay = CharDelay::new(config.wait_delay_per_character);
        let tts = Tts::new(
            devices.speech,
            clock.clone(),
            log.clone(),
            char_delay.clone(),
            params.tts_digit_coefficient,
        );
        let channel = VoiceChannel::new(
            tts,
            audio.clone(),
            sounds.clone(),
            clock.clone(),
            log.clone(),
            config.debug_mode,
            devices.utterances,
        );
        let voice = Voice::new(channel, input.clone(), clock.clone());
        let desktop = devices.desktop;
        let title_desktop = desktop.clone();
        res.register(Box::new(move |mods, soundpacks| {
            title_desktop.set_title(&app_title(mods, soundpacks));
        }));
        let help = Help {
            game: read_help_table(&res, InterfaceFile::GAME_HELP, &log),
            menu: read_help_table(&res, InterfaceFile::MENU_HELP, &log),
            context: String::new(),
            index: -1,
        };
        Client {
            paths,
            config,
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
            char_delay,
            fullscreen: false,
            help,
        }
    }

    /// The window title for the active mods and soundpacks.
    pub fn app_title(&self) -> String {
        app_title(&self.res.mods, &self.res.soundpacks)
    }

    pub fn save_config(&self) {
        self.config.save(&self.paths.settings_file(), &self.log);
    }

    /// Stops every sound, and speech too if `voice_too`.
    pub fn stop_sound(&mut self, voice_too: bool) {
        if voice_too {
            self.voice.stop_speech();
        }
        self.audio.stop(voice_too);
    }

    /// Raises (1) or lowers (-1) the main volume by a tenth, saves it, and
    /// says it. The level is an answer: it interrupts what is being said,
    /// and news it cuts off is said again after it. No sound is stopped.
    pub fn modify_volume(&mut self, incr: i32) {
        let percent = (i32::from(self.config.main_volume) + 10 * incr).clamp(0, 100);
        self.set_main_volume(percent as u8);
        self.voice
            .item(&[nb2msg(i64::from(percent)), msg(Phrase::PercentVolume)].concat());
    }

    /// Sets the main volume, in percent, and saves it.
    pub fn set_main_volume(&mut self, percent: u8) {
        self.audio.set_main_volume(f64::from(percent) / 100.0);
        if self.config.main_volume != percent {
            self.config.main_volume = percent;
            self.save_config();
        }
    }

    /// Sets the speech's seconds per character, which the end of speech is
    /// estimated from, and saves it.
    pub fn set_wait_delay_per_character(&mut self, seconds: f64) {
        self.char_delay.set(seconds);
        self.config.wait_delay_per_character = seconds;
        self.save_config();
    }

    /// Switches the full screen flag and says it. The window shows nothing,
    /// so only the flag changes.
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        let m = if self.fullscreen {
            Phrase::DisplayOn
        } else {
            Phrase::DisplayOff
        };
        self.voice.item(&msg(m));
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Says the sounds or texts of a campaign's cut scene, outside a game,
    /// in order, as the story's policy says: a key skips a line, Escape the
    /// rest (see [`crate::voice::Voice::tell_now`]).
    pub fn play_sequence(&mut self, parts: &Msg) {
        let policy = self.config.news.of(Category::Story);
        if policy == Policy::Speak {
            self.stop_sound(true);
        }
        let story = parts
            .iter()
            .map(|part| News::new(Category::Story, vec![part.clone()]))
            .collect();
        self.voice.tell_now(story, policy);
    }

    /// Gives news to the voice, as the player's policy for its category
    /// says.
    pub fn news(&mut self, news: News) {
        let policy = self.config.news.of(news.category);
        self.voice.news(news, policy);
    }

    /// Whether news of this category comes with the sound of its event.
    pub fn cue(&self, category: Category) -> bool {
        self.config.news.of(category).plays_sound()
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

    /// Reads the `[parameters]` of `settings.toml` again and applies the
    /// sound volumes. A file that cannot be read or is refused keeps the
    /// current parameters.
    pub fn reload_parameters(&mut self) {
        let path = self.paths.settings_file();
        let read = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                Config::from_toml(&String::from_utf8_lossy(&bytes)).map_err(|e| e.to_string())
            });
        match read {
            Ok(c) => {
                let p = c.parameters;
                self.config.parameters = p.clone();
                self.res.set_params(p.clone());
                self.sounds.update_volumes(p);
            }
            Err(e) => self
                .log
                .warning(format!("cannot reload {}: {e}", path.display())),
        }
    }

    /// Stops sounds before the program ends.
    pub fn close(&mut self) {
        self.stop_sound(true);
    }
}
