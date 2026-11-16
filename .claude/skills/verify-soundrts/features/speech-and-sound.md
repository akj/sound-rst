# Speech and sound

For a blind player, what the screen reader says and what the audio device plays is the whole interface. The game sends each message to NVDA or JAWS, or to the system voice when no screen reader runs, estimates when the message has ended, and then sends the next one. Most of this feature has proof by ear only.

## Sub-features

- `speech-queue` (harness): the voice says the answer to a key at once, makes news wait its turn, says a cut-off news message again, and keeps every message in a log.
- `speech-written-record` (harness): with `debug_mode = true` in `settings.toml`, `tmp/client.log` gets one `SAID:` line for each message.
- `speech-detected` (program): the built program names the screen reader it would speak through.
- `speech-heard` (ears): the screen reader says each message, and a new key silences the message before it.
- `speech-pace` (ears): messages follow each other without a long silence and without one cutting off the other, at the player's own speech rate, after the Speech rate item has measured it.
- `speech-nvda-direct` (ears): with `nvda_direct = true`, the next message starts when NVDA has finished the current one. The README says that nobody has checked this route by ear yet.
- `sound-heard` (ears): the game's sounds play on the audio device, at the volume set, to the left or the right of the listener.

## How to get to it (user POV)

- Start the game with a screen reader running. Every menu and every game speaks.
- In the options menu, open `sound and speech` for the volume, the news categories, the NVDA item, and the speech rate.
- Set `debug_mode = true` in `settings.toml` for the written record.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` ends with `doctor: ready`.

- **Check the rules of the voice.** A test queues messages of each kind on the recorded speech device and presses keys while they are said. Run `verify.ps1 test speech-and-sound voice`. Every test of `crates/client/tests/voice.rs` passes. On the recorded device a message lasts a tenth of a second for each character, so these tests prove the order and the text of what is sent, not when a screen reader stops speaking.
- **Check the written record.** A scripted player walks the main menu with `debug_mode = true`. Run `verify.ps1 test speech-and-sound menus in_debug_mode_the_log_has_what_the_voice_said`. One test passes. It reads the `SAID:` lines back from the client's log.
- **Name the screen reader.** Run `verify.ps1 doctor`. The line `ok: the built program starts and finds the game data. It would speak through:` ends with the screen reader that the program found, for example `NVDA`. This proves detection only. Nothing was spoken.

### By ear

For Andrew, with NVDA running at your usual speech rate.

1. In `%APPDATA%\SoundRTS\settings.toml`, set `debug_mode = true`.
2. In a terminal, run `cargo run -p soundrts -- --root vendor/game-data`. NVDA's own voice says "SoundRTS 1.3.8.1, please make a selection".
3. Press Down four times, with a pause after each. You hear a click, then the whole item, each time.
4. Press Down four times quickly. Each item silences the one before it at once.
5. Open `options`, then `sound and speech`. Press End, then Home. You hear the new level each time, for example "40 percent of the volume", then "50 percent of the volume". The click is quieter after End.
6. Go down to "speech rate" and press Enter. Follow what you hear: press a key, listen to the sentence, and press a key the moment the sentence ends. You hear "measured:" and a number of characters a second.
7. Go back to the main menu and start a game on jl1 with a quiet computer. While the sounds of your base play, the news messages follow one another without a long silence, and none cuts another off.
8. Quit the game. Before you start it again, send `%APPDATA%\SoundRTS\tmp\client.log`. Its `SAID:` lines are the text the game sent. Say where what you heard differs from them.

Run these steps only when the change touches `nvda_direct`:

9. Run `pwsh scripts/fetch-nvda-controller.ps1`, then build and start the game again.
10. In `sound and speech`, press Enter on "NVDA tells the game when it has finished speaking". You hear "on". Quit and start the game again.
11. Repeat steps 3, 4, and 7. Each queued message starts as soon as NVDA has finished the one before it.
12. During a long message, press Control. NVDA falls silent, and the next queued message starts.

## Gotchas

- A passing harness test says nothing about NVDA. `docs/open-findings.md` states the limit: every speech fix is proved on the recorded speech device only.
- The recorded speech device never reports the end of a message, as most screen readers do not. The rules of the `nvda_direct` route have unit tests against a fake NVDA in `crates/client/src/tts/nvda.rs`. The helper does not run unit tests, and those tests do not load the real controller client.
- The transcripts record which sounds start and with which left and right volumes. No test opens the audio device.
- The game empties `tmp/client.log` at each start. Copy the log before the next start.
- Whether a sound is in front or behind, and how far, is judged in the listening room, a separate program that this map does not cover.
- Changing audio devices, NVDA, or its speech settings is Andrew's decision. An agent changes none of them.
