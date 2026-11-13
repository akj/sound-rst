//! Background noises of objects and terrain: a sound in a loop, or one of
//! several sounds every few seconds, following the object.

use crate::rng::ClientRng;
use crate::sound::{Audio, Placement, SourceId};
use crate::sound_cache::Sounds;
use crate::style::Noise;

/// What the noise needs to know of its object at each update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseSite {
    /// In meters.
    pub x: f64,
    pub y: f64,
    /// The object is in the fog of war: the noise is quieter.
    pub fow: bool,
}

/// The services a noise plays through.
pub struct NoisePlayer<'a> {
    pub audio: &'a Audio,
    pub sounds: &'a Sounds,
    pub rng: &'a ClientRng,
    pub now: f64,
    /// The volume factor of objects in the fog of war.
    pub fog_of_war_factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Loop {
        sound: String,
        volume: f64,
    },
    Repeat {
        interval: f64,
        sounds: Vec<String>,
        ambient: bool,
        next: Option<f64>,
    },
}

/// A playing noise. It names the style attribute it comes from, so that a
/// change of attribute replaces it even when both say the same.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseSource {
    pub attr: String,
    pub style: Noise,
    kind: Kind,
    source: Option<SourceId>,
}

impl NoiseSource {
    /// The noise of a style value from `attr`; nothing for a silent or
    /// unreadable value.
    pub fn new(attr: &str, style: &Noise) -> Option<NoiseSource> {
        let kind = match style {
            Noise::Loop { sound, volume, .. } => Kind::Loop {
                sound: sound.clone(),
                volume: *volume,
            },
            Noise::Repeat {
                interval,
                sounds,
                ambient,
            } => Kind::Repeat {
                interval: *interval,
                sounds: sounds.clone(),
                ambient: *ambient,
                next: None,
            },
            _ => return None,
        };
        Some(NoiseSource {
            attr: attr.to_owned(),
            style: style.clone(),
            kind,
            source: None,
        })
    }

    /// Whether this noise was made from `attr` with this value.
    pub fn is(&self, attr: &str, style: &Noise) -> bool {
        self.attr == attr && self.style == *style
    }

    /// Plays, follows the object, or starts the next repeated sound.
    pub fn update(&mut self, p: &NoisePlayer, site: NoiseSite) {
        let fow = if site.fow { p.fog_of_war_factor } else { 1.0 };
        match &mut self.kind {
            Kind::Loop { sound, volume } => {
                let volume = *volume * fow;
                match self.source.filter(|&s| !p.audio.source_ended(s)) {
                    Some(s) => {
                        p.audio.set_source_volume(s, volume);
                        p.audio.move_source(s, site.x, site.y);
                    }
                    None => {
                        self.source = p.audio.play_loop(
                            p.sounds.get_sound(sound),
                            Placement::new(volume, site.x, site.y, -10),
                        );
                    }
                }
            }
            Kind::Repeat {
                interval,
                sounds,
                ambient,
                next,
            } => match *next {
                None => *next = Some(p.now + p.rng.random() * *interval),
                Some(n) => {
                    if p.now > n && self.source.is_none_or(|s| p.audio.source_ended(s)) {
                        let placement = Placement {
                            ambient: *ambient,
                            ..Placement::new(fow, site.x, site.y, -20)
                        };
                        self.source = if sounds.is_empty() {
                            None
                        } else {
                            let name = &sounds[p.rng.below(sounds.len())];
                            p.audio.play(p.sounds.get_sound(name), placement)
                        };
                        *next = Some(p.now + *interval * (0.8 + p.rng.random() * 0.4));
                    }
                }
            },
        }
    }

    pub fn stop(&self, audio: &Audio) {
        if let Some(s) = self.source {
            audio.stop_source(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{AudioEvent, Harness, Script};

    fn gains(h: &Harness) -> Vec<(f32, f32)> {
        h.audio
            .events()
            .into_iter()
            .filter_map(|e| match e {
                AudioEvent::Gains { left, right, .. } => Some((left, right)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn noises_in_the_fog_of_war_are_quieter() {
        for (fow, factor) in [(false, 1.0f32), (true, 0.5)] {
            let h = Harness::new("en", Script::new());
            let cx = &h.client;
            let player = NoisePlayer {
                audio: &cx.audio,
                sounds: &cx.sounds,
                rng: &cx.rng,
                now: 0.0,
                fog_of_war_factor: 0.5,
            };
            let style = Noise::Loop {
                sound: "1016".into(),
                volume: 0.5,
                ambient: false,
            };
            let mut n = NoiseSource::new("noise", &style).unwrap();
            assert!(n.is("noise", &style) && !n.is("noise_if_damaged", &style));
            // At the listener: both speakers at sin(45°) of the volume,
            // times the main volume (0.5).
            n.update(
                &player,
                NoiseSite {
                    x: 0.0,
                    y: 0.0,
                    fow,
                },
            );
            let expected = std::f32::consts::FRAC_1_SQRT_2 * 0.5 * 0.5 * factor;
            let g = gains(&h);
            assert_eq!(g.len(), 1, "{fow}");
            assert!((g[0].0 - expected).abs() < 1e-6, "{fow}: {g:?}");
            assert!((g[0].1 - expected).abs() < 1e-6, "{fow}: {g:?}");
        }
    }

    #[test]
    fn a_repeated_noise_waits_then_plays_at_the_fog_volume() {
        let h = Harness::new("en", Script::new());
        let cx = &h.client;
        let style = Noise::Repeat {
            interval: 5.0,
            sounds: vec!["1016".into()],
            ambient: false,
        };
        let mut n = NoiseSource::new("noise", &style).unwrap();
        let at = |now| NoisePlayer {
            audio: &cx.audio,
            sounds: &cx.sounds,
            rng: &cx.rng,
            now,
            fog_of_war_factor: 0.5,
        };
        let site = NoiseSite {
            x: 0.0,
            y: 0.0,
            fow: true,
        };
        // The first update only draws when the first sound comes.
        n.update(&at(0.0), site);
        assert!(h.audio.played().is_empty());
        n.update(&at(5.0), site);
        assert_eq!(h.audio.played().len(), 1);
        let expected = std::f32::consts::FRAC_1_SQRT_2 * 0.5 * 0.5;
        let g = gains(&h);
        assert!((g[0].0 - expected).abs() < 1e-6, "{g:?}");
        assert!(NoiseSource::new("noise", &Noise::Silent).is_none());
    }
}
