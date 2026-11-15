# One listening script for everything built and not yet heard

Everything the refactor changed in what you hear has been checked only on the recorded speech device. This script gathers the listening steps of every merged pull request into one, most important first. Written 15 November 2026 against `main` at `383b182`; part 6 updated at `40347d9`, and listening to a recorded game added to part 4 at `2220932`.

Each step is one action and what you should hear. Each has a number such as 1.3, so a report can be short: "1.3 fail: said twice". A step you skip is worth saying too.

The parts are in order of importance. Part 1 decides whether the rest is worth doing that day. Parts 1 to 6 need only you. Part 7 needs one other player on another computer, and part 8 needs two.

## How to report

Write the step number, pass or fail, and for a fail what you heard instead. Put it in a file, as you did for the listening room, or in a message. Anything that surprised you and has no step is as useful as the steps.

## Before you start

Build and start the game from the base repository:

```powershell
cd C:\Users\andrew\.t3\projects\sound-rst
git fetch origin
git switch --detach origin/main
cargo run --release -p soundrts -- --root vendor/soundrts-upstream
```

If the game is ever silent and does not answer, press Alt+F4. NVDA keeps working outside the game. Then tell me at which step.

For parts 7 and 8, the other players need the same build. On your machine, `pwsh scripts/stage-release.ps1 -Name soundrts-test` after a release build makes a folder under `dist` that you can zip and send; the README's Release section has the full commands. Their game downloads its data the first time it starts.

## Part 1. Does it speak, and is news kept

This is the oldest unheard change: speech moved to its own thread in pull request 11, and news stopped being dropped in pull requests 24 and 29.

1.1. Start the game. Expected: the main menu is spoken, and the arrows read its items as they always did.

1.2. Start a training game on jl1 against a quiet computer. Expected: the intro and the objective are told. Press an arrow while they are told: each press skips one message.

1.3. Start another training game. While the story is told, press Escape once. Expected: nothing more of the story is said, and no start of a line is heard.

1.4. Press W for the town hall, A until "recruit peasant", then Enter four times quickly. While the peasants train, keep pressing arrows, Space, Z and X, so that the voice is always answering you. Expected: after your answers, every "peasant N ready!" is still said. None is skipped. A gold or wood count is said once, with its latest value.

1.5. While several messages queue, listen at your usual speech rate. Expected: each follows the one before without a silence and without cutting it off.

1.6. While a long message is being said, press Control. Expected: speech stops at once.

1.7. Order more peasants, and press F4 while they finish. Expected: something like "production 2". Press F4 again: each unheard message, one at a time. When none are left, F4 says "nothing!".

1.8. Press Slash. Expected: "production". Press Comma a few times: the peasant messages, newest first. Press Period: forward again.

1.9. Press F5 and F6. Expected: they walk through every message, as before.

1.10. Press F10. Expected: the game menu opens, the game pauses and the units are silent. Press Escape: the game goes on.

1.11. Open the console with the backquote key, type `a`, a space, `zzz`, and press Enter. Expected: a beep, and the game's sounds do not stop.

## Part 2. Keys and questions

2.1. Press Control and let go. Expected: the square's description, when you let go, not when you press. NVDA may also silence speech when you press Control.

2.2. Hold Control and press Tab, then let go of both. Expected: only the target, such as "a meadow to the northwest", with no square description before it.

2.3. Wait for some news, such as "peasant 1 ready", and press Shift+Up while it is being said. Expected: the cursor moves, or you hear the edge-of-map sound. The news is not skipped: it is said again after the square.

2.4. Hold Control for two seconds, then let go. Expected: the square's description once, when you let go.

2.5. Hold an arrow key. Expected: it repeats at your Windows keyboard speed, after your Windows repeat delay.

2.6. Press Alt alone and let go, then press an arrow. Expected: Alt skips the current message, and the arrow is answered normally. If the arrow does nothing, Windows has opened the window's system menu, and I need to know.

2.7. Press Control+F1. Expected: "key help on: press a key to hear what it does, Escape to leave". Press Z, then Control+Tab, then Shift+S: each says its name and what it does, and nothing happens in the game. Press Insert: "Insert: no command". Press Escape: "key help off".

