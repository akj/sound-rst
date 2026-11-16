//! `settings.toml`: the player's settings, read and written in temporary
//! folders.

use std::num::NonZeroU8;
use std::sync::Arc;

use soundrts::config::{Config, Login, login_is_valid};
use soundrts::log::Log;
use soundrts::message::{Category, Policies, Policy};
use soundrts::parameters::{Packages, Parameters};
use soundrts::resource::preferred_language;

const DEFAULT_FILE: &str = r#"login = "player"
mods = ""
soundpacks = ""
num_channels = 16
speed = 1.0
main_volume = 50
debug_mode = false
announce_arrivals = true
nvda_direct = false
wait_delay_per_character = 0.1

[news]
alert = "speak"
production = "speak"
economy = "speak"
scouting = "speak"
chat = "speak"
system = "speak"
story = "speak"

[parameters]
tts_digit_coefficient = 3.0
flattening_factor = 2.0
fog_of_war_factor = 0.5
animation_delay = 0.1
render_nearby_land = true
render_nearby_objects = true
silence_previous_square = true
play_movement_sound = true
observer_reactivity = 1000.0

[parameters.default_volume]
res = 1.0
soundpack = 0.8

[parameters.volume]
1000 = 0.5

[parameters.packages]
base = "res"
additional = ["", "user", "user/downloaded"]
"#;

#[test]
fn test_login_is_valid() {
    assert!(login_is_valid("a"));
    assert!(login_is_valid(&"a".repeat(20)));
    assert!(!login_is_valid(&"a".repeat(21)));
    assert!(!login_is_valid(""));
    assert!(!login_is_valid("a "));
    assert!(login_is_valid("a2"));
    assert!(!login_is_valid("wrong-login"));
}

#[test]
fn a_missing_file_is_written_with_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let c = Config::load(&path, &Log::silent());
    assert_eq!(c, Config::default());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
}

/// A value for each setting, none of them the default.
fn every_setting_changed() -> Config {
    let mut news = Policies::default();
    let others = [Policy::Sound, Policy::Log, Policy::Drop];
    for (category, policy) in Category::ALL.into_iter().zip(others.into_iter().cycle()) {
        news.set(category, policy);
    }
    let c = Config {
        login: Login::parse("bo2").unwrap(),
        language: Some("fr".into()),
        mods: "orc,crazy".into(),
        soundpacks: "loud".into(),
        num_channels: NonZeroU8::new(32).unwrap(),
        speed: 0.5,
        main_volume: 70,
        debug_mode: true,
        announce_arrivals: false,
        nvda_direct: true,
        wait_delay_per_character: 0.06,
        news,
        parameters: Arc::new(Parameters {
            tts_digit_coefficient: 1.0,
            flattening_factor: 0.5,
            fog_of_war_factor: 0.25,
            animation_delay: 0.2,
            render_nearby_land: false,
            render_nearby_objects: false,
            silence_previous_square: false,
            play_movement_sound: false,
            observer_reactivity: 5.0,
            default_volume: [("res".into(), 0.9)].into(),
            volume: [("1001".into(), 0.25), ("1000".into(), 0.75)].into(),
            packages: Packages {
                base: "base".into(),
                additional: vec!["more".into()],
            },
        }),
    };
    // Each field differs from its default, so a setting that is not read
    // back fails the test below.
    let d = Config::default();
    let p = &c.parameters;
    let dp = &d.parameters;
    assert!(c.login != d.login && c.language != d.language && c.mods != d.mods);
    assert!(c.soundpacks != d.soundpacks && c.num_channels != d.num_channels);
    assert!(c.speed != d.speed && c.main_volume != d.main_volume);
    assert!(c.debug_mode != d.debug_mode && c.announce_arrivals != d.announce_arrivals);
    assert!(c.nvda_direct != d.nvda_direct);
    assert!(c.wait_delay_per_character != d.wait_delay_per_character);
    for category in Category::ALL {
        assert_ne!(c.news.of(category), d.news.of(category), "{category:?}");
    }
    assert!(p.tts_digit_coefficient != dp.tts_digit_coefficient);
    assert!(p.flattening_factor != dp.flattening_factor);
    assert!(p.fog_of_war_factor != dp.fog_of_war_factor);
    assert!(p.animation_delay != dp.animation_delay);
    assert!(p.render_nearby_land != dp.render_nearby_land);
    assert!(p.render_nearby_objects != dp.render_nearby_objects);
    assert!(p.silence_previous_square != dp.silence_previous_square);
    assert!(p.play_movement_sound != dp.play_movement_sound);
    assert!(p.observer_reactivity != dp.observer_reactivity);
    assert!(p.default_volume != dp.default_volume && p.volume != dp.volume);
    assert!(p.packages.base != dp.packages.base);
    assert!(p.packages.additional != dp.packages.additional);
    c
}

#[test]
fn each_setting_is_read_back_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let log = Log::silent();
    let c = every_setting_changed();
    c.save(&path, &log);
    let text = std::fs::read_to_string(&path).unwrap();
    for line in [
        "login = \"bo2\"\n",
        "language = \"fr\"\n",
        "\nspeed = 0.5\n",
        "\ndebug_mode = true\n",
        "\nannounce_arrivals = false\n",
        "\nnvda_direct = true\n",
        "\n[news]\nalert = \"sound\"\nproduction = \"log\"\neconomy = \"drop\"\n",
        "\nrender_nearby_land = false\n",
    ] {
        assert!(text.contains(line), "{line:?} in {text}");
    }
    assert_eq!(Config::load(&path, &log), c);
}

