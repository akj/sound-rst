# Interface parity with the original client

For a blind player the spoken output is the whole interface. The files
here record what the original Python client says when it is driven by a
fixed script of key presses, so that the Rust client can replay the same
scripts and be checked against the same speech.

- `drive_reference.py` drives the original client (`vendor/soundrts-upstream`)
  headlessly and deterministically, and writes the transcripts.
- `scenarios/*.txt` are the scripts.
- `expected/<scenario>.txt` are the reference transcripts: everything said,
  the window title, the steps, and a hash of the positional sounds of each
  step.
- `expected/<scenario>.sounds.txt` is the full positional-sound stream, kept
  only when it is smaller than 256 KiB (the menus, the campaign, `french`
  and `square_orders`). For the longer games only the per-step hashes are
  committed; `--full DIR` writes the full streams.

## Regenerate and check

Create the venv once (pygame and the small pure-Python dependencies; never
the real `accessible_output2`, which talks to the screen reader):

    uv venv vendor/.venv-ui --python 3.11
    uv pip install --python vendor/.venv-ui/Scripts/python.exe pygame cloudpickle chardet requests

Then, from the repository root:

    vendor\.venv-ui\Scripts\python.exe crates\client\tests\parity\drive_reference.py [SCENARIO...] [--full DIR]
    vendor\.venv-ui\Scripts\python.exe crates\client\tests\parity\drive_reference.py --check
    vendor\.venv-ui\Scripts\python.exe crates\client\tests\parity\drive_reference.py --perturb
    vendor\.venv-ui\Scripts\python.exe crates\client\tests\parity\drive_reference.py --perturb-random

- Without a mode it rewrites `expected/`.
- `--check` runs every scenario twice, each from a fresh scratch root, and
  compares the two runs with each other and with `expected/`, byte for byte
  (transcripts and full positional streams).
- `--perturb` runs with two other orders for every hash-based container
  (other `PYTHONHASHSEED`, other hashes for world objects); any output that
  depends on such an order shows up as a difference.
- `--perturb-random` runs with two other seeds for the client's global
  `random` module and compares the voice only (it changes the positional
  sounds, never the voice).

Each scenario runs in its own child process whose current directory is a
fresh scratch game root, `vendor/ui-scratch/<scenario>-<run>/`, holding
copies of `res`, `cfg`, `single` and `mods` and an empty `user` directory.
Nothing is written to the upstream checkout or to `%APPDATA%`. The child's
own log is `driver.log` in its scratch root; temporary transcripts go to
`target/ui-parity/`.

## Safety

