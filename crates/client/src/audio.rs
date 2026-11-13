//! The audio engine: decoded sounds, mixer channels with left and right
//! gains, and the stereo math of positional sound.
//!
//! Channel 0 is reserved for the voice; the other channels play effects.
//! The output device sits behind [`AudioOutput`], so tests can record what
//! would play.

use std::f64::consts::PI;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// The volume of each speaker for a sound "in front of you" (about 0.7).
pub const DEFAULT_VOLUME: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// A decoded sound, as stereo samples at the file's sample rate.
pub struct SoundBuffer {
    /// The sound's name: the file name without `.ogg`.
    pub key: String,
    /// The package layer it came from ("default" for the base package).
    pub mod_name: String,
    /// Interleaved left and right samples.
    pub samples: Arc<[f32]>,
    pub sample_rate: u32,
    volume: AtomicU32,
}

/// A sound as the game holds it. Two handles are the same sound only if
/// they point to the same buffer.
pub type Sound = Arc<SoundBuffer>;

impl fmt::Debug for SoundBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<Sound {} from {}>", self.key, self.mod_name)
    }
}

impl SoundBuffer {
    pub fn new(
        key: &str,
        mod_name: &str,
        samples: Vec<f32>,
        sample_rate: u32,
        volume: f64,
    ) -> Self {
        SoundBuffer {
            key: key.to_owned(),
            mod_name: mod_name.to_owned(),
            samples: samples.into(),
            sample_rate: sample_rate.max(1),
            volume: AtomicU32::new((volume as f32).to_bits()),
        }
    }

    /// Decodes an Ogg Vorbis file. Mono becomes two equal channels; beyond
    /// two channels, the first two are kept. The sound ends where the
    /// stream says it ends (its last granule position), not at the end of
    /// the last decoded block.
    pub fn decode(bytes: Vec<u8>, key: &str, mod_name: &str, volume: f64) -> Result<Self, String> {
        let end = last_granule_position(&bytes);
        let decoder =
            rodio::Decoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let channels = usize::from(rodio::Source::channels(&decoder).get());
        let rate = rodio::Source::sample_rate(&decoder).get();
        let raw: Vec<f32> = decoder.collect();
        let mut samples: Vec<f32> = match channels {
            1 => raw.iter().flat_map(|&s| [s, s]).collect(),
            2 => raw,
            n => raw.chunks(n).flat_map(|f| [f[0], f[1]]).collect(),
        };
        if let Some(frames) = end.and_then(|e| usize::try_from(e).ok()) {
            samples.truncate(frames.saturating_mul(2));
        }
        Ok(SoundBuffer::new(key, mod_name, samples, rate, volume))
    }

    /// The sound's own volume, from the parameters, between 0 and 1.
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, volume: f64) {
        self.volume
            .store((volume.clamp(0.0, 1.0) as f32).to_bits(), Ordering::Relaxed);
    }

    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        (self.samples.len() / 2) as f64 / f64::from(self.sample_rate)
    }
}

/// The granule position of the last page of an Ogg stream: for Vorbis, the
/// number of samples per channel in the whole stream. `None` if the bytes
/// end without a valid page.
fn last_granule_position(bytes: &[u8]) -> Option<u64> {
    let start = bytes.windows(4).rposition(|w| w == b"OggS")?;
    let header = bytes.get(start..start + 14)?;
    if header[4] != 0 {
        return None; // unknown version
    }
    let granule = i64::from_le_bytes(header[6..14].try_into().ok()?);
    u64::try_from(granule).ok().filter(|&g| g > 0)
}

/// An effects channel: any mixer channel except the voice channel. Only
/// the mixer makes them, so an effect can never land on the voice channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectChannel(u16);

impl EffectChannel {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// A mixer channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Channel 0, reserved for the voice.
    Voice,
    Effect(EffectChannel),
}

impl Channel {
    pub fn index(self) -> usize {
        match self {
            Channel::Voice => 0,
            Channel::Effect(c) => c.index(),
        }
    }
}

