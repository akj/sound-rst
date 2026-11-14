# Transcripts of the interface

For a blind player the spoken output is the whole interface.
`../transcripts.rs` drives the client with fixed scripts of key presses and
checks what it says and plays against recorded transcripts.

- `scenarios/*.txt` are the scripts.
- `expected/<scenario>.txt` are the transcripts: everything said, the
  window title, the steps, the length of every sound played, and a hash of
  the positional sounds of each step.
- `expected/<scenario>.sounds.txt` is the full positional-sound stream. It
  is kept only when it is 256 KiB or smaller: the menus, the campaign,
  `french` and `square_orders`. For the longer games only the hashes of
  each step are kept.

The transcripts were recorded from the original Python game. Up to the Git
tag `last-python-parity`, the port matched them byte for byte. Since then
they are the port's own. The driver that recorded them, and its
instructions, are in that tag.

## Run, compare, record again

Run every scenario:

    cargo test -p soundrts --test transcripts

To run one scenario, add its name, for example `menus_main`.

Every run writes the client's transcript and positional-sound stream to
`target/transcripts/<scenario>.txt` and `target/transcripts/<scenario>.sounds.txt`.
The test then compares them with `expected/`, byte for byte. On a
difference, it shows the first line that differs, with its context. It
compares the voice, the titles and the steps first, because a difference
there says more than the sound summaries it shifts.

When the change is intended, record again:

1. Run the tests with `UPDATE_EXPECTED=1`. The tests write the transcripts
   and the streams to `expected/` and pass. A stream that grows past
   256 KiB is removed from `expected/`. A stream that shrinks to 256 KiB or
   less is added. In PowerShell:

       $env:UPDATE_EXPECTED = '1'
       cargo test -p soundrts --test transcripts
       Remove-Item Env:UPDATE_EXPECTED

2. Review the diff of `expected/`.

## The model

The test replaces the clock, the speech device, the audio device, the
keyboard and the window with its own. The rest is the client's own code.

### Clock

- Virtual time is an integer number of microseconds, starting at 0.
- The clock reads `(10^15 + µs) / 10^6` as an f64: the virtual epoch is
  10^9 s. The client's comparisons and sums on these values (for example
  `now >= previous_animation + 0.1`, `next_update = now + 0.3`) are f64
  operations. Boundaries fall where f64 puts them.
- Only sleeping advances the clock: a sleep of `s` seconds adds
  `round(s × 10^6)` µs. Computation takes no time. The client sleeps 1 ms
  per pass of the game loop, 10 ms per menu poll, and 100 ms per pass of a
  blocking voice loop (a message said now, a flush). The text entry loop
  sleeps 1 ms, then is an input point, then reads the queue.
- Transcript times are whole milliseconds: the microseconds divided by
  1000.

### Speech

The speech device records the text and says nothing. The client's own
estimate (`src/tts.rs`) decides when speech ends:

- Saying `text` speaks until
  `now + round(len(text) × d × 10^6) + round(n × (c − 1) × d × 10^6)` µs.
  `d` is `wait_delay_per_character` (0.1, the default of `SoundRTS.ini`).
  `n` is the number of numeric characters in `text`. `c` is
  `tts_digit_coefficient` (3, from `cfg/parameters.toml`). The products
  are f64, left to right.
- Speech goes on while its end is later than now. Stopping ends it. A new
  text replaces the previous one.

### Sounds

- Playing a sound makes its channel busy for the sound's length, or until
  it is stopped for a loop. Stopping a channel frees it.
- The length is the number of frames at 44100 Hz divided by 44100 in
  single precision, rounded to the microsecond. A sound ends at the last
  granule position of its Ogg stream. Every length used is in the
  transcript header (`length NAME µs`).
- Channel 0 is the voice channel. Positional and stereo sounds use
  channels 1 to 15 (`num_channels` is 16).

### The voice

The client's voice runs on top of the devices above. A message is recorded
when the voice channel starts it, as its translated and collapsed parts:
consecutive texts joined with a space, recorded sounds by name.

The voice is idle when the voice channel is not busy (no speech, channel 0
not busy, no queued part) and no message is waiting to be said.

### Input

- Steps add key presses only. No key release is ever added. The keys do
  not repeat. The modifiers of a chord (`key CTRL SHIFT F2`) are not
  pressed as keys of their own: only the main key is pressed, with the
  modifiers held. A modifier alone is a step of its own (`key LCTRL`).
