# sound-rst: what still needs changing

33 open findings from the design review of the Rust port of SoundRTS, grouped by what they affect: what you hear, multiplayer, gameplay, and code structure. The six multiplayer fixes, the game record and the recording cutover are merged and are not repeated here.

Written 14 November 2026 against commit `612afd1` on `main`. Status updated 15 November 2026 against commit `1a1f1f7`, after pull requests 6 to 54.

## Status on 15 November 2026

Each status below was checked by reading the code at `1a1f1f7`, not from pull request titles. A fixed finding names the pull request and the test or the code that shows it. Where no test would fail if the problem came back, it says "fixed by reading, no test".

Two limits apply to every status.

- **What you hear.** Every fix is proved on the recorded speech device only. You have not yet reported on a listening check of the game itself. So "fixed" for a finding about what you hear means fixed in the recorded speech tests, not yet heard through NVDA.
- **Multiplayer.** Every fix is proved only by tests through a real server on 127.0.0.1, or against the client's fake server where a finding says so. Nobody has played over a real network.

Fixed, 10 findings:

- H1, H2, H3, H4, H5, H8
- M3, M7, M9
- G3

Partly fixed, 11 findings:

- H6
- M1, M2, M4
- G1, G5
- C1, C2, C4, C6, C7

Open, 7 findings:

- H7
- M5, M6, M8
- G2, G4
- C5

Changed, 3 findings: the finding no longer describes the code, and something different is true now.

- M10, M11, M12

Decided against, or put off on purpose, 2 findings:

- C3, C8

That is 33. Seventeen findings from the work since, N1 to N17, are at the end.

## How far each finding was checked

Every finding comes from reading code. Nothing here was measured, and nothing was heard through a screen reader. Each finding carries one of three labels.

- **Checked today:** I read the code at commit `612afd1` and the finding still holds. File and line references on these findings are current.
- **From the review:** a reviewer traced it in the code before the three merges and cited it. I have not re-read it since, so I name the file but not the line.
- **Suspected:** inferred from the code and never traced or reproduced.

These labels and their line numbers are from `612afd1`. On 15 November, file paths were brought up to date only in findings that are still open; the status paragraph of every finding names the current code.

