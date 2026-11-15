# sound-rst verification map

This folder is the maintained source for verifying what a player of sound-rst can do. Read this index before you drive anything, then use the matching feature file as the recipe.

`verify.ps1` below means `pwsh -NoProfile -File .claude/skills/verify-soundrts/verify.ps1`.

## Baseline preconditions

- `verify.ps1 doctor` ends with `doctor: ready`. If it does not, run `verify.ps1 launch`.
- Run one helper command at a time in a checkout.
- Only Andrew starts a game that speaks. The helper starts the program with `--probe` or `--fetch-data` only.

## Layers of proof

Every sub-feature in the map names the layer that proves it. [SKILL.md](../SKILL.md) defines the three layers. In short:

- `harness` is the client on recorded devices. It proves the text sent to the speech device.
- `program` is the built `soundrts.exe` with `--probe` or `--fetch-data`. It proves that the program starts and finds its data.
- `ears` is Andrew at the keyboard with a screen reader. It alone proves what a player hears.

A sub-feature marked `ears` has no proof below that layer. For a sub-feature marked `harness`, the same behavior through a real screen reader still needs the feature's `By ear` script.

## Driving conventions

- Type every command as written. Test names and scenario names are exact.
- A quoted phrase such as `"options menu"` is the text of a `voice` line in a transcript or of an assertion in a test. It is what the client sends to the speech device.
- Each harness test starts from a new, empty settings folder and ends by itself.
- Hand a `By ear` script to Andrew as numbered steps, adjusted to the change you made. Wait for pass or fail on each step, and record the answer where the proof is reported.

## Proof and skip reporting

- Quote the `step` and `voice` lines, or the command and its output, with the layer and the path of the evidence file.
- Name the feature, the sub-feature, and the entry point with every proof.
- Report a failed or unreachable drive with the command, its last lines, and the precondition that was missing.
- A sub-feature you did not drive is not verified. Do not report it as verified through another sub-feature or a lower layer.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph that describes what the player can do. It then has exactly four H2 sections in this order.

1. `Sub-features` lists short IDs, each with its layer and one line for the behavior.
2. `How to get to it (user POV)` lists every entry point a player has.
3. `Driving it with verify.ps1` starts with `Preconditions:` and uses labeled bullets. Each bullet pairs what the player does with an exact command and the result that proves it. The section ends with `By ear`, a numbered script for Andrew.
4. `Gotchas` lists what can waste or invalidate a run, and what no layer here proves.

Keep implementation details out of the map. Name only what the player does, the commands, the required state, and the proof.

## Features

- [Menus](./menus.md) covers the main menu, the options menu, and the sound and speech settings.
- [Single-player games](./single-player.md) covers a game against the computer, the in-game commands, and the campaign.
- [Multiplayer](./multiplayer.md) covers hosting, joining, the lobby and rooms, chat, a game on a server, returning after a lost connection, and watching.
- [First start](./first-start.md) covers finding the game data, fetching it, the probe, and the release folder.
- [Speech and sound](./speech-and-sound.md) covers what reaches the screen reader and the audio device. Most of it has proof by ear only.

The map does not yet cover these parts of the README: the standalone `soundrts-server` program, the listening room, mods and soundpacks beyond their menus, the languages other than English and French, and the installer.