2.8. With key help on, note whether NVDA's own key echo or input help gets in the way.

2.9. Press B with nothing in production. Expected: "nothing!". Recruit a peasant and press B again. Expected: "recruit peasant", the seconds left, then "at town hall 1".

2.10. Press N. Expected: the idle workers and how many are on each resource, such as "0 idle , 1 on gold".

2.11. Control two or more units, for example with Control+E once you have two peasants, and press V. Expected: the group's hit points against its most, then the units.

2.12. Move to a square where your peasant is and press M. Expected: "stronger , 1 peasant against nothing!".

2.13. Hold Shift, press X, release Shift. Expected: "gold , wood" with their amounts, and nothing else first.

2.14. Press F12. Expected: each computer's name, then "enemy". Press F11: the names only.

2.15. Select a peasant or footman and press Shift+V. Expected: its damage, range and armor, then its hit points and speed.

2.16. Play against an aggressive computer until you hear "alert at" a square. Move the cursor elsewhere, then press Delete. Expected: the cursor moves to that square and its description is said.

2.17. Press F5 or Comma until you hear a message, then press Control+Shift+C. Expected: "copied". Paste into a text editor: you should get that message's words. If the editor gets nothing or old text, the clipboard writer is failing, which I suspect it may.

2.18. Create two footmen, give one an order, and press Shift+F on their square. Expected: only the idle footman is selected. Press Control+Shift+F: every idle footman on the map.

## Part 3. Settings

3.1. Open Options, then Sound and speech. Arrow through the items. Expected: each says its name first, then its value.

3.2. On Volume, press End twice, quit the game and start it again. Open Sound and speech. Expected: the volume says 30 percent, and the game's sounds are quieter than before.

3.3. On Speech rate, press Enter. At your usual NVDA speech rate, press a key, and press a key again the moment the sentence ends. Expected: it says a number of characters a second. Tell me the number.

3.4. After that, repeat step 1.5. Expected: no worse than before, and better if it was off.

3.5. On Economy, press Enter until it says "kept silently in the log". Start a game and gather. Expected: no gold or wood counts are spoken, and F4 counts them under "economy". Set it back afterwards.

3.6. In Single player, choose Start a game on, pick a map, start it and quit. Choose Start a game on again and press Down. Expected: the map you just played, once. Press Down again: the map after it, not the first of the list.

3.7. Copy `203.0.113.7` from a text editor. In Multiplayer, choose Enter the IP address of the server and press Control+V. Expected: the address is spelled. Press Escape.

## Part 4. Saves, the story and objectives

4.1. In a training game, after a minute, press F10, Up twice to "save game", then Enter. Expected: "saved", and the game goes on.

4.2. Press F10 and choose "cancel game". In the single-player menu, open "load a game". Expected: two entries, "last game" first, each with the map, the game time and how long ago.

4.3. Choose "last game". Expected: "Loading", then "loaded" and a square. Your units, cursor and selection are where you left them.

4.4. Load again and press Escape while it loads. Expected: "loading stopped". On a short game it may load before you can press.

4.5. In the list, press Delete on an entry, then choose "delete". Expected: "delete this saved game?", then "deleted".

4.6. Save a campaign mission, quit, then load it. Expected: it plays with the mission's own sounds, and winning it unlocks the next chapter.

4.7. Start the campaign's first mission. While its intro is told, press F10. Expected: the game menu opens. Press Escape: the intro goes on.

4.8. In chapter 1, after both objectives are announced, press F9 three times. Expected: "build 1 farm , not done , 1 of 2", then the barracks objective, then the farm again. Shift+F9 goes back.

4.9. During a story, press F1, and while help is speaking press Escape. Expected: the help stops at once.

4.10. During a story, press Control+Shift+C. Expected: "copied", then the line again, and the story goes on.

4.11. In a training game, let the computer beat you. Expected: "You have been defeated" is said while the battle sounds carry on.

### Listening to a recorded game

Since pull request 80. First play a training game on jl1 against the quiet computer for a couple of minutes, then quit it from the game menu.

