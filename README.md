# sound-rst

A Rust port of [SoundRTS](https://github.com/soundmud/soundrts), the real-time strategy audio game. The game has no graphics to speak of. You hear the map through speech and positional sound, and you play with the keyboard.

The port imports the original game's data: rules, maps, sounds, translations, and key bindings. The import keeps what each file means, and gives every phrase and every sound a name in place of the original's number. Speech goes through your screen reader (NVDA or JAWS). When no screen reader runs, the system voice speaks.

## What works

Single-player and multiplayer games work.

- The main menu, the options menu (login, sound and speech, mods, soundpacks), and the single-player menus.
- Games against the computer on every multiplayer map, with the quiet computer (`easy`), the aggressive computer, and the random computer (`ai2`), each quiet or aggressive computer at a level from 1 to 3, and an income for each player. See [Levels and income](#levels-and-income). The random computer gives random orders, has its resources set to 1000 of each every time it plays, and ignores requirements; the game names it "random computer with unlimited resources and no requirements" wherever you choose or hear it.
- The campaigns, with cut scenes, objectives, the chapter bookmark, and the level of their computers.
- Saving and loading a single-player game. See [Saved games](#saved-games).
- Listening to a recorded game, yours or someone else's. See [Listen to a game](#listen-to-a-game).
- Multiplayer: hosting a server from the game, joining one, the lobby, game rooms, chat, and games with other players and computers.
- Every in-game command in `res/ui/bindings.txt` except fullscreen.
- The 13 languages of the original.

The following parts of the original are not ported:

- Games with players of the original game. The port has its own server and protocol.
- The map editor.
- The on-screen map. The window stays blank.
- The Documentation menu item, the update check, and the statistics upload.

## Set up

You need Windows with the MSVC build tools, [rustup](https://rustup.rs), Git, and PowerShell 7. `rust-toolchain.toml` pins the Rust version, and rustup installs that version on the first build.

1. Fetch the original game and import it. The script checks out a pinned commit into `vendor/soundrts-upstream`, and imports its data into `vendor/game-data`, which is what the game reads. In a Git worktree, the script keeps its one clone in the main checkout's `vendor/soundrts-upstream` and puts only the files of the commit, without a `.git` folder, into the worktree's `vendor/soundrts-upstream`. Then `git worktree remove` has no nested repository to delete.

   ```powershell
   pwsh scripts/fetch-upstream.ps1
   ```

2. Build and run the game.

   ```powershell
   cargo run -p soundrts -- --root vendor/game-data
   ```

The game data stays outside this repository because the original does not state a license for it. `vendor/` is ignored by Git.

To check what the game found on your machine without starting it, run the probe. The probe prints the data folder, the settings folder, the language, the audio device, and the screen reader, and then exits. It does not speak or open a window.

```powershell
cargo run -p soundrts -- --root vendor/game-data --probe
```

## The first start

A downloaded build of the game does not contain the data of the original game: its rules, maps, sounds and translations. The original game does not state a license for that data, so we do not share it.

The first time you start the game from the desktop, the Start menu or File Explorer, it asks whether to download the data. Choose Yes. The game downloads about 10 megabytes from the original game's repository on GitHub and imports them. It says how far it has got, and then it starts. If you choose No, the game closes. The data goes in `%APPDATA%\SoundRTS\data`, and the next start uses it.

To download the data without starting the game, run this in a terminal:

```powershell
soundrts --fetch-data
```

It prints where the data is and exits. If the game already has its data, it downloads nothing. A game started from a terminal or a script never asks. It tells you to run `soundrts --fetch-data`.

## Play

The keys are those of the original. `vendor/game-data/res/ui/bindings.txt` lists every key. To change keys, put your own `bindings.txt` in the settings folder.

### Key help

To hear what a key does without doing it, press Control F1 in a game. The game says "key help on". Each key or chord you press then says its name and what it does, for example "Control Tab: next useful target", and does nothing else. A key with no command says "no command". A modifier pressed alone, such as Left Control, is described when you release it, because it acts then. Escape or Control F1 turns key help off. The game goes on while key help is on, as it does on a server.

The settings folder is `user` inside the data folder if that folder exists. Otherwise the settings folder is `%APPDATA%\SoundRTS`. It holds your settings in `settings.toml`, the campaign bookmarks and levels, and the log `tmp/client.log`.

`settings.toml` is a text file. The options menu writes it, and you can edit it in a text editor while the game is closed. A setting that the file leaves out takes its default. Settings that mean yes or no take `true` or `false`. If the game cannot read the file, it uses the defaults, keeps your file as `settings.toml.old`, writes a new `settings.toml`, and names the line of the first error in the log. The `[parameters]` table holds the tuning values of the original's `cfg/parameters.toml`, such as the volume of each sound. A sound is named there as in the game's files: `"ambience/path" = 0.5`. The game does not read the original's `SoundRTS.ini`, `cfg/parameters.toml` or `cfg/language.txt`.

To get a written record of everything the game says, set `debug_mode = true` in `settings.toml`. The log then has one `SAID:` line for each message.

To play in another language, write its code in `settings.toml`, for example `language = "fr"`. Without it, the game uses the language of Windows.

Home and End, or the keypad's plus and minus, raise and lower the volume of the game's sounds by a tenth, in the menus and in a game. The game says the new level at once and remembers it for the next start. A message that the level cuts off is said again after it, and the sounds go on playing.

In a text field, such as your login, a server's address, a chat message or the console, Control V pastes the text of the clipboard. The field keeps only the characters it allows, drops line breaks, and holds at most 400 characters. It says what it pasted, then the whole line if the line already had text. Enter still sends the line.

The speed you choose in the game menu of a single-player game is also the speed of your next single-player game.

The map lists remember the map you chose last and open on it: press Down to hear it. A letter goes to the next item that starts with it, from where you are, and wraps around.

### Levels and income

In the players menu of a training game, the invite items add a quiet computer, an aggressive computer or the random computer. The game names a quiet or an aggressive computer with its level, for example "quiet computer, level 3", in the menus and in the game. A computer starts at level 3, which plays as the original's computers do. Levels 1 and 2 are weaker. The menu has a choice for each other level, for example "set quiet computer, level 3, to level 2". The random computer has no level.

Every player but the random computer also has an income: the share of what the player's workers gather that the player receives. It is 100 percent unless you change it to 50, 75, 150 or 200 percent, for example with "set player to 150 percent income". The menu then names the player with the income: "player, 150 percent income". An income evens out a game between players of different strength. The random computer has no income to set, because its resources are set to 1000 of each anyway.

In a campaign's menu, after the chapters, a choice for each other level sets the level of the computers of its missions, for example "set the computers, level 3, to level 2". The menu says the new level at once. The game keeps the level beside the campaign's bookmark, and every mission that you start from then on uses it. A saved mission keeps the level that it was played at.

### Saved games

In a training game or a campaign mission, "save game" in the game menu saves the game and says "saved". The game goes on.

When you quit a game that is not over, or close the window during one, the game saves it by itself as the last game. There is one last game for training and one for missions, and each new one replaces the old one. Closing the window by accident loses nothing.

To load a game, choose "load a game" in the single-player menu. The item is there once you have a save. The list starts with the newest save. For each save it says the map or mission, the game time, and how long ago you saved it. The last game starts with "last game". Delete removes the save you are on after you confirm.

Loading plays the game again from its start, silently. You hear "loading", then every few seconds how far it has got. An hour of play takes a few seconds on a fast computer. Escape stops loading. When the game is ready, you hear "loaded" and the square under the cursor. The game goes on from where you saved it, with the same cursor and the same units selected. The game does not keep the log of messages, the news you have heard, the zoom, or an order you were choosing.

A save loads only in the version of the game that made it. After an update, an old save says "cannot load, made by another version of the game".

A mission's save is separate from the campaign's bookmark. Winning a loaded mission unlocks the next chapter, as winning it from the campaign menu does.

The saves are in the `saves` folder of the settings folder.

### Listen to a game

You can listen to a game again, yours or someone else's, to learn from it or to settle what happened. Choose "listen to a game" in the single-player menu. The list has your saves and the games of the `records` folder of the settings folder, newest first:

- "last finished game": the game you played last to its end, won, lost or left. There is one for training and one for missions, and each new one replaces the old one.
- The games you kept with "keep this game's record".
- Any record you put in the `records` folder: a save someone sent you, or a record that a server wrote (see [Run a server](#run-a-server)). A record without a save's title is said by its file name and how long ago it was written.

You listen as a watcher watches a game: from the side of one player at a time, as that player heard it. You hear "watching" and the player's name, then the game from its start. Every question key, the cursor, the log and the news work as in a game. You give no orders: a key that gives an order says "watching: you give no orders", once. Control F12 follows the next player still in the game, and Control Shift F12 the previous one. You can also follow a computer: you hear its units, what it sees and its resources, but not the events of a turn, such as an attack or a unit ready, because the game tells a computer none. When the player you follow leaves the game, you follow the next person still in it. When nobody is left, you hear the result and the score of the player you followed.

The game plays at normal speed, as it was played, and you set the pace:

| Key | Does |
| --- | --- |
| Equals | Faster: fast 2, 4 and 8, then fastest. At the fastest pace, speech and sounds stop, and you hear only how far the game has got, as when loading. |
| Minus | Slower, down to slow. |
| Shift Space | Pause, or resume. While paused, the keys still answer. |
| Right bracket | Go forward a minute of game time. With Shift, five minutes. |
| Left bracket | Go back a minute. With Shift, five minutes. |
| F3 | Where you are, of the whole game, the pace, and whether it is paused: "3 minutes 20 seconds of 12 minutes 0 seconds , speed normal". |

A jump forward plays the game again silently and fast; a jump back plays it again from its start. Then you hear where you are. Nothing of the stretch you skipped is said: the news that waited is kept in the log, unheard. An hour of play takes a few seconds at most on a fast computer. The game menu has the same choices, for the pace, the pause and the jumps, and "keep this game's record", which copies the game you listen to into the `records` folder, so that the next game does not replace it.

When the game ends, you hear "end of the recording", then the result and the score of the player you follow. A player still in the game when the recording ends has a game "Not over". If a part of the file is damaged, you hear "the recording is damaged: it stops here". The game stays where it stopped, and you can still review what came before, go back, or leave from the game menu. A game made by another version of the game is refused, as loading refuses it.

### Sound and speech settings

In the options menu, Sound and speech has these items. Each item says its name, then its value. Press Enter to change the value. The game saves the change at once.

- Volume: Enter raises it by a tenth, and from 100 percent goes back to 0. Home and End also change it.
- One item for each category of news: alert, production, economy, scouting, chat, system and story. Enter goes through the four values that the next section describes: spoken, its sound only, kept silently in the log, and dropped.
- NVDA tells the game when it has finished speaking: on or off. See [Speech that ends when NVDA says so](#speech-that-ends-when-nvda-says-so).
- Speech rate: how many characters a second your screen reader says. Enter measures it. Press a key to hear a sentence, then press a key the moment the sentence ends. The game computes your rate from the time between the start of the sentence and your key, and says it. A key that comes too early or too late to be the end of the sentence changes nothing. Escape stops the measure. Measure again whenever you change your speech rate.

### News

While you play, the game tells you news: alerts, production, economy, scouting, chat, system news, and the story. News waits its turn and does not interrupt what you asked for. News is never lost: a message cut off is said again, newer news about the same thing replaces older news you have not heard yet (the latest gold count, an attack on the same square), and every message is kept in a log, heard or not.

Nothing said stops the game. Important messages, such as a player defeated, a new objective or the console's answer, are said before other news waiting, while the game goes on. In single-player, the game waits on purpose while the story is told (a mission's intro, the objective, a cut scene) and while the game menu or the console is open. While the story is told, the game menu, the history and news keys, the copy key, help and the volume keys work. Escape skips the rest of the story, and any other key skips the message being said. The lines you skip are not said at all, even when other news waits behind the story, and they stay in the story category of the log, for Comma and Period to review. A campaign's cut scene between missions works the same way: any key skips a line, and Escape skips the rest. In a game on a server, nothing waits.

| Key | Does |
| --- | --- |
| F4 | What did I miss: says how many messages you have not heard in each category. Press again to hear them one at a time: the newest alert first, then the others, oldest first. |
| Comma | Says the previous message of the category you review. |
| Period | Says the next message of the category you review. |
| Slash | Changes the category you review, and says which. It starts at alerts. |
| Control Shift C | Copies the message you review to the clipboard, and says "copied". If you are not reviewing, it copies the last message said. Another key's answer, or an important message, ends a review. |
| Delete | Moves to the square of the last alert and says it. Press again to go to the alert before. |
| F5, F6 | Walk back and forth through every message, as before. |

The options menu's Sound and speech items, or the `[news]` table of `settings.toml`, say what you hear of each category. Each of `alert`, `production`, `economy`, `scouting`, `chat`, `system` and `story` takes one of these values, in quotes, for example `chat = "log"`:

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
| Shift+V | How the first unit you control fights, with the upgrades that apply to it: its damage and how often it strikes, its range and armor, then its most hit points and its speed, for example "footman , damage 6 every 1.5 seconds , range 1 , armor 1.5 . 15 hit points , speed 1.5". |
| M | How the fight on the square goes: stronger, evenly matched or weaker, then the units that can fight on your side and your allies' against the enemies' you see. A side is stronger when its units do at least twice the damage of the other side's, by the rules of their types: upgrades are not counted. |
| Shift+X | Every resource, also those that a map adds. |
| F12 | The other players: each an ally or an enemy, and whether defeated. |
| F11 | The players' names, as before. When the rules have more than one faction, as with the orc mod, each name is followed by the player's faction. |
| Shift+F11 | Your faction, also the one a random choice gave you. |
| F9, Shift+F9 | The next or the previous objective, one at a time: its description, whether it is done, and its place, such as "build 1 farm , done , 1 of 2". The map's own objective comes first, without whether it is done. After the last objective, F9 goes back to the first. |

The score is said only at the end of the game: during the game it would tell you about enemy units you have not seen, for example how many units a transport that you destroyed carried.

### Idle soldiers

The keys F to Semicolon select every soldier of a type on the square, as in the original: F the first type, such as footmen, G the second, such as archers, and so on. With Shift, they select only the idle ones, those without orders, on the square. With Control and Shift, they select the idle ones all over the map.

### Speech that ends when NVDA says so

Most screen readers do not tell a program when they have finished speaking, so the game estimates the end of each message from its length: 0.1 seconds a character at first, the `wait_delay_per_character` setting. At a fast speech rate you hear a silence before each queued message. At a slow rate the next message can cut the current one off. To fit the estimate to your speech rate with any screen reader, JAWS included, measure it with the Speech rate item of Sound and speech in the options menu.

NVDA 2024.1 and later can tell the game when speech has ended, and whether you silenced it. This route is off by default and has not yet been checked by ear. To try it:

1. Fetch NVDA's controller client. The script downloads a pinned release into `vendor/nvda-controller` and checks its hash. The next build copies `nvdaControllerClient.dll` next to the program.

   ```powershell
   pwsh scripts/fetch-nvda-controller.ps1
   ```

2. In the options menu, in Sound and speech, switch "NVDA tells the game when it has finished speaking" on. In `settings.toml`, that is `nvda_direct = true`. The change applies the next time you start the game.

With `nvda_direct = true`, the game speaks through NVDA directly and starts the next message as soon as NVDA has finished the current one. A message that you silence with Control counts as heard. With another screen reader, an older NVDA, or no `nvdaControllerClient.dll`, the game speaks through Tolk as before. To turn the route off, switch the item off, or set `nvda_direct = false`. Turn it off if messages stop, overlap, or wait too long. The log `tmp/client.log` says which route the game uses.

The game finds its data folder in this order: the `--root` option, the `SOUNDRTS_ROOT` environment variable, the folder of the program, the current folder, and the downloaded data in `%APPDATA%\SoundRTS\data`. A folder given with `--root` or `SOUNDRTS_ROOT` must hold the data, and the game then looks nowhere else. The data is imported data: the folder holds `res/rules.txt` and the file `import.txt`, which names the import that made it. The game does not read the folder of the original game. It says so, and names the command that imports it.

## The game's files

The game reads its rules, maps, words and sounds from text files and recordings. A text editor and a file manager are enough to change them. A package is a folder or a zip file: the game itself (`res`), a mod (`mods/<name>`), a campaign (`single/<name>`), or a map with files of its own (a folder or a zip file in `multi`). A package can hold these files:

- `rules.txt` and `ai.txt` are the rules and the computer players.
- `ui/style.txt` gives each type its name, its sounds, and its place in the menus. `ui/bindings.txt` holds the keys. `ui/game_help.txt` and `ui/menu_help.txt` hold the help.
- `text/en.txt` holds the English words, and `text/fr.txt` the French ones, with one file for each language. A line is `name = words`. A line that starts with `#` is a comment.
- `sounds/` holds the recordings, as `.ogg` files in folders. The name of a sound is its path without `.ogg`: `footstep/grass_1` is the file `sounds/footstep/grass_1.ogg`.
- `lang/<language>/` holds what differs in one language: its own `ui/style.txt`, a help file, or recordings under `sounds/`.

A style, a map and a help file name a phrase by the name of its line, `cannot_build_there`, and a sound by its path, `footstep/grass_1`. A whole number in the same place is a count, which the game reads aloud. In a map, the names are the words of the `title`, `objective` and `intro` lines and of the `add_objective` and `cut_scene` actions.

The game checks the names when it loads the files:

- A word of a style, a help file or a map that names no phrase and no sound says nothing. The log `tmp/client.log` gives the file, the line and the word.
- An attribute that the game does not have in a style is ignored, with its file and line in the log.
- A language file with a line that is not `name = words` is refused whole, with the line.
- Rules that name a unit or give a type a property that does not exist are refused whole, with the file and the line.

Each package's names are its own. A mod that you install can give other words, or another recording, for a name of the game. A map that came from a server can only add names: see [Play with others](#play-with-others).

### Import a mod or a map of the original

The original SoundRTS names every phrase and every sound with a number. To use one of its mods, campaigns or maps, import it first:

```powershell
soundrts --import <from> <to>
```

`<from>` is a folder, a `.zip` file, or the `.txt` file of a map. `<to>` is the folder or the file to make, and it must not exist yet. End `<to>` with `.zip` to get a zip file. The command prints what it left as it was, and how many files it wrote.

A number that the original game has gets the game's name for it. A number that only the package has becomes `phrase_<number>` or `unnamed/sound_<number>`. You can give it a better name: change it in the language files, or rename the recording, and change it wherever the style and the maps use it.

Put the result in the `mods`, `single` or `multi` folder of the settings folder. `<from>` can also be the folder of the original game itself: the result is then a data folder for `--root`.

## Play with others

Choose Multiplayer in the main menu. If your login is still `player`, the game asks for a new one first: the other players hear your login.

To host a game, choose Server. The game starts a server on your computer, says the address to give to the other players, and copies it to the clipboard. Paste the address into a message to the other players. The game asks your router to forward the port (UPnP). If the router does not, players on the internet cannot reach your server, but players on your local network can. The server stops when you leave its menus.

To join a game, choose one of these:

- Choose a server in a list: the servers on your local network, the public list, your favorite servers, and the servers you used recently.
- Enter the IP address of the server: type the address that the host gave you, for example `203.0.113.7` or `203.0.113.7:2502`, or paste it with Control V. For a server on the Internet that encrypts its connections, the address starts with `wss://`, for example `wss://soundrts.example.org`.

A connection to an address that starts with `wss://` is encrypted: nobody between you and the server can read your login, the chat or the game. The welcome then ends with "encrypted". The game checks the server's certificate against the authorities that it carries, the same on every computer, and checks that the certificate is for the server's name. If it is not, the game does not connect, and says why: for example "not connected: the server's certificate is for another name", "is out of date" or "is not trusted". It never connects to that address without encryption. An address without `wss://`, such as a server that a player hosts, connects as before, without encryption. After a lost connection, the game returns the same way it first connected.

On a server, the menu lists the invitations you received, the open games you can join, and the games you can watch. To create a game, choose a map and a speed. Only you and the players you invite can join a game that you start with Start a game on. Anyone can join a game that you start with Start a public game on. The speed menu also has the item "watchers": Enter changes it from "welcome" to "not welcome" and back. It is "welcome" unless you change it. A player who does not have the map of a game gets it from the server, and the game keeps it in the `downloaded\multi` folder of the settings folder.

A map from a server can bring its own rules, and names and sounds for its own units. It cannot change your keys, the game's own words, sounds or help, or anything a mod you installed defines. The game ignores those parts of the map and notes them in `tmp/client.log`. A map that you put in a `multi` folder yourself keeps every power.

In a game room, the host invites players and computers, sets the alliances, the factions, the computers' levels and every player's income, and starts the game. The host's menu has the same level and income choices as a training game, for example "set quiet computer, level 3, to level 2" and "set bob to 150 percent income". See [Levels and income](#levels-and-income). An income is never hidden: everyone in the room hears each change, for example "bob, 150 percent income", and the room names a player whose income is not 100 percent with the income. The other players choose their faction and say when they are ready.

| Key | In the menus | In a game |
| --- | --- | --- |
| F7 | Write to the room, or to the lobby outside a room | Write to every player of the game |
| Shift+F7 | | Write to your allies |
| Ctrl+F7 | | Point your allies at the square under the cursor: a flare |
| Ctrl+F5 | Read the previous chat message again | Read the previous chat message again |
| Ctrl+F6 | Read the next chat message again | Read the next chat message again |
| Ctrl+F12 | | While you watch: follow the next player |
| Ctrl+Shift+F12 | | While you watch: follow the previous player |

A chat message plays a short sound, then the game says who wrote it and what they wrote, after the message that is being said. In a game, the game goes on while you type. The multiplayer menu has an option that stops the game from saying when players arrive on the server or leave it.

A flare points your allies at a square. Put the cursor on the square and press Ctrl+F7. You hear "flare sent" once the server has sent it. Each ally hears an alert sound from that square, then, as an alert, for example "andrew : flare at b 2"; Delete, the key that goes to the last alert, takes them there. Those who watch you or an ally hear it too, and nobody else. You can send at most 3 flares in 10 seconds; the server refuses more, and you hear "too fast: wait a moment". With no ally left in the game, Ctrl+F7 says "no ally in the game" and sends nothing; a computer ally counts as an ally. In a game against the computer, it says "nobody to tell". A watcher sends no flare.

In a game on a server, the server runs the game and tells each player only what that player knows: the fog of war holds. The game does not wait for a slow player or a lost connection. When the host starts the game, their computer sends the server the rules, the computer players and the map, as the host's game has them with its mods; every player needs the same mods, as before. The start of the game names those rules, the map's own included, by their hash. A player whose rules differ, even in the order of the unit types, would hear the wrong names for units, so their game does not play: they hear "cannot play: your game data differs from this game's", and go back to the server's menu.

If your connection is lost during a game on a server, the game does not end. You hear "connection lost, trying to return", and the game connects to the same server again, every second, for as long as the server holds your seat (90 seconds on a server with the usual settings). Meanwhile your units keep their last orders. You can still review the log and ask about what you knew when the connection was lost; an order you give is not taken, and the game says so once. A chat line you type or a flare is not sent either, and you hear "not sent: not connected" once. When the server gives your seat back, you hear "back in the game" and play on from where the game now is. If another connection used your login when you came back, for example a second window, the server gave you a numbered login such as "andrew2"; you still have your seat, and the host's items if you are the host. If that connection is gone when your seat comes back, your login is yours again; if not, you keep the numbered one until you leave the server. If it cannot, you hear why, for example "could not return: your seat was given up", and you go back to the multiplayer menu. Escape, or quitting from the game menu, leaves at once. The game notices a connection that has stopped answering, as when a cable is pulled, after 15 seconds of silence.

The game says, in the system news, when a player loses the connection, comes back, does not come back in time, leaves the game or is removed from it, and when the server stops a game or cannot start it. In the game menu, the speeds change the speed of the game for everyone, and only the room's host can change it: the others hear "only the organizer can change the speed". Everyone hears the new speed and who set it. The room's host also has an item that reads "pause", or "resume" while the game is paused: everyone hears "paused by" or "resumed by" and the host's login. A game on a server has no console and no cheats.

When the room's host leaves the game, is defeated, or does not come back in time, the next player still in the game, in the room's order, becomes the host, never a watcher. Everyone hears it once, for example "bob is now the organizer", and Bob's game menu now has the pause item; he alone can change the speed or remove a player. A host whose connection is lost stays the host while the server holds the seat. A game the host paused stays paused for the next host to resume. While a game on a server is paused, cancel game in the game menu leaves it at once. When no player is left in the game, a paused game runs on to its end.

Your favorite and recent servers are in `favorite_servers.txt` and `recent_servers.txt` in the settings folder: one server a line, its address, then optionally a space and its name. An address that starts with `wss://` is encrypted. To add the server you are on to your favorites, use the item in the server's menu. The public list is [`servers.txt`](servers.txt) in this repository.

### Watch a game

You can join a game only to watch it: a friend's game, a game you teach or referee, or a game that has already started. In the server's menu, choose "watch", then the host and the map. You can watch a game before it starts, from its room, or while it runs. You can watch an open game, or a game you are invited to before it starts, if its host lets people watch. A game takes at most 8 watchers. When a game takes no more watchers, you hear "no room for another watcher". When its host does not let people watch, you hear "this game takes no watchers".

A watcher hears the game from the side of one player at a time, as that player hears it: the same map, the same news and the same answers to the questions. You hear "watching" and the player's name, and the cursor is on the square of that player's first unit. You hear nothing of what happened before you came. Control F12 follows the next player still in the game, and Control Shift F12 the previous one. You hear "watching" and that player's name. You can follow only people, not computers. When the player you follow leaves the game, you follow the next player. When no player is left, you hear the result and the score of the player you followed last.

A watcher gives no orders. When you press a key that gives an order, you hear "watching: you give no orders", once. The server takes no order, speed, pause or removal from a watcher, whatever a client sends.

Nothing is hidden from the players. Everyone in the room or the game hears "carol is watching" when Carol starts to watch, and "carol stopped watching" when she stops. When a room changes, its list of players names the watchers after the players. In a game, F11 names the watchers after the players. A watcher's chat message reaches everyone, and you hear "watching" after the watcher's name. A watcher cannot write to the allies.

If your connection is lost while you watch, you return to the game as a player does. The players hear nothing unless you stay away longer than the server holds your seat. Then they hear that you stopped watching. To stop watching, leave the game from the game menu.

A player who is defeated hears the score and goes back to the server's menu, as before. To keep listening to an open game that takes watchers, choose "watch" for it there.

## Run a server

You can also run a server on its own, for example on a computer that stays on. The server is a lobby and a chat, and it runs the games played on it. It does not need the game data: the player who starts a game sends what the server builds it from.

```powershell
cargo run -p soundrts-server -- --port 2501 --name "My server"
```

The server listens on TCP port 2501 for players and answers on UDP port 2501 to players who look for servers on the local network. Add `--upnp` to ask the router to forward the port to players on the internet. Run `cargo run -p soundrts-server -- --help` for every option. Press Ctrl+C to stop the server.

Add `--records <folder>` to keep a record of each game played on the server. The server writes the record when the game ends, or when it stops the game, and keeps the newest 100. A record holds what the game was built from and every player's commands, so it replays the whole game. It also holds the players' logins. It does not hold the chat. A game stopped because a turn took too long is not recorded.

To listen to a record, put its file in the `records` folder of the settings folder, and choose "listen to a game" in the single-player menu. See [Listen to a game](#listen-to-a-game). To replay a record, or a saved game, from the command line, run `soundrts --check-record <file>`. It says how many turns ran and who was still in the game at the end, or why the file cannot be replayed. Use it to reproduce a bug report that comes with a record or a save. It needs no game data.

### A public server

A server on the Internet should encrypt its connections, so that logins, chat and games do not cross the Internet as they are. The server does not speak TLS itself. Put it behind a reverse proxy that does, such as Caddy or nginx, as web servers are run. With Caddy:

1. Point a domain name, for example `soundrts.example.org`, at the machine. Open TCP ports 80 and 443 in its firewall. Caddy uses port 80 to get its certificate.

2. Start the server so that only the proxy on the same machine can reach it:

   ```sh
   soundrts-server --ip 127.0.0.1 --port 2501 --name "My server"
   ```

3. Install [Caddy](https://caddyserver.com), and put this in its `Caddyfile`, with your domain name:

   ```text
   soundrts.example.org {
       reverse_proxy 127.0.0.1:2501
   }
   ```

   Caddy gets a certificate from Let's Encrypt, renews it, and passes the players' WebSocket connections to the server.

4. Players connect to `wss://soundrts.example.org`. If the proxy listens on another port than 443, add it: `wss://soundrts.example.org:8443`.

To add the server to the public list, add a line to [`servers.txt`](servers.txt) with the `wss://` address, then the name:

```text
wss://soundrts.example.org My server
```

A game older than encrypted connections cannot read a `wss://` address: it lists the server, and says that it cannot reach it. It never connects to it without encryption.

## Listening room

The listening room is a separate program that helps decide how the game's positional sound should work. It plays the game's own sounds through several renderers and asks what you hear. It prints one instruction at a time and reads your answers as typed lines, so your screen reader reads it as it reads any console program. Use headphones.

The renderers are:

- `current`: the game's sound as it is now, left-right placement and loudness.
- `stereo-plus`: the same, with volume changes that cannot click, a duller tone for sounds behind you, and a muffled tone through a closed exit.
- `binaural`: Steam Audio with a model of the head, for headphones.
- `binaural-plus`: `binaural` with plain cues added for behind: a sound behind you is also duller, with the same filter as `stereo-plus`, and a little quieter.

The two binaural renderers are in the program only if you build it with `--features binaural`.

```powershell
cargo run -p listening-room -- --root vendor/game-data
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
cargo run -p listening-room --features binaural -- --root vendor/game-data
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

The tests need the original game in `vendor/soundrts-upstream` and its imported data in `vendor/game-data`. The fetch script makes both. The tests fail with a pointer to the fetch script when one is missing, or when the imported data was made with another table of names: run the script again after a change to `crates/import/src/original_names.txt`. No test speaks, plays sound, or opens a window. The client tests run on a recorded speech device, a recorded audio device, scripted keys, and a manual clock. The multiplayer tests in `crates/client/tests/net_play.rs`, `crates/client/tests/multiplayer.rs`, `crates/client/tests/returning.rs` and `crates/client/tests/watching.rs` also run real servers, on 127.0.0.1 only; `returning.rs` pulls a player's connection out through a proxy on 127.0.0.1 and plugs it back in. One of them plays the same game twice, on the client and through a server, and checks that the player hears and plays the same. No test asks a router to forward a port.

The checks on GitHub run the same tests with [cargo-nextest](https://nexte.st): every test in its own process, several at a time. A test must therefore pass alone and next to any other test. To run them that way yourself, install cargo-nextest and run `cargo nextest run --workspace`.

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

## Release

A release has three downloads, with their SHA-256 checksums in `SHA256SUMS`:

- `soundrts-<version>-windows-x64.zip`: the game, the server, the DLLs they load, and the licenses. Unzip it anywhere and start `soundrts.exe`. `soundrts.pdb` holds the file and line numbers of the game: with it beside the program, a crash writes its call stack to `tmp/client.log` with the place of every step.
- `soundrts-<version>-windows-x64-setup.exe`: an installer of the same files. It installs for the current user, asks for no administrator password, and adds a Start menu entry. Its uninstaller leaves the settings folder.
- `soundrts-server-<version>-linux-x64.tar.gz`: the server for Linux.

No download holds the original game's data. The game fetches it the first time it starts. The downloads are not signed, so Windows SmartScreen warns about them.

To make a release:

1. Set the version in `Cargo.toml`, under `[workspace.package]`, and merge that change. Use a commit of `main` whose checks passed.
2. Tag the commit `v` and the version, for example `v0.1.0`, and push the tag. For a build to test, add a hyphen and a name, for example `v0.1.0-test.1`. A tag that does not start with the version in `Cargo.toml` is refused.

   ```powershell
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. The Release workflow builds the downloads and checks them: a crash in a release build names its file and line, the game in the zip fetches the data and finds it, the installer installs and uninstalls. Then it makes a draft release with the downloads. A tag with a hyphen is marked as a pre-release.
4. Read the draft on the Releases page of the repository, edit its notes, and publish it. Nobody else sees the draft before that.

A pull request that changes the workflow, `packaging/`, or the scripts it runs builds and checks the same downloads, and keeps them with the run, without a release. To get a build of any branch the same way, start the Release workflow by hand on the Actions page.

To make the Windows folder on your machine:

```powershell
pwsh scripts/fetch-nvda-controller.ps1
cargo build --release --locked -p soundrts -p soundrts-server
pwsh scripts/stage-release.ps1 -Name soundrts-test
pwsh scripts/check-release.ps1 -Folder dist/soundrts-test
```

The check speaks nothing and opens no window. It fetches the data into a temporary folder, not into your settings folder.

## Layout

- `crates/world` is the simulation: rules, maps, units, orders, and computer players. It takes text and a seed, and it never touches a file, a sound device, or a window. The same input gives the same game on every machine. A host owns one game: it builds the world from the game's set-up, takes each player's commands, runs the game one turn at a time, and gives each player a view of what that player knows. It also tells each player that knowledge as plain data, made to be sent: the game's map and rules once, then one message a turn, from which a view of the same answers is made without the world. After a seat's first message, what changed since its last one is enough to make the next.
- `crates/client` is the program: resources, sounds, speech, the voice queue, key bindings, menus, and the in-game interface.
- `crates/import` is the importer: it converts the original's files into the files the game reads, as a library that the game runs at its first start and for `--import`, and as the `soundrts-import` program that the fetch script runs. `crates/import/src/original_names.txt` is its table: the name of each numbered phrase and sound of the original. Only the importer knows the numbers.
- `crates/protocol` is the messages between a client and a server. Its `game` module encodes what a game run by a server sends each player and what a player sends it, in a compact binary form.
- `crates/server` is the multiplayer server, as a library that the game can start and as the `soundrts-server` program.
- `crates/listening-room` is the listening room.
- `scripts/fetch-upstream.ps1` fetches the original game and imports it. `scripts/upstream-commit.txt` names its pinned commit, which the game also downloads and imports when it has no data.
- `scripts/name-numbers.ps1` keeps `crates/import/src/original_names.txt`, which gives a name to each numbered phrase and sound of the original. It adds a row for a number that has none and keeps every name that the table has.
- `scripts/fetch-nvda-controller.ps1` fetches NVDA's controller client, for the `nvda_direct` setting.
- `scripts/stage-release.ps1` gathers the files of the Windows download, `scripts/check-release.ps1` checks them, and `packaging/` holds the installer's script and the download's README.

## Licenses

The code is a port of the SoundRTS source code and keeps its BSD 3-clause license. See [LICENSE.txt](LICENSE.txt).

On Windows the program links [Tolk](https://github.com/dkager/tolk) (LGPL 3) to reach the screen reader. Tolk loads `nvdaControllerClient64.dll` (LGPL 2.1, from NV Access) and `SAAPI64.dll`. The build copies both files next to the program. Keep them there when you copy the program somewhere else.

For the `nvda_direct` setting, `scripts/fetch-nvda-controller.ps1` fetches NVDA's current controller client, `nvdaControllerClient.dll` (LGPL 2.1, from [NV Access](https://github.com/nvaccess/nvda/tree/master/extras/controllerClient)), with its license, into `vendor/nvda-controller`. The game loads it at run time and does not change it. The build copies the DLL next to the program when it has been fetched. The repository does not contain it.

The listening room built with `--features binaural` links [Steam Audio](https://github.com/ValveSoftware/steam-audio) (Apache 2.0) through `phonon.dll`. The build copies `phonon.dll` next to `listening-room.exe`, with `phonon-THIRDPARTY.md`, which lists the licenses of the code inside the DLL. Keep both files next to the program when you copy it somewhere else.
