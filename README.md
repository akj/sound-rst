# sound-rst

A Rust port of [SoundRTS](https://github.com/soundmud/soundrts), the real-time strategy audio game. The game has no graphics to speak of. You hear the map through speech and positional sound, and you play with the keyboard.

The port reads the original game's data files unchanged: rules, maps, sounds, translations, and key bindings. Speech goes through your screen reader (NVDA or JAWS). When no screen reader runs, the system voice speaks.

## What works

Single-player and multiplayer games work.

- The main menu, the options menu (login, mods, soundpacks), and the single-player menus.
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

The game finds its data folder in this order: the `--root` option, the `SOUNDRTS_ROOT` environment variable, the folder of the program, and the current folder.

## Play with others

Choose Multiplayer in the main menu. If your login is still `player`, the game asks for a new one first: the other players hear your login.

To host a game, choose Server. The game starts a server on your computer, says the address to give to the other players, and copies it to the clipboard. Paste the address into a message to the other players. The game asks your router to forward the port (UPnP). If the router does not, players on the internet cannot reach your server, but players on your local network can. The server stops when you leave its menus.

To join a game, choose one of these:

- Choose a server in a list: the servers on your local network, the public list, your favorite servers, and the servers you used recently.
- Enter the IP address of the server: type the address that the host gave you, for example `203.0.113.7` or `203.0.113.7:2502`.

On a server, the menu lists the invitations you received and the open games you can join. To create a game, choose a map and a speed. Only you and the players you invite can join a game that you start with Start a game on. Anyone can join a game that you start with Start a public game on. A player who does not have the map of a game gets it from the server, and the game keeps it in the `downloaded\multi` folder of the settings folder.

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

- `crates/world` is the simulation: rules, maps, units, orders, and computer players. It takes text and a seed, and it never touches a file, a sound device, or a window. The same input gives the same game on every machine.
- `crates/client` is the program: resources, sounds, speech, the voice queue, key bindings, menus, and the in-game interface.
- `crates/protocol` is the messages between a client and a server.
- `crates/server` is the multiplayer server, as a library that the game can start and as the `soundrts-server` program.
- `scripts/fetch-upstream.ps1` fetches the original game.

## Licenses

The code is a port of the SoundRTS source code and keeps its BSD 3-clause license. See [LICENSE.txt](LICENSE.txt).

On Windows the program links [Tolk](https://github.com/dkager/tolk) (LGPL 3) to reach the screen reader. Tolk loads `nvdaControllerClient64.dll` (LGPL 2.1, from NV Access) and `SAAPI64.dll`. The build copies both files next to the program. Keep them there when you copy the program somewhere else.
