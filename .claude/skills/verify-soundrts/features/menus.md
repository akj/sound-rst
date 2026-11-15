# Menus

The game opens on a spoken main menu. The player moves through a menu with the arrows, Tab, or a first letter, hears each item, chooses with Enter, and goes back with Escape. The options menu changes the login, the sound and speech settings, the mods, and the soundpacks.

## Sub-features

- `menus-main` (harness): the main menu says its title and each item, wraps around, searches by first letter, says its help, and Escape quits.
- `menus-options` (harness): the options menu changes the login from typed keys, refuses a bad login, and opens the mods and soundpacks menus.
- `menus-sound-and-speech` (harness): each item of Sound and speech says its name, then its value. Enter changes the value, and the game writes the change to `SoundRTS.ini`.
- `menus-volume-keys` (harness): Home and End change the volume in a menu and say the new level.
- `menus-paste` (harness): Control V pastes the clipboard into a text field and keeps only the characters the field allows.
- `menus-heard` (ears): a screen reader says each of these messages, in order, and a new key cuts the message before it.

## How to get to it (user POV)

- Start the game. The main menu speaks first.
- Press Down, Up, Tab, or Shift Tab to move. Press a letter to go to the next item that starts with it.
- Press Enter on `options` for the options menu, then Enter on `Modify your login`, `sound and speech`, `Mods`, or `Soundpacks`.
- Press Escape to go back one menu. In the main menu, Escape quits.

## Driving it with verify.ps1

Preconditions:

- `verify.ps1 doctor` ends with `doctor: ready`.

- **Walk the main menu and the options menu.** A scripted player presses Down, Up, Tab, letters, F1, Home, End, and Escape in the main menu, then types a login and visits each options submenu. Run `verify.ps1 test menus transcripts menus_main menus_options`. Two tests pass, and the client's output matches the recorded transcripts byte for byte. In `menus/transcripts/menus_main.txt`, `step 1 key DOWN` is followed by `voice item "single player, , play on your computer"`, and the last step, `key ESCAPE`, by `voice confirmation "quit,"`.
- **Check the menu rules.** A scripted player covers the same menus in English and French, the first-letter search, a menu that remembers its last choice, the volume keys, and Control V in a text field. Run `verify.ps1 test menus menus`. Every test of `crates/client/tests/menus.rs` passes.
- **Change the sound and speech settings.** A scripted player opens Sound and speech, changes the volume, each news category, and the NVDA item, and measures the speech rate. Run `verify.ps1 test menus sound_and_speech`. Every test passes. Tests such as `the_volume_is_saved_when_it_changes_and_survives_a_restart` and `the_nvda_item_switches_and_is_written_to_the_file` read the values back from the `SoundRTS.ini` that the client wrote.

### By ear

For Andrew, with NVDA running. Each step passes when NVDA says the quoted words.

1. In a terminal, run `cargo run -p soundrts -- --root vendor/soundrts-upstream`. You hear "SoundRTS 1.3.8.1, please make a selection".
2. Press Down three times, with a pause after each. You hear a click and "single player, play on your computer", then "multiplayer, play on the Internet", then "options, modify your parameters".
3. Press Down twice quickly. The second item cuts the first one off.
4. Press O, then Enter. You hear "options, modify your parameters", then "options", then "options menu".
5. Press Down twice. You hear "Modify your login", then "sound and speech".
6. Press Escape. You hear "go back to the previous menu", then the main menu's title.
7. Press Escape. You hear "quit", and the game closes.

## Gotchas

- Up as the first key in a menu says the last item, but Enter then chooses nothing. The original game does the same, and the transcripts record it.
- In the Mods menu, the back item also closes the options menu.
- Choosing a mod or a soundpack opens a new main menu inside the old one. Quitting it closes the game.
- The harness types on a US keyboard layout. No harness test proves another layout.
- A transcript changes when the spoken text changes on purpose. The transcripts README in `crates/client/tests/transcripts` says how to record again. Review the diff of `expected/` before you report a pass on new recordings.
