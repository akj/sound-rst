# News said while it is fresh

Status: phase 1 built, 18 November 2026, in the recorded speech tests; not yet heard through NVDA. Phase 2 is a proposal.

## The problem

In the game of 17 November (`%APPDATA%\SoundRTS\log-archive\run-2026-11-17_152854\client.log`), the news "1 gold mine and 1 wood at m 2" started about 20 times between 15:31:30 and 15:38:00. Each time, the player's next key cut it off, and a message cut off goes back to the front of the queue. Everything else waited behind it. "town hall 2 complete" was said at 15:38:03, more than seven minutes after the town hall started to recruit. When the game ended, the rest of the news was never said. Alerts wait in the same line, so an attack is said after old production news.

Two rules cause this:

1. A message cut off is said again from its start, however much of it was said (`news_cut_off_after_more_than_a_second_is_said_again` in `crates/client/tests/voice.rs`, and H3 in `docs/open-findings.md`).
2. Only chat, system news and the story go ahead of the other news (`Category::AHEAD` in `crates/client/src/message.rs`). Alerts do not.

That game ran without `nvda_direct`, so the game guessed when speech ended. A guess that runs long makes the player cut more messages off. An exact end of speech does not remove either rule.

## The idea

Keep two jobs apart:

- **The log remembers.** Every message goes into the log, heard or not, and the player reviews it with the keys that exist today. "News is never lost" stays true because of the log. Newer news about a subject still replaces the unheard news about it, as today.
- **The voice says what matters now.** A message cut off is not said again, except an alert, once. Alerts come right after system news. The keys that stop speech stop it, and nothing else starts until there is new news.

Sounds already play at the moment of their event: a finished building, a unit trained, a deposit exhausted, an alert, a chat line. They never wait for speech, and this design leaves them as they are. The sounds are the announcer for frequent events. Speech is for the events that need words.

## Two phases

Phase 1 fixes what the game of 17 November showed. Phase 2 adds freshness and the joining of news of the same kind. Build phase 2 only if a listener still hears old news after phase 1 (step 7 of the listening script). Without the rule that says a cut message again, the queue drains by one message at each key, so a backlog may never build.

## What stays

- The log, its limit of 1000 messages, and the keys F4 (what did I miss), Comma, Period, Slash, Control Shift C, Delete, F5 and F6.
- The seven categories and the `[news]` settings: `speak`, `sound`, `log` and `drop`.
- Newer news about a subject replaces the unheard news about it, and counts join (`News::join`, `Subject`). The words of the replaced news leave the log, as today.
- What stops the game: in single-player the story and the open game menu, on a server only the organizer's pause.
- The story, in a game and between missions. Control or Alt alone acts as any other key there: it skips the line, and Escape skips the rest. The same goes for `Voice::flush` (`crates/client/src/game/source.rs`). Typing in the chat, the console or a login field is unchanged.
- The end of a game (`Voice::end_game`), which still says the waiting system, story and chat news, and a jump in a recorded game.

## Phase 1

### 1. A message cut off

`Heard::Waiting` gains one field, and keeps `Copy` and `Eq`:

```rust
Waiting {
    sounds_only: bool,
    /// Cut off once already: an alert is said again once, nothing else is.
    cut: bool,
}
```

A key's answer, a menu title, an important message or a message of the history cuts off the waiting message that is being said. `Voice::answer`, `Voice::say_now` and `Voice::say_entry` call one function, `give_up_cut_news`:

- An alert with `cut` not set stays waiting, with `cut` set. It is said again after the answer.
- Every other message, and an alert cut a second time, becomes `Heard::No`. F4 counts it, and the review keys find it.
- A line of a story that single-player waits for waits again, as before, however it is cut off: by any answer (the game menu and its items, the history and news keys, copy, help, key help, the volume), by `silence` (Control or Alt alone in the game menu), or by a cancel that the screen reader reports. The voice knows these lines: `GameInterface::tell_story` gives their ids to `Voice::wait_for_story`, and the story mode asks `Voice::story_is_told` and `Voice::skip_story`. Only a skip gives a line up: Escape, or any other key in the story, which calls `say_next`. A line that the screen reader silenced is skipped the same way, so with `nvda_direct` Control alone still skips one line: Control down cancels the line, which waits, nothing starts while Control is held, and on release `say_next` skips that line. Shift alone does nothing in a story. The world waits for the line, so no backlog builds. Key help turned on during the story ends the wait (`Voice::stop_waiting_for_story`): the line it cut off is said once more, and the rest of the story is news like any other, since the world runs again. On a server nothing waits, an objective is ordinary story news, and the rule above applies.
- A cancel that the screen reader reports is a cut, with the same rule: an alert cancelled the first time is said once more.
- F5, the history, cuts off the message being said as an answer does, before it walks back.

Newer news about the same subject replaces the entry with `cut` not set, because a new event is new news. Example: a second attack on the same square after 10 quiet seconds.

The voice counts a message as cut off if the voice channel was still busy. Without `nvda_direct` that is the guess of the end of speech. A wrong guess now only marks a message unheard that the player heard. It no longer repeats it.

### 2. The order

When speech is free, the voice says the first waiting message of the first category in this order: system, alert, story, chat, production, economy, scouting. Within a category, the oldest comes first. `Category::ORDER` replaces `Category::AHEAD` in `next_waiting`.

Two changes:
- Alerts move ahead of production, economy and scouting news.
- Chat moves from first to fourth, behind system news, alerts and the story.

`end_game` keeps its own list of the three categories it still says: system, story and chat.

### 3. Stopping speech: Control and Alt alone; Shift does nothing

A new command, `silence` (`Voice::silence`), stops speech. The message being said and every waiting message become `Heard::No`, except the lines of a story that the world waits for (section 1). Nothing starts until new news comes, or until the player asks for something. A walk through the history with F5 and F6 ends too. It replaces `Voice::silent_flush`, whose callers (a game's start, a replay's jumps) call it instead.

The command runs on Control or Alt, pressed and released alone (H4 made them work this way). Shift alone does nothing. Both rules hold in a game, in the menus and in the lobby:

- **In a game:** the port's client bindings (`CLIENT_BINDINGS` in `crates/client/src/bindings.rs`) bind `LCTRL`, `RCTRL`, `LALT` and `RALT` to `silence`, and `LSHIFT` and `RSHIFT` to a new command `nothing`, which does nothing. They come after the original's bindings, so they replace them, and a player's own `bindings.txt`, read last, can still bind any of them. A command that does nothing uses the file mechanism that exists; the alternative, a way for the client's file to remove a binding, needs new syntax for one case. The test `the_clients_keys_are_free_in_the_original_and_name_its_commands` names these six keys as its one exception.
- **In the menus and the lobby:** `menu.rs` binds its keys in code. Today Alt calls `say_next(false)`, Shift does nothing, and Control falls through to "press Tab to select. press Enter to confirm.". Alt and Control call `silence` instead. Shift still does nothing. In the log of 17 November, the menu says that sentence twice in the first five seconds, probably for Control presses that silenced NVDA.

Two further rules make the stop clean:

- **While Control or Alt is held, the voice starts no waiting message.** NVDA and JAWS silence their speech when Control goes down. With `nvda_direct`, NVDA reports the cancel at once, and today the voice then starts the next message (`VoiceChannel::update` and `Voice::update` in `voice.rs`). With this rule, nothing starts before the key is released. Released alone, the key silences. Released after a chord, the chord's answer is said, and then the queue goes on.
- **A message that the screen reader reports as cancelled becomes `Heard::No`, not `Heard::Yes`.** The player stopped it, so F4 finds it. Today's README says that such a message counts as heard. That sentence changes.

