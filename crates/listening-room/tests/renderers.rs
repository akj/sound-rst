//! The renderers, checked from the buffers they render. Nothing here plays
//! a sound.

use listening_room::render::{BLOCK, Current, Renderer, StereoPlus, render_take};
use listening_room::rng::Rng;
use listening_room::scene::{Listener, Place, Point, RATE, Scene, Sound, Source};

fn noise(seconds: f64) -> Sound {
    let mut rng = Rng::new(3);
    (0..(seconds * f64::from(RATE)) as usize)
        .map(|_| 0.5 * rng.noise())
        .collect()
}

fn scene(sound: &Sound, position: Point, loudness: f64, blocked: bool, place: Place) -> Scene {
    Scene {
        listener: Listener::NORTH,
        sources: vec![Source {
            id: 1,
            sound: sound.clone(),
            position,
            loudness,
            blocked,
        }],
        place,
    }
}

/// Renders one source standing still.
fn still(r: &mut dyn Renderer, sound: &Sound, position: Point, seconds: f64) -> Vec<f32> {
    render_take(r, seconds, &|_| {
        scene(sound, position, 1.0, false, Place::Dry)
    })
}

fn channel(out: &[f32], c: usize) -> Vec<f32> {
    out.iter().skip(c).step_by(2).copied().collect()
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&s| f64::from(s) * f64::from(s)).sum()
}

/// The share of a signal's energy in its sample-to-sample changes: a
/// measure of treble that does not depend on loudness.
fn brightness(x: &[f32]) -> f64 {
    let diff: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    energy(&diff) / energy(x)
}

fn largest_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

const LEFT: Point = Point::new(-3.0, 0.0, 0.0);
const FRONT: Point = Point::new(0.0, 3.0, 0.0);
const BEHIND: Point = Point::new(0.0, -3.0, 0.0);

#[test]
fn a_source_on_the_left_has_more_energy_on_the_left() {
    let sound = noise(0.5);
    let renderers: [Box<dyn Renderer>; 2] = [Box::new(Current), Box::<StereoPlus>::default()];
    for mut r in renderers {
        let out = still(r.as_mut(), &sound, LEFT, 0.5);
        let (l, rt) = (energy(&channel(&out, 0)), energy(&channel(&out, 1)));
        assert!(l > 10.0 * rt, "left {l}, right {rt}");
        assert!(l > 0.0);
    }
}

#[test]
fn stereo_plus_dulls_a_source_behind_and_current_does_not() {
    let sound = noise(0.5);
    let tone =
        |r: &mut dyn Renderer, at: Point| brightness(&channel(&still(r, &sound, at, 0.5), 0));

    let (front, behind) = (tone(&mut Current, FRONT), tone(&mut Current, BEHIND));
    assert!(
        (front - behind).abs() < 0.001 * front,
        "current: front {front}, behind {behind}"
    );

    let front = tone(&mut StereoPlus::default(), FRONT);
    let behind = tone(&mut StereoPlus::default(), BEHIND);
    assert!(
        behind < 0.5 * front,
        "stereo-plus: front {front}, behind {behind}"
    );
}

#[test]
fn a_jump_in_loudness_clicks_in_current_and_not_in_stereo_plus() {
    // A steady signal that outlasts the take, so every change in the
    // output comes from the gain.
    let sound: Sound = vec![0.5; 2 * RATE as usize].into();
    let take = |r: &mut dyn Renderer| {
        render_take(r, 1.0, &|t| {
            let loudness = if t < 0.5 { 0.1 } else { 1.0 };
            scene(
                &sound,
                Point::new(0.0, 1.0, 0.0),
                loudness,
                false,
                Place::Dry,
            )
        })
    };
    let current = channel(&take(&mut Current), 0);
    let plus = channel(&take(&mut StereoPlus::default()), 0);
    // The game's gain in front at 1 metre is 0.707 times the loudness, so
    // the output goes from 0.5 * 0.0707 to 0.5 * 0.707.
    for out in [&current, &plus] {
        assert!((out[0] - 0.035_355).abs() < 1e-4, "starts at {}", out[0]);
        let settled = out[RATE as usize * 3 / 4];
        assert!((settled - 0.353_55).abs() < 1e-4, "settles at {settled}");
    }
    assert!(largest_step(&current) > 0.3, "current's largest step");
    let plus = largest_step(&plus);
    assert!(plus < 0.001, "stereo-plus's largest step {plus}");
}

#[test]
fn stereo_plus_muffles_a_source_through_a_closed_exit() {
    let sound = noise(0.5);
    let east = Point::new(12.0, 0.0, 0.0);
    let take = |blocked: bool| {
        let out = render_take(&mut StereoPlus::default(), 0.5, &|_| {
            scene(&sound, east, 1.0, blocked, Place::Dry)
        });
        channel(&out, 1)
    };
    let (open, closed) = (take(false), take(true));
    assert!(brightness(&closed) < 0.2 * brightness(&open));
    assert!(energy(&closed) < 0.5 * energy(&open));
}

#[test]
fn stereo_plus_echoes_longer_in_a_mountain_pass_than_in_a_meadow_and_not_when_dry() {
    // A short burst, then silence: what follows the burst is echo.
    let burst: Sound = noise(0.05)
        .iter()
        .copied()
        .chain(vec![0.0; RATE as usize])
        .collect();
    let tail = |place: Place| {
        let out = render_take(&mut StereoPlus::default(), 1.0, &|_| {
            scene(&burst, FRONT, 1.0, false, place)
        });
        energy(&out[2 * (RATE as usize / 5)..])
    };
    assert_eq!(tail(Place::Dry), 0.0);
    let (meadow, pass) = (tail(Place::Meadow), tail(Place::MountainPass));
    assert!(meadow > 0.0);
    assert!(pass > 2.0 * meadow, "meadow {meadow}, pass {pass}");
}

