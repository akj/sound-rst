//! The audio output on the default device, through rodio's mixer. Each
//! channel is a source whose left and right gains are atomics, so they can
//! change while it plays.

use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

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

#[derive(Default)]
struct Playback {
    stop: AtomicBool,
    done: AtomicBool,
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
    /// Opens the default output device (playing nothing yet).
    pub fn open(num_channels: usize) -> Result<RodioOutput, String> {
        let mut sink = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
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
        let playback = Arc::new(Playback::default());
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
            .is_some_and(|p| !p.done.load(Ordering::Relaxed))
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
