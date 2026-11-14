//! The renderers that run without Steam Audio: the game's maths as it is
//! now, and the same with smooth volume, a duller tone behind and an echo.

use std::collections::HashMap;

use soundrts::audio::{angle, stereo};

use crate::scene::{Place, RATE, Scene, Sound, Source};

/// Frames per block. A renderer reads the scene once per block.
pub const BLOCK: usize = 512;

/// Turns a scene into what the left and right ears hear.
pub trait Renderer {
    /// Adds the scene's sound for the [`BLOCK`] frames that start at frame
    /// `start` of the take to `out`: interleaved left and right samples,
    /// `2 * BLOCK` of them.
    fn render(&mut self, scene: &Scene, start: usize, out: &mut [f32]);
}

/// Renders `seconds` of a take. `scene_at(t)` gives the scene `t` seconds
/// in; it is read at the start of each block. Returns interleaved left and
/// right samples at [`RATE`].
pub fn render_take(
    renderer: &mut dyn Renderer,
    seconds: f64,
    scene_at: &dyn Fn(f64) -> Scene,
) -> Vec<f32> {
    let frames = (seconds * f64::from(RATE)).ceil() as usize;
    let blocks = frames.div_ceil(BLOCK);
    let mut out = vec![0.0; blocks * BLOCK * 2];
    for (b, chunk) in out
        .as_chunks_mut::<{ BLOCK * 2 }>()
        .0
        .iter_mut()
        .enumerate()
    {
        let start = b * BLOCK;
        renderer.render(&scene_at(start as f64 / f64::from(RATE)), start, chunk);
    }
    out
}

/// A source's samples for the block that starts at `start`, silent past
/// the end of its sound.
pub fn block_of(sound: &Sound, start: usize) -> [f32; BLOCK] {
    let mut block = [0.0; BLOCK];
    if let Some(rest) = sound.get(start..) {
        let n = rest.len().min(BLOCK);
        block[..n].copy_from_slice(&rest[..n]);
    }
    block
}

/// The game's left and right volumes for a source, from
/// `soundrts::audio::stereo`: left-right pan, half volume behind, and one
/// over the distance. Height is ignored, as in the game.
fn game_gains(scene: &Scene, s: &Source) -> (f32, f32) {
    let l = &scene.listener;
    let (left, right) = stereo(
        l.position.x,
        l.position.y,
        s.position.x,
        s.position.y,
        l.facing,
        s.loudness,
        false,
    );
    (left as f32, right as f32)
}

/// The game as it is now. Each block sets each source's left and right
/// volumes from the game's own `stereo` function, at once, with no
/// smoothing, and ignores height, closed exits and echo.
#[derive(Default)]
pub struct Current;

impl Renderer for Current {
    fn render(&mut self, scene: &Scene, start: usize, out: &mut [f32]) {
        for s in &scene.sources {
            let (gl, gr) = game_gains(scene, s);
            for (frame, x) in out
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(block_of(&s.sound, start))
            {
                frame[0] += gl * x;
                frame[1] += gr * x;
            }
        }
    }
}

/// Two one-pole low-pass filters in a row (12 dB an octave).
#[derive(Clone, Copy)]
struct LowPass {
    a: f32,
    z1: f32,
    z2: f32,
}

impl LowPass {
    fn new(cutoff: f32) -> LowPass {
        let a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / RATE as f32).exp();
        LowPass {
            a,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        self.z1 += self.a * (x - self.z1);
        self.z2 += self.a * (self.z1 - self.z2);
        self.z2
    }
}

/// Where the tone behind goes: a sound straight behind is low-passed here.
const BEHIND_CUTOFF: f32 = 2_500.0;
/// A sound through a closed exit is low-passed here...
const BLOCKED_CUTOFF: f32 = 1_400.0;
/// ...and played at this fraction of its volume.
const BLOCKED_GAIN: f32 = 0.8;

/// What [`StereoPlus`] keeps for a source between blocks.
struct Voice {
    gains: (f32, f32),
    /// 0 beside or in front, 1 straight behind.
    behind: f32,
    /// 0 clear, 1 through a closed exit.
    blocked: f32,
    behind_filter: LowPass,
    blocked_filter: LowPass,
}

/// The game's maths with three changes: each volume change is spread
/// evenly over a block, so it cannot click; a sound behind is blended
/// towards a low-passed copy of itself, the more the further behind; and
/// a sound through a closed exit is low-passed and quieter. The place's
/// echo is [`Echo`]. Height is ignored, as in the game.
#[derive(Default)]
pub struct StereoPlus {
    voices: HashMap<u32, Voice>,
    echo: Echo,
}