The original binds Control alone to `examine`, and Alt or Shift alone to `history_stop`, which skips to the next message. Both meanings change; the developer chose this on 17 and 18 November. One case from H4 changes with them: Alt pressed and released while two other keys are held now silences, where it skipped one message before. Shift is left to the screen reader: NVDA's own Shift pauses its speech, and a second Shift resumes it. The listening script checks that the game does not fight that pause.

### 4. Examine moves to Shift C

Examine ("describe the target or the square") moves to Shift C. The original leaves Shift C free (`c` is the food status, Control C the game menu), and neither Windows nor a screen reader takes it. It sits with the status keys, Z to V, near where the left hand held Control.

### 5. A change of side gives up the waiting news

A watcher who follows another player, and a seat that comes back after a lost connection, call `Voice::new_side`. Today that gives up only the held counts, so the old side's waiting news is still said (`docs/open-findings.md`, the watcher findings). `new_side` now also gives up the waiting news, as a jump in a recorded game does: it stays in the log, unheard.

As built: a seat that comes back to the same seat calls `new_side` before "back in the game" is said (`back` in `crates/client/src/game/server.rs`); one moved to another player calls it again when it follows that player. News that belongs to no side, system news, the story and chat, the categories that `end_game` keeps, is still said. The message being said goes on; an old side's alert being said is marked cut, so it is not said again if it is cut off.

## Phase 2, only if a listener still hears old news

### 6. Freshness

A waiting message goes stale after a time set by its category. A stale message becomes `Heard::No` when the voice next picks a message, and is not said.

| Category | Fresh for (seconds) |
|---|---|
| System, story, chat | always |
| Alert | 10 |
| Production | 8 |
| Scouting | 8 |
| Economy | 6 |

The seconds are real seconds during which the world runs. The game's own time is not used, because the console can set any speed, and at the speed "fast 4" a message of 3 seconds would use up most of its freshness while it is said. The voice keeps a fresh clock. The game interface starts it when a game starts, stops it where single-player pauses (the story, the game menu, the console), and clears it at the end of a game. Outside a game the voice has no fresh clock, and nothing goes stale.

Data: `Entry` gains `queued: Option<f64>`, the fresh clock when the entry last became waiting. `Category::fresh_for() -> Option<f64>` holds the table. Replacement by newer news resets `queued`. The numbers are constants, to tune by ear.

### 7. News of the same kind joins

A burst of the same news ("footman 3 ready", "footman 4 ready", up to "footman 10 ready") is said as the newest one. Two new subjects do this with the existing `.about(subject)` replacement:

- `Subject::Complete(type)` for a unit trained or a building finished, in `on_complete` (`crates/client/src/game/entity.rs`).
- `Subject::Added(type)` for a unit that appears without being trained, in `EntityEvent::Added`.

## The simpler options

- **Only `nvda_direct`.** The game knows when NVDA has finished, so fewer messages count as cut off. A message the player does cut off still blocks the queue, alerts still wait, and it does nothing for JAWS.
- **Phase 1** is the simplest change that ends the loop of 17 November and puts alerts first. It changes one enum field, the order of the categories, one new command with its bindings, and `new_side`. It adds no module.
- **Phase 2** adds a clock that the game interface must start and stop, and two subjects. It is worth its cost only if a listener reports old news after phase 1.

A separate announcer queue beside the log is not proposed. It would hold a copy of the state the log already holds. A recorded announcer voice, which could play over the screen reader's speech, needs recordings of every phrase in every language. The original has only text, so that is left for a sound pack.

## Every way to play

- **Single-player:** the story keeps its own keys. Phase 2's fresh clock stops while the world waits.
- **A game on a server:** the same rules.
- **Seats:** a watcher hears the news of the player that it follows. A change of player and a seat that comes back give up the waiting news (section 5).
- **Input from strangers:** no new input. A map, a message or a package cannot change the order or the freshness.
- **Packages:** a player's own `bindings.txt` can still bind any key, the modifiers included.
- **Keys:** Control and Alt alone silence, in a game and in the menus. Shift alone does nothing. Examine moves to Shift C.
- **The README:** see below.