4.12. In the single-player menu, choose "listen to a game", then press Down. Expected: "last finished game , jl1" and its length and age.

4.13. Press Enter. Expected: "watching" and your login, then "objective: neutralized enemy", then the game as you heard it when you played.

4.14. Press F3. Expected: the time, "of" the length, then "speed normal".

4.15. Press Shift+Space. Expected: "paused". Wait five seconds: nothing moves. Press Z: your gold. Press Shift+Space again: "speed normal".

4.16. Press Equals. Expected: "speed fast 2". Press Minus: "speed normal".

4.17. Press the right bracket. Expected: a time about a minute later, "of" the length, and no news of the skipped minute. Press the left bracket: back about a minute.

4.18. Press Control+F12. Expected: "watching" and the computer's name. You hear its units and resources when you ask, but no events. Control+Shift+F12: "watching" and your login.

4.19. Press Backspace. Expected: "watching: you give no orders". Pressing it again does not repeat that.

4.20. Press F10, then K, then Enter. Expected: "keep this game's record", then "kept in the records folder".

4.21. Press Equals until you hear "speed fastest, silent". Expected: only percentages, then "end of the recording", the result and the score, then the single-player menu.

4.22. Throughout: does NVDA pass Minus, Equals, the brackets and Shift+Space to the game, or does it take any of them?

## Part 5. What you should not know, sound behind you, and upgrades

Use headphones for 5.4 to 5.6.

5.1. In a training game on jl1 against a quiet computer, press Left from your start square. Expected: "a 1, meadows, in the fog of war, 1 gold mine".

5.2. Look at a computer peasant while it walks, then move your unit away. Go back to that square while it is in the fog. Expected: the remembered peasant makes no woodcutting or gold-mining noise.

5.3. See an enemy soldier, lose sight of it, and wait a few minutes. Tab to the remembered unit. Expected: only the upgrades it had when you saw it. When you see it again, it says them all.

5.4. Stay on your start square. Expected: your town hall and peasants sound as they always have.

5.5. Press Up to the square north of your start. Your base is now behind you. Expected: its sounds are quieter, as before, and also duller. Press Down: clear again.

5.6. Move between squares while units are busy. Expected: no clicks or crackles as sounds get louder, quieter or stop.

5.7. Select your town hall, choose the rallying point order, aim it at another square and press Enter. Upgrade the town hall to a keep and wait. Train a peasant in the keep. Expected: it walks to that square.

5.8. Select two archers and press Control+6. Order both to upgrade to dark archer and wait. Select another unit, then press 6. Expected: "You control 2 dark archer".

5.9. Build two scout towers. Upgrade the first to a guard tower from the order menu and wait. Select the second and press Alt+R. Expected: it names the upgrade to guard tower and its cost, and does not say "requires sawmill" if you have one.

## Part 6. The computer players

These changed how hard the game is. There is no right answer, only your judgment.

6.1. Play a training game against the easy computer on jl1. Does it feel different from what you remember?

6.2. Play against the aggressive computer on jl1. It now attacks about a minute sooner. Is it still a fair fight?

Since pull request 77 the computers know only what a player in their seat knows. Measured over many games, they play about as strongly as before.

6.3. In the single-player menu, start a training game and go through the list of opponents. Expected: one is named "random computer with unlimited resources and no requirements". Is that name too long to sit through in a menu? A shorter one is "random computer, unlimited resources".

6.4. Play against the aggressive computer on jl1 once more with this in mind: does it ever seem to know something it has not seen, such as where your units are before it has scouted them?

## Part 7. With one other player

One of you hosts from the game: Multiplayer, then Server. The game copies its address; send it to the other player.

7.1. The other player chooses Multiplayer, then "enter the IP address of the server", pastes with Control+V, and presses Enter. Expected: "welcome" and the server's name.

7.2. You choose "start a game on", jl1, normal speed, and invite them. They choose "I am ready". You choose Start. Expected: both hear the objective. There is no countdown.

7.3. Play for a minute. Expected: each hears only what their own units see.

7.4. You press F10 during the game. Expected: "Menu", and the game's sounds and news carry on behind it. Press Escape.

