# sound-rst

A Rust port of [SoundRTS](https://github.com/soundmud/soundrts), the real-time strategy audio game. The game has no graphics to speak of. You hear the map through speech and positional sound, and you play with the keyboard.

The port reads the original game's data files unchanged: rules, maps, sounds, translations, and key bindings. Speech goes through your screen reader (NVDA or JAWS). When no screen reader runs, the system voice speaks.

## What works

Single-player and multiplayer games work.

- The main menu, the options menu (login, sound and speech, mods, soundpacks), and the single-player menus.
- Games against the computer on every multiplayer map, with the `easy`, `aggressive`, and `ai2` computer players.
- The campaigns, with cut scenes, objectives, and the chapter bookmark.
- Multiplayer: hosting a server from the game, joining one, the lobby, game rooms, chat, and games with other players and computers.
- Every in-game command in `res/ui/bindings.txt` except fullscreen.
- The 13 languages of the original.

The following parts of the original are not ported:

- Games with players of the original game. The port has its own server and protocol.
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

Home and End, or the keypad's plus and minus, raise and lower the volume of the game's sounds by a tenth, in the menus and in a game. The game says the new level at once and remembers it for the next start. A message that the level cuts off is said again after it, and the sounds go on playing.

The speed you choose in the game menu of a single-player game is also the speed of your next single-player game.

### Sound and speech settings

In the options menu, Sound and speech has these items. Each item says its name, then its value. Press Enter to change the value. The game saves the change at once.

- Volume: Enter raises it by a tenth, and from 100 percent goes back to 0. Home and End also change it.
- One item for each category of news: alert, production, economy, scouting, chat, system and story. Enter goes through the four values that the next section describes: spoken, its sound only, kept silently in the log, and dropped.
- NVDA tells the game when it has finished speaking: on or off. See [Speech that ends when NVDA says so](#speech-that-ends-when-nvda-says-so).
- Speech rate: how many characters a second your screen reader says. Enter measures it. Press a key to hear a sentence, then press a key the moment the sentence ends. The game computes your rate from the time between the start of the sentence and your key, and says it. A key that comes too early or too late to be the end of the sentence changes nothing. Escape stops the measure. Measure again whenever you change your speech rate.

### News

While you play, the game tells you news: alerts, production, economy, scouting, chat, system news, and the story. News waits its turn and does not interrupt what you asked for. News is never lost: a message cut off is said again, newer news about the same thing replaces older news you have not heard yet (the latest gold count, an attack on the same square), and every message is kept in a log, heard or not.

Nothing said stops the game. Important messages, such as a player defeated, a new objective or the console's answer, are said before other news waiting, while the game goes on. In single-player, the game waits on purpose while the story is told (a mission's intro, the objective, a cut scene) and while the game menu or the console is open. While the story is told, the game menu, the history and news keys, help and the volume keys work, and any other key skips the message being said. In a game on a server, nothing waits.

| Key | Does |
| --- | --- |
| F4 | What did I miss: says how many messages you have not heard in each category. Press again to hear them one at a time: the newest alert first, then the others, oldest first. |
| Comma | Says the previous message of the category you review. |
| Period | Says the next message of the category you review. |
| Slash | Changes the category you review, and says which. It starts at alerts. |
| Delete | Moves to the square of the last alert and says it. Press again to go to the alert before. |
| F5, F6 | Walk back and forth through every message, as before. |

The options menu's Sound and speech items, or the `[news]` section of `SoundRTS.ini`, say what you hear of each category. Each of `alert`, `production`, `economy`, `scouting`, `chat`, `system` and `story` takes one of these values:

- `speak`: said in its turn, with its sound. This is the default.
- `sound`: only its sound plays, for example the bell of a finished building or the chat sound. The message waits in the log, unheard.
- `log`: kept in the log, unheard, without a sound.
- `drop`: neither said nor kept.

The original game's `verbosity` setting is read once: a category it names is spoken, and production, economy or scouting it leaves out plays only its sound. The game then rewrites the file with a `[news]` section.

### Questions

These keys answer what a sighted player sees at the edge of the screen. Each answer is one sentence, from what you know: nothing you have not seen is counted.

| Key | Says |
| --- | --- |
| B | What your buildings train, research or upgrade to, soonest done first, with the seconds left and where. Production that waits for food comes last. |
| N | How many of your workers are idle, how many gather each resource, and how many build. |
| V | With more than one unit controlled: the hit points of the group against its most, how many units are below half their hit points, and how many of each type. With one unit, as before. |
| M | How the fight on the square goes: stronger, evenly matched or weaker, then the units that can fight on your side and your allies' against the enemies' you see. A side is stronger when its units do at least twice the damage of the other side's, by the rules of their types: upgrades are not counted. |
| Shift+X | Every resource, also those that a map adds. |
| F12 | The other players: each an ally or an enemy, and whether defeated. F11 still says the names only. |

The score is said only at the end of the game: during the game it would tell you about enemy units you have not seen, for example how many units a transport that you destroyed carried.

### Speech that ends when NVDA says so

Most screen readers do not tell a program when they have finished speaking, so the game estimates the end of each message from its length: 0.1 seconds a character at first, the `wait_delay_per_character` setting. At a fast speech rate you hear a silence before each queued message. At a slow rate the next message can cut the current one off. To fit the estimate to your speech rate with any screen reader, JAWS included, measure it with the Speech rate item of Sound and speech in the options menu.

NVDA 2024.1 and later can tell the game when speech has ended, and whether you silenced it. This route is off by default and has not yet been checked by ear. To try it:

1. Fetch NVDA's controller client. The script downloads a pinned release into `vendor/nvda-controller` and checks its hash. The next build copies `nvdaControllerClient.dll` next to the program.

   ```powershell
   pwsh scripts/fetch-nvda-controller.ps1
   ```

2. In the options menu, in Sound and speech, switch "NVDA tells the game when it has finished speaking" on. In `SoundRTS.ini`, that is `nvda_direct = 1` in the `[tts]` section. The change applies the next time you start the game.

With `nvda_direct = 1`, the game speaks through NVDA directly and starts the next message as soon as NVDA has finished the current one. A message that you silence with Control counts as heard. With another screen reader, an older NVDA, or no `nvdaControllerClient.dll`, the game speaks through Tolk as before. To turn the route off, switch the item off, or set `nvda_direct = 0`. Turn it off if messages stop, overlap, or wait too long. The log `tmp/client.log` says which route the game uses.

The game finds its data folder in this order: the `--root` option, the `SOUNDRTS_ROOT` environment variable, the folder of the program, and the current folder.

## Play with others

Choose Multiplayer in the main menu. If your login is still `player`, the game asks for a new one first: the other players hear your login.

To host a game, choose Server. The game starts a server on your computer, says the address to give to the other players, and copies it to the clipboard. Paste the address into a message to the other players. The game asks your router to forward the port (UPnP). If the router does not, players on the internet cannot reach your server, but players on your local network can. The server stops when you leave its menus.

To join a game, choose one of these:

- Choose a server in a list: the servers on your local network, the public list, your favorite servers, and the servers you used recently.
- Enter the IP address of the server: type the address that the host gave you, for example `203.0.113.7` or `203.0.113.7:2502`.

On a server, the menu lists the invitations you received and the open games you can join. To create a game, choose a map and a speed. Only you and the players you invite can join a game that you start with Start a game on. Anyone can join a game that you start with Start a public game on. A player who does not have the map of a game gets it from the server, and the game keeps it in the `downloaded\multi` folder of the settings folder.

A map from a server can bring its own rules, and names and sounds for its own units. It cannot change your keys, the game's own words, sounds or help, or anything a mod you installed defines. The game ignores those parts of the map and notes them in `tmp/client.log`. A map that you put in a `multi` folder yourself keeps every power.

In a game room, the host invites players and computers, sets the alliances and factions, and starts the game. The other players choose their faction and say when they are ready.

| Key | In the menus | In a game |
| --- | --- | --- |
| F7 | Write to the room, or to the lobby outside a room | Write to every player of the game |
| Shift+F7 | | Write to your allies |
| Ctrl+F5 | Read the previous chat message again | Read the previous chat message again |
| Ctrl+F6 | Read the next chat message again | Read the next chat message again |

A chat message plays a short sound, then the game says who wrote it and what they wrote, after the message that is being said. In a game, the game goes on while you type. The multiplayer menu has an option that stops the game from saying when players arrive on the server or leave it.

In a game on a server, every player's computer runs the same game. If one computer computes something different, the game is out of sync: the players whose game differs leave it, and the others play on. Each player's game then saves a record in the `replays` folder of the settings folder, for example `out-of-sync-andrew-seed1234-turn56.jsonl`. The record holds the rules, the map, the players' logins and every order. It replays the game on any computer, without the game data.

To see whether a computer computes the same game, check the record on that computer. The check prints how many turns it replayed, and the first turn that differs if there is one. To compare the records of two players, compare the two files: each turn is one line.

```powershell
cargo run -p soundrts -- --check-record out-of-sync-andrew-seed1234-turn56.jsonl
```

Your favorite and recent servers are in `favorite_servers.txt` and `recent_servers.txt` in the settings folder: one server a line, its address, then optionally a space and its name. To add the server you are on to your favorites, use the item in the server's menu. The public list is [`servers.txt`](servers.txt) in this repository.

## Run a server

You can also run a server on its own, for example on a computer that stays on. The server is a lobby, a chat, and a turn clock. It does not need the game data.

```powershell
cargo run -p soundrts-server -- --port 2501 --name "My server"
```

The server listens on TCP port 2501 for players and answers on UDP port 2501 to players who look for servers on the local network. Add `--upnp` to ask the router to forward the port to players on the internet. Run `cargo run -p soundrts-server -- --help` for every option. Press Ctrl+C to stop the server.

## Listening room

The listening room is a separate program that helps decide how the game's positional sound should work. It plays the game's own sounds through several renderers and asks what you hear. It prints one instruction at a time and reads your answers as typed lines, so your screen reader reads it as it reads any console program. Use headphones.

The renderers are:

- `current`: the game's sound as it is now, left-right placement and loudness.
- `stereo-plus`: the same, with volume changes that cannot click, a duller tone for sounds behind you, and a muffled tone through a closed exit.
- `binaural`: Steam Audio with a model of the head, for headphones.
- `binaural-plus`: `binaural` with plain cues added for behind: a sound behind you is also duller, with the same filter as `stereo-plus`, and a little quieter.

The two binaural renderers are in the program only if you build it with `--features binaural`.

```powershell
cargo run -p listening-room -- --root vendor/soundrts-upstream
```

The script has seven steps:

1. A footstep circling you. You rate how well you could follow it.
2. In front or behind: three footsteps stand in front of you or behind you, and you say which.
3. In front or behind, with a bright sound: the same, with a short burst of noise in place of the footsteps. This step is only for `binaural` and `binaural-plus`.
4. Footsteps walking past: they walk from your right to your left, in front of you or behind you, and you say which.
5. A sound passing overhead.
6. A sound moving away.
7. A neighbour through an open exit and a closed one.

Steps 2, 3 and 4 have 4 trials for each renderer, half in front and half behind. With every renderer, a session has 40 of these trials. The other steps ask for a rating from 1 to 5.

Each step plays the same sounds through each renderer in a random order, and tells you the order at the end of the step. To run only one step, add `--step 4`. To compare only some renderers, add for example `--renderers current,binaural`. Type `q` at any question to stop.

The program saves your answers after each step in `listening-room-results.txt`, in the current folder. For steps 2, 3 and 4, the file records every trial: where the sound was, and what you answered. Send that file back.

To build the binaural renderer, you need Clang: the C++ Clang tools of Visual Studio, or LLVM. If the build cannot find Clang, set `LIBCLANG_PATH` to the folder that holds `libclang.dll`. The first build downloads Steam Audio 4.8.1 (about 180 MB) into `target/steamaudio-cache` and checks its SHA-256.

```powershell
cargo run -p listening-room --features binaural -- --root vendor/soundrts-upstream
```

To try another model of the head, add `--sofa <file>`. The file must be a SOFA file in the SimpleFreeFieldHRIR convention.

With `--render-to <folder>`, the program writes each sound to a WAV file in that folder and plays nothing. The tests use this option. Run the program with `--help` for every option.

### What the first session found

In the first session, `binaural` placed sounds well when they moved, but it got only 1 of 8 trials right in the in-front-or-behind step. Two measurements explain it:

- The game's footstep has almost nothing above 4 kHz. Over headphones, the cues for front and behind are mostly above 4 to 5 kHz, so the head model had little to work with. The bright sound in step 3 has most of its energy above 6 kHz. If `binaural` places the bright sound correctly, the problem is the footstep sound, not the renderer.
- Steam Audio's head model plays sound from behind louder than sound in front below 2 kHz, where the footstep has most of its energy, and mostly quieter above it. So the footstep was about half a decibel louder behind. If you take louder to mean in front, you hear it backwards. `binaural-plus` adds a duller tone and a lower volume behind, so the footstep and the bright sound are quieter behind than in front.

The first session also showed that an echo cannot tell terrains apart, so the listening room no longer has one. Ambient sounds and footsteps for each terrain will do that job when the new sounds exist.

## Test

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

The tests need the original game in `vendor/soundrts-upstream`. They fail with a pointer to the fetch script when it is missing. No test speaks, plays sound, or opens a window. The client tests run on a recorded speech device, a recorded audio device, scripted keys, and a manual clock. The multiplayer tests in `crates/client/tests/net_play.rs` also run real servers, on 127.0.0.1 only, and their clients on real time. No test asks a router to forward a port.

Two sets of tests check the port against recordings:

- `crates/world/tests/traces` checks the simulation. The port plays scripted games from a fixed seed, and the state of every player and unit after every turn must match the recorded hash. [The traces README](crates/world/tests/traces/README.md) explains the cases.
- `crates/client/tests/transcripts` checks the interface. The client runs on scripted keys, and what it says and plays must match the recorded transcript. [The transcripts README](crates/client/tests/transcripts/README.md) describes the model.

The recordings were made from the original Python game. Up to the Git tag `last-python-parity`, the port matched them byte for byte. Since then they are the port's own. A change that alters them shows as a diff of the `expected/` folders. Review that diff.

To record again after an intended change, run the tests with `UPDATE_EXPECTED=1`. The tests write what the port produces to `expected/` and pass. In PowerShell:

```powershell
$env:UPDATE_EXPECTED = '1'
cargo test -p soundrts-world --test traces
cargo test -p soundrts --test transcripts
Remove-Item Env:UPDATE_EXPECTED
```

A change to the traces also needs a new simulation version. The traces README explains how.

These tests show what text the port sends to the speech device. They cannot show what a screen reader does with that text. Test that part by ear.

## Layout

- `crates/world` is the simulation: rules, maps, units, orders, and computer players. It takes text and a seed, and it never touches a file, a sound device, or a window. The same input gives the same game on every machine. A host owns one game: it builds the world from the game's set-up, takes each player's commands, runs the game one turn at a time, and gives each player a view of what that player knows. It also tells each player that knowledge as plain data, made to be sent: the game's map and rules once, then one message a turn, from which a view of the same answers is made without the world. After a seat's first message, what changed since its last one is enough to make the next.
- `crates/client` is the program: resources, sounds, speech, the voice queue, key bindings, menus, and the in-game interface.
- `crates/protocol` is the messages between a client and a server. Its `game` module encodes what a game run by a host sends and receives, in a compact binary form. Nothing sends it yet.
- `crates/server` is the multiplayer server, as a library that the game can start and as the `soundrts-server` program.
- `crates/listening-room` is the listening room.
- `scripts/fetch-upstream.ps1` fetches the original game.
- `scripts/fetch-nvda-controller.ps1` fetches NVDA's controller client, for the `nvda_direct` setting.

## Licenses

The code is a port of the SoundRTS source code and keeps its BSD 3-clause license. See [LICENSE.txt](LICENSE.txt).

On Windows the program links [Tolk](https://github.com/dkager/tolk) (LGPL 3) to reach the screen reader. Tolk loads `nvdaControllerClient64.dll` (LGPL 2.1, from NV Access) and `SAAPI64.dll`. The build copies both files next to the program. Keep them there when you copy the program somewhere else.

For the `nvda_direct` setting, `scripts/fetch-nvda-controller.ps1` fetches NVDA's current controller client, `nvdaControllerClient.dll` (LGPL 2.1, from [NV Access](https://github.com/nvaccess/nvda/tree/master/extras/controllerClient)), with its license, into `vendor/nvda-controller`. The game loads it at run time and does not change it. The build copies the DLL next to the program when it has been fetched. The repository does not contain it.

The listening room built with `--features binaural` links [Steam Audio](https://github.com/ValveSoftware/steam-audio) (Apache 2.0) through `phonon.dll`. The build copies `phonon.dll` next to `listening-room.exe`, with `phonon-THIRDPARTY.md`, which lists the licenses of the code inside the DLL. Keep both files next to the program when you copy it somewhere else.