## Rules and documents that change

- `AGENTS.md`, "Speech and sound are the whole interface": "News is never lost. A message that is cut off is said again." becomes "News is never lost: every message stays in the log, and the player can hear what was missed. Alerts are said first. A message that is cut off is not said again, except an alert, once."
- `AGENTS.md`, "The original's files, imported": Control and Alt alone silence, Shift alone does nothing, and examine moves to Shift C. This is an exception to "the original's keys keep their meaning", by the developer's choice.
- `README.md`, News: the paragraph about news that is never lost, the order, and the keys that silence. In "Speech that ends when NVDA says so", "A message that you silence with Control counts as heard" becomes "Control silences speech, and the news you did not hear stays in the log for F4".
- `docs/open-findings.md`: H3 points to this design, and records the game of 17 November as the report by ear that H3 asked for. The watcher finding about news after a change of player is fixed by section 5.

## Tests for phase 1

Each test expects the literal words, and fails without the change:

1. A production message cut off by an arrow key is not said again. F4 then says "production 1".
2. An alert cut off by an arrow key is said once more. Cut off again, it is not said a third time.
3. An alert that comes after three waiting production messages is said before them.
4. Control alone, while a message is said and two wait, silences. Nothing is said next. F4 counts the three.
5. Alt alone does the same as Control alone, in a game and in a menu. Shift alone, while a message is said and two wait, changes nothing: the message is not cut, and the two are said after it.
6. Control Up (a chord) moves the cursor and leaves the waiting news waiting. After the answer, the next waiting message is said.
7. With the fake NVDA reporting a cancel while Control is held, no message starts until Control is released. The cancelled message is `Heard::No`.
8. In a story, Control alone skips the line and the story goes on, as another key does.
9. Shift C says the square. Control alone no longer says it.
10. A watcher who changes player hears none of the news that waited for the first player.
11. The 17 November game in short, on the speech device that guesses the end of speech: two scouting messages and three production messages wait, and a key comes every 2 seconds for 60 seconds. No message starts more than twice, and only an alert starts twice.

Two existing tests turn around: `news_cut_off_after_more_than_a_second_is_said_again` (`crates/client/tests/voice.rs`) expects the message not to be said again, and `a_count_cut_off_is_said_again_with_what_it_counted_since` keeps only the case where newer counts replace the cut news. The transcript recordings change where news was said again. Record them again only after the tests above pass, and read the changes before you accept them.

## Listening script for the pull request

Play the training game on jl1 against an aggressive computer, with `nvda_direct = true`. Then play it once more with `nvda_direct = false`.

1. Press Shift C. Expected: the square under the cursor, as Control said it before.
2. Press and release Control while news is said. Expected: speech stops, and no other news starts.
3. Press F4. Expected: the categories of the news you stopped, for example "production 2".
4. Press and release Alt while news is said. Expected: speech stops, and no other news starts.
5. Press and release Shift while news is said. Expected: NVDA pauses. Press Shift again. Expected: NVDA goes on with the same message, and the game does not start another over it.
6. Let the computer attack you while you press arrow keys. Expected: "alert at …" soon after the horn, before production news.
7. Play fast for five minutes, then stop pressing keys. Expected: within ten seconds the voice is quiet, or says only news of the last few seconds. If it says old news, phase 2 is needed.
8. In the main menu, press and release Control. Expected: silence, not "press Tab to select".

## Decisions for the developer

1. The key for examine: Shift C as proposed, or another key the original leaves free.
2. Build phase 1 alone first, and phase 2 only if step 7 of the listening script fails.

Decided on 18 November: Alt alone silences, as Control does. Shift alone does nothing.
