---
name: verify-soundrts
description: Prove that a feature of sound-rst (the Rust port of the audio game SoundRTS, played with the keyboard and heard through a screen reader) works, and name the layer of the proof. Use before you report that a change to the menus, a game, multiplayer, the first start, or speech works.
---

# Verify sound-rst

A player presses keys in the game's window and hears the game through a screen reader and positional sound. An agent can do neither. The built program takes keys only from real key presses in its focused window, and a running game speaks through the screen reader of the person at the machine and takes their keyboard.

So every proof here is on one of three layers. Say which layer in every claim you make.

- **harness.** The client's own code runs in a test process on a recorded speech device, a recorded audio device, scripted keys, and a manual clock. The multiplayer tests also run real servers on 127.0.0.1. A harness proof shows the text that the client sends to the speech device, the sounds it starts, and the files it writes. It does not show the window, the audio device, a screen reader, or a real network.
- **program.** The built `soundrts.exe` runs with `--probe` or `--fetch-data`. A program proof shows that the program starts, finds or fetches its data, and reports the audio device and the screen reader it found. It shows no menu and no game.
- **ears.** Andrew plays the game with NVDA or JAWS. Only this layer shows what a screen reader says, what the sounds are like, and that the window takes keys. Give Andrew the numbered script from the feature file and wait for pass or fail on each step.

Write "the harness test passes, and the client sent `quit,` to the recorded speech device". A harness proof or a program proof never supports "the game says" or "NVDA reads".

Start the game only through the helper. A game started without `--probe` or `--fetch-data` speaks and takes the keyboard, so `cargo run -p soundrts` without one of those flags belongs to Andrew alone. Send no keys to the desktop, and leave audio devices, NVDA, and speech settings as they are. A proof that needs scripted keys in the built program needs a product change. Stop and ask.

## Launch

The helper needs Windows, PowerShell 7, Git, and rustup. Run every command below from the root of the checkout.

```powershell
pwsh -NoProfile -File .claude/skills/verify-soundrts/verify.ps1 launch
```

`launch` fetches the original game's data into `vendor/soundrts-upstream` at the pinned commit, builds the game and the client's tests, and then runs `doctor`. On a new checkout the build takes several minutes. Run `launch` in the foreground with the longest time limit your tool allows, and stay in your turn until it ends. If the time limit cuts `launch` off, run it again. `launch` is safe to rerun, and Cargo keeps what it has built.

The checkout is ready when the last line is `doctor: ready`. Nothing keeps running after `launch`. Each drive starts its own test process or its own short run of the program, with a new, empty settings folder.

## Doctor

```powershell
pwsh -NoProfile -File .claude/skills/verify-soundrts/verify.ps1 doctor
```

Run `doctor` first whenever something looks off. It checks four things and stops at the first that fails:

1. `rustc` is the version that `rust-toolchain.toml` pins.
2. `vendor/soundrts-upstream` is the commit that `scripts/upstream-commit.txt` pins.
3. The built program starts with `--probe` and finds that data.
4. The harness builds a client on that data and walks the main menu on the recorded devices.

`doctor` changes only Cargo's build folder and its own evidence file. The `It would speak through` line names the screen reader that a real game would use on this machine.

## Drive

Each feature has a file in [features](features/README.md) with its exact commands. Read the index first. There are three ways to drive:

- **harness.** `verify.ps1 test <feature> <test-file> [<test-name>...]` runs `cargo test -p soundrts --test <test-file>` and keeps the output. `<feature>` is the name of a feature file, and `<test-file>` is a file of `crates/client/tests`. With test names, only those tests run. The command fails when a name matches no test, because `cargo test` alone would pass.
- **program.** `verify.ps1 first-start` runs the built program from an empty data folder. `verify.ps1 release-check` builds a release folder and runs `scripts/check-release.ps1` on it. The release build takes minutes.
- **ears.** The `By ear` script of the feature file, for Andrew.

To prove a change, drive every sub-feature that the change touches, at the highest layer the feature file offers for it. When the change alters what a player hears, the harness proof is half of the proof. The other half is Andrew's ear script.

## Evidence

Evidence goes to `target/verify-soundrts/<run>/` in the checkout. `<run>` is the short commit hash, with `-dirty` added when the checkout has uncommitted changes.

- `doctor.log`
- `<feature>/<test-file>.log` for each `test` command
- `<feature>/transcripts/<scenario>.txt` for the `transcripts` test file. Each `step` line is a key the scripted player pressed. Each `voice` line is a message the client sent to the speech device.
- `first-start/first-start.log` and `first-start/release-check.log`

Each log starts with the feature or the word `doctor`, the layer, the commit, the changed files, and the start time. Then it has each program that the helper ran, with its output and exit code, and an `ok:` line for each check that passed. Its last line starts with `passed`, `doctor: ready`, or `FAILED`. A rerun of a command replaces its log.

Hold every proof to these standards:

- Quote the action and the result: the `step` and `voice` lines of a transcript, or the command and its output. A final `ok` alone is not a proof.
- Prefer a test that presses scripted keys through the real menus to a test that calls an internal function.
- Check side effects where the feature has them: the settings written to `SoundRTS.ini`, the data that `--fetch-data` wrote. A harness log does not show a file that a test read back, so name the test that checks the file.
- When a precondition is missing, report the failure and its reason. A missing network, a test that no longer exists, or game data at the wrong commit is a failed drive. Never replace the drive with an easier check and call the feature verified.
- Report a feature as verified only for the entry points you drove. The feature file lists the others.

The harness is a stand-in for the game a player starts. In every report of a harness proof, list what the harness sets up that the real game does not. Those parts stay unverified until an ear pass covers them:

- the speech device, the audio device, the keyboard, the clock, the window, and the clipboard are recorded or scripted ones;
- the settings folder is new and empty for each test, and the language is given by the test;
- the random seed is fixed;
- the network is 127.0.0.1 on a free port, with no router, no discovery of other machines, and no public server list.

The helper's runs of the program are a stand-in for a player's start too. In every report of a program proof, list these differences:

- `%APPDATA%` is an empty scratch folder, and `SOUNDRTS_ROOT` is cleared;
- a script starts the program and gives it streams, so the program never asks a question in a message box;
- `first-start` runs the debug build. Only `release-check` runs the release build.

`--probe` and `--fetch-data` are the program's safe modes. Observed at commit `ad6a74d`, `--probe` creates no file in the settings folder, but it does ask Windows for the audio device and the screen reader. `--fetch-data` downloads about 10 megabytes from github.com.

## Cleanup

```powershell
pwsh -NoProfile -File .claude/skills/verify-soundrts/verify.ps1 cleanup
```

Every helper command waits for the programs it starts and stops a game that outlives its time limit, so no process is left to stop. Never stop `soundrts.exe` by name, because one may be Andrew's own game.

`cleanup` removes the scratch folder in the system's temporary folder and `dist/soundrts-verify`, then lists the evidence, which stays in `target/verify-soundrts/<run>/`. Run `cleanup` after a failed drive too. `cargo clean` deletes the evidence, so copy what a report needs before you run it.

`vendor/` and `target/` stay. Git ignores both.

## Helpers

`verify.ps1`, next to this file, is the only helper. Its commands are `launch`, `doctor`, `test`, `first-start`, `release-check`, and `cleanup`, each shown above. It changes no file that Git tracks.

Two checkouts can run the helper at the same time. In one checkout, run one helper command at a time.

## Keep the map true

When the game changes, the feature map drifts. Use the `maintain-verification-skill` skill to audit this skill and its feature map.
