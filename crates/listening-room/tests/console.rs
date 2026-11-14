//! The program as the player uses it: answers typed on standard input,
//! everything said on standard output. Every test renders to a folder, so
//! nothing plays.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn game_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        root.join("res/rules.txt").is_file(),
        "the game data is missing: run scripts/fetch-upstream.ps1"
    );
    root
}

/// Runs the program with `args`, typing `answers`, one per line. Returns
/// what it printed.
fn run(args: &[&str], answers: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_listening-room"))
        .args(args)
        .env_remove("SOUNDRTS_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = answers.join("\n");
    input.push('\n');
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

fn wav_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".wav"))
        .collect();
    names.sort();
    names
}

#[test]
fn the_front_or_behind_step_keeps_score() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let root = game_root();
    let mut answers = Vec::new();
    for _ in 0..2 {
        answers.push("");
        answers.extend(["f"; 8]);
    }
    answers.push("");
    let stdout = run(
        &[
            "--root",
            root.to_str().unwrap(),
            "--render-to",
            out,
            "--step",
            "2",
            "--seed",
            "7",
            "--renderers",
            "current,stereo-plus",
        ],
        &answers,
    );
    assert!(stdout.contains("Step 2 of 6: in front or behind."));
    assert!(stdout.contains("Sounds: the game's own, from"));
    // Half the trials are in front, so answering "in front" every time
    // scores 4 of 8 for each version.
    assert_eq!(stdout.matches(": 4 of 8 right.").count(), 2, "{stdout}");
    assert!(
        stdout.contains("Version 1 was current, version 2 was stereo-plus.")
            || stdout.contains("Version 1 was stereo-plus, version 2 was current."),
        "{stdout}"
    );

    let files = wav_files(dir.path());
    assert_eq!(files.len(), 16, "{files:?}");
    for kind in ["current", "stereo-plus"] {
        let front = files
            .iter()
            .filter(|f| {
                f.starts_with(&format!("step2-front-or-behind-{kind}-"))
                    && f.ends_with("-front.wav")
            })
            .count();
        assert_eq!(front, 4, "{kind}: {files:?}");
    }
    let wav = std::fs::read(dir.path().join(&files[0])).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert!(wav.len() > 44 + 4 * 2 * 48_000);

    let results = std::fs::read_to_string(dir.path().join("listening-room-results.txt")).unwrap();
    assert!(results.contains("Seed: 7."), "{results}");
    assert!(
        results.contains("  current: 4 of 8 right (in front 4 of 4, behind 0 of 4)"),
        "{results}"
    );
    assert!(
        results.contains("current: in front or behind, 4 of 8 right."),
        "{results}"
    );
}

#[test]
fn the_whole_script_runs_and_writes_the_answers() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().to_str().unwrap();
    let root = game_root();
    let mut answers: Vec<&str> = Vec::new();
    // Step 1: a bad answer, a replay, then a rating; a comment.
    answers.extend(["", "7", "r", "3", "", "3", "Smooth enough"]);
    // Step 2: "behind" every time.
    for _ in 0..2 {
        answers.push("");
        answers.extend(["b"; 8]);
    }
    answers.push("");
    // Steps 3 and 4.
    answers.extend(["", "4", "", "4", ""]);
    answers.extend(["", "5", "", "5", ""]);
    // Steps 5 and 6.
    answers.extend(["", "2", "", "2", ""]);
    answers.extend(["", "1", "", "1", ""]);
    let stdout = run(
        &[
            "--root",
            root.to_str().unwrap(),
            "--render-to",
            out,
            "--renderers",
            "current,stereo-plus",
        ],
        &answers,
    );
    assert!(stdout.contains("Please type a number from 1 to 5, or r."));
    assert!(stdout.contains("The game today has no echo"));
    assert!(stdout.contains("That is the end of the script."));
    assert!(stdout.contains(
        "current: average rating 3.0 out of 5 over 5 steps; in front or behind, 4 of 8 right."
    ));
    assert!(stdout.contains(
        "stereo-plus: average rating 3.0 out of 5 over 5 steps; in front or behind, 4 of 8 right."
    ));

    let files = wav_files(dir.path());
    for name in [
        "step1-circling-current.wav",
        "step1-circling-stereo-plus.wav",
        "step3-overhead-current.wav",
        "step4-receding-stereo-plus.wav",
        "step5-neighbour-stereo-plus.wav",
        "step6-terrain-stereo-plus.wav",
    ] {
        assert!(files.iter().any(|f| f == name), "{name} in {files:?}");
    }
    assert!(files.iter().any(|f| f == "step6-terrain-current.wav"));

    let results = std::fs::read_to_string(dir.path().join("listening-room-results.txt")).unwrap();
    for line in [
        "Step 1: a footstep circling you",
        "  current: 3 out of 5",
        "  Comment: Smooth enough",
        "Step 6: the echo of the place",
        "  stereo-plus: 1 out of 5",
        "Summary",
    ] {
        assert!(results.contains(line), "{line} in:\n{results}");
    }
}

#[test]
fn without_the_game_data_it_says_so_and_uses_noise_bursts() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let stdout = run(
        &[
            "--root",
            empty.path().to_str().unwrap(),
            "--render-to",
            dir.path().to_str().unwrap(),
            "--step",
            "1",
            "--renderers",
            "current",
        ],
        &["", "2", ""],
    );
    assert!(
        stdout.contains("the game's sounds were not found") && stdout.contains("noise bursts"),
        "{stdout}"
    );
    assert_eq!(wav_files(dir.path()), ["step1-circling-current.wav"]);
    let results = std::fs::read_to_string(dir.path().join("listening-room-results.txt")).unwrap();
    assert!(results.contains("noise bursts take their place"));
    assert!(results.contains("  current: 2 out of 5"));
}

#[test]
fn q_stops_and_keeps_the_answers_so_far() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_root();
    let stdout = run(
        &[
            "--root",
            root.to_str().unwrap(),
            "--render-to",
            dir.path().to_str().unwrap(),
            "--renderers",
            "stereo-plus",
        ],
        // Stops at the comment prompt of step 1.
        &["", "4", "q"],
    );
    assert!(stdout.contains("Stopped. Your answers so far are in"));
    let results = std::fs::read_to_string(dir.path().join("listening-room-results.txt")).unwrap();
    assert!(results.contains("Step 1: a footstep circling you\n  stereo-plus: 4 out of 5\n"));
    assert!(!results.contains("Step 2"));
}

#[test]
fn a_renderer_this_build_does_not_have_is_refused_plainly() {
    let output = Command::new(env!("CARGO_BIN_EXE_listening-room"))
        .args(["--renderers", "surround"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("There is no renderer called surround.")
    );
}

#[cfg(not(feature = "binaural"))]
#[test]
fn a_head_model_needs_the_binaural_build() {
    let output = Command::new(env!("CARGO_BIN_EXE_listening-room"))
        .args(["--sofa", "head.sofa"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Build with --features binaural."));
}

#[cfg(feature = "binaural")]
#[test]
fn a_head_model_that_cannot_be_read_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.sofa");
    let output = Command::new(env!("CARGO_BIN_EXE_listening-room"))
        .args(["--renderers", "binaural", "--step", "1", "--render-to"])
        .arg(dir.path())
        .arg("--sofa")
        .arg(&missing)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Steam Audio cannot use the head model"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
