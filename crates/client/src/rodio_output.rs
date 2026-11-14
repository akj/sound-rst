//! The audio output on the default device, through rodio's mixer. Each
//! channel is a source whose level (left and right gains, and how far
//! behind the listener its sound is) is held in atomics, so it can change
//! while the sound plays. The source moves to each new level over
//! [`RAMP`], so a change cannot click, and dulls a sound behind.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rodio::Source;

use crate::audio::{AudioOutput, BEHIND_CUTOFF, Channel, LowPass, Sound};

/// How long a change of level takes, in seconds: the listening room's
/// block, 512 frames at 48 kHz (about 11 ms).
const RAMP: f64 = 512.0 / 48_000.0;

/// What a channel plays at.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Level {
    left: f32,
    right: f32,
    /// 0 in front or beside, 1 straight behind: how far the sound is
    /// blended towards its low-passed self.
    behind: f32,
}

impl Level {
    fn towards(self, to: Level, t: f32) -> Level {
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Level {
            left: lerp(self.left, to.left),
            right: lerp(self.right, to.right),
            behind: lerp(self.behind, to.behind),
        }
    }
}

/// A channel's level, as the game last set it.
struct Gains {
    left: AtomicU32,
    right: AtomicU32,
    behind: AtomicU32,
}

impl Gains {
    fn get(&self) -> Level {
        Level {
            left: f32::from_bits(self.left.load(Ordering::Relaxed)),
            right: f32::from_bits(self.right.load(Ordering::Relaxed)),
            behind: f32::from_bits(self.behind.load(Ordering::Relaxed)),
        }
    }

    fn set(&self, level: Level) {
        self.left.store(level.left.to_bits(), Ordering::Relaxed);
        self.right.store(level.right.to_bits(), Ordering::Relaxed);
        self.behind.store(level.behind.to_bits(), Ordering::Relaxed);
    }
}

/// A level that moves in a straight line to each new target, over a fixed
/// number of frames, starting again from where it is when the target
/// changes on the way.
struct Ramp {
    from: Level,
    to: Level,
    now: Level,
    frame: u32,
    frames: u32,
}

impl Ramp {
    fn settled_at(level: Level, frames: u32) -> Ramp {
        Ramp {
            from: level,
            to: level,
            now: level,
            frame: frames,
            frames,
        }
    }

    fn head_for(&mut self, to: Level) {
        if to != self.to {
            self.from = self.now;
            self.to = to;
            self.frame = 0;
        }
    }

    /// Moves on a frame. The last frame of a ramp is the target exactly.
    fn step(&mut self) {
        if self.frame < self.frames {
            self.frame += 1;
            self.now = if self.frame == self.frames {
                self.to
            } else {
                self.from
                    .towards(self.to, self.frame as f32 / self.frames as f32)
            };
        }
    }

    fn is_settled(&self) -> bool {
        self.frame == self.frames
    }
}

/// How long after its length a sound may still count as playing: the
/// device plays what it has buffered a little after the last sample is taken.
const LATE: Duration = Duration::from_millis(500);

struct Playback {
    stop: AtomicBool,
    done: AtomicBool,
    /// When a sound that plays once has ended for certain. A device that
    /// went away stops taking samples, so `done` alone would never be set
    /// and whoever waits for the sound would wait forever.
    deadline: Option<Instant>,
}

impl Playback {
    fn starting(sound: &Sound, looping: bool, now: Instant) -> Playback {
        let length = Duration::try_from_secs_f64(sound.duration()).unwrap_or_default();
        Playback {
            stop: AtomicBool::new(false),
            done: AtomicBool::new(false),
            deadline: (!looping).then(|| now + length + LATE),
        }
    }

    fn is_busy(&self, now: Instant) -> bool {
        !self.done.load(Ordering::Relaxed) && self.deadline.is_none_or(|end| now < end)
    }
}

/// What the device pulls a channel's sound from, sample by sample on the
/// sound card's thread: nothing here allocates or waits.
struct ChannelSource {
    sound: Sound,
    pos: usize,
    looping: bool,
    gains: Arc<Gains>,
    playback: Arc<Playback>,
    /// `None` until the first frame, which plays at the channel's level as
    /// it is then: a sound starts at its level, not faded in.
    ramp: Option<Ramp>,
    ramp_frames: u32,
    /// The duller tone behind, for the left and the right samples.
    filters: [LowPass; 2],
}

