# Single-player games

A player starts a game against computer players on a map, or a chapter of the campaign. In the game the player moves a cursor over the squares of the map, hears what is there, selects units, gives orders, and hears news. The game ends with a victory or a defeat and a score.

## Sub-features

- `single-menus` (harness): the single player menu, the map list, and the players menu, where the player invites computers, starts, or cancels.
- `single-game` (harness): a game on the map jl1 against a quiet computer, with every command of `res/ui/bindings.txt` except `fullscreen` and `say`, the game menu, the console, quitting, and the score.
- `single-long` (harness): about 1,700 turns on jl5 against an aggressive computer, with sightings, alerts, losses, defeat, and the score.
- `single-events` (harness): research, an impossible order and its reason, an exhausted gold mine, fights, victory, loading and unloading a boat, and an order to a square.
- `single-campaign` (harness): the prologue cut scene, chapter 1 with its objectives, the victory and defeat menus, and the chapter bookmark in `campaigns.ini`.
- `single-french` (harness): the main menu and a short game in French.
- `single-heard` (ears): a screen reader says the game's messages while the positional sounds play, and news waits its turn.

## How to get to it (user POV)

- In the main menu, press Enter on `single player`.
- For a game against the computer, choose `start a game on`, a map such as `jl1 1`, then `invite quiet computer` or another computer, then `start`.
- For the campaign, choose `campaign`, then a chapter such as `prologue`.
- In a game, press F10 for the game menu. `cancel game` in that menu ends the game.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` ends with `doctor: ready`.

- **Play the recorded games.** A scripted player goes through the single-player menus, plays the games and the campaign listed above, and quits. Run `verify.ps1 test single-player transcripts menus_single training_jl1 training_long campaign events transport square_orders french`. Eight tests pass, and the client's output matches the recorded transcripts byte for byte. In `single-player/transcripts/training_jl1.txt`, the step `key RETURN` on `start ,` is followed by `voice confirmation "start"` and `voice info "objective: neutralized enemy"`. In `campaign.txt`, the chapter starts with `voice important "chapter 1"`.
- **Play a training game through the menus.** A scripted player starts a game on jl1 from the main menu, moves the cursor, gives orders, and waits for their results. Run `verify.ps1 test single-player game`. Every test of `crates/client/tests/game.rs` passes.
- **Win chapter 1.** A scripted player hears the prologue, builds the farm and the barracks that the objectives ask for, and wins. Run `verify.ps1 test single-player campaign`. Every test passes, and the test reads the bookmark that the client wrote to `campaigns.ini`.

### By ear

For Andrew, with NVDA running. Each step passes when NVDA says the quoted words.

1. In a terminal, run `cargo run -p soundrts -- --root vendor/soundrts-upstream`. Wait for the main menu.
2. Press Down, then Enter. You hear "single player", then "please make a selection".
3. Press S, then Enter. You hear "start a game on" three times: the item, your choice, and the title of the map list.
4. Press J until you hear "jl1 1", then press Enter. You hear "jl1 1", then "please make a selection".
5. Press Down, then Enter. You hear "invite quiet computer" twice, then "please make a selection".
6. Press Down until you hear "start", then press Enter. You hear "start", then "objective: neutralized enemy". The sounds of your base start.
7. Press Z. You hear "10 gold".
8. Press F10. You hear "Menu". Press Down. You hear "cancel game".
9. Press Enter. You hear "cancel game", then "Defeat" with the time played, the statistics, and a score, then "please make a selection".
10. Press Escape twice. You hear "go back to the previous menu", then "quit", and the game closes.

## Gotchas

- The transcripts fix the random seed. A real game draws its own seed, so the computer's moves differ from the transcript.
- In single-player the game waits while a story is told, and while the game menu or the console is open. A game on a server never waits.
- The `events` scenario and the campaign test use the console and its cheats to reach their states. They do not prove that a player can reach those states by playing.
- The first square of a game is said up to a tenth of a second after the game starts and cuts off what the first key said.
- Saved games, replays, and the on-screen map are not ported. The window stays blank.
- The harness proves two languages, English and French. The other 11 have no harness proof.
