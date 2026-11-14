//! The numbered script: what each step plays, what it asks, and the
//! results file. Everything goes through plain lines of text, so a screen
//! reader reads the questions and the player types the answers.

use std::f64::consts::TAU;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::render::{Current, Renderer, StereoPlus, render_take};
use crate::rng::Rng;
use crate::scene::{Listener, Point, RATE, Scene, Sound, Source};
use crate::sounds::{Clip, Origin, Sounds, train};

/// A renderer, by the name the player chooses it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Current,
    StereoPlus,
    Binaural,
    BinauralPlus,
}

impl Kind {
    pub const ALL: [Kind; 4] = [
        Kind::Current,
        Kind::StereoPlus,
        Kind::Binaural,
        Kind::BinauralPlus,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Current => "current",
            Kind::StereoPlus => "stereo-plus",
            Kind::Binaural => "binaural",
            Kind::BinauralPlus => "binaural-plus",
        }
    }

    /// Whether the renderer is on Steam Audio, and so uses the head model.
    pub fn is_binaural(self) -> bool {
        matches!(self, Kind::Binaural | Kind::BinauralPlus)
    }

    /// Whether this build has the renderer.
    pub fn is_built(self) -> bool {
        !self.is_binaural() || cfg!(feature = "binaural")
    }

    /// The renderers this build has.
    pub fn built() -> Vec<Kind> {
        Kind::ALL.into_iter().filter(|k| k.is_built()).collect()
    }

    pub fn parse(name: &str) -> Result<Kind, String> {
        let kind = Kind::ALL
            .into_iter()
            .find(|k| k.name() == name)
            .ok_or_else(|| {
                format!("There is no renderer called {name}. The renderers are current, stereo-plus, binaural and binaural-plus.")
            })?;
        if kind.is_built() {
            Ok(kind)
        } else {
            Err(format!(
                "This build has no {name} renderer. Build it with: cargo build -p listening-room --release --features binaural"
            ))
        }
    }

    /// A new renderer of this kind, with nothing playing yet.
    pub fn make(self, sofa: Option<&Path>) -> Result<Box<dyn Renderer>, String> {
        match self {
            Kind::Current => Ok(Box::new(Current)),
            Kind::StereoPlus => Ok(Box::<StereoPlus>::default()),
            #[cfg(feature = "binaural")]
            Kind::Binaural => Ok(Box::new(crate::binaural::Binaural::new(sofa)?)),
            #[cfg(feature = "binaural")]
            Kind::BinauralPlus => Ok(Box::new(crate::binaural::Binaural::plus(sofa)?)),
            #[cfg(not(feature = "binaural"))]
            Kind::Binaural | Kind::BinauralPlus => {
                let _ = sofa;
                Err(format!("This build has no {} renderer.", self.name()))
            }
        }
    }
}

/// What the player chose on the command line.
#[derive(Clone, Debug)]
pub struct Options {
    pub root: Option<PathBuf>,
    /// Only this step, from 1.
    pub step: Option<usize>,
    pub kinds: Vec<Kind>,
    /// A head model for the binaural renderer.
    pub sofa: Option<PathBuf>,
    /// Write each take to this folder instead of playing it.
    pub render_to: Option<PathBuf>,
    pub seed: u64,
}

/// A stretch of sound to render: its length, and the scene at each time.
pub struct Take {
    pub seconds: f64,
    pub scene: Box<dyn Fn(f64) -> Scene>,
}

impl Take {
    fn new(seconds: f64, scene: impl Fn(f64) -> Scene + 'static) -> Take {
        Take {
            seconds,
            scene: Box::new(scene),
        }
    }
}

/// One source, heard by a listener at the origin facing north.
fn one(sound: &Sound, position: Point, blocked: bool) -> Scene {
    Scene {
        listener: Listener::NORTH,
        sources: vec![Source {
            id: 1,
            sound: sound.clone(),
            position,
            loudness: 1.0,
            blocked,
        }],
    }
}

fn circling(s: &Sounds) -> Take {
    let sound = train(&s.footsteps, 0.5, 20);
    Take::new(10.5, move |t| {
        // Clockwise seen from above: ahead, right, behind, left.
        let a = TAU * t / 10.0;
        one(&sound, Point::new(3.0 * a.sin(), 3.0 * a.cos(), 0.0), false)
    })
}

