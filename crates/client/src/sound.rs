//! Positional and stereo sound, the volumes, and the voice channel's
//! sounds: the [`Audio`] handle the game uses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::audio::{AudioOutput, Channel, EffectChannel, Mixer, Sound, stereo};
use crate::clock::ClockRef;

/// Where the player listens from. `o` is the facing in degrees (90 is
/// north). Out of immersion mode, the listener always faces north and
/// distances along y are flattened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub x: f64,
    pub y: f64,
    pub o: f64,
    pub immersion: bool,
}

impl Default for Listener {
    fn default() -> Self {
        Listener {
            x: 0.0,
            y: 0.0,
            o: 90.0,
            immersion: false,
        }
    }
}

/// How a positional sound is placed and ranked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub volume: f64,
    pub x: f64,
    pub y: f64,
    /// A sound may take the channel of a playing sound of lower priority.
    pub priority: i32,
    /// The same sound does not start again within this many seconds.
    pub limit: f64,
    /// Ambient sounds play at random volumes (looping ones at full volume).
    pub ambient: bool,
}

impl Placement {
    pub fn new(volume: f64, x: f64, y: f64, priority: i32) -> Placement {
        Placement {
            volume,
            x,
            y,
            priority,
            limit: 0.0,
            ambient: false,
        }
    }
}

/// A playing positional sound, for moving or stopping it later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(u64);

/// The volume of a stereo (not positional) sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StereoVolume {
    Both(f64),
    Split(f64, f64),
}

struct Source {
    id: SourceId,
    sound: Sound,
    v: f64,
    x: f64,
    y: f64,
    priority: i32,
    ambient: bool,
    looping: bool,
    channel: Option<EffectChannel>,
    previous_vol: (f64, f64),
    ended: bool,
}

/// The mixer, the positional sources and the volumes.
pub struct AudioState {
    mixer: Mixer,
    clock: ClockRef,
    flattening_factor: f64,
    listener: Listener,
    sources: Vec<Source>,
    next_id: u64,
    start_times: HashMap<(String, String), f64>,
    rng: u64,
    /// The volume of everything, between 0 and 1.
    pub main_volume: f64,
    /// The volume of sounds on the voice channel (not of speech).
    pub voice_volume: f64,
}

/// The handle to the audio engine. Clones share the same engine.
#[derive(Clone)]
pub struct Audio(Rc<RefCell<AudioState>>);

fn sound_key(sound: &Sound) -> (String, String) {
    (sound.mod_name.clone(), sound.key.clone())
}

impl AudioState {
    fn now(&self) -> f64 {
        self.clock.now()
    }

    fn should_be_played(&self, sound: &Sound, limit: f64) -> bool {
        self.start_times
            .get(&sound_key(sound))
            .is_none_or(|&t| t + limit < self.now())
    }

    fn remember_start_time(&mut self, sound: &Sound) {
        let now = self.now();
        self.start_times.insert(sound_key(sound), now);
    }

    fn random(&mut self) -> f64 {
        // xorshift64*: enough for the crackle of ambient sounds.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn stereo_volume(&mut self, x: f64, y: f64, v: f64) -> (f64, f64) {
        let flattening = if self.listener.immersion {
            1.0
        } else {
            self.listener.o = 90.0;
            self.flattening_factor
        };
        let l = self.listener;
        stereo(l.x, l.y / flattening, x, y / flattening, l.o, v, false)
    }

    fn is_playing(&self, i: usize) -> bool {
        let s = &self.sources[i];
        s.channel.is_some_and(|c| {
            self.mixer
                .sound_on(Channel::Effect(c))
                .is_some_and(|on| Arc::ptr_eq(on, &s.sound))
        })
    }

    fn volume_too_low(&mut self, i: usize) -> bool {
        let (x, y, v) = (self.sources[i].x, self.sources[i].y, self.sources[i].v);
        let (l, r) = self.stereo_volume(x, y, v);
        l.max(r) < 0.02
    }

    fn update_volume(&mut self, i: usize, force: bool) {
        let Some(c) = self.sources[i].channel else {
            return;
        };
        if self.volume_too_low(i) {
            self.mixer.stop(Channel::Effect(c));
            self.sources[i].channel = None;
            return;
        }
        let vol = if self.sources[i].ambient {
            if self.sources[i].looping {
                (1.0, 1.0)
            } else {
                (self.random(), self.random())
            }
        } else {
            let (x, y, v) = (self.sources[i].x, self.sources[i].y, self.sources[i].v);
            self.stereo_volume(x, y, v)
        };
        if force || vol != self.sources[i].previous_vol {
            let m = self.main_volume;
            self.mixer
                .set_gains(Channel::Effect(c), vol.0 * m, vol.1 * m);
            self.sources[i].previous_vol = vol;
        }
    }