The reviewers were three Opus agents (simulation, client, and the original game's design) and one Sol investigation (multiplayer). Sizes are my estimates: small is under a day of agent work, medium is one pull request, large is several.

**One correction to my earlier summary.** I said the computer player cannot play a mod faction because unit names are hard-coded. That overstated it. The names go through each faction's table of equivalents, so the orc mod works. The accurate finding is G1.

## Where I would start

Status, 15 November: items 1 and 3 are built and wait for your ears. Item 4 is mostly done: G3 is fixed and G1 partly. Item 2 is half done, on the server side. This list is kept as it was written.

1. **Speech and news, H1 to H4, as one piece of work.** It changes what you hear in every game, and H2 and H3 were blocked until the cutover. It needs your ears at the end.
2. **Reconnecting after a dropped connection, M1 with M2.** It builds directly on the game record that is now merged.
3. **The questions you cannot ask, H5.** Each is a small new command on a new key.
4. **Computer fairness and inherited bugs, G1 and G3.** Each fix changes the recordings, so batch them and re-record once.

I would not start a structure project by itself. Typed commands (C1) and a player view (C2) each earn their cost only when a feature needs them, and each finding says which feature that is.

## What you hear

Speech, news, keys, settings and sound. For a blind player this is the whole interface.

### H1. The game guesses when speech has ended

**Fixed** in pull requests 11 and 27, in the recorded speech tests; not yet heard through NVDA. Speech runs on its own thread. With `nvda_direct = 1` in `SoundRTS.ini`, or the item in Options, Sound and speech, NVDA reports when speech ends; it is off by default until you have heard it. Other screen readers still use the guess, now scaled by a speech-rate measure in Options, Sound and speech. Stop always reaches the screen reader (`Tts::stop` in `crates/client/src/tts.rs`). Tests: `stop_reaches_the_screen_reader_when_the_estimate_says_speech_is_over` in `crates/client/src/tts.rs`; `a_finished_report_starts_the_next_queued_message_without_waiting_for_the_estimate` in `crates/client/tests/voice.rs`; `the_speech_rate_is_measured_from_the_start_of_the_sentence_to_the_key` in `crates/client/tests/sound_and_speech.rs`. The line numbers below are from before.

Checked today. Size: small for the setting, medium for the exact NVDA fix. Needs your ears.

**Now.** The client assumes each character takes 0.1 seconds to speak, with digits counted longer. It never asks the screen reader. That guess decides when queued news starts, when the next part of a message plays, and whether a cut message counts as heard.

**Effect.** At a fast speech rate you get silence before each queued message. At a slow rate the next message cuts the current one off. The stop keys only reach the screen reader while the guess says speech is still going, so they cannot silence speech that runs past it.

**Where.** `crates/client/src/tts.rs`: `estimate_us` at line 53, `stop` at line 73, `is_speaking` at line 81.

**Simplest change.** A speech-rate setting in the options menu that scales the guess, and make `stop` always reach the screen reader. This gives up exactness: you tune it once by ear.

**Exact alternative, NVDA only.** From memory, NVDA's newer controller client can speak marked text and call back when a mark is reached, which would report the real end of speech. Verify that against NV Access's documentation before planning on it. The controller client bundled with Tolk is the 2020 build and exports only `speakText`, `cancelSpeech`, `brailleMessage` and `testIfRunning`; I inspected that file earlier. JAWS and the system voice would still use the guess.

### H2. Some spoken messages stop the game

**Fixed** in pull request 29, in the recorded speech tests; not yet heard through NVDA. In a game, `Voice::say_now` in `crates/client/src/voice.rs` no longer waits. An important message is queued as system or story news. In single player a cut scene pauses the world on purpose while the keys stay live; on a server it pauses nothing. Tests in `crates/client/src/game/tests.rs`: `an_important_message_is_said_after_the_news_being_said_and_before_the_news_waiting_while_the_world_goes_on`, `a_cut_scene_stops_the_world_but_not_the_keys_and_once_skipped_the_world_goes_on`, and, against the client's fake server, `on_a_server_neither_a_cut_scene_nor_an_important_message_stops_the_turns`. Still blocking, on purpose: a campaign's cut-scene chapter between missions, when no game is running.

Checked today. Size: medium. Changes the transcripts. Needs your ears.

**Now.** Messages the game marks as important are spoken as blocking messages, and a cut scene is a run of them. The game loop stops until the message is said or a key is pressed, and keys are checked only every 100 milliseconds meanwhile.

**Effect.** The world and its sounds freeze during the message. In a server game that client stops taking turns. After 11 turns the server makes every player wait, and after 60 seconds it removes the player. This is the one stall the game-menu fix did not cover.

**Where.** `crates/client/src/game/mod.rs`, `process_srv_event` at line 478. `crates/client/src/voice.rs`, `say_now` at line 437.

**Simplest change.** Queue these messages with priority, as chat already is, and keep the loop running. Do it for server games first, as the game menu did. This gives up the guarantee that nothing happens while an important message is spoken; mission cut scenes can keep blocking.

### H3. News is dropped while you are busy

**Fixed** in pull requests 11 and 24, in the recorded speech tests; not yet heard through NVDA. Point by point:

- A message cut off comes back, however long it played: `news_cut_off_after_more_than_a_second_is_said_again` in `crates/client/tests/voice.rs`.
- The 3 second rule (`EVENT_LIMIT` in `crates/client/src/game/mod.rs`) now drops only the sound: `news_handled_late_is_said_without_its_sound` in `crates/client/src/game/alerts.rs`.
- The resource expiry and the attack throttle are deleted. Newer news about the same subject replaces unheard older news instead: `newer_news_about_a_subject_replaces_the_unheard_news_about_it_and_not_another` in `voice.rs`.
- An attack on a square is one alert until that square has been quiet for 10 seconds (`ATTACK_QUIET` in `alerts.rs`). This is new, chosen in pull request 24. Whether a long siege should remind you more often is for your ears.
- The history keeps 1000 messages and drops the oldest one at a time: `the_log_drops_only_its_oldest_message_past_its_limit` in `voice.rs`.

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

**Fixed** in pull request 26, in the recorded speech tests; not yet heard through NVDA. A modifier pressed alone acts when it is released, and only if no other key went down meanwhile. A held modifier never repeats, and other keys repeat as Windows repeats them (`EventQueue` in `crates/client/src/input.rs`). Tests in `crates/client/tests/keys.rs`: `control_alone_examines_when_it_is_released`, `control_held_with_tab_does_not_examine`, `a_held_modifier_does_not_repeat`, `shift_and_an_arrow_do_not_skip_the_news_being_said`. One rare case remains, which pull request 26 declined to fix: with two other keys already held, such as Control and Shift, pressing and releasing Alt runs Alt's command and skips the news. A chord pressed the usual way, modifiers first and then the other key, no longer does. NVDA itself silences speech when Control goes down, which no test can show.

Checked today; the key repeat is suspected. Size: small. Changes the transcripts. Needs your ears.

**Now.** Control alone runs "examine". Alt or Shift alone stops the current message. Both fire on key press, so every Control chord first speaks the examine text, and every Shift or Alt chord first skips whatever news was being said.

**Suspected.** Holding a modifier for more than half a second repeats its command every 100 milliseconds, because the game's own key repeat covers modifiers.

**Where.** The bindings come from the original's `res/ui/bindings.txt`. Key releases already reach the game and are ignored: `crates/client/src/game/mod.rs` line 450.

**Simplest change.** Run a modifier-only command on release, and only if no other key was pressed in between. This gives up immediacy: examine speaks when you let go of Control.

### H5. Questions you cannot ask the game

**Fixed** in pull requests 24 and 28, in the recorded speech tests; not yet heard through NVDA. One question was decided against.

- Where am I under attack: alerts stay in the log. Slash reviews the alert category, Comma and Period step through it, and F4 counts unheard alerts (pull request 24).
- Jump to the last alert: Delete (pull request 24). Test: `the_jump_goes_to_the_square_of_the_last_alert_then_the_one_before` in `crates/client/src/game/alerts.rs`.
- Production everywhere is B, the workers N, the group's health V with more than one unit, allies, enemies and the defeated F12, and every resource Shift+X (pull request 28). Tests in `crates/client/src/game/questions.rs`, such as `production_is_said_for_every_building_soonest_done_first` and `every_resource_of_the_rules_is_said`.
- The score before the end: decided against in pull request 28. Kills include units you never perceived, so a running score would tell you what an enemy transport carried.

Pull request 50 added more of the same kind: objectives one at a time (F9), how a unit fights (Shift+V), and factions (F11 and Shift+F11).

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

**Partly fixed** in pull requests 24, 26 and 27. Fixed, in the recorded speech tests and not yet heard through NVDA:

- The volume is saved and restored: `the_volume_is_saved_when_it_changes_and_survives_a_restart` in `crates/client/tests/sound_and_speech.rs`.
- The game menu's speed starts the next single-player game: `the_speed_chosen_in_the_game_menu_starts_the_next_game` in the same file.
- Options, Sound and speech sets the volume, each category of news, NVDA's report of the end of speech, and the speech rate. `verbosity` is replaced by a `[news]` section with one setting per category (pull request 24).
- Key repeat follows Windows (pull request 26): `a_held_key_repeats_at_the_systems_delay_and_rate` in `crates/client/tests/keys.rs`.
- A volume key no longer cuts speech or stops sounds: `a_volume_key_interrupts_with_the_level_only_and_the_sentence_is_said_in_full` in `sound_and_speech.rs`.

Not fixed: game keys can still be remapped only by editing a file, and menu keys not at all. Pull request 46 built a key-remapping menu and left it out for a separate decision, because it came to about 680 lines.

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

**Open.** Unchanged. The README lists the on-screen map as not ported. The paths below are updated.

Checked today. Size: medium. A decision for you.

**Now.** The window is 400 by 75 and nothing is drawn in it. Control F2 says the display is on, and nothing changes. The original drew each spoken message as a subtitle, and a grid of the map.

**Effect.** A sighted helper or a low-vision player sees nothing.

**Where.** `crates/client/src/window.rs`, the `with_inner_size` call. `crates/client/src/client.rs`, `toggle_fullscreen`.

**Simplest change.** Draw the last spoken message as text. Draw it as pixels: a native text control might be read a second time by the screen reader (suspected). If the window should stay blank, remove the Control F2 announcement instead.

### H8. Front and back sound the same

**Fixed** in pull request 21, by tests on the audio output's samples; not yet heard in the game. Your listening-room session (pull request 12) chose this renderer, but nobody has heard it inside the game. A sound behind is duller as well as quieter, and volume changes glide over about 11 milliseconds (`behind` in `crates/client/src/audio.rs`). Tests in `crates/client/src/rodio_output.rs`: `a_source_behind_is_duller_than_the_same_source_in_front`, `a_change_of_gains_moves_evenly_over_the_ramp_and_settles_exactly`, `a_stopped_sound_fades_out_and_frees_its_channel_at_once`.

Checked today; the clicks are suspected. Size: medium. Needs your ears.

**Now.** A sound's position becomes left-right pan and loudness. A sound behind you is the same sound at half volume. Volume changes are applied with no smoothing, which may click.

**Where.** `crates/client/src/audio.rs`, `stereo`, line 252.

**Simplest change.** Muffle sounds behind the listener with a low-pass filter, and ramp volume changes. Both fit below the audio output layer, so recordings do not change. Low priority unless you find front and back hard to tell apart.

## Multiplayer

Findings from the Sol investigation, plus leftovers from the three merged pull requests.

Since pull request 54, the server runs the game and each client plays from what the server tells it. Lockstep is deleted.

### M1. A dropped connection forfeits your army

**Partly fixed.** The server side is built (pull request 48). A dropped seat is held for 90 seconds with its units on their last orders, and the seat's token takes it back (`take_back` in `crates/server/src/game.rs`, `return_to_game` in `crates/server/src/lobby.rs`). Tests through a real server: `a_dropped_player_returns_with_their_token` and `a_player_not_back_in_time_has_quit` in `crates/server/tests/games.rs`. Not built: the client keeps the token but does not use it, so a player cannot return yet (pull request 54). That work is in progress in another branch. The simplest change below describes replaying turns; the server now sends the returning seat the whole current message instead.

From the review. Size: large.

**Now.** When your connection closes, the server treats it as leaving the game and your units are removed. Coming back under the same name puts you in the lobby.

**Where.** `crates/server/src/lobby.rs` and `crates/server/src/game.rs`.

**Simplest change.** The server keeps the turns it has released, holds a dropped player's slot for a short grace period, and gives the returning client the turns to replay. A token proves it is the same player. The game record merged in pull request 4 already has the format and the replay.

**Gives up.** Everyone waits during the grace period, and catching up on a long game takes time. Measure replay speed on a long record before committing.

### M2. A silently dead connection hangs the game

**Partly fixed** in pull request 48, on the server only. The server pings each connection every 5 seconds and closes it after 20 seconds of silence. Test through a real server: `a_dead_connection_is_closed_by_the_heartbeat` in `crates/server/tests/games.rs`. The client still has no deadline of its own: its connection loop, `serve` in `crates/client/src/net/conn.rs`, waits for the next frame with no time limit. So the effect below still holds for the client, and nothing is said meanwhile. Found by reading.

From the review; I confirmed today that the client and server contain no heartbeat. Size: small.

**Now.** If the network stops carrying traffic without closing the connection, the client waits for a turn that never comes until Windows reports the failure or you leave.

**Where.** `crates/client/src/net/conn.rs`.

**Simplest change.** A heartbeat with a deadline, and a spoken message when it passes. This can disconnect a player during a long outage that would have recovered. Do it with M1.

### M3. One player can stall the game again and again

**Fixed** in pull requests 48 and 54. The server runs the game on a clock that waits for nobody, so a slow player holds up no one. The removal timer went with lockstep. Test through a real server: `a_seat_that_stops_reading_is_caught_up_and_holds_up_nobody` in `crates/server/tests/games.rs`. The server also lets the room's host remove a player (`remove` in `crates/server/src/game.rs`; test `the_host_pauses_resumes_sets_the_speed_and_removes_a_player`), but the game has no key or menu item that asks for it yet.

Checked today. Size: small.

**Now.** The 60 second removal timer restarts each time a slow player catches up. A player who catches up just in time, on purpose or because their machine is overloaded, can hold everyone repeatedly and never be removed. The host has no way to remove a player.

**Where.** `crates/server/src/game.rs`, the `holding_since` field, cleared at lines 302 and 342.

**Simplest change.** Let the host remove a player through the existing leave path. This gives the host that power alone. The alternative is a total stall allowance per player, which needs a number chosen.

### M4. No pause and no speed change in a server game

**Partly fixed** in pull requests 48 and 54. Speed: the room's host changes it from the game menu, and every player hears who set it. The server half is tested through a real server, in `the_host_pauses_resumes_sets_the_speed_and_removes_a_player` in `crates/server/tests/games.rs`. The client half is tested against the client's fake server, in `the_rooms_host_sets_the_speed_of_a_game_on_a_server` and `only_the_rooms_host_sets_the_speed_of_a_game_on_a_server` in `crates/client/tests/multiplayer.rs`. Pause: the server can pause and resume, and the client says so when told. But the client has no key or item that asks for a pause, so nobody can pause a game on a server yet. Adapting to ping no longer applies, because the server's clock waits for nobody.

From the review. Size: medium.

**Now.** Speed is fixed when the game is created. Nobody can pause. The original adapted its turn timing to each player's ping; the port's clock is fixed.

**Simplest change.** The host can pause and change speed, taking effect at a turn every client agrees on. Adapting to ping automatically can stay out until play over the internet shows it is needed.

### M5. A game hosted from the game dies with its host

**Open.** Unchanged: the game shuts its server down when the hosting player leaves it (`connect` in `crates/client/src/multiplayer.rs`). The README's "Run a server" section now suggests the standalone server "on a computer that stays on", but it does not say that games on a server hosted from the game end with it.

From the review. Size: small, documentation only.

**Now.** If the hosting player's game crashes, their machine sleeps, or they leave the server, every game on that server ends.

**Simplest change.** Say so in the README and point to the standalone `soundrts-server` program for games that should outlive one player. Moving a running game to another host is far more machinery, and I would not build it.

### M6. Player names are not verified

**Open.** Unchanged. The seat token of pull request 48 proves only that a returning player held that seat in a game; it does not reserve a name. The path below is updated.

Checked today. Size: medium. Matters only on a public server.

**Now.** A name is taken on request. The only check is that nobody connected already has it, ignoring case. Someone can connect under an absent player's name and receive invitations meant for them. They cannot take over a player who is connected.

**Where.** `crates/server/src/lobby.rs`, `unique_login`.

**Simplest change.** Optional reserved names with a token per name, set up on the server. This adds setup work and identifies players on that server only.

### M7. A public server can be overloaded

**Fixed** in pull request 16, through a real server on 127.0.0.1. At most 32 connections may wait to say hello (`MAX_PENDING_HELLOS` in `crates/server/src/lib.rs`). Each connection may have 1 MiB queued to the lobby or its game, counted in bytes until handled (`MAX_QUEUED_BYTES` in `crates/server/src/connection.rs`). Maps have their own allowance and a rate limit. Tests in `crates/server/tests/connect.rs`: `connections_that_have_not_said_hello_are_limited`, `a_message_larger_than_the_server_holds_closes_the_connection`, `a_flood_of_messages_is_slowed_down`; `maps_asked_for_are_rate_limited` in `crates/server/tests/rooms.rs`; and `an_allowance_counts_what_is_queued_until_it_is_handled` in `connection.rs`. The game task of pull request 48 is fed through the same allowance (`pass_to_game` in `crates/server/src/lobby.rs`). The real effect on a public server is still unmeasured.

Checked today for the first two points; the impact is suspected and unmeasured. Size: medium. Matters only on a server open to the internet.

**Now.**

- Every accepted connection gets its own task before the player limit is checked. `crates/server/src/lib.rs`, the accept loop near line 234.
- The queues into the lobby and each game have no size limit. `crates/server/src/game.rs` line 61.
- Rate limits count messages, not bytes. From the review.

**Simplest change.** Limit connections that have not said hello yet, limit queued bytes, and give map transfers their own allowance. This refuses bursts of honest connections too.

### M8. Nobody can join only to watch

**Open.** Not built. The top of `crates/server/src/game.rs` says where a watcher would attach. Issue 45 is open, for joining a game in progress. Pull requests 48 and 54 list watchers as later work. The fork review notes that late watchers and returning players (M1) share machinery.

From the review. Size: medium.

**Now.** A defeated player can stay and watch. A friend or referee cannot join a game as a watcher.

**Simplest change.** Watcher slots chosen before the game starts, with no units and no part in victory. This gives up joining after the start, which would need the whole game state sent.

### M9. Two map limits are still missing

**Fixed** in pull requests 8 and 15. A square width outside 1 to 100 refuses the map (`MAX_SQUARE_WIDTH` and `load_and_build_map` in `crates/world/src/map.rs`). Test: `a_map_with_squares_too_wide_or_empty_is_refused` in `crates/world/tests/maps.rs`. Load time is bounded by named limits on text size, lines and counts, checked before the work they bound. Test: `changed_maps_load_and_play_or_are_refused` in `crates/world/tests/hostile.rs` loads changed copies of every shipped map, each within 1 second.

Checked today for the square width; the timing point is from Sol's review of pull request 3. Size: small.

**Now.** Lines, meadows and starting units are capped. Two things are not:

- The square width is multiplied by 1000 with no upper bound, so a huge value overflows. `crates/world/src/map.rs` line 627.
- A hostile map can still cost load time in proportion to its text, up to the 16 megabyte map limit.

**Simplest change.** Cap the square width the way the other three are capped. Leave the timing point until a map is found that actually takes long.

### M10. Game records stop at out-of-sync reports

**Changed.** The out-of-sync record and `--check-record` were deleted with lockstep in pull request 54, since a game on a server can no longer go out of sync. So no game is recorded at all now. There is still no way to listen to a game, save one or load one. The blocker named below is gone: since pull request 10 the four cheats go through the host as commands (`Cheat` in `crates/world/src/command.rs`, sent from `crates/client/src/game/misc.rs`), and a game on a server has no cheats. The server's own record of a game is not built. Pull requests 48 and 54 list it as later work, and the top of `crates/server/src/game.rs` says where it would attach.

Checked today. Size: small for the first step, large for saved games.

**Now.** A record is written only when a server game goes out of sync. There is no way to listen to a recorded game, no saved games, and no record of a single-player game. The README still lists saved games and replays as not ported.

**What blocks single-player records.** Four console cheats change the world directly without going through the command stream: resources, all technologies, mana, and "play as the next player". `crates/client/src/game/misc.rs`, lines 210 to 226 and `cmd_change_player` at line 265.

**Simplest change.** Send those four through the command stream. Then every game can be recorded, a save is the record so far, and a load replays it.

**Gives up.** Loading is not instant, and a save stops loading after any change to the simulation. Measure how long a one-hour game takes to replay before building saves on this.

### M11. Two different machines have never been compared

**Changed.** A live game no longer depends on two machines computing the same thing, because only the server runs it (pull request 54). Since pull request 15, CI also runs the simulation's tests, recorded traces included, on Linux as well as Windows, against the same expected files. Both runners are x86-64, so a second processor type has still never been compared. That matters again only when replays are shared between machines, and there are none yet. The three floating-point places are unchanged.

From the review. Size: small.

**Now.** The multiplayer test runs both clients on one machine, and the CI job runs on Windows only. The simulation is integer maths except in three places: decimal numbers in data files are parsed through floating point, the square root starts from a floating-point estimate before an integer correction, and trigger timers compare floating-point values. No concrete difference was found.

**Where.** `crates/world/src/nofloat.rs`, `crates/world/src/triggers.rs`.

**Simplest change.** Run the simulation crate's recorded traces in CI on a second processor type, such as an ARM Linux runner, against the same expected files. The simulation crate has no Windows dependency that I know of; confirm that first.

### M12. Small leftovers from the three pull requests

**Changed.** The first three points no longer apply: the sync check, the record and its faction setting were deleted with lockstep in pull request 54. Of the branches, `game-record` is gone from GitHub; `mp-safety` and `golden-cutover` are still there.

Checked during those reviews. Size: small each. None is urgent.

- The sync check does not cover whether two remembered units are the same snapshot, because that is decided by memory address. Sol raised it and accepted leaving it. It goes away with C3.
- A record replays with the faction setting of a multiplayer game, and no test can tell the two settings apart while only one faction is loaded. A test with the orc mod would close this.
- The cost of the sync check, about 0.7 milliseconds on a late game, was measured once by the implementing agent and not repeated.
- The branches `mp-safety`, `game-record` and `golden-cutover` are still on GitHub. I left them because deleting is yours to decide.

## Gameplay

All of these change the recordings. Since the cutover that means re-recording with `UPDATE_EXPECTED=1` and reading the differences. All are from the review unless marked.

### G1. Computer players play by different rules

**Partly fixed** in pull request 23. Fixed:

- Computers remember a unit for 3 minutes, as a human does (`memory_duration` in `crates/world/src/player.rs`). Test: `a_computer_forgets_a_unit_three_minutes_after_losing_sight_of_it` in `crates/world/src/upstream_tests.rs`.
- A computer chooses a warehouse place by what it has seen (`is_ok_for_warehouse` in `crates/world/src/ai.rs`). Test: `a_computer_judges_a_warehouse_place_by_what_it_has_seen` in `upstream_tests.rs`.

Not fixed:

- Computer 2 still gets 1000 of each resource every time it plays and ignores requirements (`computer2_play` in `crates/world/src/ai.rs`). Pull request 23 left this as a separate decision. The design's decision 6, fair computers with a handicap setting, is not built.
- The four roles are unchanged. The gate is still looked up by name.

Pull request 52 added a measurement over many maps and seeds, `crates/world/tests/ai_sweep.rs`, to judge changes like these. N1 to N3 come from it.

Checked today, except the warehouse point. Size: small to change, then playtesting.

**Now.**

- The "aggressive computer 2" type has its resources reset to 1000 of each every time it plays, and ignores all requirements. `crates/world/src/ai.rs`, `computer2_play`, line 246.
- Every computer remembers a unit for 10 hours after it goes out of sight. A human remembers it for 3 minutes. `crates/world/src/player.rs`, `memory_duration`, line 485.
- When choosing where to put a warehouse, a computer reads what is really on a square, not what it has seen. From the review.
- The computer's fixed routine is built around four roles: peasant, town hall and farm, looked up through the faction's equivalents, and gate, by name. A faction must supply all four.

**Simplest change.** Give computers the human memory time, which is one number. Then decide whether computer 2 stays as it is, labelled as a cheating opponent, or goes. This makes the computers weaker, so it needs playing against.

### G2. Orders other strategy games have

**Open.** None of the four is built. Units are still offensive or defensive, there are still four recallable groups, and a unit still queues at most 10 orders (`ORDERS_QUEUE_LIMIT` in `crates/world/src/orders.rs`). Orders are now typed values (C1), which is the point this finding said a new order should wait for.

From the review. Size: medium each.

- No hold position. Units are either offensive or defensive.
- No automatic spell use for a human's units. Computers cast on their own.
- Four recallable groups, on keys 6 to 9.
- At most 10 queued orders per unit.

**Simplest change.** Hold position first: it is one new mode beside the two that exist. A new order is also the point where typed commands, C1, start to pay.

### G3. Bugs kept from the original

**Fixed** in pull request 23. Fifteen rows are fixed, each with a test that fails without its fix. Two rows were shown not to be bugs, and the third was not seen in a large sample. Row by row, in the table's order; the tests are in `crates/world/src/upstream_tests.rs` unless another file is named:

- Allies counted twice: fixed. `a_healer_heals_its_own_allied_units_once_per_turn`.
- Splash centred on the map corner: fixed. `a_splash_that_kills_its_target_still_hits_the_units_beside_it`.
- Victory check skips a player: fixed. `a_victory_defeats_every_other_player_when_the_losers_quit`.
- End of game skips every other player: fixed. `every_player_quits_when_nobody_is_playing_any_more`.
- Next building project skipped: fixed. `every_abandoned_building_project_gives_its_money_back`.
- Threat counted on the wrong neighbour: fixed. `a_ranged_enemy_that_harms_makes_its_own_square_cataclysmic`.
- "Ignore everything else" stays an attack: cannot happen. Every unit in a group belongs to the player who gives the order, so the check gives the same answer for each.
- Load all and unload all fail silently: fixed, refused at once. `load_all_aimed_at_a_unit_inside_something_is_refused_at_once`.
- Two parts of a square are the same target: fixed. `two_points_of_a_square_are_different_targets`.
- Removing one of two overlapping objects: cannot happen. Objects are placed only on free cells, so no two share a cell.
- Occupied space wraps at the map edge: fixed. `an_object_at_the_right_edge_does_not_occupy_the_left_edge_of_the_next_row` in `crates/world/src/collision.rs`.
- Ids sorted as text: fixed. `of_two_meadows_as_near_the_one_with_the_smaller_id_comes_first`.
- Computer counts a square twice: fixed. `a_square_reached_by_two_ways_gives_its_candidates_once` in `crates/world/src/ai.rs`.
- Computer gives up on a tie: fixed. `on_a_tie_the_builders_place_is_the_square_and_not_the_transport` in `ai.rs`.
- Computer 2 gives every "use" a target: fixed. `computer2_aims_only_the_abilities_that_ask_for_a_target` in `ai.rs`.
- "Add units" deletes the newest unit: fixed. `adding_a_unit_where_there_is_no_space_leaves_the_other_units_alone`.
- 1.005 read as 1.004: fixed. `to_int_reads_three_decimals_as_written` in `crates/world/src/nofloat.rs`.
- Square root off by one: not seen. Pull request 23 sampled 15.3 million values, including values near powers of two and the top of the range, and all were exact. That is a sample, not a proof. Separately, for values near the largest 64-bit number the correction step in `int_sqrt` (`crates/world/src/nofloat.rs`) overflows. Squared distances in a game are far below that.

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

**Open.** By the same count as before, `Abort::new` in `crates/world/src`, there are 109 such stops outside tests, against 114 at `612afd1`. Three of them are now in `crates/world/src/questions.rs`, which the interface shares.

Count checked today. Size: large in total. Do it alongside other work, not as a project.

**Now.** 114 places in the simulation stop a unit's or a player's turn part-way, carrying the text of the Python error the original would have raised. Whatever was changed before the stop stays changed. Only the soak test watches the warnings these produce.

**Effect.** Real defects hide behind them, and they are hard to tell from the copied ones.

**Simplest change.** Whenever a fix lands in a function, turn its stops into states that cannot occur or into assertions.

### G5. The client still carries code that only kept it in step with Python

**Partly fixed.** The menu quirk is fixed in pull request 49: a remembered menu opens on the last choice and lists it once (`Menu::append` in `crates/client/src/menu.rs`). Test: `a_remembering_menu_opens_on_its_last_choice_and_lists_it_once` in `crates/client/tests/menus.rs`. The rest remains:

- A seed is still drawn when a fixed seed replaces it, so that later sounds stay the same (`Session::new_seed` in `crates/client/src/session.rs`).
- 13 errors in the game interface still carry Python's text, such as "'NoneType' object has no attribute 'title'", in `crates/client/src/game/entity.rs`, `selection.rs` and `squares.rs`.
- A failed lookup is still a beep (`Oops::missing` in `crates/client/src/game/mod.rs`).

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

**Partly fixed** in pull requests 10, 19, 53 and 54. The interface builds typed commands (`Command` in `crates/world/src/command.rs`). A game on a server sends them as typed binary frames. Every order the simulation gives itself is a typed `Request`, so a misspelt keyword does not compile. An order in a unit's queue now holds only its own kind's typed state (`OrderKind` in `crates/world/src/orders.rs`). Text is read only for map triggers and, behind the `truth` feature, for the recorded traces. These are proved by the compiler and by unchanged recordings, not by a test that would fail. Not yet: a rallying point is still stored as text and parsed again for each unit trained, and a zoom target keeps a text id; see N9.

From the review. Size: large.

**Now.** The client builds an order as a line of text. The protocol carries the line and the simulation parses it. Inside the simulation, units, the computer player and triggers also build text orders, in about 30 places in `ai.rs` alone. A typing mistake in an order name compiles.

**What changed since the review.** I first listed this as the way to close the cheating hole. A flag on the world closed it, so safety no longer needs it.

**Worth doing when.** The first new order or command is added (G2, H5). Parse once where the order enters, into one shared type.

### C2. The interface reads the whole world, not what the player knows

**Partly fixed** in pull requests 14, 18, 20 and 47. The interface reads only what its seat is told: the game's start, one message a turn, and the end. A CI step builds it without the world (`cargo check -p soundrts --lib --bins`), so a read of the world does not compile. The three suspected leaks were real. Pull request 14 found nine more, and pull request 18 closed all twelve. Tests include `an_exit_is_blocked_only_by_a_blocker_the_player_knows` in `crates/world/src/seat_view/tests.rs` and `an_order_names_its_target_as_the_player_knows_it` in `crates/client/src/game/entity.rs`. One leak remains, named in the code: a unit's number (N13).

From the review; the leaks are suspected. Size: small to check the leaks, large for the full change.

**Now.** The client holds the live simulation and reads from it directly. Three places may tell a player things they have not seen: what is blocking an exit, the owner's unit list when a unit dies, and the live unit behind an order's target.

**Worth doing now.** Trace those three. If any is real, it is a gameplay bug.

**Worth doing later.** A read-only "what this player knows" layer earns its cost when watchers (M8) or listening to records (M10) is built, because both need to present a game from a chosen player's side.

### C3. Remembered units are told apart by memory address

**Put off on purpose.** The design leaves it as it is (`docs/refactor-design.md`, appendix A). It mattered for the sync check and for writing the world to disk. The sync check is deleted (pull request 54), and nothing writes the world to disk. The comparison is still there: `Arc::ptr_eq` in `crates/world/src/entity.rs` (the `Target::Memory` case) and in `crates/world/src/world.rs`. The line numbers below are from before.

Checked today. Size: small.

**Now.** Whether a remembered unit is still the current memory is decided by comparing addresses. That cannot be written to a file or included in the sync check.

**Where.** `crates/world/src/entity.rs` line 106. `crates/world/src/world.rs` line 486.

**Simplest change.** A version number on each memory. It needs a test that an order aimed at a stale memory still treats the target as forgotten.

**Worth doing when.** Anything needs to write the world to disk, such as instant saves.

### C4. Per-turn work that grows with the game

**Partly fixed.**

- The move test no longer copies the occupied space (pull request 8): `would_collide_if` in `crates/world/src/unit.rs` calls `would_collide_without`. Every trace stayed the same, so this is fixed by reading, no test of its own.
- The sync check that walked every unit each turn is off the play path (pull request 54). The digest remains for tests.
- Still true: a deleted unit is never freed. The list of entities in `crates/world/src/world.rs` only grows.
- Each turn the server now builds one message per seat. Pull request 20 timed it at 0.026 milliseconds a turn for eight seats on jl7, against about 0.65 milliseconds for the turn itself.
- The other points are unmeasured.

First two checked today; the rest from the review. Nothing measured. Size: small for the first, unknown for the rest.

- Every attempt to move a unit copies the whole set of occupied space. The original removed the unit's own cells, tested, and put them back. `crates/world/src/unit.rs`, `would_collide_if`, line 630.
- A deleted unit is never freed, and the sync check walks every unit ever created, every turn. `crates/world/src/world.rs`.
- Each turn, every unit the player can perceive is copied to hand to the interface.
- The current order and whole unit types are copied on each use.
- Looking up a rule scans the list and copies the value.
- Several scans kept from the original run over all units once per unit.

**Simplest change.** Time one late-game turn on a long recorded trace first. Fix the move copy regardless: testing in place gives the same answer. Fix the others only if the timing says they matter.

### C5. Two very large structures

**Open.** The world still has 12 public fields, as at `612afd1`. But the interface can no longer reach the world (C2), so only the simulation and its tests can change them. The interface structure, `GameInterface` in `crates/client/src/game/mod.rs`, has 52 fields, against 55 at `612afd1` by the same count; the "about 70" below was an estimate. The long functions were not measured again.

From the review. Size: medium each. Low priority.

**Now.** The world exposes its main fields for any code to change, and seven of its functions run from about 90 to about 260 lines. The game interface is one structure of about 70 fields.

**Worth doing when.** Work lands in one of those functions. Split the piece being changed, not the whole.

### C6. Everything runs on one thread between a key and its speech

**Partly fixed.** Speech has its own thread (pull request 11; `SpeechThread` in `crates/client/src/tts/worker.rs`; test `commands_reach_the_speaker_in_order_on_its_thread`). In a game on a server, the simulation runs on the server (pull request 54). Still on the key thread: the simulation turn in single player, and the first decoding of each sound (`get_sound` in `crates/client/src/sound_cache.rs` decodes a sound on first use). Nothing was timed.

From the review; the delay is suspected and unmeasured. Size: small to measure.

**Now.** A simulation turn, the call to the screen reader, and the first decoding of each sound all run on the thread that handles your keys. The original sent speech through a separate thread.

**Simplest change.** Time the turn and the screen reader call in a late game. Decode sounds ahead of time in the background. Move the simulation to its own thread only if the numbers call for it.

### C7. The game checks for input on a timer

**Partly fixed.** The 100 millisecond wait no longer happens in a game (pull request 29). Still true: outside a game, a message said now polls every 100 milliseconds (`say_now` in `crates/client/src/voice.rs`); the game loop sleeps 1 millisecond a pass (`crates/client/src/game/mod.rs`); and menus sleep 10 milliseconds (`crates/client/src/menu.rs`).

From the review. Size: medium. Low priority.

**Now.** The game loop sleeps 1 millisecond per pass, menus 10, and blocking speech 100.

**Simplest change.** Wait for the next key or the next deadline, whichever comes first. The 100 millisecond wait goes away with H2. The transcripts record every sleep, so this needs a re-record.

### C8. Windows only in practice

**Decided against** for the client. The design's decision 7 (`docs/refactor-design.md`, section 13) keeps the client Windows-only. The simulation, the protocol and the server are built and tested on Linux in CI since pull request 15.

From the review. Low priority unless other systems become a goal.

**Now.** Off Windows, speech goes to the system voice and not to a screen reader, and there is no clipboard.

## What I would leave alone

- **The 300 millisecond turn.** Unit speeds, costs and timings in the data files are all written against it.
- **The integer maths.** It is what keeps every client's game identical.
- **The copy of Python's random number generator.** Any seeded generator would do now, but this one is small, fast and tested, and replacing it buys nothing.
- **The fixed limits:** 26 columns, 10 resource types, 144 units per teleport, a food limit of 80. No finding depends on raising them.
- **Moving a running game to another host.** See M5.

## Waiting on your ears

Status, 15 November: the first check still applies, now to a game the server runs (pull request 54). The second no longer applies: lockstep and its two out-of-sync sentences were deleted in pull request 54. Every pull request since that changes what you hear has its own numbered script, and you have not reported on any of them yet. The listening room (pull request 12) is the only listening so far.

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

## Found since

Found during the work of pull requests 6 to 54 and not in the list above. Each says where it came from and how far it was checked. Some items from that work are recorded under their old findings: returning to a game after a lost connection is in M1, pause in a game on a server is in M4, watchers are in M8, the server's record of a game is in M10, and the key-remapping menu is in H6.

### N1. Computer players build no boats

From the measurement in pull request 52. Open. Measured, not read in the computer's script.

**Now.** On jl7 each start is an island of three squares. The computer's ground explorer never leaves its island; only a flying machine does. Over 40 seeds, an aggressive computer attacked in 11 of 320 cases. 8 of 320 never saw an enemy in 20 minutes.

**Where.** `crates/world/src/ai.rs` has no rule about water or boats. The computer's plan also comes from `ai.txt` in the game data, which was not read for this.

**How to check a change.** `cargo test --release -p soundrts-world --test ai_sweep -- --ignored --nocapture`, with `SWEEP_MAPS=jl7`.

### N2. On jl7 a computer sits at its population limit, short of gold

From the measurement in pull request 52. Open.

**Now.** 88 of 320 aggressive computers on jl7, in 37 of 40 seeds, sat at their population limit for over a minute without the gold for a farm. A probe of one of them found 1 worker on gold and 9 on wood, with about 190 wood stored.

**Simplest change.** The fork's: send workers to the resource a farm is short of. The sweep measures whether it helps.

### N3. On jl7 a computer's island runs out of meadows for farms

From the measurement in pull request 52. Open.

**Now.** On jl7 the time at the population limit has a median of 72 seconds and reaches 437.7 seconds, mostly with the cost of a farm in hand. A probe of one computer found no free meadow left on its island, so it had nowhere to build a farm. With N1, a computer on an island map can neither grow nor reach anyone.

### N4. An upgraded keep or tower loses its rallying point

From the review of pull request 51. Open. Checked by reading, not reproduced.

**Now.** An upgrade replaces the building with a new one. The new building keeps the old one's hit points, blocked exit, groups and the units inside, but not its rallying point. Units trained after the upgrade do not go to the point that was set.

**Where.** `crates/world/src/orders.rs`, `production_complete`, the `Product::UpgradeTo` branch.

### N5. An order picked but not yet aimed still names the old unit after an upgrade

From the review of pull request 51. Open. Checked by reading; what you hear was not traced.

**Now.** When a unit is upgraded, the interface moves its group and its selected object to the new unit. An order already picked from the menu and waiting for a target is not moved. It still names the unit it was listed for, which no longer exists.

**Where.** `crates/client/src/game/selection.rs`, `follow_replaced_units`. The order's `unit` field is on `OrderTypeView` in `crates/client/src/game/order.rs`.

### N6. A converted unit can keep more hit points than its new maximum

From the review of pull request 51. Open. Checked by reading, not reproduced. Matters only with rules where an upgrade raises maximum hit points.

**Now.** A unit that changes hands takes its type's stats again, plus its new owner's upgrades. Its hit points are left as they were. If the old owner's upgrade had raised its maximum, it can keep more hit points than its new maximum.

**Where.** `crates/world/src/unit.rs`, `upgrade_to_player_level`.

### N7. A "use" order can be impossible and complete at once

From pull request 53, as a suspected bug. Not traced here.

**Now.** A teleportation or recall with no room for its units is marked impossible by `use_is_not_necessary`, then complete by `use_execute`.

**Where.** `crates/world/src/orders.rs`, those two functions.

### N8. An upgrade that needs a meadow and finds none keeps its cost

From pull request 53, as a suspected bug. Checked by reading, not reproduced.

**Now.** When an upgrade into a building that needs a meadow finds no meadow, the player hears that the order is impossible. The order is then marked complete, and nothing is refunded.

**Where.** `crates/world/src/orders.rs`, `production_complete`, the `Product::UpgradeTo` branch, and its caller, which marks the order complete.

### N9. Some game state is still text

From pull request 53's scope notes. Open. Checked by reading.

**Now.**

- A rallying point is kept as the text of its target and parsed again for each unit trained (`rallying_point` on `Entity` in `crates/world/src/entity.rs`).
- A zoom target keeps the text id it was given (`ZoomTarget` in `entity.rs`).
- The order menus list types and requirements as strings (`OrderTypeView` in `crates/client/src/game/order.rs`).

Pull request 53 typed the rest of an order's state and left these.

### N10. The simulation does not check that a square in a command is on the map

From the review of pull request 48. Open. Checked by reading.

**Now.** The server refuses a command naming a square that is not on the map (`on_the_map` in `crates/server/src/game.rs`). The simulation itself does not check: `Host::queue` passes the command on, and `World::square` indexes the list of squares directly. A single-player client sends only squares from its own map, so today nothing reaches it.

**Simplest change.** Check in `Host::queue`, so the server's check is not the only one.

### N11. Nothing checks that players have the same sounds and names

From pull request 54. Open.

**Now.** Joining a room needs the same content hash, which covers the simulation version, the rules and the computer players' scripts (`content_hash` in `crates/client/src/resource.rs`). Each client names units and plays sounds from its own files. Nothing checks that those match what the server's game uses.

### N12. Every player still needs the map on their own computer

From pull request 54. Open.

**Now.** The server builds the game from the host's rules, computer players and map. Each other client still supplies the map's rules, sounds and names from its own files. So map transfer between clients through the room is still needed.

### N13. A unit's number is told to every player

From pull request 20, where it was named and kept. Open. Checked by reading.

**Now.** A unit's number, the `3` of `footman 3`, is in the part of a unit that every player who sees it is told. The code marks it as a leak: it follows the owner's other units of that type. Since pull request 54 the server sends it to every seat. The interface reads it for units a map adds, to decide whether to announce their arrival.

**Where.** `crates/world/src/known.rs`, the `number` field.

### N14. Host powers are not handed on when the room's host leaves a game on a server

From pull requests 48 and 54. Open. Checked by reading, not run.

**Now.** A room hands its host on to another player while the room is gathering. During a game it does not. Only the room's host can change the speed or remove a player, so after the host leaves, nobody can.

**Where.** `crates/server/src/lobby.rs`, `take_out_of_room` and `host_control`.

### N15. Connections are not encrypted

From pull request 54, which lists TLS as later work. Open.

**Now.** The client connects with `ws://` (`crates/client/src/net/conn.rs`). Chat, logins and seat tokens cross the network as they are.

### N16. Escape does not skip a whole cut scene everywhere

From pull request 50. Open.

**Now.**

- In a campaign's cut-scene chapter between missions, Escape skips one line, as any key does.
- In a game, when other news waits behind a story, one Escape still skips the rest of the story. But each skipped line starts and is cut off at once, so you may hear the start of each.

Pull request 50 notes that a small change in `crates/client/src/voice.rs` would fix both.

### N17. Copying to the clipboard may fail

From pull request 49. Suspected, not tested with the real clipboard.

**Now.** The game opens the clipboard without a window (`OpenClipboard(0)` in `crates/client/src/clipboard.rs`). Microsoft's documentation says writing can then fail. Step 7 of pull request 49's script checks it.