/// Three of `clips`, standing 3 metres in front or behind.
fn standing(clips: &[Clip], behind: bool) -> Take {
    let sound = train(clips, 0.45, 3);
    let y = if behind { -3.0 } else { 3.0 };
    Take::new(2.0, move |_| one(&sound, Point::new(0.0, y, 0.0), false))
}

fn footsteps_front_or_behind(s: &Sounds, behind: bool) -> Take {
    standing(&s.footsteps, behind)
}

fn bright_front_or_behind(s: &Sounds, behind: bool) -> Take {
    standing(std::slice::from_ref(&s.bright), behind)
}

/// Footsteps that walk from 4 metres to the right to 4 metres to the
/// left, 2 metres in front or behind, in 4 seconds.
fn walking_past(s: &Sounds, behind: bool) -> Take {
    let sound = train(&s.footsteps, 0.5, 9);
    let y = if behind { -2.0 } else { 2.0 };
    Take::new(4.5, move |t| {
        let x = 4.0 - 8.0 * (t / 4.0).min(1.0);
        one(&sound, Point::new(x, y, 0.0), false)
    })
}

fn overhead(s: &Sounds) -> Take {
    let sound = train(&s.wings, 0.35, 20);
    Take::new(7.5, move |t| {
        let y = 15.0 - 30.0 * (t / 7.0).min(1.0);
        one(&sound, Point::new(0.0, y, 4.0), false)
    })
}

fn receding(s: &Sounds) -> Take {
    let sound = train(&s.footsteps, 0.5, 24);
    Take::new(12.5, move |t| {
        let y = 1.0 + 39.0 * (t / 12.0).min(1.0);
        one(&sound, Point::new(0.0, y, 0.0), false)
    })
}

fn neighbour(s: &Sounds) -> Take {
    let every = s.building.len().max(1) as f64 / f64::from(RATE);
    let part = train(
        std::slice::from_ref(&s.building),
        every,
        (4.0 / every).ceil() as usize,
    );
    let gap = RATE as usize;
    let mut samples = part.to_vec();
    samples.resize(part.len() + gap, 0.0);
    samples.extend_from_slice(&part);
    let switch = (part.len() + gap / 2) as f64 / f64::from(RATE);
    let seconds = samples.len() as f64 / f64::from(RATE) + 0.5;
    let sound: Sound = samples.into();
    // A building on the next square east, 12 metres away. It is loud, so
    // that the closed exit stays audible at that distance.
    Take::new(seconds, move |t| {
        let mut scene = one(&sound, Point::new(12.0, 0.0, 0.0), t >= switch);
        scene.sources[0].loudness = 4.0;
        scene
    })
}

/// What a step asks after each version.
enum Ask {
    /// A rating from 1 to 5.
    Rating {
        question: &'static str,
        take: fn(&Sounds) -> Take,
    },
    /// [`TRIALS`] trials, half in front and half behind, in random order.
    FrontOrBehind {
        /// What the summary calls the step.
        label: &'static str,
        take: fn(&Sounds, bool) -> Take,
    },
}

struct Step {
    title: &'static str,
    slug: &'static str,
    intro: &'static str,
    /// Said when the current renderer is in the step: the game today
    /// cannot do what the step tries, so its version is the baseline.
    current_note: Option<&'static str>,
    /// The only renderers the step is for, if not all of them.
    only: Option<&'static [Kind]>,
    ask: Ask,
}

/// Trials for each renderer in a front-or-behind step. With all four
/// renderers, a whole session has ten such runs: 40 trials.
const TRIALS: usize = 4;

