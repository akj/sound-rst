//! The renderers, checked from the buffers they render. Nothing here plays
//! a sound.

use listening_room::render::{BLOCK, Current, Renderer, StereoPlus, render_take};
use listening_room::rng::Rng;
use listening_room::scene::{Listener, Point, RATE, Scene, Sound, Source};
use listening_room::sounds::Sounds;

fn noise(seconds: f64) -> Sound {
    let mut rng = Rng::new(3);
    (0..(seconds * f64::from(RATE)) as usize)
        .map(|_| 0.5 * rng.noise())
        .collect()
}

fn scene(sound: &Sound, position: Point, loudness: f64, blocked: bool) -> Scene {
    Scene {
        listener: Listener::NORTH,
        sources: vec![Source {
            id: 1,
            sound: sound.clone(),
            position,
            loudness,
            blocked,
        }],
    }
}

/// Renders one source standing still.
fn still(r: &mut dyn Renderer, sound: &Sound, position: Point, seconds: f64) -> Vec<f32> {
    render_take(r, seconds, &|_| scene(sound, position, 1.0, false))
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

/// The energy of `x` between `low` and `high` Hz, from its spectrum.
fn band_energy(x: &[f32], low: f64, high: f64) -> f64 {
    let n = x.len().next_power_of_two();
    let mut re: Vec<f64> = x.iter().map(|&s| f64::from(s)).collect();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    // An in-place radix-2 FFT.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = -std::f64::consts::TAU / len as f64;
        for s in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wi, wr) = (step * k as f64).sin_cos();
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
    // Scaled so that the whole spectrum holds the signal's energy.
    let hz = f64::from(RATE) / n as f64;
    2.0 / n as f64
        * (0..n / 2)
            .filter(|&k| (low..high).contains(&(k as f64 * hz)))
            .map(|k| re[k] * re[k] + im[k] * im[k])
            .sum::<f64>()
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
fn the_spectrum_measure_finds_a_tone_in_its_band() {
    let tone: Vec<f32> = (0..RATE as usize / 10)
        .map(|i| (std::f64::consts::TAU * 9_000.0 * i as f64 / f64::from(RATE)).sin() as f32)
        .collect();
    let all = band_energy(&tone, 0.0, 24_000.0);
    assert!((all / energy(&tone) - 1.0).abs() < 0.01, "Parseval: {all}");
    assert!(band_energy(&tone, 8_000.0, 10_000.0) > 0.99 * all);
}

#[test]
fn the_bright_sound_is_short_and_has_real_content_above_12_khz() {
    let bright = Sounds::generated(String::new()).bright;
    assert!(
        bright.len() <= RATE as usize / 20,
        "{} samples",
        bright.len()
    );
    let all = band_energy(&bright, 0.0, 24_000.0);
    let above_6 = band_energy(&bright, 6_000.0, 24_000.0) / all;
    let above_12 = band_energy(&bright, 12_000.0, 24_000.0) / all;
    assert!(above_6 > 0.6, "share above 6 kHz {above_6}");
    assert!(above_12 > 0.3, "share above 12 kHz {above_12}");
    // It starts at full strength within a millisecond.
    let peak = bright.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let first = bright.iter().position(|s| s.abs() >= 0.5 * peak).unwrap();
    assert!(
        first < RATE as usize / 1000,
        "reaches half its peak at {first}"
    );
}

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
            scene(&sound, Point::new(0.0, 1.0, 0.0), loudness, false)
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
            scene(&sound, east, 1.0, blocked)
        });
        channel(&out, 1)
    };
    let (open, closed) = (take(false), take(true));
    assert!(brightness(&closed) < 0.2 * brightness(&open));
    assert!(energy(&closed) < 0.5 * energy(&open));
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
    use listening_room::sounds::{Origin, train};

    fn binaural() -> Binaural {
        Binaural::new(None).expect("Steam Audio starts")
    }

    fn plus() -> Binaural {
        Binaural::plus(None).expect("Steam Audio starts")
    }

    /// The first frame at which a channel reaches a tenth of its peak.
    fn onset(x: &[f32]) -> usize {
        let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        x.iter().position(|s| s.abs() >= 0.1 * peak).unwrap()
    }

    /// The game's own sounds, as the script plays them.
    fn game_sounds() -> Sounds {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
        let sounds = Sounds::load(Some(&root));
        assert!(
            matches!(sounds.origin, Origin::Game(_)),
            "the game data is missing: run scripts/fetch-upstream.ps1"
        );
        sounds
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

    /// The head model's front-behind cue points the right way: the ear
    /// hears more above 6 kHz from a source in front than from the same
    /// source behind.
    #[test]
    fn a_broadband_source_in_front_has_more_treble_than_behind_in_both_ears() {
        let sound = noise(0.5);
        let front = still(&mut binaural(), &sound, FRONT, 0.5);
        let behind = still(&mut binaural(), &sound, BEHIND, 0.5);
        for (ear, c) in [("left", 0), ("right", 1)] {
            let f = band_energy(&channel(&front, c), 6_000.0, 24_000.0);
            let b = band_energy(&channel(&behind, c), 6_000.0, 24_000.0);
            eprintln!("binaural, {ear} ear, energy above 6 kHz: front {f:.4e}, behind {b:.4e}");
            assert!(f > 1.05 * b, "{ear} ear: front {f}, behind {b}");
        }
    }

    #[test]
    fn binaural_plus_dulls_a_source_behind_and_leaves_it_alone_in_front() {
        let sound = noise(0.5);
        let plain_front = still(&mut binaural(), &sound, FRONT, 0.5);
        let plus_front = still(&mut plus(), &sound, FRONT, 0.5);
        assert_eq!(plain_front, plus_front);

        let plain_behind = still(&mut binaural(), &sound, BEHIND, 0.5);
        let plus_behind = still(&mut plus(), &sound, BEHIND, 0.5);
        for c in 0..2 {
            let (plain, plus) = (
                brightness(&channel(&plain_behind, c)),
                brightness(&channel(&plus_behind, c)),
            );
            eprintln!("behind, ear {c}: brightness binaural {plain}, binaural-plus {plus}");
            assert!(plus < 0.5 * plain, "ear {c}: binaural {plain}, plus {plus}");
        }
    }

    /// The script's front-or-behind takes: three sounds 3 metres in front
    /// or behind. Plain binaural plays the game's footstep louder behind
    /// (Steam Audio's head model lifts 1 to 2 kHz behind, where the
    /// footstep has most of its energy); binaural-plus never does.
    #[test]
    fn binaural_plus_is_quieter_behind_than_in_front() {
        let sounds = game_sounds();
        let level = |r: &mut dyn Renderer, clips: &[Vec<f32>], at: Point| {
            energy(&still(r, &train(clips, 0.45, 3), at, 2.0))
        };
        let footstep = |r: fn() -> Binaural, at| level(&mut r(), &sounds.footsteps, at);
        let (front, behind) = (footstep(binaural, FRONT), footstep(binaural, BEHIND));
        let db = |a: f64, b: f64| 10.0 * (a / b).log10();
        eprintln!(
            "binaural, footstep: behind against front {:+.2} dB",
            db(behind, front)
        );
        assert!(behind > front, "front {front}, behind {behind}");

        let bright = std::slice::from_ref(&sounds.bright);
        for (name, clips) in [
            ("footstep", &sounds.footsteps[..]),
            ("bright sound", bright),
        ] {
            let front = level(&mut plus(), clips, FRONT);
            let behind = level(&mut plus(), clips, BEHIND);
            eprintln!(
                "binaural-plus, {name}: behind against front {:+.2} dB",
                db(behind, front)
            );
            assert!(behind < front, "{name}: front {front}, behind {behind}");
        }
    }

    #[test]
    fn binaural_muffles_a_source_through_a_closed_exit() {
        let sound = noise(0.5);
        let east = Point::new(12.0, 0.0, 0.0);
        let take = |blocked: bool| {
            let out = render_take(&mut binaural(), 0.5, &|_| scene(&sound, east, 1.0, blocked));
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
}
