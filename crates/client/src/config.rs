//! `SoundRTS.ini`: the player's settings, read and written in the format of
//! Python's configparser so an existing file keeps working.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::log::Log;

/// A login: 1 to 20 ASCII letters and digits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login(String);

impl Login {
    pub fn parse(s: &str) -> Option<Login> {
        login_is_valid(s).then(|| Login(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Login {
    fn default() -> Self {
        Login(DEFAULT_LOGIN.to_owned())
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub const DEFAULT_LOGIN: &str = "player";

pub fn login_is_valid(login: &str) -> bool {
    (1..=20).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The settings of `SoundRTS.ini`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub login: Login,
    pub mods: String,
    pub soundpacks: String,
    pub num_channels: i64,
    pub speed: i64,
    pub verbosity: String,
    pub debug_mode: i64,
    pub timeout: f64,
    pub fpct_coef: f64,
    pub fpct_max: i64,
    pub require_humans: i64,
    pub wait_delay_per_character: f64,
    /// Say when a player arrives on or leaves the server (1) or not (0).
    pub announce_arrivals: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            login: Login::default(),
            mods: String::new(),
            soundpacks: String::new(),
            num_channels: 16,
            speed: 1,
            verbosity:
                "menu_changed,unit_added,unit_complete,scout_info,food,resources,resource_exhausted"
                    .to_owned(),
            debug_mode: 0,
            timeout: 60.0,
            fpct_coef: 2.3,
            fpct_max: 3,
            require_humans: 0,
            wait_delay_per_character: 0.1,
            announce_arrivals: 1,
        }
    }
}

/// An ini file read as sections of options. Option names are lower case,
/// as configparser stores them.
type Ini = HashMap<String, HashMap<String, String>>;

/// Formats a float as Python's `str()` does for ordinary values.
fn py_float(v: f64) -> String {
    format!("{v:?}")
}

fn parse_ini(text: &str) -> Result<Ini, String> {
    let mut ini = Ini::new();
    let mut section: Option<String> = None;
    let mut last_key: Option<String> = None;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.is_empty() {
            last_key = None;
            continue;
        }
        let indented = raw.starts_with([' ', '\t']);
        if indented && let (Some(s), Some(k)) = (&section, &last_key) {
            let value = ini
                .get_mut(s)
                .and_then(|o| o.get_mut(k))
                .expect("known key");
            value.push('\n');
            value.push_str(line);
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if ini.contains_key(name) {
                return Err(format!("line {}: section [{name}] repeated", number + 1));
            }
            ini.insert(name.to_owned(), HashMap::new());
            section = Some(name.to_owned());
            last_key = None;
            continue;
        }
        let Some(s) = &section else {
            return Err(format!("line {}: no section header", number + 1));
        };
        let split = line.find(['=', ':']);
        let Some(at) = split else {
            return Err(format!("line {}: not an option: {line}", number + 1));
        };
        let key = line[..at].trim().to_lowercase();
        let value = line[at + 1..].trim().to_owned();
        let options = ini.get_mut(s).expect("current section");
        if options.contains_key(&key) {
            return Err(format!("line {}: option {key} repeated", number + 1));
        }
        options.insert(key.clone(), value);
        last_key = Some(key);
    }
    Ok(ini)
}

/// What a value of the file became.
enum Read<T> {
    Value(T),
    Missing,
    Invalid(String),
}

fn read<T>(ini: &Ini, section: &str, option: &str, convert: impl Fn(&str) -> Option<T>) -> Read<T> {
    match ini.get(section).and_then(|o| o.get(option)) {
        None => Read::Missing,
        Some(raw) => match convert(raw) {
            Some(v) => Read::Value(v),
            None => Read::Invalid(raw.clone()),
        },
    }
}

fn int(s: &str) -> Option<i64> {
    s.trim().replace('_', "").parse().ok()
}

fn float(s: &str) -> Option<f64> {
    s.trim().replace('_', "").parse().ok()
}

fn text(s: &str) -> Option<String> {
    Some(s.to_owned())
}

impl Config {
    fn rows(&self) -> [(&'static str, &'static str, String); 13] {
        [
            ("general", "login", self.login.to_string()),
            ("general", "mods", self.mods.clone()),
            ("general", "soundpacks", self.soundpacks.clone()),
            ("general", "num_channels", self.num_channels.to_string()),
            ("general", "speed", self.speed.to_string()),
            ("general", "verbosity", self.verbosity.clone()),
            ("general", "debug_mode", self.debug_mode.to_string()),
            (
                "general",
                "announce_arrivals",
                self.announce_arrivals.to_string(),
            ),
            ("server", "timeout", py_float(self.timeout)),
            ("server", "fpct_coef", py_float(self.fpct_coef)),
            ("server", "fpct_max", self.fpct_max.to_string()),
            ("server", "require_humans", self.require_humans.to_string()),
            (
                "tts",
                "wait_delay_per_character",
                py_float(self.wait_delay_per_character),
            ),
        ]
    }

    /// The file content, as configparser writes it.
    pub fn to_ini(&self) -> String {
        let mut out = String::new();
        let mut current = "";
        for (section, option, value) in self.rows() {
            if section != current {
                if !current.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[{section}]\n"));
                current = section;
            }
            out.push_str(&format!("{option} = {value}\n"));
        }
        out.push('\n');
        out
    }

    /// Reads the settings from an ini text. Each missing or invalid value
    /// takes its default; the flag says whether that happened.
    pub fn from_ini(text_: &str, log: &Log) -> (Config, bool) {
        let ini = match parse_ini(text_) {
            Ok(ini) => ini,
            Err(e) => {
                log.warning(format!("cannot read the configuration: {e}"));
                return (Config::default(), true);
            }
        };
        let mut c = Config::default();
        let mut error = false;
        macro_rules! field {
            ($section:literal, $name:ident, $convert:expr) => {
                match read(&ini, $section, stringify!($name), $convert) {
                    Read::Value(v) => c.$name = v,
                    Read::Missing => {
                        log.info(format!(
                            "{:?} option is missing (will be: {:?})",
                            stringify!($name),
                            c.$name.to_string()
                        ));
                        error = true;
                    }
                    Read::Invalid(raw) => {
                        log.warning(format!(
                            "{} will be {:?} instead of {:?}",
                            stringify!($name),
                            c.$name.to_string(),
                            raw
                        ));
                        error = true;
                    }
                }
            };
        }
        field!("general", login, Login::parse);
        field!("general", mods, text);
        field!("general", soundpacks, text);
        field!("general", num_channels, int);
        field!("general", speed, int);
        field!("general", verbosity, text);
        field!("general", debug_mode, int);
        field!("server", timeout, float);
        field!("server", fpct_coef, float);
        field!("server", fpct_max, int);
        field!("server", require_humans, int);
        field!("tts", wait_delay_per_character, float);
        // Not an option of the original, whose files lack it: missing is
        // not an error.
        match read(&ini, "general", "announce_arrivals", int) {
            Read::Value(v) => c.announce_arrivals = v,
            Read::Missing => {}
            Read::Invalid(raw) => {
                log.warning(format!(
                    "announce_arrivals will be \"1\" instead of {raw:?}"
                ));
                error = true;
            }
        }
        (c, error)
    }

    /// Reads `path`. A missing file is created with the defaults. A file
    /// with errors is copied to `<path>.old`, then rewritten. A file that
    /// exists but cannot be read is left alone, and the defaults are used.
    pub fn load(path: &Path, log: &Log) -> Config {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                let c = Config::default();
                if e.kind() == std::io::ErrorKind::NotFound {
                    c.save(path, log);
                } else {
                    log.warning(format!("cannot read {}: {e}", path.display()));
                }
                return c;
            }
        };
        let (c, error) = Config::from_ini(&String::from_utf8_lossy(&bytes), log);
        if error {
            log.warning(format!("Error in {}.", path.display()));
            let mut old = path.as_os_str().to_owned();
            old.push(".old");
            match std::fs::copy(path, &old) {
                Ok(_) => log.warning("made a copy of the old config file"),
                Err(_) => log.warning("could not make a copy of the old config file"),
            }
            log.warning(format!("Rewriting {}...", path.display()));
            c.save(path, log);
        }
        c
    }

    pub fn save(&self, path: &Path, log: &Log) {
        if let Err(e) = std::fs::write(path, self.to_ini()) {
            log.warning(format!("cannot write {}: {e}", path.display()));
        }
    }
}