const STEPS: [Step; 7] = [
    Step {
        title: "a footstep circling you",
        slug: "circling",
        intro: "Footsteps go once round you: they start in front, go to your right, behind you, to your left, and back in front.",
        current_note: None,
        only: None,
        ask: Ask::Rating {
            question: "How well could you follow the footsteps all the way round?",
            take: circling,
        },
    },
    Step {
        title: "in front or behind",
        slug: "front-or-behind",
        intro: "You hear three footsteps, either in front of you or behind you. After each, say which. Each version has 4 of them, half in front and half behind, in random order.",
        current_note: None,
        only: None,
        ask: Ask::FrontOrBehind {
            label: "footsteps",
            take: footsteps_front_or_behind,
        },
    },
    Step {
        title: "in front or behind, with a bright sound",
        slug: "front-or-behind-bright",
        intro: "Step 2 again, for the two binaural renderers, with a short bright burst of noise in place of the footsteps. It tells whether the footstep sound or the renderer made front and behind hard. You hear three bursts, either in front of you or behind you. After each, say which. Each version has 4 of them, half in front and half behind, in random order.",
        current_note: None,
        only: Some(&[Kind::Binaural, Kind::BinauralPlus]),
        ask: Ask::FrontOrBehind {
            label: "bright sound",
            take: bright_front_or_behind,
        },
    },
    Step {
        title: "footsteps walking past, in front or behind",
        slug: "walking-past",
        intro: "Footsteps start at your right and walk to your left, passing either in front of you or behind you. After each, say which. Each version has 4 of them, half in front and half behind, in random order.",
        current_note: None,
        only: None,
        ask: Ask::FrontOrBehind {
            label: "walking past",
            take: walking_past,
        },
    },
    Step {
        title: "a sound passing overhead",
        slug: "overhead",
        intro: "A dragon's wings come from in front of you, pass over your head, and go away behind you.",
        current_note: None,
        only: None,
        ask: Ask::Rating {
            question: "How much did it sound like it passed over your head?",
            take: overhead,
        },
    },
    Step {
        title: "a sound moving away",
        slug: "receding",
        intro: "Footsteps start just in front of you and walk straight away into the distance.",
        current_note: None,
        only: None,
        ask: Ask::Rating {
            question: "How much did it sound like the footsteps moved away, and not only like they got quieter?",
            take: receding,
        },
    },
    Step {
        title: "a neighbour through an open exit and a closed one",
        slug: "neighbour",
        intro: "A blacksmith works on the next square, to your right. First you hear it through an open exit, then, after a short pause, through a closed one.",
        current_note: Some(
            "The game today cannot muffle a sound, so the current renderer's version plays the open and the closed exit the same way.",
        ),
        only: None,
        ask: Ask::Rating {
            question: "How clear was the difference between the open exit and the closed one?",
            take: neighbour,
        },
    },
];

/// The number of steps in the script.
pub const NUM_STEPS: usize = STEPS.len();

/// Why a session ended early.
enum Stop {
    /// The player typed q, or the input ended.
    Quit,
    Io(io::Error),
    Failed(String),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Stop {
        Stop::Io(e)
    }
}

type Flow<T> = Result<T, Stop>;

/// Where takes go.
enum Player {
    /// Into WAV files in this folder.
    Folder(PathBuf),
    /// To the default sound device.
    Speakers(rodio::MixerDeviceSink),
}

impl Player {
    /// Plays a take, or writes it. Returns the file it wrote, if any.
    fn play(&mut self, file: &str, samples: &[f32]) -> Result<Option<PathBuf>, String> {
        match self {
            Player::Folder(dir) => {
                let path = dir.join(file);
                crate::wav::write(&path, samples)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Some(path))
            }
            Player::Speakers(sink) => {
                // Lets the screen reader finish the last line (such as
                // "Right.") before the next sound starts.
                std::thread::sleep(std::time::Duration::from_secs(1));
                let two = std::num::NonZero::new(2).expect("two channels");
                let rate = std::num::NonZero::new(RATE).expect("a rate");
                sink.mixer().add(rodio::buffer::SamplesBuffer::new(
                    two,
                    rate,
                    samples.to_vec(),
                ));
                let seconds = samples.len() as f64 / 2.0 / f64::from(RATE);
                std::thread::sleep(std::time::Duration::from_secs_f64(seconds + 0.2));
                Ok(None)
            }
        }
    }
}

/// A step's answers, as written to the results file.
struct StepResult {
    lines: Vec<String>,
}

