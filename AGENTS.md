# sound-rst

sound-rst is a Rust port of [SoundRTS](https://github.com/soundmud/soundrts), a real-time strategy game that you play by ear. The game draws nothing. The player hears the map through speech and positional sound, and plays with the keyboard. Speech goes through the player's screen reader, NVDA or JAWS, or through the system voice when no screen reader runs.

## What the game must keep

Players rely on the four things below. Do not trade one of them for a feature.

### 1. Speech and sound are the whole interface

A blind player has no screen to fall back on. Treat a message that comes late, is cut off, or is lost as you would treat a broken screen. Nothing the game says stops the game. Single-player waits on purpose while a story is told, and the menu, history, and news keys still work. News is never lost. A message that is cut off is said again. Each answer to a question is one sentence.

### 2. A player hears only what that player knows

The fog of war applies to everything the game says and plays. The interface answers from what its player was told, never from the whole world. An answer counts nothing that the player has not seen.

### 3. The original's files, as they are

The port reads the original's rules, maps, sounds, translations, and key bindings unchanged. Rules, maps, and computer players are text files, so a modder needs only a text editor. The original's keys keep their meaning. The port adds commands only on keys that the original leaves free.

### 4. The same input gives the same game

The simulation takes text and a seed, and it reads no file, clock, sound device, or window. The traces check the result turn by turn, on Windows and on Linux.

## How to read the rest

The rest of this file helps you find your way and change the game without breaking it. Treat it as good defaults. The developer's preferences override it. If a rule here conflicts with your task, say so and get the developer's answer before you break the rule.

## A small glossary

- **the developer** means the person you work for.
- **the player** means a person who plays the game.
- **the original** means the Python SoundRTS at the commit in `scripts/upstream-commit.txt`. It supplies the game data, and its source is the reference for the port.
- **said** means the game's voice started the message, or a recorded speech device received its text. A test can show that.
- **heard** means a person listened through a screen reader and reported the result. No test can show that.
- **recording** means a file in an `expected/` folder. A _trace_ records the simulation, and a _transcript_ records what the interface says and plays.
- **seat** means one player's or one watcher's place in a game.

## Walk every way to play

Most features reach several of the places below. Before you call a change to the game done, walk this list and say which entries applied:

- **Single-player and a game on a server.** In single-player the client runs the game, and the game waits while a story is told or while the game menu or the console is open. On a server the server runs the game, and there is no console and no cheats. A story or an open menu does not stop a game on a server. Only the organizer's pause does.
- **Seats.** A player, a watcher, and a player who returns after a lost connection. A watcher hears one player's side and gives no orders.
- **Input from strangers.** A map, a network message, and a command can come from someone hostile. Refuse a bad one. It must never crash the game or the server. Add the case to the `hostile` tests of the crate that reads the input.
- **Keys.** A new command takes a key that the original's `res/ui/bindings.txt` leaves free, and that neither Windows nor a screen reader takes.
- **The README.** The README is the player's manual. When a change alters what a key does or what the player hears, change the sentence that describes it.

## Verifying

- A new worktree has no `vendor/`. Run `pwsh scripts/fetch-upstream.ps1` once. The tests read the original's data from there. In a worktree the script copies the files from the main checkout's clone, so the worktree holds no nested Git repository. Do not clone the original into a worktree by hand.
- Before you push, run what CI checks:

  ```powershell
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo check -p soundrts --lib --bins
  cargo test --workspace
  ```

  The third command builds the interface without the simulation's `truth` feature. It fails when the interface reads the whole world.
- While you iterate, run one test, for example `cargo test -p soundrts --test transcripts menus_main`. The package of `crates/client` is `soundrts`. The other packages are `soundrts-world`, `soundrts-protocol`, `soundrts-server`, and `listening-room`.
- No test speaks, plays a sound, or opens a window. The client tests run on a recorded speech device, a recorded audio device, scripted keys, and a manual clock. The multiplayer tests run real servers on 127.0.0.1 only.
- CI runs every test in its own process, several at a time. A test must pass alone and beside any other test. CI also runs the tests of `soundrts-world`, `soundrts-protocol`, and `soundrts-server` on Linux. The client needs Windows.
- A change to what the game says ships with a test that expects the literal words and fails without the change.
- A change that is meant to keep the behaviour leaves `expected/` as it is. If only the way the simulation stores its state changed, only the `*.digest.txt` files may change.
- To prove that a feature works, follow `.claude/skills/verify-soundrts/SKILL.md`. It gives the commands for each feature, and it says what each proof covers: the tests, the built program, or a listener.

## Said is not heard

The tests show what the game said: each message that the voice started, in order, and each sound that the game started, on a clock that the test moves. A transcript records a message when the voice starts it. A later key can cut that message before all of its parts reach the speech device. The tests cannot show what a screen reader does with the text. No test shows how NVDA or JAWS pronounces a message, whether speech is cut off or late at a real speech rate, whether a key reaches the game while a screen reader runs, or what a sound is like to the ear.

- Claim the layer that you ran. Write "fixed in the recorded speech tests, not yet heard through NVDA". From the tests alone, never write that a change works with NVDA, sounds right, or was checked by ear. Only a listener's report supports those words, so name the listener.
- A test on the audio output's samples shows the arithmetic of a sound. It does not show what a person hears.
- The multiplayer tests show a real server on 127.0.0.1. They do not show a real network.
- When a change alters what the player hears, put a short numbered listening script in the pull request. Each step is one action and the exact words that the listener should hear. Until a listener reports pass or fail on each step, the change is built and not yet heard. Do not call it done. Record the listener's report in the pull request.

## Pull requests

- Write the title as one plain sentence that says what the player gets, for example `Get back into a game on a server after your connection drops, instead of losing it`.
- In the body, give the problem as the player meets it, then the change, then the proof. The proof names the tests and commands that you ran, and says what nobody has heard yet. Put the listening script last.

## How it works

The _simulation_ (`crates/world`) runs the game in _turns_. A _host_ owns one game. It takes each seat's _commands_, runs one turn, and gives each seat a _message_ of what that seat's player knows. In single-player the client runs the host. In a game on a server, the server (`crates/server`) runs the host and sends each seat its message in the binary form of `crates/protocol`. The _interface_ (`crates/client`) answers every key from those messages. What the interface says goes through the _voice_, which queues news by category, to the speech device. On Windows the speech device is Tolk by default. Tolk reaches NVDA and JAWS, and falls back to the system voice. With the `nvda_direct` setting, speech goes to NVDA's own controller client first.

## Where to read more

- `README.md` is the player's manual. Its Set up, Test, Release, and Layout sections are for you. Layout says what each crate and script is.
- Read `crates/world/tests/traces/README.md` before you change the simulation. It says how to find the first turn that differs, and how to record again.
- Read `crates/client/tests/transcripts/README.md` before you change what the client says, plays, or waits for. It defines the test's clock, speech, keys, and scenario format. It also lists inherited behaviour that looks like a bug and is recorded on purpose.
- `docs/open-findings.md` lists the known problems and the status of each. Search it before you report or fix a problem.
- `docs/refactor-design.md` is the design. Section 13 has the decisions, and section 14 says what to leave alone.

## Taste

- The simulation reads no file, clock, sound device, or window. No check enforces this rule yet, so keep to it without one. Outside data arrives as text, and time arrives as turns.
- Put what you know into types. Use an id that is resolved when the rules load, an enum with one variant per kind, and `Option` for absence. Avoid a name compared as text, a group of flags, and a sentinel value.
- One thing has one name in what the player hears. Use the original's word where the original has one, as the game does with "organizer".