    fn start(&mut self, i: usize) {
        if !self.volume_too_low(i) {
            let priority = self.sources[i].priority;
            let channel = self.find_a_channel(priority);
            self.sources[i].channel = channel;
            if let Some(c) = channel {
                self.mixer.stop(Channel::Effect(c));
                self.update_volume(i, true);
                if let Some(c) = self.sources[i].channel {
                    let sound = self.sources[i].sound.clone();
                    let looping = self.sources[i].looping;
                    self.mixer.play(Channel::Effect(c), &sound, looping);
                }
            }
        }
        if self.is_playing(i) {
            let sound = self.sources[i].sound.clone();
            self.remember_start_time(&sound);
        }
    }

    fn stop_source(&mut self, i: usize) {
        if self.is_playing(i)
            && let Some(c) = self.sources[i].channel.take()
        {
            self.mixer.stop(Channel::Effect(c));
        }
        self.sources[i].ended = true;
    }

    fn update_source(&mut self, i: usize) {
        if self.sources[i].ended {
            return;
        }
        if self.is_playing(i) {
            self.update_volume(i, false);
        } else if self.sources[i].looping {
            self.start(i);
        } else {
            self.stop_source(i);
        }
    }

    /// An idle effects channel, else the channel of the quietest playing
    /// source of the lowest priority below `priority`, stopped.
    fn find_a_channel(&mut self, priority: i32) -> Option<EffectChannel> {
        if let Some(c) = self.mixer.find_idle_channel() {
            return Some(c);
        }
        let mut playing: Vec<usize> = (0..self.sources.len())
            .filter(|&i| self.is_playing(i) && self.sources[i].priority < priority)
            .collect();
        playing.sort_by(|&a, &b| {
            let (sa, sb) = (&self.sources[a], &self.sources[b]);
            (sa.priority, sa.previous_vol.0.max(sa.previous_vol.1))
                .partial_cmp(&(sb.priority, sb.previous_vol.0.max(sb.previous_vol.1)))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let c = self.sources[*playing.first()?].channel?;
        self.mixer.stop(Channel::Effect(c));
        Some(c)
    }

    fn new_source(&mut self, sound: Sound, p: Placement, looping: bool) -> usize {
        self.next_id += 1;
        self.sources.push(Source {
            id: SourceId(self.next_id),
            sound,
            v: p.volume,
            x: p.x,
            y: p.y,
            priority: p.priority,
            ambient: p.ambient,
            looping,
            channel: None,
            previous_vol: (0.0, 0.0),
            ended: false,
        });
        let i = self.sources.len() - 1;
        let sound = self.sources[i].sound.clone();
        if self.should_be_played(&sound, p.limit) {
            self.start(i);
        }
        i
    }

    fn index(&self, id: SourceId) -> Option<usize> {
        self.sources.iter().position(|s| s.id == id)
    }
}

impl Audio {
    /// The engine, with `num_channels` mixer channels (the first for the
    /// voice).
    pub fn new(
        output: Box<dyn AudioOutput>,
        num_channels: usize,
        clock: ClockRef,
        flattening_factor: f64,
    ) -> Audio {
        let seed = (clock.now().to_bits() ^ 0x9E37_79B9_7F4A_7C15) | 1;
        Audio(Rc::new(RefCell::new(AudioState {
            mixer: Mixer::new(output, num_channels),
            clock,
            flattening_factor,
            listener: Listener::default(),
            sources: Vec::new(),
            next_id: 0,
            start_times: HashMap::new(),
            rng: seed,
            main_volume: 0.5,
            voice_volume: 1.0,
        })))
    }

    pub fn main_volume(&self) -> f64 {
        self.0.borrow().main_volume
    }

    pub fn set_main_volume(&self, v: f64) {
        self.0.borrow_mut().main_volume = v;
    }

    pub fn voice_volume(&self) -> f64 {
        self.0.borrow().voice_volume
    }

    pub fn set_voice_volume(&self, v: f64) {
        self.0.borrow_mut().voice_volume = v;
    }

    pub fn listener(&self) -> Listener {
        self.0.borrow().listener
    }

    pub fn set_listener(&self, listener: Listener) {
        self.0.borrow_mut().listener = listener;
    }

    pub fn num_channels(&self) -> usize {
        self.0.borrow().mixer.num_channels()
    }

    /// Plays a positional sound once. Returns its id while it plays; a
    /// sound too quiet, without a channel, or within its limit is dropped.
    pub fn play(&self, sound: Option<Sound>, placement: Placement) -> Option<SourceId> {
        let sound = sound?;
        let mut a = self.0.borrow_mut();
        let i = a.new_source(sound, placement, false);
        if a.is_playing(i) {
            Some(a.sources[i].id)
        } else {
            a.sources.remove(i);
            None
        }
    }

    /// Plays a positional sound in a loop. It keeps trying to get a channel
    /// until stopped.
    pub fn play_loop(&self, sound: Option<Sound>, placement: Placement) -> Option<SourceId> {
        let sound = sound?;
        let mut a = self.0.borrow_mut();
        let i = a.new_source(sound, placement, true);
        Some(a.sources[i].id)
    }

    /// Plays a sound that is not positional, at priority 10.
    pub fn play_stereo(&self, sound: Option<Sound>, volume: StereoVolume, limit: f64) {
        let Some(sound) = sound else {
            return;
        };
        let mut a = self.0.borrow_mut();
        if !a.should_be_played(&sound, limit) {
            return;
        }
        if let Some(c) = a.find_a_channel(10) {
            let m = a.main_volume;
            a.mixer.play(Channel::Effect(c), &sound, false);
            let (l, r) = match volume {
                StereoVolume::Both(v) => (v, v),
                StereoVolume::Split(l, r) => (l, r),
            };
            a.mixer.set_gains(Channel::Effect(c), l * m, r * m);
            a.remember_start_time(&sound);
        }
    }

    /// Follows the listener and the sources; forgets the ended sources.
    pub fn update(&self) {
        let mut a = self.0.borrow_mut();
        let mut i = 0;
        while i < a.sources.len() {
            a.update_source(i);
            if a.sources[i].ended {
                a.sources.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Moves a source and updates its volume.
    pub fn move_source(&self, id: SourceId, x: f64, y: f64) {
        let mut a = self.0.borrow_mut();
        if let Some(i) = a.index(id)
            && !a.sources[i].ended
            && (x, y) != (a.sources[i].x, a.sources[i].y)
        {
            a.sources[i].x = x;
            a.sources[i].y = y;
            a.update_source(i);
        }
    }

    /// Changes a source's volume; it applies at the next update or move.
    pub fn set_source_volume(&self, id: SourceId, volume: f64) {
        let mut a = self.0.borrow_mut();
        if let Some(i) = a.index(id) {
            a.sources[i].v = volume;
        }
    }

    pub fn stop_source(&self, id: SourceId) {
        let mut a = self.0.borrow_mut();
        if let Some(i) = a.index(id) {
            a.stop_source(i);
        }
    }

    /// Whether the source has ended (a forgotten source has).
    pub fn source_ended(&self, id: SourceId) -> bool {
        let a = self.0.borrow();
        a.index(id).is_none_or(|i| a.sources[i].ended)
    }

    /// Stops every positional source; they are forgotten at the next update.
    pub fn stop_sources(&self) {
        let mut a = self.0.borrow_mut();
        for i in 0..a.sources.len() {
            a.stop_source(i);
        }
    }

    /// Stops the sources and every effects channel, and the voice channel
    /// too if asked. Speech is the voice's to stop.
    pub fn stop(&self, voice_too: bool) {
        self.stop_sources();
        let mut a = self.0.borrow_mut();
        if voice_too {
            a.mixer.stop_all();
        } else {
            a.mixer.stop_effects();
        }
    }

    /// Plays a sound on the voice channel at the given speaker volumes,
    /// scaled by the main and voice volumes.
    pub fn voice_play(&self, sound: &Sound, lv: f64, rv: f64) {
        let mut a = self.0.borrow_mut();
        let v = a.main_volume * a.voice_volume;
        a.mixer.set_gains(Channel::Voice, lv * v, rv * v);
        a.mixer.play(Channel::Voice, sound, false);
    }

    pub fn voice_stop(&self) {
        self.0.borrow_mut().mixer.stop(Channel::Voice);
    }

    pub fn voice_busy(&self) -> bool {
        self.0.borrow().mixer.is_busy(Channel::Voice)
    }
}
