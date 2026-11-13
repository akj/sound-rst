# sound-rst

A Rust port of [SoundRTS](https://github.com/soundmud/soundrts), the real-time strategy audio game. The game has no graphics to speak of. You hear the map through speech and positional sound, and you play with the keyboard.

The port reads the original game's data files unchanged: rules, maps, sounds, translations, and key bindings. Speech goes through your screen reader (NVDA or JAWS). When no screen reader runs, the system voice speaks.

## What works

Single-player games work.

- The main menu, the options menu (login, mods, soundpacks), and the single-player menus.
- Games against the computer on every multiplayer map, with the `easy`, `aggressive`, and `ai2` computer players.
- The campaigns, with cut scenes, objectives, and the chapter bookmark.
- Every in-game command in `res/ui/bindings.txt` except chat and fullscreen.
- The 13 languages of the original.

The following parts of the original are not ported:

- Multiplayer, the server, and chat.
- Saved games and replays.
- The map editor.
- The on-screen map. The window stays blank.
- The Documentation menu item, the update check, and the statistics upload.

## Set up

You need Windows with the MSVC build tools, [rustup](https://rustup.rs), Git, and PowerShell 7. `rust-toolchain.toml` pins the Rust version, and rustup installs that version on the first build.

1. Fetch the original game. The script checks out a pinned commit into `vendor/soundrts-upstream`.

   ```powershell
   pwsh scripts/fetch-upstream.ps1
   ```

2. Build and run the game.

   ```powershell
   cargo run -p soundrts -- --root vendor/soundrts-upstream
   ```

The game data stays outside this repository because the original does not state a license for it. `vendor/` is ignored by Git.

To check what the game found on your machine without starting it, run the probe. The probe prints the data folder, the settings folder, the language, the audio device, and the screen reader, and then exits. It does not speak or open a window.

```powershell
cargo run -p soundrts -- --root vendor/soundrts-upstream --probe
```

## Play

The keys are those of the original. `vendor/soundrts-upstream/res/ui/bindings.txt` lists every key. To change keys, put your own `bindings.txt` in the settings folder.

The settings folder is `user` inside the data folder if that folder exists. Otherwise the settings folder is `%APPDATA%\SoundRTS`. It holds `SoundRTS.ini`, the campaign bookmarks, and the log `tmp/client.log`.

To get a written record of everything the game says, set `debug_mode = 1` in `SoundRTS.ini`. The log then has one `SAID:` line for each message.

To play in another language, write its code, for example `fr`, in `cfg/language.txt` in the data folder.

The game finds its data folder in this order: the `--root` option, the `SOUNDRTS_ROOT` environment variable, the folder of the program, and the current folder.

## Test

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The tests need the original game in `vendor/soundrts-upstream`. They fail with a pointer to the fetch script when it is missing. No test speaks, plays sound, or opens a window. The client tests run on a recorded speech device, a recorded audio device, scripted keys, and a manual clock.

Two sets of tests compare the port with the original Python game:

- `crates/world/tests/parity` compares the simulation. The original and the port play the same scripted games from the same seed, and the state of every player and unit must match after every turn. [The simulation parity README](crates/world/tests/parity/README.md) explains how to regenerate the reference.
- `crates/client/tests/parity` compares the interface. A driver runs the original client without a screen reader, a sound device, or a window, presses scripted keys, and records what the client says and plays. The port replays the same scripts and must produce the same transcript. [The interface parity README](crates/client/tests/parity/README.md) defines the model.

These tests show that the port sends the same text to the speech device as the original. They cannot show what a screen reader does with that text. Test that part by ear.

## Layout

- `crates/world` is the simulation: rules, maps, units, orders, and computer players. It takes text and a seed, and it never touches a file, a sound device, or a window. The same input gives the same game on every machine.
- `crates/client` is the program: resources, sounds, speech, the voice queue, key bindings, menus, and the in-game interface.
- `scripts/fetch-upstream.ps1` fetches the original game.

## Licenses

The code is a port of the SoundRTS source code and keeps its BSD 3-clause license. See [LICENSE.txt](LICENSE.txt).

On Windows the program links [Tolk](https://github.com/dkager/tolk) (LGPL 3) to reach the screen reader. Tolk loads `nvdaControllerClient64.dll` (LGPL 2.1, from NV Access) and `SAAPI64.dll`. The build copies both files next to the program. Keep them there when you copy the program somewhere else.