- A key press carries the key, the physical key (the same), the modifiers
  (CTRL, ALT, SHIFT; a modifier key alone holds its own modifier), and the
  text a US layout types. For a letter, the text is the letter, upper case
  with SHIFT, and the control character with CTRL (`a` gives `\x01`). For a
  digit or a punctuation key, it is the US character, shifted with SHIFT,
  and empty with CTRL. `RETURN` and `KP_ENTER` give `\r`, `TAB` gives
  `\t`, `BACKSPACE` gives `\x08`, `ESCAPE` gives `\x1b`, `DELETE` gives
  `\x7f`, `SPACE` gives a space, and the keypad keys give `+ - * / .`.
  Every other key gives no text (arrows, F keys, modifiers, `PAGEUP`...).
  Game commands are bound by the physical key and the modifiers. Menus and
  text entry read the key, the modifiers and the text.
- An input point is a read at which the client takes a key press as the
  player's next action. Only an input point may deliver a step, except an
  `interrupt_after` step, which may also arrive at the end of a sleep (see
  Steps). There are four kinds:
  - `menu`: a menu waits for a choice;
  - `text`: a line of text is being typed;
  - `game`: the game loop reads the keys;
  - `voice`: a message being said looks for a key that interrupts it.

  Every other read, such as the read that drops the keys pressed during an
  alert, or clearing the queue, never delivers.

### Settled

At an input point, the client is settled when:

1. no key press is waiting in the queue;
2. the voice is idle;
3. at a `game` point only: no server event waits to be handled, and the
   interface has handled at least one end of turn (`Voila`) since the
   previous step was delivered.

The `voice` point is never settled for key steps: speech is in progress
there by definition.

### Steps

Steps are taken in order. `t_prev` is the time the previous step was
delivered (0 before the first). `turn_prev` is the world's turn then (0 if
no game was running).

| step | delivered when |
|---|---|
| `key K` | at a `menu`, `text` or `game` input point, when settled |
| `type abc` | one `key` step per character (`A` → `SHIFT a`, space → `SPACE`, `.` → `PERIOD`) |
| `interrupt_after D key K` | as soon as `now ≥ t_prev + D`: at the end of the sleep that reaches it, or at an input point. Not settled first. The key may interrupt speech, or be dropped (after an alert) |
| `wait S` | at an input point, when `now ≥ t_prev + S` |
| `wait_turn N` | at an input point, when a game runs and its turn is `N` or more |
| `wait_turns N` | at an input point, when a game runs and its turn is `turn_prev + N` or more |
| `wait_game_end` | at an input point, when no game runs |

Delivering a key step adds its key press to the queue. At an input point,
the reading function then reads it in the same call. An `interrupt_after`
key delivered at the end of a sleep waits in the queue for the next read,
which may be one that drops it (after an alert).

Note the order inside one loop pass: the input point comes before the
voice update. A message said at once (`item`) whose speech ends at exactly
`now` already counts as idle, so the next key is delivered in that pass. A
queued message (`info`) stays unsaid until the voice update marks it said,
so the next key is delivered one pass later, unless the voice update
started another queued message. After a step that is not a key, the next
step is checked at once, at the same input point. When every step has been
delivered, the scenario ends at the first `menu`, `text` or `game` input
point where the client is settled (`end`), or earlier if the client exits
(`exit`).

The world's turn is its time divided by 300. The time grows by 300 at the
end of each world update.

### The game loop and the world

The world runs in the client's thread. One pass of the game loop:

1. if not waiting for a world update and `now >= next_update`: the orders
   written since the last update go to the world in order;
   `next_update = now + 0.3 / speed`; the world updates and queues its
   server events for the interface (entity events, important messages,
   sequences, speed changes), then `Voila`, the end of the turn; at the end
   of a game, the events of the players quitting follow it;
2. the animations, if `now >= previous_animation + animation_delay`
   (0.1 s);
3. the key presses: the `game` input point, then every queued key;
4. one server event;
5. the voice update;
6. a sleep of 1 ms.

So the events of an update reach the interface one per loop pass, and
`Voila` last. A pass takes 1 ms, or longer when its event is said with a
blocking message. A key pressed in a game reaches the world with the next
update.

### Fixed orders

The client iterates the sets of the world in fixed orders, so the spoken
order of square summaries ("1 town hall, 1 farm and 1 peasant") and of
some news does not change from run to run:

- the memory and the perception of a turn come in ascending entity id.
  This decides the insertion order of the interface's objects, which most
  summaries iterate. An entity that moves from memory to perception is
  removed and inserted again at the end.
- the observed squares are iterated in ascending square id;
- the squares to report (scouting news) are iterated in ascending square
  id;
- the units alert reports its squares in ascending square id.

Folder listings are sorted, and the mods are listed in sorted name order.

### Randomness