7.5. The other player presses F10 and chooses a speed. Expected: "only the organizer can change the speed".

7.6. You press F10 and choose fast. Expected: both hear "speed fast set by" and your login.

7.7. You press F10 and choose "pause". Expected: both hear "paused by" and your login. In the menu again it reads "resume"; choose it: "resumed by" and your login.

7.8. The other player switches their Wi-Fi off for 30 seconds. Expected: within about 15 seconds they hear "connection lost, trying to return", and you hear their login and "lost the connection".

7.9. While it is off, they ask a status question, then give an order. Expected: the question is answered. The order says "order not taken: not connected", once.

7.10. They switch the Wi-Fi back on. Expected: within a few seconds they hear "back in the game", and you hear their login and "is back". Their units are where the game now has them, and no sound is stuck looping.

7.11. They switch the Wi-Fi off for more than 2 minutes. Expected: "could not return:" and a reason, then the multiplayer menu. You hear that they did not come back.

7.12. In a new game, they switch the Wi-Fi off, wait for "connection lost, trying to return", then press Escape. Expected: their game ends at once.

## Part 8. With two other players

Call them Bob and Carol.

8.1. You choose "Start a public game on" and pick a map. In the speed menu, go down to "watchers". Expected: it says "welcome". Choose Normal.

8.2. Carol joins the server and chooses "watch", your login and the map. Expected: Carol hears the map and "watchers welcome". You hear "carol is watching".

8.3. Bob joins and says he is ready, and you start. Expected: Carol hears "watching" and your login, then the game as you hear it.

8.4. Carol presses Control+D. Expected: "watching: you give no orders". Pressing it again does not repeat that.

8.5. Carol presses Control+F12. Expected: "watching bob". Control+Shift+F12 goes back to you.

8.6. Carol presses F7 and writes a line. Expected: you and Bob hear "carol watching says" and the line. You press F11: the players, then "watching: carol".

8.7. Carol presses F10 and chooses "cancel game" once. Expected: she leaves at once, and both players hear "carol stopped watching".

8.8. Start a new game on jl2 with Bob in your alliance and Carol on her own. Move to any square and press Control+F7. Expected: you hear "flare sent". Bob hears an alert sound, then your login, "flare at" and the square. Carol hears nothing.

8.9. Bob presses Delete. Expected: the square you pointed at.

8.10. You press Control+F7 four times quickly. Expected: "flare sent" three times, then "too fast: wait a moment".

8.11. You press F10 and choose "pause", then F10 and "cancel game", once. Expected: your game ends at once. Bob and Carol hear that you left the game, then "bob is now the organizer".

8.12. Bob opens the game menu. Expected: it has "resume". He chooses it, and everyone hears "resumed by bob".

## Part 9. Optional

9.1. NVDA says when speech ends. Run `pwsh scripts/fetch-nvda-controller.ps1`, then in Options, Sound and speech, turn on the NVDA item. Repeat steps 1.4 to 1.6. Expected: no silences between queued messages and nothing cut off early, at any speech rate. If the game goes silent, press Alt+F4 and turn the setting off in `SoundRTS.ini` with `nvda_direct = 0`.

9.2. Encrypted connections. In Multiplayer, choose "enter the IP address of the server" and enter `wss://wrong.host.badssl.com`. Expected: "not connected: the server's certificate is for another name". With `wss://expired.badssl.com`: "is out of date". With `wss://self-signed.badssl.com`: "is not trusted". Nothing offers to connect anyway.

9.3. In the options menu, turn on the orc mod and start a training game. Expected: F11 says each player followed by human or orc, and Shift+F11 says your own faction.

## Part 10. The second listening-room session

This decides the sound work, which has not started. Use headphones.

```powershell
cd C:\Users\andrew\.t3\projects\sound-rst
$env:LIBCLANG_PATH = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\Llvm\x64\bin'
cargo run -p listening-room --features binaural -- --root vendor/soundrts-upstream
```

It runs seven steps over four renderers and saves your answers in `listening-room-results.txt` in that folder. Type `q` at any question to stop; what you answered so far is kept. Send the file back.
