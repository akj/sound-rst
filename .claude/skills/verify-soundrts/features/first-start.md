# First start

A downloaded build of the game does not hold the original game's data. The game finds its data folder, or fetches the data once into `%APPDATA%\SoundRTS\data`, and then starts. `soundrts --probe` prints what the game found without starting it.

## Sub-features

- `first-start-missing` (program): with no data anywhere, a start from a terminal or a script stops, says where it looked, and says to run `soundrts --fetch-data`.
- `first-start-fetch` (program): `soundrts --fetch-data` downloads the data of the pinned commit into a folder named after that commit in `%APPDATA%\SoundRTS\data`, and says where it is. A second run downloads nothing.
- `first-start-probe` (program): `soundrts --probe` prints the data folder, the settings folder, the language, the audio device, and the screen reader, and exits.
- `first-start-release` (program): the release folder that `scripts/stage-release.ps1` makes does the same, the fetched data has the same bytes as `vendor/soundrts-upstream`, and `soundrts-server.exe` answers `--help`.
- `first-start-installer` (GitHub only): the installer installs, the installed game finds the data, and the uninstaller removes the program and leaves the data. The Release workflow checks this on a GitHub runner.
- `first-start-ask` (ears): a game started from the desktop with no data asks in a message box whether to download it, says how far the download has got, and then starts.

## How to get to it (user POV)

- Start `soundrts.exe` from the desktop, the Start menu, or File Explorer.
- Run `soundrts` in a terminal, with or without `--root <folder>` or the `SOUNDRTS_ROOT` environment variable.
- Run `soundrts --fetch-data` in a terminal.
- Run `soundrts --probe` in a terminal.
- Run the installer from a release.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` ends with `doctor: ready`.
- The machine reaches github.com. Without a network, `first-start` fails at the fetch, and that failure is the result to report.

- **Start with no data, fetch, and start again.** A player with an empty `%APPDATA%` runs the probe, fetches the data, and runs the probe again. Run `verify.ps1 first-start`. The helper builds the debug program and runs it four times with `%APPDATA%` set to an empty scratch folder. The log shows `--probe` ending with exit code 1 and `To download it, run: soundrts --fetch-data`, then `--fetch-data` ending with `The game data is ready in`, then `--probe` with a `game root:` line inside the scratch folder, then a second `--fetch-data` answering `The game data is already in`. The last line is `passed on the built program: first-start`.
- **Check a release folder.** Run `verify.ps1 release-check`. The helper fetches NVDA's controller client into `vendor/nvda-controller`, builds the release programs, stages them in `dist/soundrts-verify`, and runs `scripts/check-release.ps1` on that folder. The log ends with `The fetched data is the checkout's`, the number of files, `The server answers --help.`, and `passed on the built program: the release folder`.

### By ear

For Andrew, with NVDA running. The script needs a release folder, for example an unzipped download, and no data in `%APPDATA%\SoundRTS\data`.

1. If `%APPDATA%\SoundRTS\data` exists, rename it to `data-kept`.
2. In File Explorer, press Enter on `soundrts.exe` in the release folder. NVDA reads a message box that starts with "SoundRTS needs the data of the original game", with the buttons Yes and No.
3. Choose No. The game closes, and `%APPDATA%\SoundRTS\data` does not exist.
4. Start `soundrts.exe` again and choose Yes. You hear "Downloading the game data.", perhaps a number of megabytes, then "The game data is ready.", then the main menu's title.
5. Press Escape to quit. Delete the new `data` folder and rename `data-kept` back to `data`.

## Gotchas

- The helper never touches the real `%APPDATA%`. It gives the program a scratch folder in the system's temporary folder and empties that folder when the command ends. `verify.ps1 cleanup` removes the folder itself.
- The game looks for its data in this order: `--root`, `SOUNDRTS_ROOT`, the folder of the program, the current folder, then the downloaded data. The helper clears `SOUNDRTS_ROOT` for the program and runs it in the scratch folder, so only the downloaded data can be found.
- The question in a message box appears only when the game has no standard stream and no launching terminal. A game that a script starts never asks, so no script can prove `first-start-ask`.
- Run the installer check only on GitHub. `scripts/check-release.ps1 -Installer` installs the game for the current user and refuses to run where the game is installed.
- `first-start` proves the debug build. Only `release-check` proves the release build that players get.
- After `release-check`, later builds copy `nvdaControllerClient.dll` from `vendor/nvda-controller` next to the program. The probe's `nvda_direct` line changes accordingly.