/// The device side of the mixer: plays one sound per channel, with left
/// and right gains that can change while it plays. Gains belong to the
/// channel and stay until changed.
pub trait AudioOutput {
    /// Starts `sound` on `channel`, replacing what played there.
    fn play(&mut self, channel: Channel, sound: &Sound, looping: bool);
    fn set_gains(&mut self, channel: Channel, left: f32, right: f32);
    fn stop(&mut self, channel: Channel);
    /// Whether the channel is still playing.
    fn is_busy(&self, channel: Channel) -> bool;
}

/// An output that plays nothing; every channel is idle.
pub struct NullOutput;

impl AudioOutput for NullOutput {
    fn play(&mut self, _: Channel, _: &Sound, _: bool) {}
    fn set_gains(&mut self, _: Channel, _: f32, _: f32) {}
    fn stop(&mut self, _: Channel) {}
    fn is_busy(&self, _: Channel) -> bool {
        false
    }
}

/// The channels, and which sound each one plays.
pub struct Mixer {
    output: Box<dyn AudioOutput>,
    current: Vec<Option<Sound>>,
}

impl Mixer {
    /// A mixer with `num_channels` channels, the first one for the voice.
    pub fn new(output: Box<dyn AudioOutput>, num_channels: usize) -> Mixer {
        Mixer {
            output,
            current: vec![None; num_channels.max(1)],
        }
    }

    pub fn num_channels(&self) -> usize {
        self.current.len()
    }

    pub fn play(&mut self, channel: Channel, sound: &Sound, looping: bool) {
        self.current[channel.index()] = Some(sound.clone());
        self.output.play(channel, sound, looping);
    }

    pub fn set_gains(&mut self, channel: Channel, left: f64, right: f64) {
        self.output.set_gains(channel, left as f32, right as f32);
    }

    pub fn stop(&mut self, channel: Channel) {
        self.current[channel.index()] = None;
        self.output.stop(channel);
    }

    pub fn is_busy(&self, channel: Channel) -> bool {
        self.output.is_busy(channel)
    }

    /// The sound the channel is playing, if it is playing one.
    pub fn sound_on(&self, channel: Channel) -> Option<&Sound> {
        if self.is_busy(channel) {
            self.current[channel.index()].as_ref()
        } else {
            None
        }
    }

    /// Every effects channel, in order.
    pub fn effect_channels(&self) -> impl Iterator<Item = EffectChannel> + use<> {
        (1..self.current.len()).map(|n| EffectChannel(n as u16))
    }

    /// The first effects channel that is not playing.
    pub fn find_idle_channel(&self) -> Option<EffectChannel> {
        self.effect_channels()
            .find(|&c| !self.is_busy(Channel::Effect(c)))
    }

    /// Stops every channel, the voice channel included.
    pub fn stop_all(&mut self) {
        self.stop(Channel::Voice);
        for c in self.effect_channels() {
            self.stop(Channel::Effect(c));
        }
    }

    /// Stops every effects channel.
    pub fn stop_effects(&mut self) {
        for c in self.effect_channels() {
            self.stop(Channel::Effect(c));
        }
    }
}

pub fn distance(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    (dx * dx + dy * dy).sqrt()
}

/// The angle, in radians, of (x2, y2) for a listener at (x1, y1) facing
/// `o` degrees (0 is east, 90 is north).
pub fn angle(x1: f64, y1: f64, x2: f64, y2: f64, o: f64) -> f64 {
    let d = distance(x1, y1, x2, y2);
    if d == 0.0 {
        return 0.0; // too close: in front of the listener
    }
    let ac = ((x2 - x1) / d).acos();
    let a = if y2 - y1 > 0.0 { ac } else { -ac };
    a - o.to_radians()
}

