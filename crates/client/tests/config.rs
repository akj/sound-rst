//! Port of upstream `tests/test_config.py`. Files are copied to a
//! temporary folder; the upstream files are never written.

use soundrts::config::{Config, Reading, login_is_valid};
use soundrts::log::Log;
use soundrts::message::{Category, Policies, Policy};
use soundrts::testkit::upstream_tests_dir;

fn copy_to_temp(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::copy(upstream_tests_dir().join(name), &path).unwrap();
    (dir, path)
}

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
fn test_load_defaults_if_file_doesnt_exist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_a_file.ini");
    let c = Config::load(&path, &Log::silent());
    assert_eq!(c.login.as_str(), "player");
    assert_eq!(c.num_channels, 16);
    assert_eq!(c.timeout, 60.0);
    // The defaults are written, in configparser's format.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[general]\nlogin = player\nmods = \nsoundpacks = \nnum_channels = 16\nspeed = 1\n\
         debug_mode = 0\nannounce_arrivals = 1\n\n[server]\ntimeout = 60.0\nfpct_coef = 2.3\nfpct_max = 3\nrequire_humans = 0\n\n\
         [tts]\nwait_delay_per_character = 0.1\nnvda_direct = 0\n\n\
         [news]\nalert = speak\nproduction = speak\neconomy = speak\nscouting = speak\nchat = speak\nsystem = speak\nstory = speak\n\n"
    );
}

#[test]
fn nvda_direct_is_off_unless_set_and_survives_a_save() {
    let log = Log::silent();
    let (c, _) = Config::from_ini("[tts]\nwait_delay_per_character = 0.1\n", &log);
    assert_eq!(c.nvda_direct, 0);
    let (c, _) = Config::from_ini("[tts]\nnvda_direct = 1\n", &log);
    assert_eq!(c.nvda_direct, 1);
    assert!(
        c.to_ini()
            .contains("[tts]\nwait_delay_per_character = 0.1\nnvda_direct = 1\n")
    );
}

#[test]
fn test_load_file_with_changes() {
    let (_dir, path) = copy_to_temp("config_with_changes.ini");
    for _ in 0..2 {
        let c = Config::load(&path, &Log::silent());
        assert_eq!(c.login.as_str(), "test");
        assert_eq!(c.num_channels, 32);
        assert_eq!(c.timeout, 200.0);
    }
}

#[test]
fn test_load_defaults_if_file_with_errors() {
    let (_dir, path) = copy_to_temp("config_with_errors.ini");
    let original = std::fs::read_to_string(&path).unwrap();
    let c = Config::load(&path, &Log::silent());
    assert_ne!(std::fs::read_to_string(&path).unwrap(), original);
    let mut old = path.clone().into_os_string();
    old.push(".old");
    assert_eq!(std::fs::read_to_string(old).unwrap(), original);
    assert_eq!(c.login.as_str(), "player");
    assert_eq!(c.num_channels, 16);
    assert_eq!(c.timeout, 60.0);
    assert_eq!(c.mods, "");
    // The values that were valid are kept.
    assert_eq!(c.soundpacks, "");
    assert_eq!(c.fpct_coef, 2.3);
}

#[test]
fn test_load_and_save() {
    let (dir, path) = copy_to_temp("config_with_changes.ini");
    let n2 = dir.path().join("config_with_changes2.ini");
    let log = Log::silent();
    let mut c = Config::load(&path, &log);
    assert_eq!(
        (c.login.as_str(), c.num_channels, c.timeout),
        ("test", 32, 200.0)
    );
    c.login = soundrts::config::Login::parse("a").unwrap();
    c.num_channels = 1;
    c.timeout = 1.0;
    c.save(&n2, &log);
    let c = Config::load(&n2, &log);
    assert_eq!((c.login.as_str(), c.num_channels, c.timeout), ("a", 1, 1.0));
}

/// A complete file of the original game, with this `verbosity` line, then
/// `more`.
fn original_file(verbosity: &str, more: &str) -> String {
    let current = Config::default().to_ini();
    let general = current.split("[news]").next().unwrap();
    let general = general.replace(
        "speed = 1\n",
        &format!("speed = 1\nverbosity = {verbosity}\n"),
    );
    format!("{general}{more}")
}

/// A complete file of this version with this `[news]` section.
fn current_file(news: &str) -> String {
    let current = Config::default().to_ini();
    let before = current.split("[news]").next().unwrap();
    format!("{before}[news]\n{news}")
}

#[test]
fn every_category_is_spoken_unless_the_news_section_says_otherwise() {
    let log = Log::silent();
    let (c, reading) = Config::from_ini(
        &current_file("chat = drop\nalert = sound\nstory = log\n"),
        &log,
    );
    assert_eq!(reading, Reading::Clean);
    let policies: Vec<Policy> = Category::ALL.iter().map(|&k| c.news.of(k)).collect();
    let (speak, sound) = (Policy::Speak, Policy::Sound);
    assert_eq!(
        policies,
        [sound, speak, speak, speak, Policy::Drop, speak, Policy::Log]
    );
    let (again, reading) = Config::from_ini(&c.to_ini(), &log);
    assert_eq!((again.news, reading), (c.news, Reading::Clean));
}

#[test]
fn a_policy_that_is_not_one_is_an_error_and_takes_its_default() {
    let (c, reading) = Config::from_ini(&current_file("chat = shout\n"), &Log::silent());
    assert_eq!(reading, Reading::Errors);
    assert_eq!(c.news, Policies::default());
}

#[test]
fn an_old_verbosity_line_chooses_the_policies_once_and_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SoundRTS.ini");
    std::fs::write(&path, original_file("unit_complete,food", "")).unwrap();
    let log = Log::silent();
    let c = Config::load(&path, &log);
    // Production and economy are named; scouting is not, so it plays only
    // its sounds, as the original did.
    assert_eq!(c.news.of(Category::Production), Policy::Speak);
    assert_eq!(c.news.of(Category::Economy), Policy::Speak);
    assert_eq!(c.news.of(Category::Scouting), Policy::Sound);
    assert_eq!(c.news.of(Category::Alert), Policy::Speak);
    let rewritten = std::fs::read_to_string(&path).unwrap();
    assert!(!rewritten.contains("verbosity"), "{rewritten}");
    assert!(rewritten.contains("[news]\n"), "{rewritten}");
    assert!(rewritten.contains("scouting = sound\n"), "{rewritten}");
    // Not an error: no copy of the old file.
    let mut old = path.clone().into_os_string();
    old.push(".old");
    assert!(!std::path::Path::new(&old).exists());
    assert_eq!(Config::load(&path, &log).news, c.news);
}

#[test]
fn the_original_default_verbosity_speaks_everything() {
    let verbosity =
        "menu_changed,unit_added,unit_complete,scout_info,food,resources,resource_exhausted";
    let (c, reading) = Config::from_ini(&original_file(verbosity, ""), &Log::silent());
    assert_eq!(reading, Reading::Outdated);
    assert_eq!(c.news, Policies::default());
}

#[test]
fn a_verbosity_line_beside_a_news_section_is_ignored() {
    let (c, reading) = Config::from_ini(
        &original_file("food", "[news]\nalert = log\n"),
        &Log::silent(),
    );
    assert_eq!(reading, Reading::Outdated);
    assert_eq!(c.news.of(Category::Production), Policy::Speak);
    assert_eq!(c.news.of(Category::Alert), Policy::Log);
}