- The world's seed is the scenario's `seed`.
- The client's own random generator is seeded with `seed` before the
  client starts. It draws as Python's Mersenne Twister. It is used only for
  sounds: the footstep offset of each entity view (one draw per view
  made), footstep timing, the choice among a style's sound variants,
  repeated noises, and ambient volumes. A new game also draws its seed from
  it, and the fixed seed then replaces the result. No draw decides what the
  voice says. The menus draw nothing.

## Transcript grammar

`expected/<scenario>.txt`, UTF-8, LF, one item per line:

    soundrts-ui-transcript 1
    scenario <name>
    settings language=<l> login=<l> mods=<m> soundpacks=<s> seed=<n> limit=<s>
    length <sound name> <microseconds>           (every sound played, shortest name first, then by name)
    @<ms> <event>                                 (in the order they happen)

A sound name that has another length in another mod also appears as
`<name>@<mod>`.

Events:

- `title <text>`: the window title changed.
- `voice <kind> <part> <part>... [vol=<l>,<r>]`: a message started on the
  voice channel. A part is a JSON string (text for the speech device) or
  `[<name>]` (a recorded sound, by its sound-cache name). `vol=` appears
  when the message's left and right volumes are not the default (`1/√2`).
  Three decimals. The kinds:
  - `item`: says now, not kept in the history, does not block;
  - `alert`: blocks until said; keys pressed meanwhile are dropped;
  - `important`: blocks until said or a key is pressed; the key is used up;
  - `confirmation`, `menu`: block until said or a key is pressed; the key
    stays queued;
  - `info`: a queued message started by the voice update (news);
  - `history`: a message said again from the history (`F5`).
- `voice cut queue=<n>`: speech in progress was stopped other than by a new
  message (a silent flush, sounds stopped, a skip in the history...). `n`
  parts were still queued in the voice channel. When only the sounds were
  stopped, those parts stay there and play on.
- `step <n> <step>`: step `n` was delivered. A `type` step shows the key
  and, in parentheses, the whole `type` step.
- `sounds <count> <fnv64>`: the positional-sound lines of the step that
  just ended: their number and the FNV-1a 64-bit hash of those lines joined
  with `\n`, as they appear in the positional stream. Written just before
  the next `step` (or `end`), only if the step had any.
- `exception <message>`: the client logged an error. Line breaks become
  spaces.
- `end`: the script is done and the client settled, or the client left its
  main menu after the last step. `exit`: the client exited after the last
  step. `abort <reason>`: the run went past `limit`, or the client stopped
  before the end of the script.

Positional stream, `expected/<scenario>.sounds.txt`:

    @<ms> step <n>
    @<ms> pos <name> v=<v> x=<x> y=<y> prio=<p> limit=<l> [ambient] started gains=<l>,<r>
    @<ms> pos <name> ... skipped
    @<ms> loop <name> v=<v> x=<x> y=<y> prio=<p> started gains=<l>,<r> | pending
    @<ms> stereo <name> vol=<v>|<l>,<r> limit=<l> started|skipped

