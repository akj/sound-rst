//! The options menu's sound and speech settings, and the volume keys, run
//! with scripted keys on the real game data, the recorded speech device
//! and the manual clock. These check what the client sends to the speech
//! device and writes to `settings.toml`, not what a screen reader says.

use soundrts::clock::Clock;
use soundrts::config::Config;
use soundrts::input::{Key, Mods};
use soundrts::log::Log;
use soundrts::mainmenu::main_menu;
use soundrts::message::{Category, News, Policy};
use soundrts::net::words;
use soundrts::testkit::{AudioEvent, Harness, LiveKeys, Script};
use soundrts_world::msg::MsgPart;

/// Key presses 20 seconds apart: time enough for every message, so that
/// no press is dropped as piled up.
struct Keys {
    script: Script,
    time: f64,
}

impl Keys {
    fn new() -> Keys {
        Keys {
            script: Script::new(),
            time: 0.0,
        }
    }

    fn tap(mut self, key: Key) -> Keys {
        self.time += 20.0;
        self.script = self.script.at(self.time).tap(key);
        self
    }

    fn taps(self, key: Key, n: usize) -> Keys {
        (0..n).fold(self, |k, _| k.tap(key))
    }

    /// A press `seconds` after the previous one.
    fn tap_after(mut self, seconds: f64, key: Key) -> Keys {
        self.time += seconds;
        self.script = self.script.at(self.time).tap(key);
        self
    }

    /// From the main menu at start-up to the sound and speech menu.
    fn to_sound_and_speech() -> Keys {
        Keys::new()
            .taps(Key::Down, 3)
            .tap(Key::Return)
            .taps(Key::Down, 2)
            .tap(Key::Return)
    }

    /// Back from the sound and speech menu, out of the options menu, and
    /// quit.
    fn quit(self) -> Script {
        self.taps(Key::Escape, 3).script
    }
}

const ENTERED: [&str; 6] = [
    "options,",
    "options menu",
    "Modify your login",
    "sound and speech",
    "sound and speech",
    "sound and speech",
];

/// What was said from the sound and speech menu's title on.
fn said_in_submenu(h: &Harness) -> Vec<String> {
    let spoken = h.spoken();
    let at = spoken
        .iter()
        .position(|s| s == "options,")
        .expect("options chosen");
    assert_eq!(spoken[at..at + 6], ENTERED);
    spoken[at + 6..].to_vec()
}

fn config_text(h: &Harness) -> String {
    std::fs::read_to_string(h.client.paths.settings_file()).expect("settings.toml")
}

const VOLUME_50: &str = "volume , 50 percent , Enter or Home raises it, End lowers it";
const VOLUME_70: &str = "volume , 70 percent , Enter or Home raises it, End lowers it";
const BACK: &str = "go back to the previous menu";

#[test]
fn the_submenu_says_each_item_name_first_then_its_value() {
    let script = Keys::to_sound_and_speech().taps(Key::Down, 11).quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let said = said_in_submenu(&h);
    assert_eq!(
        said[..11],
        [
            VOLUME_50,
            "alert , spoken",
            "production , spoken",
            "economy , spoken",
            "scouting , spoken",
            "chat , spoken",
            "system , spoken",
            "story , spoken",
            "NVDA tells the game when it has finished speaking , off , this needs NVDA's current controller client and works from the next start: turn it off if messages stop, overlap, or wait too long",
            "speech rate , 10 characters a second , press Enter to measure it",
            BACK,
        ]
    );
}

#[test]
fn the_volume_is_saved_when_it_changes_and_survives_a_restart() {
    let script = Keys::to_sound_and_speech()
        .tap(Key::Down)
        .tap(Key::Home)
        .tap(Key::Return)
        .quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        said_in_submenu(&h)[..3],
        [
            VOLUME_50,
            "60 percent of the volume",
            "70 percent of the volume"
        ]
    );
    assert!(config_text(&h).contains("\nmain_volume = 70\n"));

    // The next start reads the file.
    let config = Config::load(&h.client.paths.settings_file(), &Log::silent());
    assert_eq!(config.main_volume, 70);
    let script = Keys::to_sound_and_speech().tap(Key::Down).quit();
    let mut again = Harness::with_config("en", script, config);
    assert_eq!(again.client.audio.main_volume(), 0.7);
    assert_eq!(main_menu(&mut again.client), Ok(()));
    assert_eq!(said_in_submenu(&again)[0], VOLUME_70);
}

