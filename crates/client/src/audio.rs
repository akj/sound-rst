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

/// The most frames (a sample for each channel) a sound may decode to:
/// five minutes at 48 kHz. A longer sound is refused. The original's
/// longest sound is 51 seconds.
pub const MAX_SOUND_FRAMES: usize = 5 * 60 * 48_000;

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
    /// the last decoded block. A sound longer than [`MAX_SOUND_FRAMES`] is
    /// refused, without decoding more than that, and so is a file the
    /// decoder fails on, even by panicking.
    pub fn decode(bytes: Vec<u8>, key: &str, mod_name: &str, volume: f64) -> Result<Self, String> {
        std::panic::catch_unwind(|| {
            Self::decode_within(bytes, key, mod_name, volume, MAX_SOUND_FRAMES)
        })
        .unwrap_or_else(|_| Err("the decoder failed on this file".to_owned()))
    }

    #[deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::string_slice,
        clippy::arithmetic_side_effects
    )]
    fn decode_within(
        bytes: Vec<u8>,
        key: &str,
        mod_name: &str,
        volume: f64,
        max_frames: usize,
    ) -> Result<Self, String> {
        let end = last_granule_position(&bytes);
        let decoder =
            rodio::Decoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let channels = usize::from(rodio::Source::channels(&decoder).get());
        let rate = rodio::Source::sample_rate(&decoder).get();
        let mut samples: Vec<f32> = Vec::new();
        let mut frame: Vec<f32> = Vec::with_capacity(channels);
        for s in decoder {
            frame.push(s);
            if frame.len() < channels {
                continue;
            }
            if samples.len() >= max_frames.saturating_mul(2) {
                return Err(format!(
                    "longer than {max_frames} frames, the most a sound may have"
                ));
            }
            match frame.as_slice() {
                [mono] => samples.extend([*mono, *mono]),
                [left, right, ..] => samples.extend([*left, *right]),
                [] => {}
            }
            frame.clear();
        }
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
#[deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]
fn last_granule_position(bytes: &[u8]) -> Option<u64> {
    let start = bytes.windows(4).rposition(|w| w == b"OggS")?;
    let header = bytes.get(start..start.checked_add(14)?)?;
    if header.get(4) != Some(&0) {
        return None; // unknown version
    }
    let granule = i64::from_le_bytes(header.get(6..14)?.try_into().ok()?);
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
    /// Starts `sound` on `channel`, replacing what played there, at the
    /// channel's gains. Set them first: the device may take the sound's
    /// first frames during this call, and a later change is ramped.
    fn play(&mut self, channel: Channel, sound: &Sound, looping: bool);
    /// Sets the channel's gains, and how far behind the listener its sound
    /// is (see [`behind`]): 0 plays the sound as recorded, 1 at its dullest.
    fn set_gains(&mut self, channel: Channel, left: f32, right: f32, behind: f32);
    fn stop(&mut self, channel: Channel);
    /// Whether the channel is still playing.
    fn is_busy(&self, channel: Channel) -> bool;
}

/// An output that plays nothing; every channel is idle.
pub struct NullOutput;

impl AudioOutput for NullOutput {
    fn play(&mut self, _: Channel, _: &Sound, _: bool) {}
    fn set_gains(&mut self, _: Channel, _: f32, _: f32, _: f32) {}
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

    pub fn set_gains(&mut self, channel: Channel, left: f64, right: f64, behind: f64) {
        self.output
            .set_gains(channel, left as f32, right as f32, behind as f32);
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

/// How far behind the listener a sound at (xo, yo) is, for the arguments
/// [`stereo`] takes: 0 in front or beside, rising smoothly to 1 straight
/// behind.
pub fn behind(x: f64, y: f64, xo: f64, yo: f64, o: f64) -> f64 {
    (-angle(x, y, xo, yo, o).cos()).max(0.0)
}

/// The cutoff of the duller tone behind, in Hz: a sound straight behind is
/// low-passed here.
pub const BEHIND_CUTOFF: f32 = 2_500.0;

/// Two one-pole low-pass filters in a row (12 dB an octave).
#[derive(Clone, Copy)]
pub struct LowPass {
    a: f32,
    z1: f32,
    z2: f32,
}

impl LowPass {
    /// A filter at `cutoff` Hz for samples at `sample_rate`.
    pub fn new(cutoff: f32, sample_rate: u32) -> LowPass {
        let a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / sample_rate as f32).exp();
        LowPass {
            a,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        self.z1 += self.a * (x - self.z1);
        self.z2 += self.a * (self.z1 - self.z2);
        self.z2
    }

    /// `x` blended towards its low-passed self by `amount`, from 0 (`x`
    /// itself, exactly) to 1 (low-passed). The filter runs either way, so
    /// the amount can change smoothly.
    pub fn blend(&mut self, x: f32, amount: f32) -> f32 {
        x + amount * (self.process(x) - x)
    }
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
    fn behind_rises_smoothly_from_beside_to_straight_behind() {
        // A listener at the origin facing north; `deg` is the direction of
        // the sound, 0 east, 90 north.
        let at = |deg: f64| {
            let a = deg.to_radians();
            behind(0.0, 0.0, a.cos(), a.sin(), 90.0)
        };
        assert_eq!(at(90.0), 0.0);
        assert!(at(0.0) < 1e-15 && at(180.0) < 1e-15, "beside");
        assert!((at(-90.0) - 1.0).abs() < 1e-12);
        assert!((at(-45.0) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        let degree = 1f64.to_radians();
        for d in -180..180 {
            let d = f64::from(d);
            assert!((at(d + 1.0) - at(d)).abs() <= degree, "at {d}");
        }
        // Behind is where `stereo` halves the volume.
        for d in [-170.0_f64, -90.0, -10.0] {
            let a = d.to_radians();
            let (l, r) = stereo(0.0, 0.0, a.cos(), a.sin(), 90.0, 1.0, true);
            let (fl, fr) = stereo(0.0, 0.0, a.cos(), -a.sin(), 90.0, 1.0, true);
            assert!(at(d) > 0.0 && l < fl && r < fr, "at {d}");
        }
    }

    #[test]
    fn mono_sounds_are_decoded_as_stereo() {
        let path = crate::testkit::game_root().join("res/sounds/ui/beep.ogg");
        let s =
            SoundBuffer::decode(std::fs::read(path).unwrap(), "ui/beep", "default", 0.5).unwrap();
        assert!(s.duration() > 0.0);
        assert_eq!(s.samples.len() % 2, 0);
        assert_eq!(s.volume(), 0.5);
    }

    #[test]
    fn a_sound_ends_at_the_last_granule_position_of_its_stream() {
        // 1003.ogg: mono, 44100 Hz, last granule position 804, as pygame
        // reads it (18231 microseconds).
        let path = crate::testkit::game_root().join("res/sounds/ui/positional_beep.ogg");
        let s = SoundBuffer::decode(
            std::fs::read(path).unwrap(),
            "ui/positional_beep",
            "default",
            1.0,
        )
        .unwrap();
        assert_eq!(s.samples.len(), 2 * 804);
        assert_eq!((s.duration() * 1e6).round(), 18231.0);
    }

    #[test]
    fn a_sound_longer_than_the_limit_is_refused() {
        // The original's longest sound: 2,261,768 frames.
        let path = crate::testkit::game_root().join("res/sounds/ambience/lake.ogg");
        let bytes = std::fs::read(path).unwrap();
        let sound = SoundBuffer::decode(bytes.clone(), "ambience/lake", "default", 1.0).unwrap();
        assert_eq!(sound.samples.len(), 2 * 2_261_768);
        let e = SoundBuffer::decode_within(bytes, "ambience/lake", "default", 1.0, 2_261_767)
            .unwrap_err();
        assert_eq!(e, "longer than 2261767 frames, the most a sound may have");
    }

    #[test]
    fn an_ogg_page_cut_short_gives_no_end() {
        assert_eq!(last_granule_position(b"OggS\0"), None);
        assert_eq!(last_granule_position(b"xxOggS\x01aaaaaaaaaa"), None);
        let mut page = b"OggS\0\0".to_vec();
        page.extend(7_i64.to_le_bytes());
        assert_eq!(last_granule_position(&page), Some(7));
    }

    /// Ogg's CRC-32 of a page.
    fn ogg_crc(page: &[u8]) -> u32 {
        let mut crc = 0_u32;
        for &byte in page {
            crc ^= u32::from(byte) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04c1_1db7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    #[test]
    fn a_sound_the_decoder_fails_on_is_refused() {
        // A shipped sound whose sixth codebook is given no dimension, with
        // its page's checksum made right again: the decoder panics on it.
        let path = crate::testkit::game_root().join("res/sounds/ui/positional_beep.ogg");
        let mut bytes = std::fs::read(path).unwrap();
        let codebook = (0..bytes.len() - 3)
            .filter(|&i| &bytes[i..i + 3] == b"BCV")
            .nth(5)
            .unwrap();
        bytes[codebook + 3] = 0;
        bytes[codebook + 4] = 0;
        let page = bytes[..codebook]
            .windows(4)
            .rposition(|w| w == b"OggS")
            .unwrap();
        let segments = usize::from(bytes[page + 26]);
        let size = 27
            + segments
            + bytes[page + 27..page + 27 + segments]
                .iter()
                .map(|&n| usize::from(n))
                .sum::<usize>();
        bytes[page + 22..page + 26].fill(0);
        let crc = ogg_crc(&bytes[page..page + size]);
        bytes[page + 22..page + 26].copy_from_slice(&crc.to_le_bytes());
        let e = SoundBuffer::decode(bytes, "ui/positional_beep", "default", 1.0).unwrap_err();
        assert_eq!(e, "the decoder failed on this file");
    }
}
