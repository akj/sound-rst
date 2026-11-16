//! `settings.toml`: the player's settings, in the settings folder. Each
//! setting the file leaves out takes its default. A file that is not
//! valid is refused as a whole, with the line of its first error.

use std::fmt;
use std::num::NonZeroU8;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::log::Log;
use crate::message::Policies;
use crate::parameters::Parameters;

/// A login: 1 to 20 ASCII letters and digits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
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

impl TryFrom<String> for Login {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Login, Self::Error> {
        Login::parse(&s).ok_or("a login is 1 to 20 letters and digits")
    }
}

impl From<Login> for String {
    fn from(login: Login) -> String {
        login.0
    }
}

pub const DEFAULT_LOGIN: &str = "player";

pub fn login_is_valid(login: &str) -> bool {
    (1..=20).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The settings of `settings.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub login: Login,
    /// The code of the language the player chose, such as `fr`; without
    /// one, the system's language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub mods: String,
    pub soundpacks: String,
    pub num_channels: NonZeroU8,
    /// The speed a single-player game starts at: 1 is normal.
    #[serde(deserialize_with = "positive")]
    pub speed: f64,
    /// The main volume of the game's sounds, in percent.
    #[serde(deserialize_with = "percent")]
    pub main_volume: u8,
    /// Write each message the voice says to the log.
    pub debug_mode: bool,
    /// Say when a player arrives on or leaves the server.
    pub announce_arrivals: bool,
    /// Speak through NVDA's own controller client, which says when speech
    /// ends, rather than through Tolk only.
    pub nvda_direct: bool,
    /// How long the screen reader takes to say one character, in seconds:
    /// the game estimates from it when speech ends.
    #[serde(deserialize_with = "positive")]
    pub wait_delay_per_character: f64,
    /// What the player hears of each category of news.
    pub news: Policies,
    pub parameters: Arc<Parameters>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            login: Login::default(),
            language: None,
            mods: String::new(),
            soundpacks: String::new(),
            num_channels: NonZeroU8::new(16).expect("16 is not 0"),
            speed: 1.0,
            main_volume: 50,
            debug_mode: false,
            announce_arrivals: true,
            nvda_direct: false,
            wait_delay_per_character: 0.1,
            news: Policies::default(),
            parameters: Arc::default(),
        }
    }
}

/// A number above 0: a speed or a duration.
fn positive<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(serde::de::Error::custom(format!(
            "{v} is not a number above 0"
        )))
    }
}

fn percent<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let v = u8::deserialize(d)?;
    if v <= 100 {
        Ok(v)
    } else {
        Err(serde::de::Error::custom(format!("{v} is above 100")))
    }
}

/// Why a settings file was refused: its first error, and the line of the
/// error when the reader knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub line: Option<usize>,
    pub reason: String,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

impl Config {
    /// The file's text.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("the settings are always valid TOML")
    }

    /// Reads the settings from the file's text. A text editor may begin
    /// the file with a byte order mark.
    pub fn from_toml(text: &str) -> Result<Config, Refused> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        toml::from_str(text).map_err(|e| Refused {
            line: e
                .span()
                .and_then(|span| text.get(..span.start))
                .map(|before| before.matches('\n').count() + 1),
            reason: e.message().trim_end().to_owned(),
        })
    }

    /// Reads `path`. A missing file is created with the defaults. A file
    /// that is refused is copied to `<path>.old`, then rewritten with the
    /// defaults. A file that exists but cannot be read is left alone, and
    /// the defaults are used.
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
        match Config::from_toml(&String::from_utf8_lossy(&bytes)) {
            Ok(c) => c,
            Err(refused) => {
                log.warning(format!(
                    "{} is refused, and the defaults are used: {refused}",
                    path.display()
                ));
                let mut old = path.as_os_str().to_owned();
                old.push(".old");
                match std::fs::copy(path, &old) {
                    Ok(_) => log.warning(format!(
                        "the refused file is kept as {}",
                        Path::new(&old).display()
                    )),
                    Err(e) => log.warning(format!("cannot keep the refused file: {e}")),
                }
                let c = Config::default();
                c.save(path, log);
                c
            }
        }
    }

    pub fn save(&self, path: &Path, log: &Log) {
        if let Err(e) = std::fs::write(path, self.to_toml()) {
            log.warning(format!("cannot write {}: {e}", path.display()));
        }
    }
}