#[test]
fn enter_on_the_volume_goes_from_the_top_back_to_silence() {
    let script = Keys::to_sound_and_speech()
        .tap(Key::Down)
        .taps(Key::Return, 2)
        .quit();
    let config = Config {
        main_volume: 90,
        ..Config::default()
    };
    let mut h = Harness::with_config("en", script, config);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        said_in_submenu(&h)[1..3],
        ["100 percent of the volume", "0 percent of the volume"]
    );
    assert_eq!(h.client.audio.main_volume(), 0.0);
}

#[test]
fn a_volume_key_interrupts_with_the_level_only_and_the_sentence_is_said_in_full() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let sound = h.client.sounds.get_sound("1029");
    h.client
        .audio
        .play_stereo(sound, soundrts::sound::StereoVolume::Both(1.0), 0.0);
    h.client.news(News::new(
        Category::System,
        vec![MsgPart::from(words::SERVER_FULL)],
    ));
    h.clock.advance(0.5);
    let before = h.audio.events().len();
    h.client.modify_volume(1);
    let end = h.clock.now() + 30.0;
    while !h.client.voice.is_idle() && h.clock.now() < end {
        h.clock.advance(0.01);
        h.client.voice.update();
    }
    assert_eq!(
        h.spoken(),
        [
            "the server is full",
            "60 percent of the volume",
            "the server is full"
        ]
    );
    // The level stops no sound: only the voice channel (0) is stopped,
    // to give the level its turn.
    let stopped: Vec<usize> = h.audio.events()[before..]
        .iter()
        .filter_map(|e| match e {
            AudioEvent::Stop { channel } => Some(*channel),
            _ => None,
        })
        .collect();
    assert!(stopped.iter().all(|&c| c == 0), "stopped {stopped:?}");
}

#[test]
fn each_news_item_cycles_through_its_four_values_and_is_written_to_the_file() {
    // alert once, production twice, economy three times, scouting four
    // times; chat, system and story stay.
    let script = Keys::to_sound_and_speech()
        .taps(Key::Down, 2)
        .taps(Key::Return, 1)
        .tap(Key::Down)
        .taps(Key::Return, 2)
        .tap(Key::Down)
        .taps(Key::Return, 3)
        .tap(Key::Down)
        .taps(Key::Return, 4)
        .quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(
        said_in_submenu(&h)[..14],
        [
            VOLUME_50,
            "alert , spoken",
            "its sound only",
            "production , spoken",
            "its sound only",
            "kept silently in the log",
            "economy , spoken",
            "its sound only",
            "kept silently in the log",
            "dropped",
            "scouting , spoken",
            "its sound only",
            "kept silently in the log",
            "dropped",
        ]
    );
    assert!(config_text(&h).contains(
        "[news]\nalert = \"sound\"\nproduction = \"log\"\neconomy = \"drop\"\n\
         scouting = \"speak\"\nchat = \"speak\"\nsystem = \"speak\"\nstory = \"speak\"\n"
    ));
    let read = Config::load(&h.client.paths.settings_file(), &Log::silent());
    assert_eq!(read.news.of(Category::Economy), Policy::Drop);
    // The game hears the new policy at once.
    assert_eq!(h.client.config.news.of(Category::Production), Policy::Log);
}

#[test]
fn the_nvda_item_switches_and_is_written_to_the_file() {
    let script = Keys::to_sound_and_speech()
        .taps(Key::Down, 9)
        .taps(Key::Return, 2)
        .tap(Key::Return)
        .quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    assert_eq!(said_in_submenu(&h)[9..12], ["on", "off", "on"]);
    assert!(config_text(&h).contains("\nnvda_direct = true\n"));
}

/// From the sound and speech menu's title to the speech rate item.
fn to_speech_rate() -> Keys {
    Keys::to_sound_and_speech().taps(Key::Down, 10)
}

const SPEECH_RATE_10: &str = "speech rate , 10 characters a second , press Enter to measure it";
const HOW: &str =
    "press a key to hear a sentence, then press a key the moment it ends. Escape cancels.";