impl Renderer for StereoPlus {
    fn render(&mut self, scene: &Scene, start: usize, out: &mut [f32]) {
        let l = &scene.listener;
        self.voices
            .retain(|id, _| scene.sources.iter().any(|s| s.id == *id));
        for s in &scene.sources {
            let gains = game_gains(scene, s);
            let a = angle(
                l.position.x,
                l.position.y,
                s.position.x,
                s.position.y,
                l.facing,
            );
            let behind = (-a.cos()).max(0.0) as f32;
            let blocked = if s.blocked { 1.0 } else { 0.0 };
            let v = self.voices.entry(s.id).or_insert_with(|| Voice {
                gains,
                behind,
                blocked,
                behind_filter: LowPass::new(BEHIND_CUTOFF),
                blocked_filter: LowPass::new(BLOCKED_CUTOFF),
            });
            let input = block_of(&s.sound, start);
            for (i, (frame, x)) in out.as_chunks_mut::<2>().0.iter_mut().zip(input).enumerate() {
                let t = (i + 1) as f32 / BLOCK as f32;
                let gl = lerp(v.gains.0, gains.0, t);
                let gr = lerp(v.gains.1, gains.1, t);
                let w = lerp(v.behind, behind, t);
                let k = lerp(v.blocked, blocked, t);
                let mut y = x + w * (v.behind_filter.process(x) - x);
                y += k * (v.blocked_filter.process(y) - y);
                y *= 1.0 - k * (1.0 - BLOCKED_GAIN);
                frame[0] += gl * y;
                frame[1] += gr * y;
            }
            v.gains = gains;
            v.behind = behind;
            v.blocked = blocked;
        }
        self.echo.process(scene.place, out);
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A small echo of our own: two delay lines that feed each other, so the
/// echoes bounce between left and right, with the treble damped on each
/// bounce. Each place has its own delays; its middle-band decay time sets
/// the feedback.
pub struct Echo {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
    delays: (usize, usize),
    feedback: f32,
    damp_left: f32,
    damp_right: f32,
}

/// The longest delay an echo uses, in seconds.
const MAX_DELAY: f64 = 0.5;
/// How loud the echo is against the sound.
const ECHO_WET: f32 = 0.35;
/// The one-pole damping coefficient on each bounce (about 9 kHz).
const ECHO_DAMP: f32 = 0.7;

impl Echo {
    pub fn new() -> Echo {
        let len = (MAX_DELAY * f64::from(RATE)) as usize + 1;
        Echo {
            left: vec![0.0; len],
            right: vec![0.0; len],
            pos: 0,
            delays: (1, 1),
            feedback: 0.0,
            damp_left: 0.0,
            damp_right: 0.0,
        }
    }

    /// The delays, in seconds, of the place's two bounces.
    fn delays_of(place: Place) -> (f64, f64) {
        match place {
            Place::Dry | Place::Meadow => (0.071, 0.089),
            Place::Forest => (0.031, 0.043),
            Place::MountainPass => (0.29, 0.37),
            Place::Town => (0.019, 0.027),
        }
    }

    /// Adds the echo of `out` (interleaved stereo) to it. In a dry place
    /// nothing new goes in, but what is already echoing dies away.
    pub fn process(&mut self, place: Place, out: &mut [f32]) {
        let feeding = if let Some(times) = place.decay_times() {
            let (dl, dr) = Echo::delays_of(place);
            self.delays = (
                (dl * f64::from(RATE)) as usize,
                (dr * f64::from(RATE)) as usize,
            );
            // Each bounce loses 60 dB over the decay time.
            let hop = (dl + dr) / 2.0;
            self.feedback = 10f32.powf(-3.0 * hop as f32 / times[1]);
            true
        } else {
            false
        };
        if !feeding && self.feedback == 0.0 {
            return;
        }
        let len = self.left.len();
        for frame in out.as_chunks_mut::<2>().0 {
            let from_left = self.left[(self.pos + len - self.delays.0) % len];
            let from_right = self.right[(self.pos + len - self.delays.1) % len];
            self.damp_left += ECHO_DAMP * (from_left - self.damp_left);
            self.damp_right += ECHO_DAMP * (from_right - self.damp_right);
            let dry = if feeding {
                (frame[0] + frame[1]) / 2.0
            } else {
                0.0
            };
            self.left[self.pos] = dry + self.feedback * self.damp_right;
            self.right[self.pos] = self.feedback * self.damp_left;
            frame[0] += ECHO_WET * from_left;
            frame[1] += ECHO_WET * from_right;
            self.pos = (self.pos + 1) % len;
        }
    }
}

impl Default for Echo {
    fn default() -> Self {
        Echo::new()
    }
}