/// Every answer so far, by renderer.
struct Results {
    header: Vec<String>,
    steps: Vec<StepResult>,
    ratings: Vec<(Kind, u8)>,
    /// Renderer, the step's label, right, out of.
    front_or_behind: Vec<(Kind, &'static str, usize, usize)>,
}

impl Results {
    fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for kind in Kind::ALL {
            let ratings: Vec<u8> = self
                .ratings
                .iter()
                .filter(|(k, _)| *k == kind)
                .map(|(_, r)| *r)
                .collect();
            let scores: Vec<String> = self
                .front_or_behind
                .iter()
                .filter(|(k, ..)| *k == kind)
                .map(|(_, label, right, of)| format!("{label} {right} of {of} right"))
                .collect();
            if ratings.is_empty() && scores.is_empty() {
                continue;
            }
            let mut parts = Vec::new();
            if !ratings.is_empty() {
                let mean =
                    ratings.iter().map(|&r| f64::from(r)).sum::<f64>() / ratings.len() as f64;
                parts.push(format!(
                    "average rating {mean:.1} out of 5 over {} {}",
                    ratings.len(),
                    if ratings.len() == 1 { "step" } else { "steps" }
                ));
            }
            if !scores.is_empty() {
                parts.push(format!("in front or behind: {}", scores.join(", ")));
            }
            lines.push(format!("{}: {}.", kind.name(), parts.join("; ")));
        }
        lines
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.header {
            out.push_str(line);
            out.push('\n');
        }
        for step in &self.steps {
            out.push('\n');
            for line in &step.lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str("\nSummary\n");
        for line in self.summary() {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

struct Session<'a> {
    input: &'a mut dyn BufRead,
    out: &'a mut dyn Write,
    player: Player,
    sounds: Sounds,
    rng: Rng,
    sofa: Option<PathBuf>,
    /// Each renderer's level against the current renderer's.
    gains: Vec<(Kind, f32)>,
    results: Results,
    results_path: PathBuf,
}

impl Session<'_> {
    fn say(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.out.flush()
    }

    /// Asks a question and returns the answer, trimmed. `q` or the end of
    /// the input stops the session.
    fn ask(&mut self, prompt: &str) -> Flow<String> {
        self.say(prompt)?;
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            return Err(Stop::Quit);
        }
        let answer = line.trim().to_owned();
        if answer.eq_ignore_ascii_case("q") || answer.eq_ignore_ascii_case("quit") {
            return Err(Stop::Quit);
        }
        Ok(answer)
    }

    fn gain(&self, kind: Kind) -> f32 {
        self.gains
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(1.0, |(_, g)| *g)
    }