#[test]
fn takes_are_whole_blocks_of_stereo() {
    let out = still(&mut Current, &noise(0.1), FRONT, 0.1);
    assert_eq!(out.len() % (2 * BLOCK), 0);
    assert!(out.len() >= (0.1 * f64::from(RATE)) as usize * 2);
}

#[cfg(feature = "binaural")]
mod binaural {
    use super::*;
    use listening_room::binaural::Binaural;

    fn binaural() -> Binaural {
        Binaural::new(None).expect("Steam Audio starts")
    }

    /// The first frame at which a channel reaches a tenth of its peak.
    fn onset(x: &[f32]) -> usize {
        let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        x.iter().position(|s| s.abs() >= 0.1 * peak).unwrap()
    }

    #[test]
    fn a_source_on_the_left_arrives_earlier_and_louder_in_the_left_ear() {
        let mut click = vec![0.0f32; RATE as usize / 4];
        click[2 * BLOCK] = 1.0;
        let click: Sound = click.into();
        let out = still(&mut binaural(), &click, LEFT, 0.25);
        let (l, r) = (channel(&out, 0), channel(&out, 1));
        eprintln!(
            "left source: left onset {}, right onset {}, left energy {}, right energy {}",
            onset(&l),
            onset(&r),
            energy(&l),
            energy(&r)
        );
        assert!(onset(&l) < onset(&r));
        assert!(energy(&l) > 2.0 * energy(&r));
    }

    #[test]
    fn a_source_behind_differs_from_the_same_source_in_front() {
        let sound = noise(0.5);
        let front = still(&mut binaural(), &sound, FRONT, 0.5);
        let behind = still(&mut binaural(), &sound, BEHIND, 0.5);
        let (bf, bb) = (
            brightness(&channel(&front, 0)),
            brightness(&channel(&behind, 0)),
        );
        let diff: Vec<f32> = front.iter().zip(&behind).map(|(a, b)| a - b).collect();
        let relative = energy(&diff) / energy(&front);
        eprintln!("binaural: brightness front {bf}, behind {bb}; difference energy {relative}");
        assert!(relative > 0.1, "front and behind differ by {relative}");
        assert!(
            (bf - bb).abs() > 0.05 * bf,
            "brightness front {bf}, behind {bb}"
        );
    }

    #[test]
    fn binaural_muffles_a_source_through_a_closed_exit() {
        let sound = noise(0.5);
        let east = Point::new(12.0, 0.0, 0.0);
        let take = |blocked: bool| {
            let out = render_take(&mut binaural(), 0.5, &|_| {
                scene(&sound, east, 1.0, blocked, Place::Dry)
            });
            channel(&out, 1)
        };
        let (open, closed) = (take(false), take(true));
        eprintln!(
            "binaural blocked: brightness open {}, closed {}; energy open {}, closed {}",
            brightness(&open),
            brightness(&closed),
            energy(&open),
            energy(&closed)
        );
        assert!(brightness(&closed) < 0.5 * brightness(&open));
        assert!(energy(&closed) < 0.5 * energy(&open));
    }

    /// Steam Audio's parametric reverb, with decay times we supply and no
    /// scene. After a short burst, the measured decay follows the middle
    /// band's time, both ears get it, and they differ.
    #[test]
    fn the_parametric_reverb_follows_the_decay_times_we_give_it() {
        let burst: Sound = noise(0.05)
            .iter()
            .copied()
            .chain(vec![0.0; 4 * RATE as usize])
            .collect();
        let take = |place: Place| {
            render_take(&mut binaural(), 4.0, &|_| {
                scene(&burst, FRONT, 1.0, false, place)
            })
        };
        let window = |out: &[f32], from: f64| {
            let a = (from * f64::from(RATE)) as usize;
            energy(&out[2 * a..2 * (a + RATE as usize / 10)])
        };
        // The time to fall 60 dB, from the fall over 0.2 to 1 second.
        let rt60 = |out: &[f32]| {
            let fall = 10.0 * (window(out, 0.2) / window(out, 0.9)).log10();
            60.0 * 0.7 / fall
        };
        let dry = take(Place::Dry);
        assert!(energy(&dry[2 * (RATE as usize / 5)..]) < 1e-9);
        let forest = rt60(&take(Place::Forest));
        let pass_take = take(Place::MountainPass);
        let pass = rt60(&pass_take);
        eprintln!(
            "parametric reverb, measured RT60: forest {forest:.2} s, mountain pass {pass:.2} s"
        );
        let middle = |place: Place| f64::from(place.decay_times().unwrap()[1]);
        assert!(
            (forest / middle(Place::Forest) - 1.0).abs() < 0.3,
            "forest {forest}"
        );
        assert!(
            (pass / middle(Place::MountainPass) - 1.0).abs() < 0.3,
            "pass {pass}"
        );

        let (l, r) = (channel(&pass_take, 0), channel(&pass_take, 1));
        let tail = (0.3 * f64::from(RATE)) as usize;
        let correlation = l[tail..]
            .iter()
            .zip(&r[tail..])
            .map(|(a, b)| f64::from(a * b))
            .sum::<f64>()
            / (energy(&l[tail..]) * energy(&r[tail..])).sqrt();
        assert!(energy(&r[tail..]) > 0.0);
        assert!(
            correlation.abs() < 0.5,
            "left-right correlation {correlation}"
        );
    }
}
