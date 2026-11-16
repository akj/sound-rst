//! The sounds the script plays: the game's own when its data is found,
//! otherwise generated noise bursts, and a bright test sound that is always
//! generated.

use std::path::{Path, PathBuf};

use soundrts::audio::SoundBuffer;
use soundrts::paths::Paths;

use crate::rng::Rng;
use crate::scene::{RATE, Sound};

/// Mono samples at [`RATE`].
pub type Clip = Vec<f32>;

/// Where the sounds came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The game's data folder.
    Game(PathBuf),
    /// Generated, because the game's sounds could not be read; says why.
    Generated(String),
}

pub struct Sounds {
    /// Two footsteps on grass (the game's `footstep/grass_1` and
    /// `footstep/grass_2`).
    pub footsteps: [Clip; 2],
    /// A dragon's wings (`move/wings_1` and `move/wings_2`).
    pub wings: [Clip; 2],
    /// A blacksmith at work (`ambience/blacksmith`).
    pub building: Clip,
    /// A short burst of white noise with a fast attack: unlike the game's
    /// footsteps, most of its energy is above 6 kHz, where the head
    /// model's cues for front and behind are.
    pub bright: Clip,
    pub origin: Origin,
}

impl Sounds {
    /// The game's sounds, from the data folder found as the game finds
    /// it (`cli_root`, then `SOUNDRTS_ROOT`, the program's folder and the
    /// current folder). Falls back to generated sounds.
    pub fn load(cli_root: Option<&Path>) -> Sounds {
        match Paths::from_environment(cli_root) {
            Ok(paths) => Sounds::from_game(&paths.root).unwrap_or_else(Sounds::generated),
            Err(e) => {
                let tried: Vec<String> = e.tried.iter().map(|p| p.display().to_string()).collect();
                Sounds::generated(format!(
                    "no game data in {}",
                    if tried.is_empty() {
                        "any folder".to_owned()
                    } else {
                        tried.join(", ")
                    }
                ))
            }
        }
    }

    fn from_game(root: &Path) -> Result<Sounds, String> {
        let effect = |name: &str| {
            let path = root.join("res/sounds").join(format!("{name}.ogg"));
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let buffer = SoundBuffer::decode(bytes, name, "default", 1.0)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            let mono: Vec<f32> = buffer
                .samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|f| (f[0] + f[1]) / 2.0)
                .collect();
            Ok::<Clip, String>(resample(&mono, buffer.sample_rate))
        };
        Ok(Sounds {
            footsteps: [effect("footstep/grass_1")?, effect("footstep/grass_2")?],
            wings: [effect("move/wings_1")?, effect("move/wings_2")?],
            building: effect("ambience/blacksmith")?,
            bright: bright(),
            origin: Origin::Game(root.to_path_buf()),
        })
    }

    /// Noise bursts in place of the game's sounds: short and bright for
    /// footsteps, longer and softer for wings, knocks for the building.
    pub fn generated(reason: String) -> Sounds {
        let mut rng = Rng::new(1);
        let mut burst = |seconds: f64, cutoff: f32, envelope: &dyn Fn(f64) -> f32| {
            let n = (seconds * f64::from(RATE)) as usize;
            let a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / RATE as f32).exp();
            let mut z = 0.0;
            (0..n)
                .map(|i| {
                    z += a * (rng.noise() - z);
                    z * envelope(i as f64 / f64::from(RATE))
                })
                .collect::<Clip>()
        };
        let step = |t: f64| 0.9 * (-t / 0.012).exp() as f32;
        let wing = |t: f64| 0.6 * (std::f64::consts::PI * t / 0.3).sin() as f32;
        let knock = |t: f64| 0.8 * (-(t % 0.25) / 0.02).exp() as f32;
        Sounds {
            footsteps: [burst(0.06, 4_000.0, &step), burst(0.06, 3_000.0, &step)],
            wings: [burst(0.3, 1_500.0, &wing), burst(0.3, 1_200.0, &wing)],
            building: burst(1.5, 5_000.0, &knock),
            bright: bright(),
            origin: Origin::Generated(reason),
        }
    }
}

/// 40 milliseconds of white noise, flat up to 24 kHz, that rises in half a
/// millisecond and dies away by 60 dB in 40.
fn bright() -> Clip {
    let mut rng = Rng::new(2);
    (0..RATE as usize / 25)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let envelope = (t / 0.0005).min(1.0) * (-t / 0.0058).exp();
            0.9 * envelope * rng.noise()
        })
        .collect()
}

/// Linear resampling to [`RATE`].
pub fn resample(input: &[f32], from: u32) -> Clip {
    if from == RATE || input.is_empty() {
        return input.to_vec();
    }
    let step = f64::from(from) / f64::from(RATE);
    let n = (input.len() as f64 / step) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * step;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j];
            let b = input.get(j + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

/// `count` clips, taken in turn from `clips`, one every `every` seconds.
pub fn train(clips: &[Clip], every: f64, count: usize) -> Sound {
    let mut out = Vec::new();
    for i in 0..count {
        let at = (i as f64 * every * f64::from(RATE)) as usize;
        let clip = &clips[i % clips.len()];
        if out.len() < at + clip.len() {
            out.resize(at + clip.len(), 0.0);
        }
        for (o, s) in out[at..].iter_mut().zip(clip) {
            *o += s;
        }
    }
    out.into()
}

/// Silence for `seconds`, then `sound`.
pub fn after(seconds: f64, sound: &[f32]) -> Clip {
    let mut out = vec![0.0; (seconds * f64::from(RATE)) as usize];
    out.extend_from_slice(sound);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_length_in_seconds() {
        let one_second = vec![0.5; 22_050];
        let out = resample(&one_second, 22_050);
        assert_eq!(out.len(), RATE as usize);
        assert!(out.iter().all(|&s| s == 0.5));
    }

    #[test]
    fn a_train_places_its_clips_at_their_times() {
        let t = train(&[vec![1.0], vec![2.0]], 0.5, 3);
        assert_eq!(t.len(), RATE as usize + 1);
        assert_eq!(t[0], 1.0);
        assert_eq!(t[RATE as usize / 2], 2.0);
        assert_eq!(t[RATE as usize], 1.0);
        assert_eq!(t.iter().filter(|&&s| s != 0.0).count(), 3);
    }
}
