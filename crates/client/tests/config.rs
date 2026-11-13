//! Port of upstream `tests/test_config.py`. Files are copied to a
//! temporary folder; the upstream files are never written.

use soundrts::config::{Config, login_is_valid};
use soundrts::log::Log;
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
         verbosity = menu_changed,unit_added,unit_complete,scout_info,food,resources,resource_exhausted\n\
         debug_mode = 0\nannounce_arrivals = 1\n\n[server]\ntimeout = 60.0\nfpct_coef = 2.3\nfpct_max = 3\nrequire_humans = 0\n\n\
         [tts]\nwait_delay_per_character = 0.1\n\n"
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