impl ChannelSource {
    fn new(sound: &Sound, looping: bool, gains: Arc<Gains>, playback: Arc<Playback>) -> Self {
        let rate = sound.sample_rate;
        ChannelSource {
            sound: sound.clone(),
            pos: 0,
            looping,
            gains,
            playback,
            ramp: None,
            ramp_frames: (f64::from(rate) * RAMP).round().max(1.0) as u32,
            filters: [LowPass::new(BEHIND_CUTOFF, rate); 2],
        }
    }

    /// Moves the level on by a frame. Once the channel is stopped, the
    /// level fades to silence and the source ends; `false` when it has.
    fn next_frame(&mut self) -> bool {
        let stopped = self.playback.stop.load(Ordering::Relaxed);
        if stopped && self.ramp.is_none() {
            // Stopped before the mixer took any of it. A mixer that
            // resamples takes the first frames as the sound is added, so
            // then the sound fades out like any other.
            return false;
        }
        let (gains, frames) = (&self.gains, self.ramp_frames);
        let ramp = self
            .ramp
            .get_or_insert_with(|| Ramp::settled_at(gains.get(), frames));
        if stopped {
            // The channel may already play another sound at other gains.
            ramp.head_for(Level {
                left: 0.0,
                right: 0.0,
                ..ramp.to
            });
            if ramp.is_settled() {
                return false;
            }
        } else {
            ramp.head_for(self.gains.get());
        }
        ramp.step();
        true
    }
}

impl Iterator for ChannelSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.sound.samples.len() {
            if self.looping && !self.sound.samples.is_empty() {
                self.pos = 0;
            } else {
                self.playback.done.store(true, Ordering::Relaxed);
                return None;
            }
        }
        let side = self.pos % 2;
        if side == 0 && !self.next_frame() {
            self.playback.done.store(true, Ordering::Relaxed);
            return None;
        }
        let level = self.ramp.as_ref()?.now;
        let gain = if side == 0 { level.left } else { level.right };
        let x = self.filters[side].blend(self.sound.samples[self.pos], level.behind);
        self.pos += 1;
        Some(x * gain * self.sound.volume())
    }
}

impl Source for ChannelSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(2).expect("two channels")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(self.sound.sample_rate).unwrap_or(NonZero::new(44_100).expect("rate"))
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct ChannelState {
    gains: Arc<Gains>,
    playback: Option<Arc<Playback>>,
}

/// Plays on the default output device.
pub struct RodioOutput {
    /// Keeps the device open. `None` when no device pulls from the mixer.
    _sink: Option<rodio::MixerDeviceSink>,
    mixer: rodio::mixer::Mixer,
    channels: Vec<ChannelState>,
}

impl RodioOutput {
    /// Opens the default output device (playing nothing yet). `on_error`
    /// receives the errors of the running device, such as its removal.
    pub fn open(
        num_channels: usize,
        on_error: impl Fn(String) + Send + Clone + 'static,
    ) -> Result<RodioOutput, String> {
        let report = move |e: rodio::cpal::Error| on_error(e.to_string());
        let mut sink = rodio::DeviceSinkBuilder::from_default_device()
            .and_then(|builder| builder.with_error_callback(report).open_sink_or_fallback())
            .or_else(|_| rodio::DeviceSinkBuilder::open_default_sink())
            .map_err(|e| e.to_string())?;
        sink.log_on_drop(false);
        let mixer = sink.mixer().clone();
        Ok(RodioOutput::on(mixer, Some(sink), num_channels))
    }

    /// Plays into `mixer`, which `sink`, if any, plays.
    fn on(
        mixer: rodio::mixer::Mixer,
        sink: Option<rodio::MixerDeviceSink>,
        num_channels: usize,
    ) -> RodioOutput {
        let channels = (0..num_channels.max(1))
            .map(|_| ChannelState {
                gains: Arc::new(Gains {
                    left: AtomicU32::new(1f32.to_bits()),
                    right: AtomicU32::new(1f32.to_bits()),
                    behind: AtomicU32::new(0f32.to_bits()),
                }),
                playback: None,
            })
            .collect();
        RodioOutput {
            _sink: sink,
            mixer,
            channels,
        }
    }

    fn channel(&mut self, channel: Channel) -> Option<&mut ChannelState> {
        self.channels.get_mut(channel.index())
    }
}

