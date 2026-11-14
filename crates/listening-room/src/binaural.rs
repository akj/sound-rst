//! The binaural renderer, on Steam Audio: each source goes through a
//! direct effect (distance, air absorption, closed exits) and then a
//! binaural effect (the head model). The place's echo is Steam Audio's
//! parametric reverb, with decay times from the scene. Nothing is
//! simulated: every value comes from the scene, and there is no geometry.

use std::collections::HashMap;
use std::path::Path;

use audionimbus::{
    AirAbsorptionModel, AudioBufferMut, AudioBufferRef, AudioSettings, BinauralEffect,
    BinauralEffectParams, BinauralEffectSettings, Context, ContextSettings, DirectEffect,
    DirectEffectParams, DirectEffectSettings, DistanceAttenuationModel, Equalizer, Hrtf,
    HrtfInterpolation, HrtfSettings, Parametric, ReflectionEffect, ReflectionEffectParams,
    ReflectionEffectSettings, Sofa, Transmission, Vector3, air_absorption, distance_attenuation,
};

use crate::render::{BLOCK, Renderer, block_of};
use crate::scene::{RATE, Scene};

/// How much of a closed exit's sound gets through, in Steam Audio's low,
/// middle and high bands.
const BLOCKED_TRANSMISSION: [f32; 3] = [0.7, 0.3, 0.1];
/// How loud the place's reverb is against the direct sound.
const REVERB_WET: f32 = 1.5;
/// The longest reverb the effect is made for, in samples.
const REVERB_LENGTH: u32 = 4 * RATE;

struct Voice {
    direct: DirectEffect,
    binaural: BinauralEffect,
}

pub struct Binaural {
    context: Context,
    settings: AudioSettings,
    hrtf: Hrtf,
    voices: HashMap<u32, Voice>,
    reverb: ReflectionEffect<Parametric>,
    /// The decay times the reverb runs with; kept after the listener
    /// leaves a place, so its echo dies away as it would.
    reverb_times: Option<[f32; 3]>,
}

fn steam(e: impl std::fmt::Display) -> String {
    format!("Steam Audio: {e}")
}

impl Binaural {
    /// Starts Steam Audio with its default head model, or the one in the
    /// SOFA file `sofa`.
    pub fn new(sofa: Option<&Path>) -> Result<Binaural, String> {
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
        let reverb = ReflectionEffect::<Parametric>::try_new(
            &context,
            &settings,
            &ReflectionEffectSettings {
                impulse_response_size: REVERB_LENGTH,
                num_channels: 1,
            },
        )
        .map_err(steam)?;
        Ok(Binaural {
            context,
            settings,
            hrtf,
            voices: HashMap::new(),
            reverb,
            reverb_times: None,
        })
    }

    fn voice(&mut self, id: u32) -> &mut Voice {
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
        })
    }
}

impl Renderer for Binaural {
    fn render(&mut self, scene: &Scene, start: usize, out: &mut [f32]) {
        self.voices
            .retain(|id, _| scene.sources.iter().any(|s| s.id == *id));
        let origin = Vector3::new(0.0, 0.0, 0.0);
        let mut reverb_in = [0.0f32; BLOCK];
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
            let mut input = block_of(&s.sound, start);
            for x in &mut input {
                *x *= s.loudness as f32;
            }
            let mut direct = [0.0f32; BLOCK];
            let mut ears = [0.0f32; 2 * BLOCK];
            let hrtf = self.hrtf.clone();
            let voice = self.voice(s.id);
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
            for (r, d) in reverb_in.iter_mut().zip(direct) {
                *r += d;
            }
        }
        let feeding = scene.place.decay_times();
        if feeding.is_some() {
            self.reverb_times = feeding;
        } else {
            reverb_in = [0.0; BLOCK];
        }
        if let Some(times) = self.reverb_times {
            let mut wet = [0.0f32; 2 * BLOCK];
            self.reverb
                .apply(
                    &ReflectionEffectParams::<Parametric>::new(times, 1, REVERB_LENGTH),
                    &AudioBufferRef::try_from(&reverb_in[..]).expect("a mono block"),
                    &mut AudioBufferMut::try_new(&mut wet[..], 2).expect("a stereo block"),
                )
                .expect("the parametric reverb takes one channel in");
            for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                frame[0] += REVERB_WET * wet[i];
                frame[1] += REVERB_WET * wet[BLOCK + i];
            }
        }
    }
}