`pos` is a positional sound. `gains` are the channel's left and right
volumes before the main volume. `loop` is a positional loop. `stereo` is a
sound with fixed stereo volumes (menu clicks, alerts, the clock's bell).
Numbers have three decimals. Only starts are recorded, not volume updates.

A comparison that ignores timing can drop the leading `@<ms> `.

## Scenario format

UTF-8 text, one item per line. `#` starts a comment line. Settings come
first, then steps.

    language en          the language (en, fr...)
    login player         the login in SoundRTS.ini
    mods                 the mods (empty: none)
    soundpacks           the soundpacks
    seed 42              the world's seed and the client's random seed
    limit 1800           virtual seconds before the run aborts
    key CTRL SHIFT F2    modifiers CTRL ALT SHIFT, then a key name of res/ui/bindings.txt
    type bob
    interrupt_after 0.5 key DOWN
    wait 2.5
    wait_turn 40
    wait_turns 150
    wait_game_end

The other `SoundRTS.ini` options keep their defaults. The settings folder
starts empty: no campaign bookmarks, no remembered menu choices.

## Scenarios

| scenario | covers |
|---|---|
| `menus_main` | the main menu: title, every item, wrap-around both ways, TAB and SHIFT TAB, first-letter search forwards, backwards and without a match, unused keys, menu help, history, volume, Escape (quits) |
| `menus_options` | modify login (typed letters, a rejected character, SHIFT, Backspace, Enter), a key during the non-interruptible "new login" alert (dropped), an empty login, Escape; the mods menu (Back also closes the options menu); the soundpacks menu; choosing a soundpack (new window title, new main menu, quitting it exits) |
| `menus_single` | the single player menu, a campaign's chapter menu, the map list (a key interrupting the confirmation and the title, browsing, first-letter search both ways), the players menu (invite quiet, aggressive, aggressive 2 until full; start; cancel), a 4-player game cancelled at once with its score, the map list remembering the last map first |
| `training_jl1` | a game on jl1 against a quiet computer: every command of `res/ui/bindings.txt` except `fullscreen` and `say` (168 of its 170 key bindings), building, training, a rally point, gathering, groups, all selection keys, zoom, first person, history with interruptions, cheat mode, the console, changing player, the game menu (speed, three ways in, a quirk), news, quitting and the score |
| `training_long` | about 1,700 turns on jl5 (water, high ground) against an aggressive computer: enemy sightings, alerts, losses, defeat, the score |
| `campaign` | the prologue cut scene, chapter 1 (title, intro, objectives, a farm completing an objective, barracks from the console, the cut scene, victory, score), the victory menu, chapter 2 (computer units attack, a unit neutralized), cancel, the defeat menu |
| `french` | `language fr`: the main menu and its help, a short game on jl1 (status keys, orders, targets, squares, time, help), quit and score |
| `square_orders` | a peasant ordered to a square, then to a subsquare in zoom mode, and its status: the order's target is a place, named by the place's own title |
| `events` | with the console: a research completing, an impossible order and its reason (no food), a small gold mine exhausted, a priest's holy vision on another square (an ability with a target), footmen against the computer's base (fights, neutralized units), victory and the score |
| `transport` | jl5 (water): a boat loads every footman of a coastal square, its status, unloads them on that square |

Not covered: the multiplayer menus and chat. `crates/client/tests/multiplayer.rs`
and `net_play.rs` test them.

## Inherited behaviour the transcripts pin

The port took the following behaviour from the original game, and the
transcripts record it. Each item may look like a bug. Each one goes away
when its behaviour is changed on purpose, and the transcripts are recorded
again.

- Pressing UP first in a menu says the last item (the index becomes -1),
  but RETURN then confirms nothing: an index of -1 is not a choice. Only
  DOWN, TAB, a letter or Escape make a usable selection (`training_jl1`,
  game menu).
- The history (`F5`) does not stop after one message: it goes on through
  every later message of the history, until the end (`training_jl1`: two
  quick F5 say "7 wood" again, then "1 footman enemy at c 2" follows by
  itself).
- An `info` message cut by a blocking message (`confirmation`,
  `important`) is said again afterwards, unless it had been playing for
  more than 1 s. Several blocking messages in a row repeat it several times
  (`campaign`: "barracks 1 at b 1" three times).
- An empty login says an `item` message with one empty text part (it ends
  at once), then the "bad login" alert (`menus_options`).
- Stopping the sounds (volume keys, cut scenes, the game menu) stops the
  speech but leaves the remaining parts of the message queued in the voice
  channel: they play on at the next voice update.
- Choosing a mod or a soundpack opens a new main menu inside the old one.
  Quitting it exits the client (`menus_options`).
- In the mods menu, Back also closes the options menu.
- The map list remembers the last map started (`tmp/mapmenu.txt` in the
  settings folder) and inserts it again at the top, without its
  explanation.
- The first square of a game is announced by the first animation after the
  first world update, up to 100 ms after the game starts, and interrupts
  whatever the first key said (`training_jl1`).
- News are queued as `info` messages and expire after 45 s. A newer message
  of the same update type, or an identical newer one, supersedes them.
- Server events are handled one per loop pass. An entity event handled
  later than `next_update + 3 s` is dropped, so a long blocking message can
  make the interface skip the sounds and the news ("peasant 2 ready!") that
  the event would have produced.
- Saying the time reports the measured speed (`speed 0.1` right after the
  game menu was open for a while: the average turn duration includes the
  pause; `training_jl1`).
- A modifier key pressed alone in a game is a command (`LCTRL` examines,
  `LSHIFT` and `LALT` stop the history). In a menu, CTRL alone says the
  "press Tab to select" explanation, ALT alone skips the current message,
  and SHIFT alone says nothing. With real presses, these modifier presses
  arrive before the chord's main key. The scripts do not press them.
- Backspace in a menu is a first-letter search for `\x08`: it says
  nothing.
- A unit's status names a square or a subsquare target by the place's own
  title ("go to c 4"). Naming a target draws one random number, as making
  an entity view does (`square_orders`).
- The `Voila` of the last turn of a game carries copies of the memory and
  the perception made before the players quit: the last animations of a
  game still hear the units (`events`).
- An error in the game loop skips the rest of the pass, the 1 ms sleep
  included. No scenario raises one.
