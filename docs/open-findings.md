# sound-rst: what still needs changing

33 open findings from the design review of the Rust port of SoundRTS, grouped by what they affect: what you hear, multiplayer, gameplay, and code structure. The six multiplayer fixes, the game record and the recording cutover are merged and are not repeated here.

Written 14 November 2026 against commit `612afd1` on `main`.

## How far each finding was checked

Every finding comes from reading code. Nothing here was measured, and nothing was heard through a screen reader. Each finding carries one of three labels.

- **Checked today:** I read the code at commit `612afd1` and the finding still holds. File and line references on these findings are current.
- **From the review:** a reviewer traced it in the code before the three merges and cited it. I have not re-read it since, so I name the file but not the line.
- **Suspected:** inferred from the code and never traced or reproduced.

The reviewers were three Opus agents (simulation, client, and the original game's design) and one Sol investigation (multiplayer). Sizes are my estimates: small is under a day of agent work, medium is one pull request, large is several.

**One correction to my earlier summary.** I said the computer player cannot play a mod faction because unit names are hard-coded. That overstated it. The names go through each faction's table of equivalents, so the orc mod works. The accurate finding is G1.

## Where I would start

1. **Speech and news, H1 to H4, as one piece of work.** It changes what you hear in every game, and H2 and H3 were blocked until the cutover. It needs your ears at the end.
2. **Reconnecting after a dropped connection, M1 with M2.** It builds directly on the game record that is now merged.
3. **The questions you cannot ask, H5.** Each is a small new command on a new key.
4. **Computer fairness and inherited bugs, G1 and G3.** Each fix changes the recordings, so batch them and re-record once.

I would not start a structure project by itself. Typed commands (C1) and a player view (C2) each earn their cost only when a feature needs them, and each finding says which feature that is.

## What you hear

Speech, news, keys, settings and sound. For a blind player this is the whole interface.

### H1. The game guesses when speech has ended

Checked today. Size: small for the setting, medium for the exact NVDA fix. Needs your ears.

**Now.** The client assumes each character takes 0.1 seconds to speak, with digits counted longer. It never asks the screen reader. That guess decides when queued news starts, when the next part of a message plays, and whether a cut message counts as heard.

**Effect.** At a fast speech rate you get silence before each queued message. At a slow rate the next message cuts the current one off. The stop keys only reach the screen reader while the guess says speech is still going, so they cannot silence speech that runs past it.

**Where.** `crates/client/src/tts.rs`: `estimate_us` at line 53, `stop` at line 73, `is_speaking` at line 81.

**Simplest change.** A speech-rate setting in the options menu that scales the guess, and make `stop` always reach the screen reader. This gives up exactness: you tune it once by ear.

**Exact alternative, NVDA only.** From memory, NVDA's newer controller client can speak marked text and call back when a mark is reached, which would report the real end of speech. Verify that against NV Access's documentation before planning on it. The controller client bundled with Tolk is the 2020 build and exports only `speakText`, `cancelSpeech`, `brailleMessage` and `testIfRunning`; I inspected that file earlier. JAWS and the system voice would still use the guess.

### H2. Some spoken messages stop the game

Checked today. Size: medium. Changes the transcripts. Needs your ears.

**Now.** Messages the game marks as important are spoken as blocking messages, and a cut scene is a run of them. The game loop stops until the message is said or a key is pressed, and keys are checked only every 100 milliseconds meanwhile.

**Effect.** The world and its sounds freeze during the message. In a server game that client stops taking turns. After 11 turns the server makes every player wait, and after 60 seconds it removes the player. This is the one stall the game-menu fix did not cover.

**Where.** `crates/client/src/game/mod.rs`, `process_srv_event` at line 478. `crates/client/src/voice.rs`, `say_now` at line 437.

**Simplest change.** Queue these messages with priority, as chat already is, and keep the loop running. Do it for server games first, as the game menu did. This gives up the guarantee that nothing happens while an important message is spoken; mission cut scenes can keep blocking.

### H3. News is dropped while you are busy

Checked today, except the two throttles marked below. Size: medium. Changes the transcripts. Needs your ears.

**Now.**

- Any arrow or status key cuts the news being spoken. If it had played for more than one second it counts as heard and is never repeated. `voice.rs`, `give_up_current_if_partially_said`, line 335.
- A game event handled more than 3 seconds late is discarded, speech and sound together. `game/mod.rs`, `EVENT_LIMIT`, line 59.
- Events are handled one per loop pass, so the burst that follows a blocking message arrives late and falls under that 3 second rule.
- Resource news expires after 1.5 seconds and is only said at round counts. From the review.
- Attack alerts are limited to one every 10 seconds, and attacks older than 5 seconds are discarded while the limit is active. From the review.
- The history keeps 200 messages and drops the oldest 100 at once.

**Effect.** "Peasant ready" and similar news can vanish with no trace in the history.

**Simplest change.** Put a cut message back in the queue, and apply the 3 second rule to sounds only. This gives up brevity: more speech queues up in a busy fight, so the expiry times may need tuning by ear.

### H4. Modifier keys act the moment they go down

Checked today; the key repeat is suspected. Size: small. Changes the transcripts. Needs your ears.

**Now.** Control alone runs "examine". Alt or Shift alone stops the current message. Both fire on key press, so every Control chord first speaks the examine text, and every Shift or Alt chord first skips whatever news was being said.

**Suspected.** Holding a modifier for more than half a second repeats its command every 100 milliseconds, because the game's own key repeat covers modifiers.

**Where.** The bindings come from the original's `res/ui/bindings.txt`. Key releases already reach the game and are ignored: `crates/client/src/game/mod.rs` line 450.

**Simplest change.** Run a modifier-only command on release, and only if no other key was pressed in between. This gives up immediacy: examine speaks when you let go of Control.

### H5. Questions you cannot ask the game

From the review. Size: small each.

**Now.** These have no command:

- Where am I under attack? It is announced once, throttled, and cannot be asked again. Control with Page Up or Page Down lists squares with known enemies, which is a different question.
- Jump to the last alert.
- What is being trained or researched, across all buildings. Today you select each building in turn.
- How many workers are idle, and how many are on each resource.
- Health of the group. V only answers with exactly one unit selected.
- The score before the game ends.
- Who is an ally, who is an enemy, and who is defeated. F11 says names only.
- A third resource, on maps that add one. Only the first two have keys.

**Simplest change.** New commands on new keys, added the way the client already adds Control F5 and Control F6 for chat history. No existing key changes.

### H6. Settings are lost or only in files

Checked today for volume, verbosity and key repeat; the rest from the review. Size: medium. Adding menu items changes the menu transcripts.

**Now.**

- The main volume always starts at half. `crates/client/src/sound.rs` line 341.
- The speed chosen in the game menu is not saved.
- Speech timing and verbosity can only be changed in `SoundRTS.ini`. Verbosity is a list of words matched by substring.
- Keys can only be remapped by editing a file, and menu keys cannot be remapped.
- Key repeat is fixed at 500 then 100 milliseconds and ignores your Windows setting. `crates/client/src/input.rs` line 269.
- Each press of a volume key also cuts all speech.

**Simplest change.** Save the volume when it changes, and add an options submenu for volume, speech rate and verbosity. The speech-rate item is the setting in H1.

### H7. The window is blank

Checked today. Size: medium. A decision for you.

**Now.** The window is 400 by 75 and nothing is drawn in it. Control F2 says the display is on, and nothing changes. The original drew each spoken message as a subtitle, and a grid of the map.

**Effect.** A sighted helper or a low-vision player sees nothing.

**Where.** `crates/client/src/window.rs` line 395. `crates/client/src/client.rs`, `toggle_fullscreen`, line 231.

**Simplest change.** Draw the last spoken message as text. Draw it as pixels: a native text control might be read a second time by the screen reader (suspected). If the window should stay blank, remove the Control F2 announcement instead.

### H8. Front and back sound the same

Checked today; the clicks are suspected. Size: medium. Needs your ears.

**Now.** A sound's position becomes left-right pan and loudness. A sound behind you is the same sound at half volume. Volume changes are applied with no smoothing, which may click.

**Where.** `crates/client/src/audio.rs`, `stereo`, line 252.

**Simplest change.** Muffle sounds behind the listener with a low-pass filter, and ramp volume changes. Both fit below the audio output layer, so recordings do not change. Low priority unless you find front and back hard to tell apart.

## Multiplayer

Findings from the Sol investigation, plus leftovers from the three merged pull requests.

### M1. A dropped connection forfeits your army

From the review. Size: large.

**Now.** When your connection closes, the server treats it as leaving the game and your units are removed. Coming back under the same name puts you in the lobby.

**Where.** `crates/server/src/lobby.rs` and `crates/server/src/game.rs`.

**Simplest change.** The server keeps the turns it has released, holds a dropped player's slot for a short grace period, and gives the returning client the turns to replay. A token proves it is the same player. The game record merged in pull request 4 already has the format and the replay.

**Gives up.** Everyone waits during the grace period, and catching up on a long game takes time. Measure replay speed on a long record before committing.

### M2. A silently dead connection hangs the game

From the review; I confirmed today that the client and server contain no heartbeat. Size: small.

**Now.** If the network stops carrying traffic without closing the connection, the client waits for a turn that never comes until Windows reports the failure or you leave.

**Where.** `crates/client/src/net/conn.rs`.

**Simplest change.** A heartbeat with a deadline, and a spoken message when it passes. This can disconnect a player during a long outage that would have recovered. Do it with M1.

### M3. One player can stall the game again and again

Checked today. Size: small.

**Now.** The 60 second removal timer restarts each time a slow player catches up. A player who catches up just in time, on purpose or because their machine is overloaded, can hold everyone repeatedly and never be removed. The host has no way to remove a player.

**Where.** `crates/server/src/game.rs`, the `holding_since` field, cleared at lines 302 and 342.

**Simplest change.** Let the host remove a player through the existing leave path. This gives the host that power alone. The alternative is a total stall allowance per player, which needs a number chosen.

### M4. No pause and no speed change in a server game

From the review. Size: medium.

**Now.** Speed is fixed when the game is created. Nobody can pause. The original adapted its turn timing to each player's ping; the port's clock is fixed.

**Simplest change.** The host can pause and change speed, taking effect at a turn every client agrees on. Adapting to ping automatically can stay out until play over the internet shows it is needed.

### M5. A game hosted from the game dies with its host

From the review. Size: small, documentation only.

**Now.** If the hosting player's game crashes, their machine sleeps, or they leave the server, every game on that server ends.

**Simplest change.** Say so in the README and point to the standalone `soundrts-server` program for games that should outlive one player. Moving a running game to another host is far more machinery, and I would not build it.

### M6. Player names are not verified

Checked today. Size: medium. Matters only on a public server.

**Now.** A name is taken on request. The only check is that nobody connected already has it, ignoring case. Someone can connect under an absent player's name and receive invitations meant for them. They cannot take over a player who is connected.

**Where.** `crates/server/src/lobby.rs`, `unique_login`, line 219.

**Simplest change.** Optional reserved names with a token per name, set up on the server. This adds setup work and identifies players on that server only.

### M7. A public server can be overloaded

Checked today for the first two points; the impact is suspected and unmeasured. Size: medium. Matters only on a server open to the internet.

**Now.**

- Every accepted connection gets its own task before the player limit is checked. `crates/server/src/lib.rs`, the accept loop near line 234.
- The queues into the lobby and each game have no size limit. `crates/server/src/game.rs` line 61.
- Rate limits count messages, not bytes. From the review.

**Simplest change.** Limit connections that have not said hello yet, limit queued bytes, and give map transfers their own allowance. This refuses bursts of honest connections too.

### M8. Nobody can join only to watch

From the review. Size: medium.

**Now.** A defeated player can stay and watch. A friend or referee cannot join a game as a watcher.

**Simplest change.** Watcher slots chosen before the game starts, with no units and no part in victory. This gives up joining after the start, which would need the whole game state sent.

### M9. Two map limits are still missing

Checked today for the square width; the timing point is from Sol's review of pull request 3. Size: small.

**Now.** Lines, meadows and starting units are capped. Two things are not:

- The square width is multiplied by 1000 with no upper bound, so a huge value overflows. `crates/world/src/map.rs` line 627.
- A hostile map can still cost load time in proportion to its text, up to the 16 megabyte map limit.

**Simplest change.** Cap the square width the way the other three are capped. Leave the timing point until a map is found that actually takes long.

### M10. Game records stop at out-of-sync reports

Checked today. Size: small for the first step, large for saved games.

**Now.** A record is written only when a server game goes out of sync. There is no way to listen to a recorded game, no saved games, and no record of a single-player game. The README still lists saved games and replays as not ported.

**What blocks single-player records.** Four console cheats change the world directly without going through the command stream: resources, all technologies, mana, and "play as the next player". `crates/client/src/game/misc.rs`, lines 210 to 226 and `cmd_change_player` at line 265.

**Simplest change.** Send those four through the command stream. Then every game can be recorded, a save is the record so far, and a load replays it.

**Gives up.** Loading is not instant, and a save stops loading after any change to the simulation. Measure how long a one-hour game takes to replay before building saves on this.

### M11. Two different machines have never been compared

From the review. Size: small.

**Now.** The multiplayer test runs both clients on one machine, and the CI job runs on Windows only. The simulation is integer maths except in three places: decimal numbers in data files are parsed through floating point, the square root starts from a floating-point estimate before an integer correction, and trigger timers compare floating-point values. No concrete difference was found.

**Where.** `crates/world/src/nofloat.rs`, `crates/world/src/triggers.rs`.

**Simplest change.** Run the simulation crate's recorded traces in CI on a second processor type, such as an ARM Linux runner, against the same expected files. The simulation crate has no Windows dependency that I know of; confirm that first.

### M12. Small leftovers from the three pull requests

Checked during those reviews. Size: small each. None is urgent.

- The sync check does not cover whether two remembered units are the same snapshot, because that is decided by memory address. Sol raised it and accepted leaving it. It goes away with C3.
- A record replays with the faction setting of a multiplayer game, and no test can tell the two settings apart while only one faction is loaded. A test with the orc mod would close this.
- The cost of the sync check, about 0.7 milliseconds on a late game, was measured once by the implementing agent and not repeated.
- The branches `mp-safety`, `game-record` and `golden-cutover` are still on GitHub. I left them because deleting is yours to decide.

## Gameplay

All of these change the recordings. Since the cutover that means re-recording with `UPDATE_EXPECTED=1` and reading the differences. All are from the review unless marked.

### G1. Computer players play by different rules

Checked today, except the warehouse point. Size: small to change, then playtesting.

**Now.**

- The "aggressive computer 2" type has its resources reset to 1000 of each every time it plays, and ignores all requirements. `crates/world/src/ai.rs`, `computer2_play`, line 246.
- Every computer remembers a unit for 10 hours after it goes out of sight. A human remembers it for 3 minutes. `crates/world/src/player.rs`, `memory_duration`, line 485.
- When choosing where to put a warehouse, a computer reads what is really on a square, not what it has seen. From the review.
- The computer's fixed routine is built around four roles: peasant, town hall and farm, looked up through the faction's equivalents, and gate, by name. A faction must supply all four.

**Simplest change.** Give computers the human memory time, which is one number. Then decide whether computer 2 stays as it is, labelled as a cheating opponent, or goes. This makes the computers weaker, so it needs playing against.

### G2. Orders other strategy games have

From the review. Size: medium each.

- No hold position. Units are either offensive or defensive.
- No automatic spell use for a human's units. Computers cast on their own.
- Four recallable groups, on keys 6 to 9.
- At most 10 queued orders per unit.

**Simplest change.** Hold position first: it is one new mode beside the two that exist. A new order is also the point where typed commands, C1, start to pay.

### G3. Bugs kept from the original

From the review; none re-read by me. Size: small each. Batch them and re-record once.

The port copied these on purpose so that it matched the Python game turn for turn. Nothing holds them in place now.

| What goes wrong | File in `crates/world/src` |
|---|---|
| A player is in their own list of allies twice, so their units are healed twice and their threat is counted twice. | `player.rs`, `map.rs`, `unit.rs` |
| When a splash attack kills its target, the splash is centred on the map corner, not on the target. | `unit.rs` |
| The victory check skips a player when a loser quits. | `playerlogic.rs` |
| The loop that ends the game for everyone skips every other player. | `world.rs` |
| After a building project's money is released, the next project in the list is skipped. | `playerlogic.rs` |
| The count of enemy threat around a square reuses the last neighbour it looked at. | `playerlogic.rs` |
| An "ignore everything else" move that turned into an attack stays an attack for the rest of the group. | `triggers.rs` |
| "Load all" and "unload all" given to a unit inside something fail without a message. | `orders.rs` |
| Any two targets that are parts of a square count as the same target. | `entity.rs` |
| Removing one of two overlapping objects frees the space they shared. | `collision.rs` |
| At the map edge, occupied space wraps round to the next row. | `collision.rs` |
| Unit ids are sorted as text in nine places, so 10 comes before 9. | `entity.rs` |
| The computer counts one square twice when it discards far building places. | `ai.rs` |
| The computer gives up choosing a building place on a tie that involves no square. | `ai.rs` |
| Computer 2 treats every "use" order as if it had a target. | `ai.rs` |
| When the "add units" cheat runs out of space it deletes the player's newest unit, not the one that failed. | `triggers.rs` |
| The number 1.005 in a data file is read as 1.004. | `nofloat.rs` |
| The integer square root can be off by one. | `nofloat.rs` |

### G4. The simulation imitates Python's errors

Count checked today. Size: large in total. Do it alongside other work, not as a project.

**Now.** 114 places in the simulation stop a unit's or a player's turn part-way, carrying the text of the Python error the original would have raised. Whatever was changed before the stop stays changed. Only the soak test watches the warnings these produce.

**Effect.** Real defects hide behind them, and they are hard to tell from the copied ones.

**Simplest change.** Whenever a fix lands in a function, turn its stops into states that cannot occur or into assertions.

### G5. The client still carries code that only kept it in step with Python

From the review. Size: medium, in one pass.

**Now.**

- Random numbers drawn and thrown away, so that later draws matched the original.
- Python error text used as error messages in about a dozen places in the game interface.
- A failed lookup becomes a beep, as Python's missing-key error did.
- A menu quirk where the position before the first item is kept.

**Simplest change.** Remove them together and re-record once. Removing them piecemeal means reading several rounds of differences.

## Code structure

None of these changes what a player hears. Each says what would make it worth doing.

### C1. Orders travel as text and are parsed again at every step

From the review. Size: large.

**Now.** The client builds an order as a line of text. The protocol carries the line and the simulation parses it. Inside the simulation, units, the computer player and triggers also build text orders, in about 30 places in `ai.rs` alone. A typing mistake in an order name compiles.

**What changed since the review.** I first listed this as the way to close the cheating hole. A flag on the world closed it, so safety no longer needs it.

**Worth doing when.** The first new order or command is added (G2, H5). Parse once where the order enters, into one shared type.

### C2. The interface reads the whole world, not what the player knows

From the review; the leaks are suspected. Size: small to check the leaks, large for the full change.

**Now.** The client holds the live simulation and reads from it directly. Three places may tell a player things they have not seen: what is blocking an exit, the owner's unit list when a unit dies, and the live unit behind an order's target.

**Worth doing now.** Trace those three. If any is real, it is a gameplay bug.

**Worth doing later.** A read-only "what this player knows" layer earns its cost when watchers (M8) or listening to records (M10) is built, because both need to present a game from a chosen player's side.

### C3. Remembered units are told apart by memory address

Checked today. Size: small.

**Now.** Whether a remembered unit is still the current memory is decided by comparing addresses. That cannot be written to a file or included in the sync check.

**Where.** `crates/world/src/entity.rs` line 106. `crates/world/src/world.rs` line 486.

**Simplest change.** A version number on each memory. It needs a test that an order aimed at a stale memory still treats the target as forgotten.

**Worth doing when.** Anything needs to write the world to disk, such as instant saves.

### C4. Per-turn work that grows with the game

First two checked today; the rest from the review. Nothing measured. Size: small for the first, unknown for the rest.

- Every attempt to move a unit copies the whole set of occupied space. The original removed the unit's own cells, tested, and put them back. `crates/world/src/unit.rs`, `would_collide_if`, line 630.
- A deleted unit is never freed, and the sync check walks every unit ever created, every turn. `crates/world/src/world.rs`.
- Each turn, every unit the player can perceive is copied to hand to the interface.
- The current order and whole unit types are copied on each use.
- Looking up a rule scans the list and copies the value.
- Several scans kept from the original run over all units once per unit.

**Simplest change.** Time one late-game turn on a long recorded trace first. Fix the move copy regardless: testing in place gives the same answer. Fix the others only if the timing says they matter.

### C5. Two very large structures

From the review. Size: medium each. Low priority.

**Now.** The world exposes its main fields for any code to change, and seven of its functions run from about 90 to about 260 lines. The game interface is one structure of about 70 fields.

**Worth doing when.** Work lands in one of those functions. Split the piece being changed, not the whole.

### C6. Everything runs on one thread between a key and its speech

From the review; the delay is suspected and unmeasured. Size: small to measure.

**Now.** A simulation turn, the call to the screen reader, and the first decoding of each sound all run on the thread that handles your keys. The original sent speech through a separate thread.

**Simplest change.** Time the turn and the screen reader call in a late game. Decode sounds ahead of time in the background. Move the simulation to its own thread only if the numbers call for it.

### C7. The game checks for input on a timer

From the review. Size: medium. Low priority.

**Now.** The game loop sleeps 1 millisecond per pass, menus 10, and blocking speech 100.

**Simplest change.** Wait for the next key or the next deadline, whichever comes first. The 100 millisecond wait goes away with H2. The transcripts record every sleep, so this needs a re-record.

### C8. Windows only in practice

From the review. Low priority unless other systems become a goal.

**Now.** Off Windows, speech goes to the system voice and not to a screen reader, and there is no clipboard.

## What I would leave alone

- **The 300 millisecond turn.** Unit speeds, costs and timings in the data files are all written against it.
- **The integer maths.** It is what keeps every client's game identical.
- **The copy of Python's random number generator.** Any seeded generator would do now, but this one is small, fast and tested, and replacing it buys nothing.
- **The fixed limits:** 26 columns, 10 resource types, 144 units per teleport, a food limit of 80. No finding depends on raising them.
- **Moving a running game to another host.** See M5.

## Waiting on your ears

Two things from the merged work have been tested as text and never heard through NVDA.

1. **The game menu during a server game.** Start the game, choose Multiplayer, then Server. Create a game on any map, add a computer, and start. Press F10: you should hear "Menu", and game sounds and news should carry on behind it. Arrow through the items, then press Escape: you should hear "continue game". Press F10 again to confirm the game still takes keys.
2. **The two out-of-sync sentences.** They cannot be triggered by hand. They are "the game is out of sync: you no longer play the same game as the others" and "other players are out of sync: they leave the game".

## Already done

| Pull request | What it settled | Merge commit |
|---|---|---|
| [Number 3](https://github.com/akj/sound-rst/pull/3) | Cheats in multiplayer, games that carry on out of sync, the incomplete sync check, mixed builds in one game, the game menu stalling everyone, and maps that exhaust memory. | `888acf0` |
| [Number 4](https://github.com/akj/sound-rst/pull/4) | A record of a server game, saved when it goes out of sync, and `--check-record` to replay it. | `3b9bcd2` |
| [Number 5](https://github.com/akj/sound-rst/pull/5) | The recordings are the port's own. The last commit checked against the Python game is tagged `last-python-parity`. | `612afd1` |

Sources: the four review reports from this session, re-read in full, and the code at `612afd1`. The reports also hold a full inventory of the original game's units, buildings, orders and keys, which is reference material and is not reproduced here.
