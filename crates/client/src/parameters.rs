//! `cfg/parameters.toml`: tuning values for sound, speech and packages.

use std::path::Path;

use crate::log::Log;

/// The parameters file, with typed accessors for the values the client
/// reads. Any other value stays available through [`Parameters::get`].
#[derive(Clone, Debug, Default)]
pub struct Parameters {
    table: toml::Table,
}

impl Parameters {
    pub fn from_toml(text: &str, log: &Log) -> Parameters {
        match text.parse::<toml::Table>() {
            Ok(table) => Parameters { table },
            Err(e) => {
                log.warning(format!("error in parameters.toml: {e}"));
                Parameters::default()
            }
        }
    }

    /// Reads the file. A missing file gives an error; a file with a syntax
    /// error gives empty parameters, as upstream did.
    pub fn load(path: &Path, log: &Log) -> std::io::Result<Parameters> {
        let bytes = std::fs::read(path)?;
        Ok(Self::from_toml(&String::from_utf8_lossy(&bytes), log))
    }

    pub fn get(&self, key: &str) -> Option<&toml::Value> {
        self.table.get(key)
    }

    /// A number, integer or float, or the default.
    pub fn number(&self, key: &str, default: f64) -> f64 {
        self.table.get(key).and_then(as_number).unwrap_or(default)
    }

    /// A flag written as a number or a boolean.
    pub fn flag(&self, key: &str) -> bool {
        self.flag_or(key, false)
    }

    /// A flag, or `default` when the file does not set it.
    pub fn flag_or(&self, key: &str, default: bool) -> bool {
        match self.table.get(key) {
            Some(toml::Value::Boolean(b)) => *b,
            Some(v) => as_number(v).is_some_and(|n| n != 0.0),
            None => default,
        }
    }

    /// When estimating how long speech lasts, a digit counts this many
    /// characters.
    pub fn tts_digit_coefficient(&self) -> f64 {
        self.number("tts_digit_coefficient", 1.0)
    }

    /// How much the y coordinate of positional sounds is flattened.
    pub fn flattening_factor(&self) -> f64 {
        self.number("flattening_factor", 2.0)
    }

    /// The volume of the sound `name` from the package `mod_name`: its own
    /// entry in `[volume]`, else its package's in `[default_volume]`, else 1.
    pub fn sound_volume(&self, name: &str, mod_name: &str) -> f64 {
        let entry = |table: &str, key: &str| {
            self.table
                .get(table)
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get(key))
                .and_then(as_number)
        };
        entry("volume", name)
            .or_else(|| entry("default_volume", mod_name))
            .unwrap_or(1.0)
    }

    /// The base package path and the additional package paths, relative to
    /// the game root.
    pub fn packages(&self) -> (String, Vec<String>) {
        let packages = self.table.get("packages").and_then(toml::Value::as_table);
        let base = packages
            .and_then(|p| p.get("base"))
            .and_then(toml::Value::as_str)
            .unwrap_or("res")
            .to_owned();
        let additional = packages
            .and_then(|p| p.get("additional"))
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(toml::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(|| vec![String::new(), "user".into(), "user/downloaded".into()]);
        (base, additional)
    }
}

fn as_number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Integer(i) => Some(*i as f64),
        toml::Value::Float(f) => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upstream_parameters_give_the_upstream_values() {
        let text =
            std::fs::read_to_string(crate::testkit::upstream_root().join("cfg/parameters.toml"))
                .unwrap();
        let p = Parameters::from_toml(&text, &Log::silent());
        assert_eq!(p.tts_digit_coefficient(), 3.0);
        assert_eq!(p.flattening_factor(), 2.0);
        assert_eq!(p.sound_volume("1000", "default"), 0.5);
        assert_eq!(p.sound_volume("1001", "soundpack"), 0.8);
        assert_eq!(p.sound_volume("1001", "default"), 1.0);
        assert_eq!(
            p.packages(),
            (
                "res".to_owned(),
                vec![
                    "".to_owned(),
                    "user".to_owned(),
                    "user/downloaded".to_owned()
                ]
            )
        );
    }
}