The driver itself enforces, before pygame or any client module is imported:
`SDL_VIDEODRIVER=dummy` and `SDL_AUDIODRIVER=dummy`; a stub
`accessible_output2` in `sys.modules` that raises if used; `socket.socket`,
`socket.create_connection`, `getaddrinfo`, `gethostbyname` and
`urllib.request.urlopen` that raise; `threading.Thread.start` that raises
(except for the world thread, which becomes a no-op); `webbrowser.open`
that raises. After `pygame.init()` it asks SDL itself for the current
drivers (`SDL_GetCurrentVideoDriver`, `SDL_GetCurrentAudioDriver` in
pygame's `SDL2.dll`) and aborts unless both are `dummy`; every later
`pygame.init()` / `pygame.display.init()` re-forces and re-checks this (the
client's `clientmedia.py` sets `SDL_VIDEODRIVER=windib` on import). The mixer
channels are fakes, so even the dummy device never receives a sound. A child
exits with status 3 if anything was blocked (urllib3's import-time IPv6
probe, refused like any socket, is not counted).

## The model the Rust replay must mirror

### Clock

- Virtual time is an integer number of microseconds, starting at 0.
- `time.time()` returns `(10^15 + µs) / 10^6` as an f64: the virtual epoch
  is 10^9 s. Upstream's own comparisons and sums on these values (for
  example `now >= previous_animation + 0.1`, `next_update = now + 0.3`) are
  f64 operations in upstream's order; boundaries fall where f64 puts them.
- Only sleeping advances the clock: `time.sleep(s)` adds `round(s × 10^6)`
  µs. Computation takes no time. The sleeps upstream does: 1 ms per game
  loop iteration, 10 ms per menu poll, 100 ms per iteration of a blocking
  voice loop (`_say_now`, `flush`). The text entry loop (`input_string`)
  busy-polls upstream; here each of its `poll()` calls first advances the
  clock by 1 ms, then is an input point, then reads the queue.
- Transcript times are `µs // 1000` (all events happen on whole
  milliseconds).

### Speech

`soundrts.lib.tts` is replaced by a fake without threads and without
output, keeping upstream's rule for when speech ends:

- `speak(text)`: speaking until
  `now + round(len(text) × d × 10^6) + round(n × (c − 1) × d × 10^6)` µs, where
  `d = wait_delay_per_character` (0.1, the default of `SoundRTS.ini`),
  `n` = number of digit characters in `text`, `c = tts_digit_coefficient` (3, from
  `cfg/parameters.toml`). Products are f64, left to right; `(c − 1)` is an
  integer.
- `is_speaking()`: `end > now`. `stop()`: not speaking. A new `speak`
  replaces the previous one.
- Upstream decides the end in a thread that polls every 0.1 to 0.2 s, so
  its exact end is fuzzy; the formula above is the reference.

### Sounds

`pygame.mixer.Channel(n)` returns a fake channel; `get_num_channels`,
`set_num_channels`, `set_reserved`, `stop`, and `Sound.play` use the fakes.

- `play(sound, loops)` makes the channel busy until
  `now + length × (loops + 1)`, or forever for `loops = -1`; `stop()` frees
  it. The length is pygame's `Sound.get_length()` with the mixer at 44100 Hz,
  16 bits, stereo (as upstream's `pre_init`), rounded to the microsecond.
  Every length used is in the transcript header (`length NAME µs`).
- Channel 0 is the voice channel; positional and stereo sounds use channels
  1 to 15 (`num_channels` 16).
- No end-of-sound events (`USEREVENT`, `USEREVENT + 1`) are posted. Upstream
  posts them when a channel finishes; they only trigger `voice.update()` and
  `psounds.update()`, which every loop calls on every iteration anyway, but
  each one would cost a menu one poll (10 ms). Not posting them is the model.

### The voice

Unchanged upstream code (`lib/voice.py`, `lib/voicechannel.py`,
`lib/message.py`) on top of the fakes above. Messages are recorded when
`VoiceChannel.play` starts them, as the translated and collapsed parts
(`Message.translate_and_collapse()`): consecutive texts joined with a space,
recorded sounds by name.

The voice is idle when `VoiceChannel.get_busy()` is false (no TTS speech,
channel 0 not busy, no queued part) and `voice.msgs` has no unsaid message.

### Input

- The event queue is a list. Steps add `KEYDOWN` events only. No `KEYUP`
  is ever added: no upstream code reads one. The modifiers of a chord
  (`key CTRL SHIFT F2`) are not pressed as keys of their own: only the main
  key's `KEYDOWN` is added, with the modifier flags. A modifier alone is a
  step of its own (`key LCTRL`: in a game, LCTRL is bound to `examine`,
  LALT and the shifts to `history_stop`). Real presses would first deliver
  the modifier's own `KEYDOWN`.
- A `KEYDOWN` carries, as SDL2 and pygame 2.6 deliver it on a US layout:
  `key` = `pygame.K_<name>` (lower case for letters), `scancode` =
  `pygame.KSCAN_<NAME>` (with `QUOTE` = 52 and `BACKQUOTE` = 53, as
  `lib/bindings.py` defines them), `mod` = `KMOD_LCTRL` / `KMOD_LALT` /
  `KMOD_LSHIFT` for CTRL / ALT / SHIFT (a modifier key alone carries its own
  flag, `KMOD_RCTRL` for RCTRL...), and `unicode`: for a letter, the letter
  (upper case with SHIFT; with CTRL the control character, `a` → `\x01`);
  for a digit or punctuation key, its US character (shifted with SHIFT,
  empty with CTRL); `\r` for RETURN and KP_ENTER, `\t` TAB, `\x08` BACKSPACE,
  `\x1b` ESCAPE, `\x7f` DELETE, ` ` SPACE, `+ - * / .` for the keypad keys;
  empty for every other key (arrows, F keys, modifiers, PAGEUP...).
  Game commands are bound by scancode and modifiers; menus and text entry
  read `key`, `mod` and `unicode`.
- `pygame.event.poll/get/clear/post/peek/pump` work on that list (`post`
  appends; `get(types)` removes and returns the matching events in order;
  `clear(types)` removes them).
- An input point is a read made by one of these five upstream functions,
  and only these may deliver a step:
  - `menu`: the `poll()` of `Menu._get_choice_from_static_menu` and of
    `Menu.step`;
  - `text`: the `poll()` of `clientmenu.input_string`;
  - `game`: the `get()` of `GameInterface._process_events`;
  - `voice`: the `get([KEYDOWN])` of `_Voice._key_hit` (inside blocking
    speech: `_say_now`, `flush`).

  Every other read, such as the `get([KEYDOWN])` that drops keys pressed
  during an alert, or `clear()`, never delivers.

### Settled

At an input point, the client is settled when:

1. no `KEYDOWN` is waiting in the queue;
2. the voice is idle;
3. at a `game` point only: the interface's server-event queue is empty and
   at least one `voila` (the result of a world update) has been processed
   since the previous step was delivered.

The `voice` point is never settled for key steps (speech is in progress
there by definition).

### Steps

Steps are taken in order. `t_prev` is the time the previous step was
delivered (0 before the first); `turn_prev` is the world's turn then (0 if
no game was running).

| step | delivered when |
|---|---|
| `key K` | at a `menu`, `text` or `game` input point, when settled |
| `type abc` | one `key` step per character (`A` → `SHIFT a`, space → `SPACE`, `.` → `PERIOD`) |
| `interrupt_after D key K` | as soon as `now ≥ t_prev + D`: at the end of the sleep that reaches it, or at an input point. Not settled first; the key may interrupt speech, or be dropped (after an alert) |
| `wait S` | at an input point, when `now ≥ t_prev + S` |
| `wait_turn N` | at an input point, when a game runs and `world.turn ≥ N` |
| `wait_turns N` | at an input point, when a game runs and `world.turn ≥ turn_prev + N` |
| `wait_game_end` | at an input point, when no game runs (`GameInterface.run_game` has returned) |

Delivering a key step appends its event to the queue, and the reading
function then reads it in the same call. Note the order inside one loop
iteration: the input point comes before `voice.update()`. A message whose
speech ends at exactly `now` already counts as idle, so the next key is
delivered in that iteration, before `voice.update()` has marked the message
said or started the next one. After a non-key step, the next
step is checked at once, at the same input point. When every step has been
delivered, the scenario ends at the first `menu`, `text` or `game` input
point where the client is settled (`end`), or earlier if the client exits
(`exit`).

`world.turn` is `world.time // 300`; `world.time` grows by 300 at the end of
each `world.update()`.

### The game loop and the world

The world runs synchronously: `World.queue_command` executes its command at
once (an order through `player.execute_command`, or `world.update`), and the
world thread is never started. One iteration of `GameInterface._loop`, as
upstream:

1. `server.update()`;
2. if not waiting for a world update and `now >= next_update`: the orders
   written since the last update are executed in order, then
   `world.update()`; the update pushes its server events (`event`, `alert`,
   `msg`, `voice_important`, `sequence`, `speed`, `quit`...) and finally
   `voila` into the interface's queue; `next_update = now + 0.3 / speed`;
3. `_animate_objects()` if `now >= previous_animation + animation_delay`
   (0.1 s);
4. `_process_events()`: the `game` input point, then every queued event;
5. `_process_srv_events()`: one server event per iteration;
6. `voice.update()`;
7. sleep 1 ms.

So the events of an update reach the interface one per millisecond, and
`voila` last. A key pressed in a game reaches the world with the next
update.

### Orders of sets

Upstream iterates some Python sets of world objects, whose order depends on
memory addresses: it changes from run to run, and with it the spoken order
of square summaries ("1 town hall, 1 farm and 1 peasant") and of some news.
The reference fixes it:

- the `memory` and `perception` given to `srv_voila` are iterated in
  ascending entity id (`int(id)`); this decides the insertion order of
  `GameInterface.dobjets`, which most summaries iterate (an entity that
  moves from memory to perception is deleted and re-inserted at the end:
  the Rust side needs an insertion-ordered map with that behaviour);
- `scouted_squares` and `scouted_before_squares` (the world's live sets)
  are iterated in ascending square id;
- `scout_info` (squares to report) is iterated in ascending square id;
- `units_alert` reports its squares in ascending square id.

Directory listings are sorted too: `FolderPackage` and `ZipPackage`
`dirnames()` / `filenames()` return sorted lists (upstream: `os.walk` order,
or a set), and `PackageStack.mods()` lists the mods in sorted name order
(upstream builds a set of names, so the order of the mods menu changes
with Python's string hash seed from one launch to the next).

Every other hash-based container is made reproducible (`PYTHONHASHSEED=0`;
`Entity`, `Square`/`Inside` and `Player` hash by creation order instead of
address). `--perturb` shows that none of those orders reaches the
transcripts. `PARITY_NO_NORMALIZE=1 ... --perturb` turns the four fixes on
the interface's sets off and shows that they are needed (square summaries
change order).

### Randomness

- The world's seed is the scenario's `seed` (it replaces the
  `random.randint(0, 10000)` of `TrainingGame` and `MissionGame`).
- The client's global `random` module is seeded with `seed` before the
  client starts. It is used only for positional sounds: footstep timing and
  each `EntityView`'s `footstep_random` (`clientgameentity.py`, one
  `random()` per `EntityView` created), the choice among a style's sound
  variants (`launch_event_style`, `on_use_complete`, `on_wounded`), repeat
  noises (`animation.py`) and ambient volumes (`lib/sound.py`). It never
  changes the voice (`--perturb-random`). Matching the positional stream
  of a game needs Python's Mersenne Twister and every consumption point,
  including each `EntityView` construction and the draw of
  `random.randint(0, 10000)` for the world's seed that the fixed seed then
  replaces; the Rust client draws at the same points, so its positional
  streams match too. In the menus there is no randomness.

## Transcript grammar

`expected/<scenario>.txt`, UTF-8, LF, one item per line:

    soundrts-ui-transcript 1
    scenario <name>
    settings language=<l> login=<l> mods=<m> soundpacks=<s> seed=<n> limit=<s>
    length <sound name> <microseconds>           (every sound played, sorted)
    @<ms> <event>                                 (in the order they happen)

Events:

- `title <text>`: the window title changed (`pygame.display.set_caption`).
- `voice <kind> <part> <part>... [vol=<l>,<r>]`: a message started on the
  voice channel. A part is a JSON string (text for the speech engine) or
  `[<name>]` (a recorded sound, by its sound-cache name). `vol=` appears
  when the message's left/right volumes are not the default (`sin(π/4)`);
  three decimals. Kinds, by the `_Voice` method that started it:
  - `item`: says now, not kept in the history, does not block;
  - `alert`: blocks until said; keys pressed meanwhile are dropped;
  - `important`: blocks until said or a key is pressed; the key is used up;
  - `confirmation`, `menu`: block until said or a key is pressed; the key
    stays queued;
  - `info`: a queued message started by `voice.update()` (news);
  - `history`: a message replayed from the history (`previous`, F5).
- `voice cut queue=<n>`: speech in progress was stopped other than by a new
  message (`silent_flush`, `sound.stop()`, `say_next`...); `n` parts were
  still queued in the voice channel (`sound.stop()` leaves them there, and
  they play on).
- `step <n> <step>`: step `n` was delivered (a `type` step shows the key
  and, in parentheses, the whole `type` step).
- `sounds <count> <fnv64>`: the positional-sound lines of the step that
  just ended: their number and the FNV-1a 64-bit hash of those lines joined
  with `\n`, as they appear in the positional stream. Written just before
  the next `step` (or `end`), only if the step had any.
- `exception <message> (<type>: <text>)`: upstream logged an error
  (usually a swallowed exception); `0x...` addresses are written `0x?`.
- `end`: the script is done and the client settled; `exit`: the client
  exited by itself; `abort <reason>`: a limit was exceeded (the run fails).

Positional stream, `expected/<scenario>.sounds.txt`:

    @<ms> step <n>
    @<ms> pos <name> v=<v> x=<x> y=<y> prio=<p> limit=<l> [ambient] started gains=<l>,<r>
    @<ms> pos <name> ... skipped
    @<ms> loop <name> v=<v> x=<x> y=<y> prio=<p> started gains=<l>,<r> | pending
    @<ms> stereo <name> vol=<v>|<l>,<r> limit=<l> started|skipped

`pos` is `psounds.play` (a positional sound; `gains` are the channel's
left/right volumes before `main_volume`), `loop` is `psounds.play_loop`,
`stereo` is `psounds.play_stereo` (menu clicks, alerts, the clock's bell).
Numbers have three decimals. Only starts are recorded, not volume updates.

A comparison that ignores timing can drop the leading `@<ms> `.

## Scenario format

UTF-8 text, one item per line; `#` starts a comment line. Settings first,
then steps.

    language en          cfg/language.txt (en, fr...)
    login player         [general] login in SoundRTS.ini
    mods                 [general] mods (empty: none)
    soundpacks           [general] soundpacks
    seed 42              the world's seed and the client's random seed
    limit 1800           virtual seconds before the run aborts
    key CTRL SHIFT F2    modifiers CTRL ALT SHIFT, then a key name of res/ui/bindings.txt
    type bob
    interrupt_after 0.5 key DOWN
    wait 2.5
    wait_turn 40
    wait_turns 150
    wait_game_end

The other `SoundRTS.ini` options keep their defaults; `user/` starts empty
(no campaign bookmarks, no remembered menu choices, no replays).

## Scenarios

| scenario | covers |
|---|---|
| `menus_main` | the main menu (at the port's scope): title, every item, wrap-around both ways, TAB and SHIFT TAB, first-letter search forwards, backwards and without a match, unused keys, menu help, history, volume, Escape (quits) |
| `menus_options` | modify login (typed letters, a rejected character, SHIFT, Backspace, Enter), a key during the non-interruptible "new login" alert (dropped), an empty login, Escape; the mods menu (Back also closes the options menu); the soundpacks menu; choosing a soundpack (new window title, new main menu, quitting it exits) |
| `menus_single` | the single player menu, a campaign's chapter menu, the map list (a key interrupting the confirmation and the title, browsing, first-letter search both ways), the players menu (invite quiet, aggressive, aggressive 2 until full; start; cancel), a 4-player game cancelled at once with its score, the map list remembering the last map first |
| `training_jl1` | a game on jl1 against a quiet computer: every command of `res/ui/bindings.txt` except `fullscreen` and `say` (168 of its 170 key bindings), building, training, a rally point, gathering, groups, all selection keys, zoom, first person, history with interruptions, cheat mode, the console, changing player, the game menu (speed, three ways in, a quirk), news, quitting and the score |
| `training_long` | about 1,700 turns on jl5 (water, high ground) against an aggressive computer: enemy sightings, alerts, losses, defeat, the score |
| `campaign` | the prologue cut scene, chapter 1 (title, intro, objectives, a farm completing an objective, barracks from the console, the cut scene, victory, score), the victory menu, chapter 2 (computer units attack, a unit neutralized), cancel, the defeat menu |
| `french` | `language fr`: the main menu and its help, a short game on jl1 (status keys, orders, targets, squares, time, help), quit and score |
| `square_orders` | a peasant ordered to a square, then to a subsquare in zoom mode, and its status: the order's target is a place, whose title the original takes from the place itself (a view's `title` raises `AttributeError`, so Python falls back to `__getattr__`) |
| `events` | with the console: a research completing, an impossible order and its reason (no food), a small gold mine exhausted, a priest's holy vision on another square (an ability with a target), footmen against the computer's base (fights, neutralized units), victory and the score |
| `transport` | jl5 (water): a boat loads every footman of a coastal square, its status, unloads them on that square |

Not covered (not in the Rust port yet): multiplayer and server menus, chat
(`say`, F7), replays, save and restore, the map editor, fullscreen and the
grid display, the Documentation item.

## The port's scope

The recordings are made at the scope of the Rust port, so that both
clients have the same menus item for item. `restrict_to_port_scope()` in
`drive_reference.py`, the one place that does this, patches the original at
run time:

| patch | removes | why |
|---|---|---|
| `clientmain.main_menu` | multiplayer, server, observe a recorded game, documentation | the port has single player, options and quit only |
| `clientmain.single_player_menu` | restore a saved game | the port cannot save or restore |
| `GameInterface.can_save` returns `False` | save, in the game menu | the same |

Everything else in those menus is upstream's, in upstream's order. Each
patch goes away when its feature is ported, and the scenarios are recorded
again.

`crates/client/tests/parity.rs` replays every scenario on the Rust client
under the model above and compares its transcript with `expected/` byte for
byte (voice, titles, steps, sound lengths, the sounds of each step), and
its positional stream with `expected/<scenario>.sounds.txt` where it is
committed. Only the class of a Python exception is not compared: an
`exception <message> (<Class>: <text>)` line is read as `exception
<message>: <text>`, as the client logs its errors. On a difference it
writes the client's transcript and stream to `target/parity/`.

## Every runtime patch, and why

| patch | why |
|---|---|
| `SDL_*DRIVER=dummy`, driver check, re-forced on every init | no window, no audio device; `clientmedia` sets `windib` |
| stub `accessible_output2` | the real one speaks through the screen reader |
| fake `soundrts.lib.tts` | no speech, no threads; upstream's end-of-speech rule on the virtual clock |
| fake mixer channels, `Sound.play` | no sound; a sound is busy for exactly its length |
| `time.time`, `time.sleep` | the virtual clock |
| `pygame.event.*` | the scripted event queue and its input points |
| `input_string`'s `poll()` advances 1 ms | upstream busy-polls; virtual time must pass |
| `pygame.display.set_caption` | records the window title |
| `World.queue_command` runs at once, `World.loop` a no-op | no world thread; one deterministic interleaving |
| `threading.Thread.start` raises | nothing else may run concurrently (the update checker, servers) |
| `revision_checker.start_if_needed` no-op | update check and statistics over HTTP |
| `restrict_to_port_scope()` | the menus at the port's scope (see The port's scope) |
| sockets, `urlopen`, `webbrowser.open` raise | nothing leaves the machine |
| `TrainingGame` / `MissionGame` seed | a fixed world seed |
| `random.seed(seed)` | reproducible client-side noises |
| `PYTHONHASHSEED=0`; `Entity`, `_Space`, `Player` hash by creation order | reproducible hash-based containers |
| `srv_voila`, `scout_info`, `units_alert` iterate by id | upstream's address order is not reproducible (see Orders of sets) |
| package `dirnames`/`filenames` sorted, `PackageStack.mods` sorted | upstream's `os.walk` and string-set orders are not reproducible |
| `config.login/mods/soundpacks` saved before the client loads | the scenario's settings |
| `sys.argv = ["soundrts"]`, cwd = scratch root | `options.py` parses the command line; `paths.py` uses `./user` |
| `sys.dont_write_bytecode` | nothing written into the upstream checkout |
| a logging handler | records logged errors in the transcript |

## Behaviour of the original a porter might get wrong

- Pressing UP first in a menu says the last item (the index becomes -1),
  but RETURN then confirms nothing: `_choice_exists` rejects index -1. Only
  DOWN, TAB, a letter or Escape make a usable selection
  (`training_jl1`, game menu).
- History replay (`F5`) does not stop after one message: it goes on through
  every later message of the history, until the end (`training_jl1`: two
  quick F5 replay "7 wood", then "1 footman enemy at c 2" follows by itself).
- An `info` message cut by a blocking message (`confirmation`, `important`)
  is said again afterwards, unless it had been playing for more than 1 s
  (`is_almost_done`); several blocking messages in a row repeat it several
  times (`campaign`: "barracks 1 at b 1" three times).
- An empty login: `voice.item([""])` starts a message with one empty text
  part (it ends at once), then the "bad login" alert.
- `sound.stop()` (volume keys, cut scenes, the game menu) stops the speech
  but leaves the remaining parts of the message queued in the voice
  channel: they play on at the next `voice.update()`.
- Choosing a mod or a soundpack opens a new main menu inside the old one;
  quitting it raises `SystemExit`: the client exits.
- In the mods menu, Back returns `CLOSE_MENU` from the options item, so it
  also closes the options menu.
- The map list remembers the last map started (`user/tmp/mapmenu.txt`) and
  inserts it again at the top, without its explanation.
- The first square of a game is announced by the first animation after the
  first world update, up to 100 ms after the game starts, and interrupts
  whatever the first key said.
- News are queued with `voice.info` and expire after 45 s; a newer message
  of the same `update_type`, or an identical newer one, supersedes them.
- Server events are processed one per loop iteration; an `event` handled
  later than `next_update + 3 s` is dropped (`EVENT_LIMIT`), so a long
  blocking message can make the interface skip the sounds and the news
  ("peasant 2 ready!") that the event would have produced.
- `say_time` reports the measured speed (`speed 0.1` right after the game
  menu was open for a while: the average turn duration includes the pause).
- A modifier key pressed alone in a game is a command (`LCTRL` examines,
  `LSHIFT`/`LALT` stop the history). In a menu, CTRL alone says the
  "press Tab to select" explanation, ALT alone skips the current message,
  SHIFT alone says nothing. With real presses, these modifier `KEYDOWN`s
  arrive before the chord's main key (this reference does not press them).
- Backspace in a menu is a first-letter search for `\x08`: it says nothing.
- A unit's status names an order's target through a view of the target
  (`EntityView(...).title`). For a square or a subsquare the property
  raises `AttributeError` (no `player`) and Python falls back to
  `__getattr__("title")`: the place's own title ("go to c 4"). Each such
  view draws one `random()` (`square_orders`).
- The `voila` of a turn carries copies of the memory and the perception
  made before the end-of-game quit deletes the units: the last
  animations of a game still hear them (`events`).
- An exception in the game loop skips the rest of the iteration, the 1 ms
  sleep included (read from `clientgame.py`; no scenario raises one).
- A sound lasts until the last granule position of its Ogg stream
  (SDL_mixer's length), not until the end of the last decoded block.
- Starting a server from the main menu starts a thread (blocked here; see
  the excluded menus).
