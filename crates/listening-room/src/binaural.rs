//! The binaural renderers, on Steam Audio: each source goes through a
//! direct effect (distance, air absorption, closed exits) and then a
//! binaural effect (the head model). Nothing is simulated: every value
//! comes from the scene, and there is no geometry.
//!
//! [`Binaural::plus`] doubles the head model's front-behind cue with plain
//! ones before the direct effect: a sound behind is duller, with the same
//! filter as `stereo-plus`, and a little quieter.

use std::collections::HashMap;
use std::path::Path;

use audionimbus::{
    AirAbsorptionModel, AudioBufferMut, AudioBufferRef, AudioSettings, BinauralEffect,
    BinauralEffectParams, BinauralEffectSettings, Context, ContextSettings, DirectEffect,
    DirectEffectParams, DirectEffectSettings, DistanceAttenuationModel, Equalizer, Hrtf,
    HrtfInterpolation, HrtfSettings, Sofa, Transmission, Vector3, air_absorption,
    distance_attenuation,
};
use soundrts::audio::{BEHIND_CUTOFF, LowPass};

use crate::render::{BLOCK, Renderer, block_of, lerp};
use crate::scene::{RATE, Scene};

/// How much of a closed exit's sound gets through, in Steam Audio's low,
/// middle and high bands.
const BLOCKED_TRANSMISSION: [f32; 3] = [0.7, 0.3, 0.1];
/// With plain cues, a sound straight behind plays at this fraction of its
/// volume (about 3 dB down).
const BEHIND_GAIN: f32 = 0.7;

struct Voice {
    direct: DirectEffect,
    binaural: BinauralEffect,
    /// With plain cues: 0 beside or in front, 1 straight behind, as of the
    /// last block.
    behind: f32,
    behind_filter: LowPass,
}

pub struct Binaural {
    context: Context,
    settings: AudioSettings,
    hrtf: Hrtf,
    voices: HashMap<u32, Voice>,
    /// Whether a sound behind is also duller and quieter.
    plain_cues: bool,
}

fn steam(e: impl std::fmt::Display) -> String {
    format!("Steam Audio: {e}")
}

impl Binaural {
    /// The head model alone: Steam Audio's default, or the one in the SOFA
    /// file `sofa`.
    pub fn new(sofa: Option<&Path>) -> Result<Binaural, String> {
        Binaural::with(sofa, false)
    }

    /// The head model, with a sound behind also duller and quieter.
    pub fn plus(sofa: Option<&Path>) -> Result<Binaural, String> {
        Binaural::with(sofa, true)
    }

    fn with(sofa: Option<&Path>, plain_cues: bool) -> Result<Binaural, String> {
        let context = Context::try_new(&ContextSettings::default()).map_err(steam)?;
        let settings = AudioSettings {
            sampling_rate: RATE,
            frame_size: BLOCK as u32,
        };
        let hrtf_settings = HrtfSettings {
            sofa_information: sofa.map(|p| Sofa::Filename(p.to_string_lossy().into_owned())),
            ..HrtfSettings::default()
        };
        let hrtf = Hrtf::try_new(&context, &settings, &hrtf_settings).map_err(|e| match sofa {
            Some(p) => format!("Steam Audio cannot use the head model {}: {e}", p.display()),
            None => steam(e),
        })?;
        Ok(Binaural {
            context,
            settings,
            hrtf,
            voices: HashMap::new(),
            plain_cues,
        })
    }

    fn voice(&mut self, id: u32, behind: f32) -> &mut Voice {
        let (context, settings, hrtf) = (&self.context, &self.settings, &self.hrtf);
        self.voices.entry(id).or_insert_with(|| Voice {
            direct: DirectEffect::try_new(
                context,
                settings,
                &DirectEffectSettings { num_channels: 1 },
            )
            .expect("a direct effect"),
            binaural: BinauralEffect::try_new(
                context,
                settings,
                &BinauralEffectSettings { hrtf: hrtf.clone() },
            )
            .expect("a binaural effect"),
            behind,
            behind_filter: LowPass::new(BEHIND_CUTOFF, RATE),
        })
    }
}

impl Renderer for Binaural {
    fn render(&mut self, scene: &Scene, start: usize, out: &mut [f32]) {
        self.voices
            .retain(|id, _| scene.sources.iter().any(|s| s.id == *id));
        let origin = Vector3::new(0.0, 0.0, 0.0);
        for s in &scene.sources {
            // Steam Audio's listener space: x right, y up, z behind.
            let (right, ahead, up) = scene.listener.relative(s.position);
            let at = Vector3::new(right as f32, up as f32, -ahead as f32);
            let distance = (right * right + ahead * ahead + up * up).sqrt() as f32;
            let direction = if distance > 1e-6 {
                Vector3::new(at.x / distance, at.y / distance, at.z / distance)
            } else {
                Vector3::new(0.0, 0.0, -1.0)
            };
            let attenuation = distance_attenuation(
                &self.context,
                at,
                origin,
                &DistanceAttenuationModel::Default,
            );
            // SAFETY: the binding's only hazard is the callback model; the
            // default model has no callback.
            let air =
                unsafe { air_absorption(&self.context, at, origin, &AirAbsorptionModel::Default) };
            let params = DirectEffectParams {
                distance_attenuation: Some(attenuation),
                air_absorption: Some(air),
                directivity: None,
                occlusion: Some(if s.blocked { 0.0 } else { 1.0 }),
                transmission: s
                    .blocked
                    .then_some(Transmission::FrequencyDependent(Equalizer(
                        BLOCKED_TRANSMISSION,
                    ))),
            };
            let behind = direction.z.max(0.0);
            let plain_cues = self.plain_cues;
            let hrtf = self.hrtf.clone();
            let voice = self.voice(s.id, behind);
            let mut input = block_of(&s.sound, start);
            for (i, x) in input.iter_mut().enumerate() {
                *x *= s.loudness as f32;
                if plain_cues {
                    // Spread over the block, as stereo-plus does, so a
                    // moving source cannot click.
                    let w = lerp(voice.behind, behind, (i + 1) as f32 / BLOCK as f32);
                    *x += w * (voice.behind_filter.process(*x) - *x);
                    *x *= 1.0 - w * (1.0 - BEHIND_GAIN);
                }
            }
            voice.behind = behind;
            let mut direct = [0.0f32; BLOCK];
            let mut ears = [0.0f32; 2 * BLOCK];
            voice
                .direct
                .apply(
                    &params,
                    &AudioBufferRef::try_from(&input[..]).expect("a mono block"),
                    &mut AudioBufferMut::try_from(&mut direct[..]).expect("a mono block"),
                )
                .expect("the direct effect takes one channel");
            voice
                .binaural
                .apply(
                    &BinauralEffectParams {
                        direction,
                        interpolation: HrtfInterpolation::Bilinear,
                        spatial_blend: 1.0,
                        hrtf,
                        peak_delays: None,
                    },
                    &AudioBufferRef::try_from(&direct[..]).expect("a mono block"),
                    &mut AudioBufferMut::try_new(&mut ears[..], 2).expect("a stereo block"),
                )
                .expect("the binaural effect takes one channel in and two out");
            for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                frame[0] += ears[i];
                frame[1] += ears[BLOCK + i];
            }
        }
    }
}