#[test]
fn the_speech_rate_is_measured_from_the_start_of_the_sentence_to_the_key() {
    // The sentence has 130 characters; the key comes 7.8 seconds after it
    // starts: 0.06 seconds a character, or 17 characters a second.
    assert_eq!(words::MEASURE_SENTENCE.chars().count(), 130);
    let script = to_speech_rate()
        .tap(Key::Return)
        .tap_after(5.0, Key::Space)
        .tap_after(7.8, Key::Space)
        .tap(Key::Down)
        .quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let said = said_in_submenu(&h);
    assert_eq!(
        said[9..14],
        [
            SPEECH_RATE_10,
            HOW,
            words::MEASURE_SENTENCE,
            "measured: 17 characters a second . to measure again, press Enter",
            BACK,
        ]
    );
    assert_eq!(h.client.config.wait_delay_per_character, 0.06);
    assert!(config_text(&h).contains("\nwait_delay_per_character = 0.06\n"));

    // The estimate of the end of speech uses it at once: ten characters
    // now take 0.6 seconds, not 1.
    h.client.voice.item(&vec![MsgPart::from("abcdefghij")]);
    h.clock.advance(0.59);
    assert!(h.client.voice.is_busy());
    h.clock.advance(0.02);
    assert!(!h.client.voice.is_busy());
}

#[test]
fn a_key_too_early_or_too_late_is_refused_and_escape_cancels() {
    let script = to_speech_rate()
        // Half a second for 130 characters: too fast to be speech.
        .tap(Key::Return)
        .tap_after(5.0, Key::Space)
        .tap_after(0.5, Key::Space)
        // A minute: too slow.
        .tap(Key::Return)
        .tap_after(5.0, Key::Space)
        .tap_after(60.0, Key::Space)
        // Escape before the sentence, then during it.
        .tap(Key::Return)
        .tap_after(5.0, Key::Escape)
        .tap(Key::Return)
        .tap_after(5.0, Key::Space)
        .tap_after(5.0, Key::Escape)
        .tap(Key::Down)
        .quit();
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let said = said_in_submenu(&h);
    assert_eq!(
        said[9..21],
        [
            SPEECH_RATE_10,
            HOW,
            words::MEASURE_SENTENCE,
            "the key came too early to be the end of the sentence , nothing changed . to try again, press Enter",
            HOW,
            words::MEASURE_SENTENCE,
            "the key came too late to be the end of the sentence , nothing changed . to try again, press Enter",
            HOW,
            "nothing changed",
            HOW,
            words::MEASURE_SENTENCE,
            "nothing changed",
        ]
    );
    assert_eq!(said[21], BACK);
    assert_eq!(h.client.config.wait_delay_per_character, 0.1);
}

#[test]
fn a_held_key_that_starts_the_sentence_does_not_end_it_by_repeating() {
    // Space goes down to start and stays down: its repeats, from half a
    // second on, are not the end. Return, 6.5 seconds after the start, is:
    // 0.05 seconds a character.
    let mut keys = to_speech_rate().tap(Key::Return);
    keys.time += 5.0;
    keys.script = keys
        .script
        .repeat(0.5, 0.1)
        .at(keys.time)
        .key_down(Key::Space);
    let script = keys
        .tap_after(6.5, Key::Return)
        .tap_after(1.0, Key::Escape)
        .taps(Key::Escape, 2)
        .script;
    let mut h = Harness::new("en", script);
    assert_eq!(main_menu(&mut h.client), Ok(()));
    let said = said_in_submenu(&h);
    assert_eq!(
        said[11..13],
        [
            words::MEASURE_SENTENCE,
            "measured: 20 characters a second . to measure again, press Enter",
        ]
    );
    assert_eq!(h.client.config.wait_delay_per_character, 0.05);
}

#[test]
fn the_speed_chosen_in_the_game_menu_starts_the_next_game() {
    let live = LiveKeys::default();
    let mut h = Harness::new("en", Script::new().keep_open().with_live(live.clone()));
    let mut g = h.start_training("jl1", &["easy"], 1);
    // F10, then the fourth item: set the game speed to fast.
    let now = h.clock.now();
    live.tap_at(now + 1.0, Key::F10, Mods::NONE);
    for i in 0..4 {
        live.tap_at(now + 20.0 * f64::from(i + 1), Key::Down, Mods::NONE);
    }
    live.tap_at(now + 100.0, Key::Return, Mods::NONE);
    while !live.is_empty() {
        h.step(&mut g);
    }
    h.run_for(&mut g, 1.0);
    assert!(
        h.spoken()
            .contains(&"set the game speed to fast".to_owned())
    );
    assert_eq!(g.game.speed, 2.0);
    assert_eq!(h.client.config.speed, 2.0);
    assert!(config_text(&h).contains("\nspeed = 2.0\n"));
    let next = h.start_training("jl1", &["easy"], 1);
    assert_eq!(next.game.speed, 2.0);
}