/// The left and right volumes of a sound at (xo, yo) for a listener at
/// (x, y) facing `o` degrees. Sounds behind are quieter; far sounds are
/// quieter unless `no_distance`.
pub fn stereo(
    x: f64,
    y: f64,
    xo: f64,
    yo: f64,
    o: f64,
    volume: f64,
    no_distance: bool,
) -> (f64, f64) {
    let a = angle(x, y, xo, yo, o);
    let d = if no_distance {
        1.0
    } else {
        distance(x, y, xo, yo).max(1.0)
    };
    let mut vg = (a.sin() + 1.0) / 2.0;
    let mut vd = 1.0 - vg;
    vg = (vg * PI / 2.0).sin();
    vd = (vd * PI / 2.0).sin();
    if a.cos() < 0.0 {
        // behind
        let k = if no_distance { 1.3 } else { 2.0 };
        vg /= k;
        vd /= k;
    }
    ((vg * volume / d).min(1.0), (vd * volume / d).min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (x, y, xo, yo, o, volume, no_distance), then (left, right).
    type Case = ((f64, f64, f64, f64, f64, f64, bool), (f64, f64));

    /// The reference values are printed by upstream's `stereo()` run in
    /// Python (vendor/scratch_stereo.py), copied as printed.
    #[test]
    #[allow(clippy::approx_constant)]
    fn stereo_matches_upstream() {
        let cases: [Case; 10] = [
            (
                (0.0, 0.0, 0.0, 0.0, 0.0, 1.0, false),
                (0.7071067811865476, 0.7071067811865476),
            ),
            ((0.0, 0.0, 10.0, 0.0, 90.0, 1.0, false), (0.0, 0.1)),
            (
                (0.0, 0.0, 0.0, 10.0, 90.0, 1.0, false),
                (0.07071067811865475, 0.07071067811865475),
            ),
            (
                (0.0, 0.0, -3.0, 4.0, 90.0, 1.0, false),
                (0.1902113032590307, 0.0618033988749895),
            ),
            (
                (0.0, 0.0, 0.0, -10.0, 90.0, 1.0, false),
                (0.035355339059327376, 0.035355339059327376),
            ),
            (
                (5.0, 5.0, 6.0, 5.0, 0.0, 1.0, false),
                (0.7071067811865476, 0.7071067811865476),
            ),
            (
                (0.0, 0.0, 3.0, 4.0, 0.0, 0.5, false),
                (0.09876883405951378, 0.015643446504023086),
            ),
            (
                (0.0, 0.0, -1.0, -1.0, 90.0, 1.0, true),
                (0.7489675212633317, 0.1753956339936138),
            ),
            (
                (10.0, 20.0, 12.0, 10.0, 45.0, 2.0, false),
                (0.01289710707288226, 0.09720622019467287),
            ),
            (
                (0.0, 0.0, 0.5, 0.2, 90.0, 1.0, false),
                (0.05614473683096407, 0.9984226402311708),
            ),
        ];
        for ((x, y, xo, yo, o, v, nd), (l, r)) in cases {
            let (gl, gr) = stereo(x, y, xo, yo, o, v, nd);
            assert!(
                (gl - l).abs() < 1e-12 && (gr - r).abs() < 1e-12,
                "{x},{y},{xo},{yo},{o}: got {gl},{gr}, want {l},{r}"
            );
        }
        assert!((angle(0.0, 0.0, -3.0, 4.0, 90.0) - 0.6435011087932843).abs() < 1e-12);
        assert_eq!(angle(1.0, 1.0, 1.0, 1.0, 0.0), 0.0);
        assert_eq!(DEFAULT_VOLUME, 0.7071067811865476);
    }

    #[test]
    fn mono_sounds_are_decoded_as_stereo() {
        let path = crate::testkit::upstream_root().join("res/ui/effects/1029.ogg");
        let s = SoundBuffer::decode(std::fs::read(path).unwrap(), "1029", "default", 0.5).unwrap();
        assert!(s.duration() > 0.0);
        assert_eq!(s.samples.len() % 2, 0);
        assert_eq!(s.volume(), 0.5);
    }

    #[test]
    fn a_sound_ends_at_the_last_granule_position_of_its_stream() {
        // 1003.ogg: mono, 44100 Hz, last granule position 804, as pygame
        // reads it (18231 microseconds).
        let path = crate::testkit::upstream_root().join("res/ui/effects/1003.ogg");
        let s = SoundBuffer::decode(std::fs::read(path).unwrap(), "1003", "default", 1.0).unwrap();
        assert_eq!(s.samples.len(), 2 * 804);
        assert_eq!((s.duration() * 1e6).round(), 18231.0);
    }
}
