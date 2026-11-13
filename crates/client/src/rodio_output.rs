//! The audio output on the default device, through rodio's mixer. Each
//! channel is a source whose left and right gains are atomics, so they can
//! change while it plays.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rodio::Source;

use crate::audio::{AudioOutput, Channel, Sound};

struct Gains {
    left: AtomicU32,
    right: AtomicU32,
}

impl Gains {
    fn get(&self) -> (f32, f32) {
        (
            f32::from_bits(self.left.load(Ordering::Relaxed)),
            f32::from_bits(self.right.load(Ordering::Relaxed)),
        )
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

struct ChannelSource {
    sound: Sound,
    pos: usize,
    looping: bool,
    gains: Arc<Gains>,
    playback: Arc<Playback>,
}

impl Iterator for ChannelSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.playback.stop.load(Ordering::Relaxed) {
            self.playback.done.store(true, Ordering::Relaxed);
            return None;
        }
        let samples = &self.sound.samples;
        if self.pos >= samples.len() {
            if self.looping && !samples.is_empty() {
                self.pos = 0;
            } else {
                self.playback.done.store(true, Ordering::Relaxed);
                return None;
            }
        }
        let (left, right) = self.gains.get();
        let gain = if self.pos.is_multiple_of(2) {
            left
        } else {
            right
        };
        let sample = samples[self.pos] * gain * self.sound.volume();
        self.pos += 1;
        Some(sample)
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
    sink: rodio::MixerDeviceSink,
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
        let channels = (0..num_channels.max(1))
            .map(|_| ChannelState {
                gains: Arc::new(Gains {
                    left: AtomicU32::new(1f32.to_bits()),
                    right: AtomicU32::new(1f32.to_bits()),
                }),
                playback: None,
            })
            .collect();
        Ok(RodioOutput { sink, channels })
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
        let source = ChannelSource {
            sound: sound.clone(),
            pos: 0,
            looping,
            gains: state.gains.clone(),
            playback,
        };
        self.sink.mixer().add(source);
    }

    fn set_gains(&mut self, channel: Channel, left: f32, right: f32) {
        if let Some(state) = self.channel(channel) {
            state.gains.left.store(left.to_bits(), Ordering::Relaxed);
            state.gains.right.store(right.to_bits(), Ordering::Relaxed);
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
}