#[test]
fn a_file_may_leave_settings_out_and_they_take_their_defaults() {
    let c = Config::from_toml("nvda_direct = true\n\n[news]\nchat = \"drop\"\n").unwrap();
    assert!(c.nvda_direct);
    assert_eq!(c.news.of(Category::Chat), Policy::Drop);
    assert_eq!(c.news.of(Category::Alert), Policy::Speak);
    assert_eq!(c.language, None);
    assert_eq!(
        Config {
            nvda_direct: false,
            news: Policies::default(),
            ..c
        },
        Config::default()
    );
    let c = Config::from_toml("[parameters.volume]\n2000 = 0.1\n").unwrap();
    assert_eq!(c.parameters.sound_volume("2000", "res"), 0.1);
    assert_eq!(c.parameters.sound_volume("1000", "res"), 1.0);
    assert_eq!(c.parameters.flattening_factor, 2.0);
    // A file saved by a text editor may start with a byte order mark.
    let c = Config::from_toml("\u{feff}login = \"bo2\"\n").unwrap();
    assert_eq!(c.login.as_str(), "bo2");
}

#[test]
fn a_bad_file_is_refused_with_the_line_of_its_error() {
    for (text, refused) in [
        (
            "login = \"player\"\nmain_volume = 150\n",
            "line 2: 150 is above 100",
        ),
        (
            "login = \"a b\"\n",
            "line 1: a login is 1 to 20 letters and digits",
        ),
        ("\nspeed = 0\n", "line 2: 0 is not a number above 0"),
        ("speed = nan\n", "line 1: NaN is not a number above 0"),
        (
            "wait_delay_per_character = -1\n",
            "line 1: -1 is not a number above 0",
        ),
        (
            "num_channels = 0\n",
            "line 1: invalid value: integer `0`, expected a nonzero u8",
        ),
        (
            "nvda_direct = 1\n",
            "line 1: invalid type: integer `1`, expected a boolean",
        ),
        (
            "\n\n[news]\nchat = \"shout\"\n",
            "line 4: unknown variant `shout`, expected one of `speak`, `sound`, `log`, `drop`",
        ),
        (
            "[news]\nverbosity = \"food\"\n",
            "line 2: unknown field `verbosity`, expected one of `alert`, `production`, \
             `economy`, `scouting`, `chat`, `system`, `story`",
        ),
        (
            "[parameters]\nflattening_factor = \"x\"\n",
            "line 2: invalid type: string \"x\", expected f64",
        ),
        (
            "[general]\nlogin = player\n",
            "line 2: string values must be quoted, expected literal string",
        ),
    ] {
        assert_eq!(
            Config::from_toml(text)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(refused.to_owned()),
            "{text:?}"
        );
    }
    let refused = Config::from_toml("login = \"player\"\nnvda_drect = true\n").unwrap_err();
    assert_eq!(refused.line, Some(2));
    assert!(refused.reason.starts_with("unknown field `nvda_drect`"));
}

#[test]
fn a_refused_file_is_kept_as_old_and_rewritten_with_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let log_path = dir.path().join("client.log");
    let log = Log::file_only(&log_path);
    let bad = "login = \"bo2\"\nmain_volume = 150\n";
    std::fs::write(&path, bad).unwrap();
    assert_eq!(Config::load(&path, &log), Config::default());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
    let old = dir.path().join("settings.toml.old");
    assert_eq!(std::fs::read_to_string(&old).unwrap(), bad);
    let logged = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        logged.contains("is refused, and the defaults are used: line 2: 150 is above 100"),
        "{logged}"
    );
}

/// Bytes that are not settings are refused, on a stack of 1 MiB, the size
/// of a Windows program's main thread, which reads the file.
#[test]
fn any_bytes_are_refused_without_a_crash() {
    let deep = format!("a = {}{}", "[".repeat(100_000), "]".repeat(100_000));
    let cases: Vec<Vec<u8>> = vec![
        vec![0xff, 0xfe, 0x00, 0x41],
        b"\0\0\0".to_vec(),
        b"num_channels = 99999999999999999999999\n".to_vec(),
        b"num_channels = 256\n".to_vec(),
        b"main_volume = -1\n".to_vec(),
        b"speed = inf\n".to_vec(),
        b"[news]\n[news]\n".to_vec(),
        b"login = \"a\"\nlogin = \"b\"\n".to_vec(),
        b"parameters = 1\n".to_vec(),
        deep.into_bytes(),
    ];
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.toml");
            for bytes in cases {
                let text = String::from_utf8_lossy(&bytes);
                let start: String = text.chars().take(40).collect();
                assert!(Config::from_toml(&text).is_err(), "{start:?}");
                std::fs::write(&path, &bytes).unwrap();
                assert_eq!(Config::load(&path, &Log::silent()), Config::default());
                assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn the_language_comes_from_the_file_else_the_system() {
    let log = Log::silent();
    let c = Config::from_toml("language = \"fr\"\n").unwrap();
    assert_eq!(
        preferred_language(c.language.as_deref(), &log),
        Some("fr".to_owned())
    );
    // An empty language is the system's, as no language is.
    assert_eq!(
        preferred_language(Some(" "), &log),
        preferred_language(None, &log)
    );
    // Without a language, the file does not name one.
    assert!(!Config::default().to_toml().contains("language"));
}