    /// Renders the take through a new renderer of `kind` and plays it.
    fn present(&mut self, kind: Kind, take: &Take, file: &str) -> Flow<()> {
        let mut renderer = kind.make(self.sofa.as_deref()).map_err(Stop::Failed)?;
        let gain = self.gain(kind);
        let mut samples = render_take(renderer.as_mut(), take.seconds, &*take.scene);
        for s in &mut samples {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
        if let Some(path) = self.player.play(file, &samples).map_err(Stop::Failed)? {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            self.say(&format!("Wrote {name}."))?;
        }
        Ok(())
    }

    fn save(&mut self) -> io::Result<()> {
        std::fs::write(&self.results_path, self.results.text())
    }

    /// Adds a line to the current step's results and saves them.
    fn note(&mut self, line: String) -> io::Result<()> {
        if let Some(step) = self.results.steps.last_mut() {
            step.lines.push(line);
        }
        self.save()
    }

    fn run_step(&mut self, number: usize, kinds: &[Kind]) -> Flow<()> {
        let step = &STEPS[number - 1];
        let mut kinds: Vec<Kind> = kinds
            .iter()
            .copied()
            .filter(|k| step.only.is_none_or(|only| only.contains(k)))
            .collect();
        if kinds.is_empty() {
            let only: Vec<&str> = step
                .only
                .unwrap_or_default()
                .iter()
                .map(|k| k.name())
                .collect();
            self.say(&format!(
                "Step {number} of {NUM_STEPS}: {}. This step is only for the {} renderers, and none of them is chosen, so it is skipped.",
                step.title,
                only.join(" and ")
            ))?;
            return Ok(());
        }
        self.say(&format!(
            "Step {number} of {NUM_STEPS}: {}. {}",
            step.title, step.intro
        ))?;
        if let Some(note) = step.current_note
            && kinds.contains(&Kind::Current)
        {
            self.say(note)?;
        }
        // The step is in the results from the start, so stopping part way
        // keeps the answers already given.
        self.results.steps.push(StepResult {
            lines: vec![format!("Step {number}: {}", step.title)],
        });
        self.save()?;
        self.rng.shuffle(&mut kinds);
        let n = kinds.len();
        if n > 1 {
            self.say(&format!(
                "You hear {n} versions, one for each renderer, in random order. You learn which was which at the end of the step."
            ))?;
        }
        for (i, &kind) in kinds.iter().enumerate() {
            let version = i + 1;
            match step.ask {
                Ask::Rating { question, take } => {
                    self.ask(&format!(
                        "Version {version} of {n}. Press Enter to play it."
                    ))?;
                    let take = take(&self.sounds);
                    let file = format!("step{number}-{}-{}.wav", step.slug, kind.name());
                    self.present(kind, &take, &file)?;
                    let rating = loop {
                        let answer = self.ask(&format!(
                            "{question} Type a number from 1, not at all, to 5, perfectly, or r to hear it again."
                        ))?;
                        if answer.eq_ignore_ascii_case("r") {
                            self.present(kind, &take, &file)?;
                        } else if let Ok(r @ 1..=5) = answer.parse::<u8>() {
                            break r;
                        } else {
                            self.say("Please type a number from 1 to 5, or r.")?;
                        }
                    };
                    self.results.ratings.push((kind, rating));
                    self.note(format!("  {}: {rating} out of 5", kind.name()))?;
                }
                Ask::FrontOrBehind { label, take } => {
                    self.ask(&format!("Version {version} of {n}. Press Enter to start."))?;
                    let mut trials = [false, true].repeat(TRIALS / 2);
                    self.rng.shuffle(&mut trials);
                    let (mut right, mut right_front, mut right_behind) = (0, 0, 0);
                    for (t, &behind) in trials.iter().enumerate() {
                        let take = take(&self.sounds, behind);
                        let truth = if behind { "behind" } else { "front" };
                        let file = format!(
                            "step{number}-{}-{}-trial{}-{truth}.wav",
                            step.slug,
                            kind.name(),
                            t + 1
                        );
                        self.present(kind, &take, &file)?;
                        let said_behind = loop {
                            let answer = self.ask(&format!(
                                "Trial {} of {TRIALS}: type f for in front, b for behind, or r to hear it again.",
                                t + 1
                            ))?;
                            match answer.to_ascii_lowercase().as_str() {
                                "f" | "front" => break false,
                                "b" | "behind" | "back" => break true,
                                "r" => self.present(kind, &take, &file)?,
                                _ => self.say("Please type f, b or r.")?,
                            }
                        };
                        if said_behind == behind {
                            right += 1;
                            if behind {
                                right_behind += 1;
                            } else {
                                right_front += 1;
                            }
                            self.say("Right.")?;
                        } else if behind {
                            self.say("Wrong: it was behind.")?;
                        } else {
                            self.say("Wrong: it was in front.")?;
                        }
                        let side = |b: bool| if b { "behind" } else { "in front" };
                        self.note(format!(
                            "  {}, trial {}: {}, answered {}",
                            kind.name(),
                            t + 1,
                            side(behind),
                            side(said_behind)
                        ))?;
                    }
                    self.say(&format!("Version {version}: {right} of {TRIALS} right."))?;
                    self.results
                        .front_or_behind
                        .push((kind, label, right, TRIALS));
                    self.note(format!(
                        "  {}: {right} of {TRIALS} right (in front {right_front} of {half}, behind {right_behind} of {half})",
                        kind.name(),
                        half = TRIALS / 2
                    ))?;
                }
            }
        }
        if n > 1 {
            let reveal: Vec<String> = kinds
                .iter()
                .enumerate()
                .map(|(i, k)| format!("version {} was {}", i + 1, k.name()))
                .collect();
            let mut text = reveal.join(", ");
            text[..1].make_ascii_uppercase();
            self.say(&format!("{text}."))?;
        }
        let comment = self
            .ask("Any comment on this step? Type it and press Enter, or press Enter to go on.")?;
        if !comment.is_empty() {
            self.note(format!("  Comment: {comment}"))?;
        }
        Ok(())
    }
}

/// How loud each renderer plays a noise 2 metres in front, against the
/// current renderer, so versions are compared at the same level.
fn calibrate(kinds: &[Kind], sofa: Option<&Path>) -> Result<Vec<(Kind, f32)>, String> {
    let mut rng = Rng::new(7);
    let noise: Sound = (0..RATE / 2).map(|_| 0.5 * rng.noise()).collect();
    let level = |kind: Kind| -> Result<f32, String> {
        let mut renderer = kind.make(sofa)?;
        let out = render_take(renderer.as_mut(), 0.5, &|_| {
            one(&noise, Point::new(0.0, 2.0, 0.0), false)
        });
        Ok((out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt())
    };
    let reference = level(Kind::Current)?;
    kinds
        .iter()
        .map(|&k| {
            let l = level(k)?;
            Ok((k, if l > 0.0 { reference / l } else { 1.0 }))
        })
        .collect()
}

/// Runs the script: reads answers from `input`, writes everything the
/// player is told to `out`.
pub fn run(opts: &Options, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<(), String> {
    let player = match &opts.render_to {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            Player::Folder(dir.clone())
        }
        None => {
            let mut sink = rodio::DeviceSinkBuilder::open_default_sink()
                .map_err(|e| format!("Cannot open the sound device: {e}"))?;
            sink.log_on_drop(false);
            Player::Speakers(sink)
        }
    };
    let results_path = std::path::absolute(
        opts.render_to
            .as_deref()
            .unwrap_or(Path::new("."))
            .join("listening-room-results.txt"),
    )
    .map_err(|e| e.to_string())?;
    let sounds = Sounds::load(opts.root.as_deref());
    let gains = calibrate(&opts.kinds, opts.sofa.as_deref())?;
    let names: Vec<&str> = opts.kinds.iter().map(|k| k.name()).collect();
    let sounds_line = match &sounds.origin {
        Origin::Game(root) => format!("Sounds: the game's own, from {}.", root.display()),
        Origin::Generated(why) => format!(
            "Sounds: the game's sounds were not found ({why}), so noise bursts take their place. Say so when you send your answers."
        ),
    };
    let head = match &opts.sofa {
        Some(p) => format!("Head model for the binaural renderers: {}.", p.display()),
        None => "Head model for the binaural renderers: Steam Audio's default.".to_owned(),
    };
    let mut header = vec![
        "Listening room results".to_owned(),
        format!("Renderers: {}.", names.join(", ")),
        sounds_line.clone(),
    ];
    if opts.kinds.iter().any(|k| k.is_binaural()) {
        header.push(head.clone());
    }
    header.push(format!(
        "Seed: {}. Run again with --seed {} to hear the same order.",
        opts.seed, opts.seed
    ));
    let mut session = Session {
        input,
        out,
        player,
        sounds,
        rng: Rng::new(opts.seed),
        sofa: opts.sofa.clone(),
        gains,
        results: Results {
            header,
            steps: Vec::new(),
            ratings: Vec::new(),
            front_or_behind: Vec::new(),
        },
        results_path: results_path.clone(),
    };
    let io = |e: io::Error| e.to_string();
    session
        .say("Listening room. It plays the game's sounds through different renderers and asks what you hear. Use headphones.")
        .map_err(io)?;
    session
        .say(&format!("Renderers: {}.", names.join(", ")))
        .map_err(io)?;
    session.say(&sounds_line).map_err(io)?;
    if opts.kinds.iter().any(|k| k.is_binaural()) {
        session.say(&head).map_err(io)?;
    }
    session
        .say(&format!(
            "Your answers are saved after each step in {}. Type q at any question to stop.",
            results_path.display()
        ))
        .map_err(io)?;
    let steps: Vec<usize> = match opts.step {
        Some(n) => vec![n],
        None => (1..=NUM_STEPS).collect(),
    };
    let mut outcome = Ok(());
    for n in steps {
        if let Err(stop) = session.run_step(n, &opts.kinds) {
            outcome = Err(stop);
            break;
        }
    }
    session.save().map_err(io)?;
    match outcome {
        Ok(()) => {
            session.say("That is the end of the script.").map_err(io)?;
            for line in session.results.summary() {
                session.say(&line).map_err(io)?;
            }
            session
                .say(&format!(
                    "Your answers are in {}. Please send that file back.",
                    results_path.display()
                ))
                .map_err(io)?;
            Ok(())
        }
        Err(Stop::Quit) => {
            session
                .say(&format!(
                    "Stopped. Your answers so far are in {}.",
                    results_path.display()
                ))
                .map_err(io)?;
            Ok(())
        }
        Err(Stop::Io(e)) => Err(e.to_string()),
        Err(Stop::Failed(e)) => Err(e),
    }
}