impl AudioOutput for RodioOutput {
    fn play(&mut self, channel: Channel, sound: &Sound, looping: bool) {
        self.stop(channel);
        let Some(state) = self.channels.get_mut(channel.index()) else {
            return;
        };
        let playback = Arc::new(Playback::starting(sound, looping, Instant::now()));
        state.playback = Some(playback.clone());
        let source = ChannelSource::new(sound, looping, state.gains.clone(), playback);
        self.mixer.add(source);
    }

    fn set_gains(&mut self, channel: Channel, left: f32, right: f32, behind: f32) {
        if let Some(state) = self.channel(channel) {
            state.gains.set(Level {
                left,
                right,
                behind,
            });
        }
    }

    fn stop(&mut self, channel: Channel) {
        if let Some(state) = self.channel(channel)
            && let Some(p) = state.playback.take()
        {
            p.stop.store(true, Ordering::Relaxed);
        }
    }

    fn is_busy(&self, channel: Channel) -> bool {
        self.channels
            .get(channel.index())
            .and_then(|s| s.playback.as_ref())
            .is_some_and(|p| p.is_busy(Instant::now()))
    }
}

/// The default output device's name and format, without opening it.
pub fn describe_default_device() -> Result<String, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let host = rodio::cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device".to_owned())?;
    let name = device
        .description()
        .map(|d| d.name().to_owned())
        .map_err(|e| e.to_string())?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    Ok(format!(
        "{name} ({} Hz, {} channels, {:?})",
        config.sample_rate(),
        config.channels(),
        config.sample_format()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SoundBuffer;

    #[test]
    fn a_sound_the_device_never_finishes_stops_being_busy_after_its_length() {
        // One second of stereo silence that no device ever takes.
        let sound: Sound = Arc::new(SoundBuffer::new(
            "s",
            "default",
            vec![0.0; 88_200],
            44_100,
            1.0,
        ));
        let start = Instant::now();
        let once = Playback::starting(&sound, false, start);
        assert!(once.is_busy(start + Duration::from_millis(1_400)));
        assert!(!once.is_busy(start + Duration::from_millis(1_600)));
        // A loop plays until it is stopped.
        let looped = Playback::starting(&sound, true, start);
        assert!(looped.is_busy(start + Duration::from_secs(3_600)));
        looped.done.store(true, Ordering::Relaxed);
        assert!(!looped.is_busy(start));
    }

    // What follows pulls samples from rodio's mixer, as the device does,
    // with no device open. The mixer runs at the sounds' rate, so it hands
    // on each sample as the channel's source made it.

    const RATE: u32 = 44_100;
    /// The output treats every channel alike.
    const CH: Channel = Channel::Voice;

    /// An output on a mixer no device plays, and that mixer's output.
    fn output() -> (RodioOutput, rodio::mixer::MixerSource) {
        let (mixer, out) =
            rodio::mixer::mixer(NonZero::new(2).unwrap(), NonZero::new(RATE).unwrap());
        (RodioOutput::on(mixer, None, 2), out)
    }

    fn ramp_frames() -> usize {
        (f64::from(RATE) * RAMP).round() as usize
    }

    fn sound(samples: Vec<f32>, volume: f64) -> Sound {
        Arc::new(SoundBuffer::new("s", "default", samples, RATE, volume))
    }

    /// Two different white noises, interleaved, between -0.5 and 0.5.
    fn noise(frames: usize) -> Vec<f32> {
        let mut state = 1_u32;
        (0..2 * frames)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect()
    }

    /// The next `frames` frames, as left and right samples.
    fn pull(out: &mut rodio::mixer::MixerSource, frames: usize) -> (Vec<f32>, Vec<f32>) {
        (0..frames)
            .map(|_| (out.next().expect("left"), out.next().expect("right")))
            .unzip()
    }

    fn largest_step(x: &[f32]) -> f32 {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    /// The share of a signal's energy in its sample-to-sample changes: a
    /// measure of treble that does not depend on loudness.
    fn brightness(x: &[f32]) -> f64 {
        let energy =
            |x: &mut dyn Iterator<Item = f32>| x.map(|s| f64::from(s).powi(2)).sum::<f64>();
        energy(&mut x.windows(2).map(|w| w[1] - w[0])) / energy(&mut x.iter().copied())
    }

    /// The game's gains and measure of behind for a source at (x, y), for a
    /// listener at the origin facing north.
    fn placed(x: f64, y: f64) -> (f32, f32, f32) {
        let (left, right) = crate::audio::stereo(0.0, 0.0, x, y, 90.0, 1.0, false);
        let behind = crate::audio::behind(0.0, 0.0, x, y, 90.0);
        (left as f32, right as f32, behind as f32)
    }

    #[test]
    fn a_change_of_gains_moves_evenly_over_the_ramp_and_settles_exactly() {
        let (mut o, mut out) = output();
        let ramp = ramp_frames();
        assert_eq!(ramp, 470);
        o.set_gains(CH, 0.1, 0.1, 0.0);
        o.play(CH, &sound(vec![0.5; 2 * RATE as usize], 1.0), false);
        let (before, _) = pull(&mut out, 1_000);
        assert!(before.iter().all(|&s| s == 0.5 * 0.1));
        o.set_gains(CH, 1.0, 1.0, 0.0);
        let (after, right) = pull(&mut out, 2 * ramp);
        assert_eq!(after, right);
        // Not there before the ramp's last frame; exactly there from it on.
        assert!(after[ramp - 2] < 0.5, "{}", after[ramp - 2]);
        assert!(after[ramp - 1..].iter().all(|&s| s == 0.5));
        // In one step it would jump 0.45.
        let even = (0.5 - 0.05) / ramp as f32;
        let step = largest_step(&[before, after].concat());
        assert!(step < 1.01 * even, "largest step {step}, even step {even}");
    }

    #[test]
    fn a_source_in_front_plays_sample_for_sample_as_before() {
        let samples = noise(RATE as usize / 2);
        let (left, right, behind) = placed(1.0, 3.0);
        assert_eq!(behind, 0.0);
        let (mut o, mut out) = output();
        o.set_gains(CH, left, right, behind);
        o.play(CH, &sound(samples.clone(), 0.8), false);
        let (l, r) = pull(&mut out, samples.len() / 2);
        // The output's formula before the duller tone behind.
        let volume = 0.8_f64 as f32;
        for (i, frame) in samples.chunks(2).enumerate() {
            assert_eq!(l[i], frame[0] * left * volume, "left, frame {i}");
            assert_eq!(r[i], frame[1] * right * volume, "right, frame {i}");
        }
        assert_eq!(out.next(), None);
    }

    #[test]
    fn a_source_behind_is_duller_than_the_same_source_in_front() {
        let samples = noise(RATE as usize / 2);
        let take = |y: f64| {
            let (left, right, behind) = placed(0.0, y);
            let (mut o, mut out) = output();
            o.set_gains(CH, left, right, behind);
            o.play(CH, &sound(samples.clone(), 1.0), false);
            pull(&mut out, samples.len() / 2)
        };
        let (front, back) = (take(3.0), take(-3.0));
        assert_eq!(placed(0.0, -3.0).2, 1.0);
        for (f, b) in [(&front.0, &back.0), (&front.1, &back.1)] {
            let (f, b) = (brightness(f), brightness(b));
            eprintln!("brightness in front {f:.4}, behind {b:.4}");
            assert!(b < 0.25 * f, "brightness in front {f}, behind {b}");
        }
    }

    #[test]
    fn a_source_moving_round_from_front_to_behind_changes_smoothly() {
        // A 301 Hz tone, one second of whole periods, looped, so that the
        // moves fall at different points of its wave. It moves 3 m
        // from the listener, from straight ahead round the right to
        // straight behind, in 20 moves 100 ms apart, as the game's
        // animations move sounds.
        let w = std::f64::consts::TAU * 301.0 / f64::from(RATE);
        let tone: Vec<f32> = (0..RATE)
            .flat_map(|i| [(0.5 * (w * f64::from(i)).sin()) as f32; 2])
            .collect();
        let at = |k: u32| {
            let a = (90.0 - 9.0 * f64::from(k)).to_radians();
            placed(3.0 * a.cos(), 3.0 * a.sin())
        };
        // Passing the right ear, the game's gains halve at once.
        assert!(at(10).1 > 1.9 * at(11).1);
        assert_eq!((at(0).2, at(20).2), (0.0, 1.0));

        let (mut o, mut out) = output();
        let (left, right, behind) = at(0);
        o.set_gains(CH, left, right, behind);
        o.play(CH, &sound(tone, 1.0), true);
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for k in 0..=20 {
            let (left, right, behind) = at(k);
            o.set_gains(CH, left, right, behind);
            let (ls, rs) = pull(&mut out, RATE as usize / 10);
            l.extend(ls);
            r.extend(rs);
        }
        // No step is larger than the tone's own largest step at the
        // loudest gain it had, give or take a tenth.
        let tone_step = |gain: f32| 0.5 * gain * (2.0 * (w / 2.0).sin()) as f32;
        let loudest =
            |side: fn((f32, f32, f32)) -> f32| (0..=20).map(|k| side(at(k))).fold(0.0, f32::max);
        for (out, gain) in [(&l, loudest(|g| g.0)), (&r, loudest(|g| g.1))] {
            let (step, tone) = (largest_step(out), tone_step(gain));
            eprintln!("largest step {step:.5}, the tone's {tone:.5}");
            assert!(step < 1.1 * tone, "largest step {step}, the tone's {tone}");
        }
    }

    #[test]
    fn a_stopped_sound_fades_out_and_frees_its_channel_at_once() {
        let (mut o, mut out) = output();
        let ramp = ramp_frames();
        o.set_gains(CH, 0.8, 0.8, 0.0);
        o.play(CH, &sound(vec![0.5; 2 * RATE as usize], 1.0), false);
        let (before, _) = pull(&mut out, 1_000);
        o.stop(CH);
        assert!(!o.is_busy(CH));
        let mut tail = Vec::new();
        while let Some(left) = out.next() {
            tail.push(left);
            out.next();
        }
        assert_eq!(tail.len(), ramp);
        assert_eq!(tail.last(), Some(&0.0));
        let even = 0.4 / ramp as f32;
        let step = largest_step(&[before, tail].concat());
        assert!(step < 1.01 * even, "largest step {step}, even step {even}");

        // A sound stopped before the device took any of it is never heard.
        o.play(CH, &sound(vec![0.5; 200], 1.0), false);
        o.stop(CH);
        assert_eq!(out.next(), None);
    }

    #[test]
    fn a_new_sound_starts_at_its_gains_while_the_one_it_replaces_fades_out() {
        let (mut o, mut out) = output();
        let ramp = ramp_frames();
        o.set_gains(CH, 0.2, 0.2, 0.0);
        o.play(CH, &sound(vec![0.5; 2 * RATE as usize], 1.0), false);
        pull(&mut out, 100);
        o.set_gains(CH, 0.6, 0.3, 0.0);
        o.play(CH, &sound(vec![0.25; 2 * RATE as usize], 1.0), false);
        let (l, r) = pull(&mut out, 2 * ramp);
        let fading = |i: usize| 0.1 * (1.0 - (i + 1) as f32 / ramp as f32);
        for i in [0, ramp / 2] {
            assert!(
                (l[i] - (0.15 + fading(i))).abs() < 1e-6,
                "left {i}: {}",
                l[i]
            );
            assert!(
                (r[i] - (0.075 + fading(i))).abs() < 1e-6,
                "right {i}: {}",
                r[i]
            );
        }
        assert!(l[ramp..].iter().all(|&s| s == 0.25 * 0.6));
        assert!(r[ramp..].iter().all(|&s| s == 0.25 * 0.3));
    }

    #[test]
    fn a_sound_starts_at_its_gains_on_a_device_at_another_rate() {
        // A 48 kHz device takes a 44.1 kHz sound's first frames as it is
        // added, before anything else happens.
        let (mixer, mut out) =
            rodio::mixer::mixer(NonZero::new(2).unwrap(), NonZero::new(48_000).unwrap());
        let mut o = RodioOutput::on(mixer, None, 2);
        o.set_gains(CH, 1.0, 1.0, 0.0);
        o.play(CH, &sound(vec![0.5; 2 * RATE as usize], 1.0), false);
        o.stop(CH);
        o.set_gains(CH, 0.6, 0.3, 0.0);
        o.play(CH, &sound(vec![0.25; 2 * RATE as usize], 1.0), false);
        // The first sound fades out from 0.5 while the second plays at
        // its gains from its first frame.
        let (l, r) = pull(&mut out, 1_000);
        assert!((l[0] - (0.15 + 0.5)).abs() < 0.01, "left {}", l[0]);
        assert!((r[0] - (0.075 + 0.5)).abs() < 0.01, "right {}", r[0]);
        assert!((l[999] - 0.15).abs() < 1e-6 && (r[999] - 0.075).abs() < 1e-6);
    }
}
