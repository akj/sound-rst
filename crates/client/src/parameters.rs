//! The `[parameters]` table of `settings.toml`: tuning values for sound,
//! speech and packages. The defaults are the values of the original's
//! `cfg/parameters.toml`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Parameters {
    /// When estimating how long speech lasts, a digit counts this many
    /// characters.
    pub tts_digit_coefficient: f64,
    /// How much the y coordinate of positional sounds is flattened: above 1,
    /// objects sound closer to each other.
    pub flattening_factor: f64,
    /// The volume of objects and terrain in the fog of war is multiplied by
    /// this factor.
    pub fog_of_war_factor: f64,
    /// The seconds between two animation updates.
    pub animation_delay: f64,
    /// Play the noises of the nearby terrain.
    pub render_nearby_land: bool,
    /// Play the noises and events of nearby objects.
    pub render_nearby_objects: bool,
    /// Stop the sounds of the square the cursor leaves.
    pub silence_previous_square: bool,
    /// Play the square's "when moving through" sound when the cursor moves.
    pub play_movement_sound: bool,
    /// How fast the listener moves to the cursor: 1000 is instant, 5 is
    /// smooth.
    pub observer_reactivity: f64,
    /// The volume of each package's sounds, by package name.
    pub default_volume: BTreeMap<String, f64>,
    /// The volume of a sound, by its name (`ambience/path`), over its
    /// package's.
    pub volume: BTreeMap<String, f64>,
    pub packages: Packages,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            tts_digit_coefficient: 3.0,
            flattening_factor: 2.0,
            fog_of_war_factor: 0.5,
            animation_delay: 0.1,
            render_nearby_land: true,
            render_nearby_objects: true,
            silence_previous_square: true,
            play_movement_sound: true,
            observer_reactivity: 1000.0,
            default_volume: BTreeMap::from([("res".into(), 1.0), ("soundpack".into(), 0.8)]),
            volume: BTreeMap::from([("ambience/path".into(), 0.5)]),
            packages: Packages::default(),
        }
    }
}

/// Where the packages are, relative to the game root; `user`, and a path
/// under it, is in the settings folder instead. A package is a folder
/// or a zip file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Packages {
    pub base: String,
    pub additional: Vec<String>,
}

impl Default for Packages {
    fn default() -> Self {
        Packages {
            base: "res".into(),
            additional: vec![String::new(), "user".into(), "user/downloaded".into()],
        }
    }
}

impl Parameters {
    /// The volume of the sound `name` from the package `mod_name`: its own
    /// entry in `volume`, else its package's in `default_volume`, else 1.
    pub fn sound_volume(&self, name: &str, mod_name: &str) -> f64 {
        self.volume
            .get(name)
            .or_else(|| self.default_volume.get(mod_name))
            .copied()
            .unwrap_or(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are the values of the original's `cfg/parameters.toml`,
    /// read as that file wrote them: flags as 0 or 1, numbers either way.
    #[test]
    fn the_defaults_are_the_originals_parameters() {
        let text =
            std::fs::read_to_string(crate::testkit::upstream_root().join("cfg/parameters.toml"))
                .unwrap();
        let original: toml::Table = text.parse().unwrap();
        let number = |key: &str| match &original[key] {
            toml::Value::Integer(i) => *i as f64,
            toml::Value::Float(f) => *f,
            v => panic!("{key} = {v}"),
        };
        let flag = |key: &str| number(key) != 0.0;
        let p = Parameters::default();
        assert_eq!(p.tts_digit_coefficient, number("tts_digit_coefficient"));
        assert_eq!(p.flattening_factor, number("flattening_factor"));
        assert_eq!(p.fog_of_war_factor, number("fog_of_war_factor"));
        assert_eq!(p.animation_delay, number("animation_delay"));
        assert_eq!(p.render_nearby_land, flag("render_nearby_land"));
        assert_eq!(p.render_nearby_objects, flag("render_nearby_objects"));
        assert_eq!(p.silence_previous_square, flag("silence_previous_square"));
        assert_eq!(p.play_movement_sound, flag("play_movement_sound"));
        assert_eq!(p.observer_reactivity, number("observer_reactivity"));
        let tables: toml::Table = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        for table in ["default_volume", "packages"] {
            assert_eq!(tables[table], original[table], "{table}");
        }
        // The original names a sound with its number: the import gives the
        // name.
        let names = soundrts_import::names::names();
        let original_volume: BTreeMap<String, f64> = original["volume"]
            .as_table()
            .unwrap()
            .iter()
            .map(|(number, volume)| {
                let name = names.game_name(number.parse().unwrap()).unwrap();
                (name.to_owned(), volume.as_float().unwrap())
            })
            .collect();
        assert_eq!(p.volume, original_volume);
        // Every value of the original is one of the parameters.
        let keys = |t: &toml::Table| {
            let mut k: Vec<String> = t.keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(keys(&tables), keys(&original));
        assert_eq!(p.sound_volume("ambience/path", "res"), 0.5);
        assert_eq!(p.sound_volume("1001", "soundpack"), 0.8);
        assert_eq!(p.sound_volume("1001", "default"), 1.0);
    }
}
