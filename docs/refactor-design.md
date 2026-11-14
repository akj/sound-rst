# sound-rst: design for the next version

A design for rebuilding sound-rst around five pieces of work, written 14 November 2026 against commit `612afd1` on `main`. It starts from the 33 open findings in `docs/open-findings.md`, the seven review reports behind them, and the state-machine review that followed.

Section 13 records the decisions taken on it. Section 12 is the order of work.

## 1. In short

The port is a faithful copy of a Python program from the 2000s. That was the right first goal, and the recordings prove it was met. It is the wrong shape for what comes next. Most of the 33 findings are not 33 separate problems. They are the same four structural facts showing up in different places:

1. **The interface reads the simulation's memory directly.** The client holds the live world and reads it in about 110 places. So does the computer player. Nothing in the code separates "what is true" from "what this player knows".
2. **Every machine runs its own copy of the game and must agree bit for bit.** Reconnecting, watching, pausing, a slow player, a different processor: each is hard only because of that choice.
3. **Speech is a queue driven by a guess**, and the game loop stops and polls while it waits. For a blind player speech is the screen, so this is the display driver being unreliable.
4. **The code still thinks in Python**: names as strings, bare numbers, errors imitated, state that only exists as "these four fields happen to agree".

I recommend five pieces of work. The first four answer those facts; the fifth is the one new capability worth designing now. They are numbered as their sections are, which is not the order they are built in.

1. **One seam** (section 4). The simulation takes typed commands and gives each player a typed view of what that player knows. Nothing else crosses, and the compiler enforces it. This goes first. It is worth doing whatever is decided about the rest, and everything else gets easier after it.
2. **One authoritative host instead of lockstep** (section 5). One process runs the game: the server in multiplayer, the client itself in single-player. Clients send commands and receive their own view. Out-of-sync stops being possible, and reconnect, watchers and pause become small features. This is the boldest call, it is the one an independent review disagreed with, and it is built after the seam and the speech work, so nothing done first commits you to it.
3. **An announcer instead of a voice queue** (section 6). Speech becomes a subsystem with typed categories, priorities and a log that never loses anything, driven by events and not by sleeping. NVDA can tell the game exactly when it has finished speaking, and the game should ask. The game never blocks on speech.
4. **Rust, not Python written in Rust** (section 7). Ids resolved at load, quantities with units, enums for orders and entities, no imitated exceptions, one strict loader for everything from outside. Done module by module under the recordings, not as a rewrite.
5. **Sound in three dimensions** (section 10), decided by a half-hour listening test before anything is built into the game.

Of the 33 findings, 26 land inside the first four pieces: 16 stop existing because their cause is gone, and 10 are still work but have one obvious place to go. Seven of the 16 depend on the host. Appendix A maps each one.

What stays: the map of named squares joined by exits, the 300 millisecond turn, every unit and cost in `rules.txt`, the text formats for rules, maps and computer players, every existing key, the 13 languages, and the lobby protocol and server written for the port.

**How this document was checked.** After I wrote it, Sol reviewed it against the code with instructions to attack it. Sol found that I had overstated what the traces prove, promised a first phase that could not be both neutral and corrective, left out much of what a host needs, and rested the host on a benchmark that did not measure a host. I checked those points and they were right. The document is changed accordingly, and one more measurement was run to fill the gap. Sol's conclusion was "keep lockstep for now". Mine, after that measurement, is still the host, later and with the full cost stated. Section 5 sets out both. The host was chosen (section 13).

## 2. What this rests on

Labels, as in the findings doc:

- **Read today:** I read the code at `612afd1` myself.
- **From the reviews:** traced by one of the seven review agents and cited by file and line. I re-read the reports in full, not the code behind every line.
- **Reproduced:** the state-machine review wrote a throwaway test that shows the bug, in the test harness or at the world's interface. Not in a real game with NVDA.
- **Measured:** a number from running code, with the method in appendix B.
- **Researched:** a fact about an outside library or API, with its source in appendix C.
- **Judgment:** my design opinion. Most of this document.

Nothing in this document has been heard through a screen reader. Every part that changes what you hear says so, and section 12 groups those into listening sessions.

**What was measured.** An Opus agent timed whole games of computers against computers, in a release build, on a laptop that was 20 to 38 percent busy with other things. Appendix B has the method. The four numbers the design leans on:

- **A turn is cheap.** A 2-player game costs about 0.1 milliseconds a turn, an 8-player game about 1, and a 16-player game on the busiest map 10 to 13. A turn lasts 300 milliseconds.
- **The sync check costs as much as the turn, or more, and its cost never stops growing.** It walks every entity ever created and every defeated player's memories. In the 16-player game it reached 20 milliseconds a turn, twice the turn itself. The earlier figure of 0.7 milliseconds holds for an 8-player game only.
- **Replaying an hour of play takes 1 to 11 seconds for games of 2 to 8 players**, and about 2 minutes for the 16-player one.
- **Nothing is freed.** The 16-player game created 10,314 entities and had 2,529 on the map at the end.

No game panicked or logged a warning.

**What was measured second.** Sol's review pointed out that those games had computer players only, and the simulation builds a view only for a human player, so nothing above says what sending views would cost. A second run switched views on for every seat in the same games, confirmed turn by turn that the games stayed identical, wrote each view out as bytes, and compared each turn with the last. In short: a few kilobytes a second for each seat, and a few percent of one processor in the worst game. Section 5 has the numbers and appendix B the method.

**What was reviewed.** Sol did two things: it surveyed where outside data enters the program (section 9), and it read the first full draft of this document against the code with instructions to attack it. Where a claim here was corrected by that review, the text says so. The document as it now stands, after those corrections, has not been reviewed again.

## 3. The spirit, and what is free to change

I take "the spirit of the game" to be these, and the design holds them fixed:

- **A map you can hold in your head.** Named squares on a grid, joined by exits. You think "c3", not coordinates. Movement between squares goes through exits, and blocking an exit matters.
- **Positions inside a square that you hear.** Footsteps move across the stereo field. Zoom divides a square into nine. This is what makes it an audio game and not a text game, so continuous positions stay.
- **A classic base-building game.** Two resources, food, a tech tree, fog of war, the medieval roster. No unit, cost or timing changes.
- **Playable at a human pace by ear.** Selection by type, a default order, groups as fractions of an army. The game never demands that you click on a thing.
- **Moddable with a text editor.** Rules, maps, computer players and sounds are text files and folders. A blind modder needs no tool but an editor. The 88 maps and the existing mods keep loading.
- **Yours to host.** A server anyone can run, including from inside the game.

Free to change, because nobody plays the game for them: how the program is divided, how machines stay in step, how speech is scheduled, the inherited bugs, the computer players' privileges, and how settings are stored.

## 4. Move 1: one seam between the game and everything that watches it

### Now

Read today. `GameInterface` owns the `World` (`crates/client/src/game/mod.rs:160`). The client calls into it about 110 times: `world.square` 28, `world.player` 20, `world.entity` 14, and it writes to it in four cheat paths. Each turn the world also hands the client a `Voila`: copies of every entity the player perceives or remembers. Those copies are whole `Entity` values (`crates/client/src/game/entity.rs`, `EntityView.model`), so the client holds an enemy unit's order queue and pathfinding scratch data along with its position.

From the reviews: the computer player reads the world the same way, which is how it knows what is really on a square when choosing a warehouse site. Commands travel the other way as text lines (`order 0 1 go 12`), built with `format!` in the client and re-parsed in the simulation, and built again as text inside the simulation in about 30 places in `ai.rs`.

### Design

Three types, and nothing else crosses the boundary.

**`Command`** is what a player asks for. An enum, parsed nowhere because it is never text:

- `Order { units, order, queue: Replace | Append, imperative }`, where `order` is an enum with one variant per order and typed arguments: `Go(Target)`, `Train(UnitTypeId)`, `Build { building, at }`, `Use { ability, target }`, `SetStance(Stance)` and so on.
- `SetGroup`, `Surrender`, and `Cheat(...)`, which the host refuses unless the game allows cheats.

A target in a command is a square, a ninth of a square, or an entity id. The simulation resolves entity ids through the sending player's knowledge, as `get_object_by_id` does today, so a command cannot name something the player has not seen.

**`View`** is what one player knows at the end of a turn:

- the player's own resources, food, upgrades, objectives and score;
- every player's public line: name, ally or enemy, playing or defeated;
- entities perceived now, and entities remembered with when they were last seen;
- which squares are observed, remembered or unknown;
- the player's control groups.

An entity in a view is a deliberate projection, not a copy. Every entity has the observable part: type, owner, place, position, facing, hit points, what it is visibly doing. The player's own and allied entities add a second part: order queue, cargo, units inside, stance, rally point. An enemy entity has no second part, so code that tries to read an enemy's orders does not compile. That is the fog of war enforced by the type and not by care.

**`Event`** is something that happened this turn that the player could perceive: a typed enum that carries the entity's observable state at the moment it happened. Today's events do the same with a whole copy (`crates/world/src/world.rs:849`). The moment matters: an attack must sound from where the unit was when it struck, not from where it stood at the end of the turn.

Static data goes once, at the start: the rules a player may know (unit types, costs, requirements) and the map layout (squares, exits, terrain).

**Order menus are an open point.** My first draft had the client compute a unit's menu from the rules and the view. Sol's review showed that is not enough (`crates/world/src/orders.rs:398`): whether an order is offered also depends on technologies the map forbids, research queued in another building, unit counts, and room in a transport. So either the view carries each of the player's units' menus, which costs bytes, or the shared function takes those extra inputs, which widens the view. While the simulation runs in the same process the client simply asks. The choice only has to be made if the game moves to a host, and the measurement in section 5 should include it.

### What the client knows today that the player should not

Sol traced the client's reads and found five places where it uses more than the player has seen. I checked three of them against the code.

- **Fight announcements.** Checked. To decide whether to say a fight was won or lost, the client reads the full unit list of the owner of the unit that died (`crates/client/src/game/entity.rs:840`).
- **A remembered enemy's upgrades.** Checked. Describing a unit you saw minutes ago uses its owner's upgrades as they are now (`entity.rs:262`).
- **Footstep sounds.** Checked. Which surface a step sounds like depends on every object truly on the square (`entity.rs:928`).
- **Exit blockers.** From Sol. Whether an exit sounds blocked looks at blockers on both sides, seen or not (`entity.rs:525`).
- **Order targets.** From Sol. Describing an order's target reads the live entity (`entity.rs:389`).

These matter for how the seam is built. A view that hides them changes what is said, so the move cannot be both "nothing you hear changes" and "the leaks are closed". It is done in two steps, and the order of work in section 12 keeps them apart:

1. **Neutral.** The view carries exactly what the client reads today, with these five marked by name as things the player should not have. Every existing recording stays byte-identical. That proves the move changed nothing, for what the recordings cover.
2. **Corrections, one at a time.** Each leak is closed with a decision about what you should hear instead, and a transcript difference that is expected and read.

### What enforces it

In the first step, the crate boundary that already exists. The client is a separate crate from the simulation, so making the world's fields private and exporting only commands, views and events is enough for the compiler to stop the client reaching in. No new crate is needed for that, and Sol was right that my first draft asked for them too early.

New crates arrive only when a rule needs one:

- **A crate for computer players**, when they move behind the view (section 8). Inside the simulation crate nothing can stop them reading the world; outside it, the compiler does.
- **A crate for the shared types and one for the host**, only if the game moves to a host (section 5), because only then must the client not link the simulation at all.

### What it gives

- **C1 and C2 close.** A misspelt order does not compile. After the corrections, the leaks above cannot come back, because the data is not there to leak.
- **Watchers, replays from any side, and fair computer players** all become "ask for a different player's view". Today each would be its own project.
- **The world's fields become private** (C5), because nobody outside needs them.
- **The client can be tested without a simulation**: feed it views, check what it says.

### The simplest alternative

Read-only accessor functions on `World` that the client calls in place of reading fields, with the client still owning the world. That is perhaps a third of the work. It gives up one thing that matters: the view is not a value. It cannot be handed to a computer player, a watcher or a test, compared with last turn's, or sent anywhere. Every later move wants that.

### Cost

Large. The client's 110 reads are rewritten against the view, and the text commands become typed ones. Session set-up, the record check and the score screen also build or read a world today (`session.rs`, `record.rs`, `stats.rs`) and move behind the same boundary.

This move is worth doing whichever way section 5 is decided. It is the keystone: moves 2, 3 and 4 all get easier after it, and none of them gets easier before it.

## 5. Move 2: one authoritative host instead of lockstep

**Decided: the host** (section 13). This section is kept as the record of the argument and the cost.

This is the part of the design two reviewers disagree on. I set out the case, the cost as Sol corrected it, what was measured, and then both conclusions.

### Now

Read today. Every client runs the whole simulation. The server is a lobby, a chat and a turn clock; it relays each player's text orders to all (`ServerMsg::Turn`), and each client reports a digest of its world after each turn (`ClientMsg::TurnDone`). If digests differ the minority is removed and a record is saved.

Lockstep exists in strategy games for one reason: when a game has thousands of units, sending state costs too much, so you send inputs and make every machine compute the same state. Everything else about it is cost:

- Every machine must compute bit-identical results. That is why the simulation avoids floating point, why M11 worries about ARM, and why the digest, the mixed-build check, the out-of-sync flow and the game record exist.
- The game runs at the pace of the slowest machine (M3), and a blocked client stops everyone (H2).
- A client that drops cannot come back without replaying the game from the start (M1). Nobody can join late or watch (M8).
- Every client holds the whole truth. A modified client can read the entire map. Fog of war is a courtesy.
- Every client parses every map and every other player's orders, which is where section 9's crashes come from.

### Design

SoundRTS does not have thousands of units. The food limit is 80 a player. Measured: at the peak of the busiest 16-player game there were 440 units among 2,860 things on the map. An 8-player game had 370 units at the end of an hour. So send state.

A **host** owns one game: the simulation, the computer players, the turn clock, the command intake, and the record. Each turn it applies the commands that arrived, steps the simulation, and sends every seat what changed in its own view, and its events. A seat is a player or a watcher.

- **Single-player:** the host runs inside the client, stepped directly. The client talks to it through the same interface as to a server. "Is this a network game?" stops being a question the game interface asks; today it is stored four times (from the reviews).
- **Multiplayer:** the server runs the host. A server started from inside the game does the same on the hosting player's machine.
- **The server needs no game data on disk**, but it does need the whole description of a game, which is more than I first wrote. See the costs below.

### What it gives, finding by finding

- **Out of sync cannot happen.** There is one world. The digest per turn, `TurnDone`, `OutOfSync`, the out-of-sync speech and the mixed-build refusal all go.
- **M1, reconnect:** the host keeps a dropped seat for a grace period. When the client returns with its session token, the host sends the current view and the events it missed. No replay. Measured: a client 50 turns behind needs about twice one turn's data, not fifty times.
- **M3, the stalling player:** the host does not wait for anyone. A slow client slows only itself.
- **M4, pause and speed:** the host stops or retimes its own clock. No agreement protocol.
- **M8, watchers:** a seat with no units and a view that follows a chosen player. Joining late is the same as reconnecting.
- **M10, records for every game:** the host writes the command log for every game, as the record does now only for games that go out of sync. A save is the log so far, and loading replays it. Measured: about a second for an hour of a 2-player game and 11 seconds for an 8-player one. Section 7 says what that does not cover.
- **H2, the blocking message:** in single-player the host pauses the world for a cut scene, on purpose and by name, while the client stays live. In multiplayer nothing pauses.
- **C6, the turn on the key thread:** the host can run on its own thread in play, and stepped by hand in tests.
- **Hidden information is real.** A client only ever receives what its player perceives.
- **Hostile maps and orders reach one machine.** Only the host parses rules and map text, and commands are typed.

### What it costs, as corrected by Sol's review

My first draft listed bandwidth, server work and the retired lockstep fixes. Sol read the code for what else a host needs, and I checked its references. All of these are real work the first draft left out:

- **A complete description of a game.** Building a world needs the final layered rules and computer-player text, the map, the seed, each seat's controller, faction and alliance, the default triggers, the cheat policy, and two start-up policies (`crates/client/src/session.rs:284`). The server receives almost none of that today. Folder maps cannot be sent at all: they have no bytes to upload (`mapfile.rs:30`). The record's set-up is written for multiplayer only and is not yet a campaign save.
- **A second contract for presentation.** A client still needs the sounds, style and translations that go with a map or mod in order to present the game. Today a content hash checks that everyone loaded the same rules. Under a host that check goes, and something must replace it for presentation data, and for the faction list in the game room, which the client now derives from its own files.
- **Events that cannot be skipped.** A view can be skipped: the newest one is enough. An event cannot: a chat line, an objective, a completed building must each arrive once, in order. So events are a stream with a position per seat, kept for a bounded time; a client that falls further behind than that gets the full view and a plain statement of how many events it missed.
- **A handshake for stories and endings.** The client must tell the host when a cut scene has finished so the world can resume. The final statistics and the victory must reach the client before the game is torn down, because unlocking the next campaign chapter depends on them.
- **A host the tests can step.** The test harness steps the game one pass at a time and reads the world directly (`crates/client/src/testkit.rs:930`). A host on a thread would break every client test. So the host is built first as something stepped by hand, with the thread added around it.
- **Cheats and the console** change the world at once today, and "play as the next player" changes who receives events. Each becomes a command the host handles.
- **Order menus**, the open point from section 4.
- **Trust moves to the host.** A player hosting from inside the game can see everything, as every player can today. A standalone server cannot be cheated by any player.
- **Yesterday's work.** The lockstep safety fixes in pull request 3 mostly retire. The game record from pull request 4 survives as the host's command log.

### What was measured

Two runs, both by Opus agents on throwaway tests; appendix B has the method and the caveats. The second exists because Sol pointed out that the first measured computer players only, and the simulation builds views only for human players. It switched views on for every seat, confirmed turn by turn that the game stayed identical, wrote each view out in a simple byte layout, and compared each turn with the last.

**The work.**

- A turn costs about 0.1 milliseconds with 2 players, 1 with 8, and 11 with 16 on the busiest map. A turn lasts 300.
- Building every seat's view with today's code adds about 6 percent to an 8-player turn and 26 percent to the 16-player one. Encoding and comparing adds about 5 microseconds a seat in the 8-player game and about 280 in the 16-player one.
- All together: about a third of one percent of a processor for an 8-player game, and about 6 percent for the worst 16-player game, with single turns up to about 18 percent.
- Today each client spends more than that in a large game, because each runs the sync check: measured at 20 milliseconds a turn in the 16-player game.

**The bytes**, per seat, uncompressed:

- Sending only the fields that changed, plus events: about 1 kilobyte a second in the 2-player game, 0.3 in the 8-player game, and 2.3 in the 16-player game. At the 99th percentile: 1.5, 1.9 and 17.
- The full view, sent once when a seat joins or returns: about 3 kilobytes with 2 players, 2 to 8 with 8, and 32 to 78 with 16.
- Events are small: usually nothing, and 240 to 350 bytes on a busy turn.
- Sending the full view every turn would not work at 16 players: over 100 kilobytes a second for each seat.

**A surprise worth fixing whatever is decided.** Each turn the simulation re-stamps the "last seen" time of remembered objects on partly observed squares, though nothing about them changed (`crates/world/src/playerlogic.rs:448`). In the 16-player game that was 73 percent of everything that changed. A protocol should not send it, and "in the fog, 3 minutes" may be reading younger than it should.

**What these numbers are not.** The byte layout is a prototype, not a protocol. The views are computer players' views; a person scouts differently. No seat had an ally, and allies' orders would add bytes. Order menus are not in it. It is one machine and one seed. And, from Sol's second round, the stream measured is smaller than the stream proposed: an event was counted as an id, a position and a small payload, not the observable state section 4 promises; chat and other events not about an entity were given no bytes; zoom and point targets were sent without coordinates; and the catch-up figure leaves out the events that pile up while a client is behind. Phase 3's first task encodes the complete contracts, catch-up events included, before anything is built on these numbers.

### Sol's conclusion, and mine

**Sol: keep lockstep for now.** The host is plausible, the document understated its cost and overstated its evidence. Define the contracts and a host that is stepped by hand, keep everything running locally while the client is moved onto them, then measure the real stream and decide about networking. Deliver speech improvements independently.

**Mine: the host, built third.** I accept every correction above and the order Sol proposes. Where I differ is the destination. The measurement Sol asked for has now been run in prototype, and it came out affordable: a few kilobytes a second and a few percent of a processor in the worst game we could construct. What is left of the objection is cost, and the cost buys things lockstep cannot give at any price: a fog of war that holds, a seat you can return to without everyone waiting, and one machine instead of every machine parsing hostile text.

**What the two positions share is the whole of the next phase.** The seam, a host stepped locally, and single-player running through it are worth having under lockstep too, and none of it commits you. Lockstep multiplayer keeps working until the day the server-run host replaces it. So this decision does not have to be made now. It is made at a gate after phase 1, on three questions:

1. With the real view type and real play by people, do the bytes stay in the range measured?
2. Is the list of costs above complete, now that the client has actually been moved onto the seam?
3. Do you still want what only the host gives?

### The simplest alternative, compared

Keep lockstep and fix the findings one at a time:

- M1: the server keeps released turns and a returning client replays them. Measured, everyone waits 11 seconds an hour into an 8-player game and two minutes in the 16-player one, and about double with the sync check on.
- M2 heartbeat, M3 host removes a player, M4 pause and speed at an agreed turn, M8 watcher slots fixed before the start.
- The sync check reworked so that it stops growing; measured, it already costs more than the turn in large games.

That is five medium-to-large pieces, each built on the bit-identical requirement. At the end the fog of war is still unenforced, a watcher still cannot join late, every client still parses every map, and every one of the desync mechanisms is still code to maintain. What lockstep keeps that the host gives up: a server that does almost no work and needs to know nothing about the game, and clients that can each keep a full record of a game.

If you choose this, the seam and the announcer are unchanged, and the multiplayer findings become their own list of fixes in place of phase 3.

### What would change my mind

- Real bytes, with people playing and allies, several times the prototype's.
- The move onto the seam turning up costs on the scale of the ones Sol found, again.
- You wanting play to continue when the host disappears (M5). Lockstep makes that possible in principle; with a host it needs a state hand-over, which this design leaves out.
- Multiplayer mattering less to you than single-player. Then the host's benefits shrink to pause and saves, and lockstep with a few fixes is enough.

## 6. Move 3: an announcer, and a client driven by events

For a blind player this is the part of the refactor you will notice. Everything in it needs your ears.

### Now

From the reviews, with the key lines read today:

- The end of speech is a guess: 0.1 seconds a character (`tts.rs`). The guess decides when the next message starts, whether a cut message counts as heard, and whether the stop key reaches the screen reader at all.
- Seven message kinds, three of which block the whole game loop while polling for keys every 100 milliseconds (`voice.rs`, `say_now`).
- News is dropped in at least six ways: cut after one second counts as heard, events three seconds late are discarded, one event is handled per loop pass, resource news expires in 1.5 seconds, attack alerts are throttled and then discarded, and the history drops 100 messages at once.
- Reproduced: a chat line or alert that arrives while a square is being described is marked "said" without ever being spoken, when you press the next arrow more than a second in.
- The screen reader is called on the same thread that handles keys.
- The game interface is one structure of about 70 fields, with input modes spread over three independent fields. Reproduced: that is how Shift+F7 in the game menu sends an allies-only message to everyone.
- Reproduced: an event from the server is stored as a flag that something must remember to clear. A late "you have been removed" leaves it set, and then every prompt in the lobby closes itself.

The state-machine review recommends a small fix for each of those three, and judged the larger shapes (one enum for what is on the voice channel, one for the input mode, one for the connection's phase) not worth their cost as separate projects. I agree with that judgment for a patch. In a rebuilt client those shapes are not extra work: they are how the new loop is written, and the three bugs become things that cannot be expressed.

### Design: the announcer

One subsystem owns everything spoken. Game code never calls the screen reader; it submits an utterance and moves on.

An **utterance** has: its parts (text and recorded sounds), a **category**, a **source** (you asked, or the game volunteered), an optional **subject key**, and whether it is logged.

**Categories are a type**, not words matched in a string: navigation, answer, alert, production, economy, scouting, chat, system, story. Each category has a policy, and the policy is data you can change in the options menu: speak it, play only its sound, log it silently, or drop it. That replaces the `verbosity` string (H6).

**Four rules replace the seven message kinds:**

1. **What you asked for speaks at once and interrupts.** Moving the cursor, asking a status key. This is today's `item`.
2. **What the game volunteers never interrupts what you asked for, and is never lost by being interrupted.** A news message cut off part-way goes back to the front of the queue. It is not "counted as heard" (H3).
3. **News about the same subject replaces older news about it.** The subject key does this: three "peasant ready" messages queue as "3 peasants ready"; a newer resource count replaces an older one. This is how the queue stays short in a fight without dropping anything, and it replaces the expiry timers.
4. **Everything is logged, spoken or not.** The log is browsable by category: all messages, or only alerts, or only chat. F5 and F6 keep working. A new key answers "what did I miss": it says how many unheard messages wait in each category and lets you step through them.

Alerts get one more thing: every alert is tied to a place, and one key jumps the cursor to the last alert's square, and again to the one before.

**The end of speech comes from the screen reader when it can tell us**, and from a measured rate when it cannot.

- **NVDA can tell us, exactly.** Researched, and I read the interface definition myself. Since NVDA 2024.1 the controller client has `nvdaController_speakSsml`. Called in its synchronous form, it returns when the speech has finished, or returns a "cancelled" error when the speech was cut off, for example because you pressed Control. So the game learns both that speech ended and whether you silenced it. A message you silenced yourself is treated as dismissed; a message the game interrupted goes back in the queue. NVDA 2026.3 adds `nvdaController_isSpeaking` as a second check. The call takes a priority, so "speak after the current message" and "speak now" can be NVDA's own queueing and not ours.
- **This needs the current client file.** The one shipped with Tolk is the 2020 build with neither function. The current one is published by NV Access under LGPL 2.1 and has the same file name, so it replaces Tolk's copy. No published Rust crate binds the new functions; the binding is about 30 lines of our own.
- **JAWS cannot tell us.** Researched: its programming interface has say, stop, and run-script calls, with no completion event and no "is speaking" query. JAWS and anything else without a signal use the measured rate.
- **The measured rate is not a number in a file.** It is a calibration in the options menu: the game speaks a sentence, you press a key when it ends, and the game computes your characters per second. That takes ten seconds, works for any screen reader, and can be redone whenever you change your speech rate.
- **Two things only your ears can settle about the NVDA route:** whether NVDA's own speech (key echo, focus changes) cuts in on the game's, and whether the synchronous call behaves at your speech rate as the documentation says.

**The stop key always reaches the screen reader.**

**The screen reader is called from its own thread.** The NVDA call above blocks until speech ends, so it must be; and it means a slow call never delays a key or a sound. A thread blocked in that call cannot hear a request to stop, so stopping goes by its own path, and each utterance has an identity so that a call returning late cannot end a newer one.

**What I would not do:** speak through UI Automation notifications or an accessibility tree in the window. Researched: those are hints to the screen reader with no way to cancel and no report of when speech ends. They are a fallback for Narrator at most.

**Tolk stays for now** as the route to JAWS and the system voice. It has had no release and its last commit was in June 2024. Prism is the maintained library that covers the same ground and more; swapping is a small, separate change to make if Tolk causes a problem, not part of this design.

### Design: the client as a state machine

The client becomes one loop that waits for the next event: a key going down or up, a message from the host, a timer coming due, or the announcer reporting that speech ended. It handles the event and waits again. No sleeping, no polling (C7).

- **Modes are one enum**: map, order shortcut, game menu, chat entry, console, story. One function routes a key by mode. The Shift+F7 bug cannot be written.
- **Nothing blocks.** A menu, a text entry and a cut scene are modes, not nested loops. The game keeps running behind the game menu in every kind of game, not only on a server.
- **Focus is one enum**: nowhere, a square, or a ninth of a square. Today that is three optional fields and an `unreachable!`.
- **Keys:** a modifier pressed alone acts on release, and only if no other key came between (H4). Key repeat follows your Windows setting. Every key, including menu keys, can be changed from inside the game. A key-help mode, like NVDA's input help, says what any key does without doing it.
- **Settings** are typed, saved when changed, and all reachable from the options menu: volumes by category, speech policy by category, speech rate calibration, keys (H6).

### Design: questions you can ask

The principle: a sighted player reads a screen edge without thinking. Everything that edge would show needs a key that answers in one short sentence. All new keys; no existing key changes. The answers come from the view, so each is a few lines:

- Where am I under attack, with a jump to each place.
- What is in production everywhere, soonest first.
- Workers: how many idle, how many on each resource.
- The group: how many of each type, and their health as a whole.
- Players: who is allied, who is an enemy, who is defeated; the score so far.
- Every resource type the rules define, not only the first two.
- How is the fight here going: the known strength on each side of the current square and whether it is moving your way. A sighted player sees this at a glance; nothing in the original offers it.

### The simplest alternative

The findings doc's version: a rate setting, requeue a cut message, limit the three-second rule to sounds, modifier on release, new query keys. That is five changes inside the existing queue, perhaps a third of the cost, and it fixes the worst symptoms. It keeps the seven message kinds, the blocking loops, the polling, and the 70-field structure, so the next speech bug costs as much to find as the last. I would take the simple version only if listening time is scarcer than I think, because both versions need the same sessions with your ears and the announcer needs them only once.

### What it costs

The transcripts record every sleep and every timestamp of the guess, so they cannot survive this move whatever is done. The plan is to design the new transcript form first (events in at given times; utterances and sounds out), record it once from the new client, and read that diff with care. The wording of what is said should not change, only when it is said.

## 7. Move 4: Rust, not Python written in Rust

### Now

From the reviews, counts read today:

- 114 places stop a unit's or player's turn part-way with the text of a Python error, leaving whatever was changed before the stop.
- Orders hold their arguments as strings; an order is one structure with the fields of every order kind and one `mode` shared by three of them.
- Rules are looked up by name through a linear scan that copies. Type names like `peasant`, `townhall`, `meadow` and `buildingsite` are written into the code.
- A remembered unit is identified by its memory address, so the state cannot be written to disk.
- An entity is never freed, and a defeated player's memories are kept. Measured: the busiest game created 10,314 entities for 2,529 still on the map, and its four defeated players still held 2,244 remembered copies.
- Eighteen bugs were kept on purpose to match Python (G3), and the state-machine review found one the original has too. Reproduced at the world's interface: with 2000 gold in the simulation's units, too little to train a peasant, "train peasant" then "cancel training" in the same turn left 6000. An order that was never paid for can be cancelled for a refund.

### Design

This is not a rewrite. The work goes module by module behind the seam from move 1, checked by the traces.

**What the traces do and do not prove.** I first wrote that they hash the whole world every turn. Sol showed that is wrong, and I checked (`crates/world/tests/traces.rs:184`). Each turn they hash a text dump of: the time, the random generator's state, each player's resources, food and counts, and for each thing on the map its position, hit points, mana, owner, the names of its queued orders and the kind of action it is doing. They leave out what each player remembers, what an order is aimed at, timers, and the computer players' own state. The random generator's state catches a great deal indirectly, because almost any change in behaviour changes a later draw. But "the traces are unchanged" is evidence, not proof. So the first task is to widen them: record `World::digest_u64` for each turn beside the dump. The digest covers the contents of the whole state. It does not cover one thing, as Sol's second round pointed out: whether two remembered copies are the same copy. The simulation decides that by address (`crates/world/src/world.rs:482`), and it decides whether an order's target is still valid. Two equal copies hash the same. So memory identity gets regression tests of its own, and even widened, the traces are strong evidence and not proof.

### Python habits, and the type that replaces each

The port is Python written in Rust. It compiles, and the compiler is told almost nothing, so it can check almost nothing. Each habit below is in the code today (from the reviews, with the counts read today), and each has a Rust shape that turns a class of runtime mistake into a compile error. This applies to the client as much as to the simulation.

1. **Names as strings become ids resolved once.** Today a unit type, an upgrade, an ability, an effect, a stat, a group and a rally point are all text, compared and looked up by text on every use; an upgrade's level is how many times its name appears in a list. After loading, nothing is a string: `UnitTypeId`, `UpgradeId`, `AbilityId`, `ResourceId`, `FactionId`, `StatId`. `rules.txt` stays what people write. Loading checks every cross-reference, so a mistake in a mod is reported with file and line when the game loads, not as a warning in a log during turn 400. The type names written into the code (`peasant`, `townhall`, `farm`, `gate`, `meadow`, `buildingsite`) become roles the rules declare, with today's names as the defaults, so the orc mod and old files keep working.
2. **Bare numbers become quantities.** Today time, distance, hit points, costs and angles are all `i64`, with "times 1000" as a convention in the programmer's head. That is where the square-width overflow and the 1.005-read-as-1.004 bug live. With `Millis`, `Millimetres`, a fixed-point `Thousandths` and `Heading` as distinct types, adding a distance to a time does not compile, and every conversion is one named function. The client's times stop being `f64` seconds.
3. **One structure with optional fields becomes an enum per kind.** Today an `Entity` is a single structure for nine Python classes, with fields that only mean something for some of them, and "is a building site" stored twice. An `Order` holds the fields of every order kind and one `mode` shared by three. As enums, each kind carries only its own state: a gather order has a gather phase, a train order has a payment state, a building site has its timer. The match that handles them must cover every case, so adding an order kind shows every place that needs to know. The refund bug becomes unrepresentable, not patched.
4. **Groups of flags become one state.** `is_impossible`, `is_complete`, `has_started` and `is_deferred` allow sixteen combinations. The state-machine review counted six that are meant, two of which are final but still cancellable for a refund. One enum allows exactly the meant ones.
5. **Sentinels become absence.** A distance of `i64::MAX` for "no path", position (0, 0) for "nowhere", the text `"None"` for "inside something", an optional boolean where "none" means "this is not a worker". Each becomes an `Option` or a named variant, and the splash-centred-on-the-map-corner bug is exactly a sentinel read as a value.
6. **Imitated exceptions become results or impossibilities.** The simulation has 114 `Abort` stops carrying Python error text, and the client has its own (`Oops`, with a "missing key" case that becomes a beep). Each one becomes one of three things: a state the types rule out, an expected failure with its own variant and defined behaviour (as `Reason` already is for a refused order), or an assertion. Done function by function as other work touches them (G4).
7. **Text built to be parsed again becomes a value.** Orders inside the simulation, in about 30 places in the computer player alone; trigger owners, conditions and actions; style lookups assembled with `format!`. All parsed once, at the edge.
8. **One shared counter and text-ordered ids become separate id spaces** with numeric order. Nine places sort ids as text today, so 10 comes before 9.
9. **One object that does everything becomes parts with narrow inputs.** Every module is `impl World`, so every function can touch everything, and about 280 `clone()` calls exist mainly to get past the borrow checker. Perception, movement, combat, production and economy each take only the stores they read and write. Most of the clones go, and what a function can affect is visible in its signature. The client's 70-field interface splits the same way in move 3.
10. **Copies kept in step by hand become one owner.** The order copied into the budget and into the computer player's tracking, with a loop to keep targets in sync, becomes one order table and ids.
11. **Sleeping and polling become waiting on events**, in the client (section 6).

### Keeping it that way

A habit that keeps coming back needs a mechanism, not a note. Three, all in CI:

- **Lints that deny** in the simulation and the loader: unwrapping, indexing that can panic, and unchecked arithmetic. Each real exception is marked where it stands, with its reason. The three crashes Sol found are one of each.
- **Ratchets.** CI counts `Abort::new`, `.clone()` in the simulation, and string-typed fields in simulation state, and fails if a count goes up. They only ever fall.
- **The file layout stops mirroring Python's.** Modules are named for what they do, not for the `.py` file they were ported from.

### The rest of the move

- **The inherited bugs fixed in one batch** (G3), with one re-recording.
- **Entities freed** when dead and unreferenced, with stable ids, and a defeated player's memories dropped. This bounds memory in a long game.
- **Speed: leave it alone.** Measured, the turn is at most 4 percent of its budget in the worst game and a third of a percent in an ordinary one. The one cost that was out of proportion is the sync check, and move 2 takes it off the play path. So I would not chase the list in C4, except the in-place collision test, which is free. If 16-player games on the largest maps ever matter, profile first: the measurements suggest the cost follows the number of things on the map and the 9,000 remembered copies, not the number of units, but that is a reading of counts and not a profile.

### What the measurements let me drop

My first draft had "the whole state writable to disk" as part of this move, so that a save could be a snapshot. The numbers say that structure is not earned yet. Replaying a command log loads an hour of an ordinary game in 1 to 11 seconds, and skipping about in a replay needs only copies in memory. Writing the state to disk costs replacing the address comparison for memories (C3), the copied orders in the budget, and a file format to keep stable.

Sol pointed out what that leaves unsaid, and it is right. A save that is a command log only loads on the exact version of the simulation that wrote it (`crates/client/src/record.rs:228`), and this design changes the simulation on purpose, many times. So with log saves, **an update to the game invalidates saved games and replays.** A snapshot would not escape that for free, since its layout changes too, but a snapshot can be migrated and a log cannot. Two things follow:

- That rule has to be stated to players, and it is acceptable for a mission you can restart from its chapter bookmark. It is not acceptable for something you would want to keep, such as a library of replays.
- A replay you share is replayed on someone else's machine, so two processors still have to compute the same game. M11's check on a second processor stays worth its small cost even if lockstep goes.

I would still leave state on disk out of the first pass, and decide it when replays or long saves become something people keep.

### What I would not do here

Replace the square-and-exit pathfinding, the in-square movement, the collision model, or the fixed-point maths. They are intricate, they work, the traces pin them, and no finding needs them changed except the two collision bugs in G3.

## 8. Computer players as players

### Now

From the reviews: a computer player reads the true world, remembers a unit for 10 hours where a human remembers 3 minutes, and may give orders humans cannot. The `ai2` type gets 1000 of each resource every turn and ignores requirements. The routine around the `ai.txt` script is fixed code built on four type names.

### Design

A computer player moves to its own crate that depends only on the shared types. It receives the same view a human's client receives and returns the same commands. It cannot see what a human could not, and the compiler enforces that, in the same way as for the client.

- `ai.txt` stays as the build-order script, so mods keep working.
- Difficulty comes from honest things: how often it acts, how well it keeps workers busy, how diligently it scouts. A handicap, if you want one, is an explicit setting announced in the game room ("aggressive computer, 150 percent income"), not a hidden privilege.
- `ai2` either goes or is relabelled as what it is.
- The behaviours only computers' units have today, fleeing and regrouping, become a stance any player can choose (section 11).
- Computer players become a free test tool: a game of bots against bots through the real host is an end-to-end test of the whole stack that needs no scripted keys.

This makes the computers weaker until the fair version is tuned. That needs playing against, and it is the one part of the design where "better" is a matter of feel.

### The simplest alternative

Change the memory duration, which is one number, and label `ai2`. That fixes the worst unfairness for almost no work and leaves the warehouse peek and the structure. I would do that first regardless, in phase 0, and move the computer player behind the view in phase 4, once the view exists and has been proved on the client.

## 9. Untrusted input: one loader, and a rule for spoken text

### Now

Sol surveyed every place outside data enters the program, by reading only. It reported 13 findings beyond the ones already known. I checked the six I lean on below against the code at the lines Sol cited, and they hold as source-level paths. None was run.

The pattern matters more than the list. Today a map, the rules inside a zipped map, and every other player's orders are text, and that text is parsed on every player's machine by code written to match Python's behaviour, not to survive hostile input.

- **A map can crash every player who loads it.** Checked. A trigger owner whose last character is not plain ASCII is split at a byte position inside the character, which is a real panic and not one of the imitated errors (`crates/world/src/map.rs:313`).
- **Rules can exhaust memory or time.** Checked. `nb_of_resource_types` is read with no upper limit and then used to size lists (`rules.rs:908`, `1079`). A type that names itself as its own parent twice makes property lookup branch two ways at each of 64 levels (`rules.rs:96`). A zipped map carries its own rules, so this arrives over the network.
- **Another player's order can carry any number.** Checked. A zoom target's coordinates are parsed as any 64-bit integer and multiplied by three with no check (`playerlogic.rs:712`, `square.rs:208`). In a debug build that panics. In a release build it wraps, with consequences nobody has traced.
- **A style file can recurse without end.** Checked. The simulation's rules lookup has a depth limit; the client's style lookup has none (`crates/client/src/style.rs:211`). Sol reports key-binding definitions that expand exponentially as well.
- **A downloaded map has power over the interface.** Checked. The map is the top resource layer, and key bindings are read from every layer (`bindings.rs:27`, `resource.rs:259`). So a map from a server can rebind your keys and replace the game's own spoken prompts for as long as that game lasts.
- **Nothing on the wire is protected.** Checked. The client builds `ws://` addresses only (`net/conn.rs:63`). Anyone on the path can read chat, or stand in for the server and swap the map.
- From Sol, not re-checked by me: packages and local maps are read with no size limit; a sound file is decoded whole with no length limit; a server can grow the client's lobby and turn lists without bound, and can make it write a game record to disk again and again; `--check-record` feeds a file's embedded rules and map into the same parsers before the program's panic handler is installed; any client can fetch any cached map by hash, including a private room's; invitations can be repeated to make spoken spam; a server's name is written unfiltered into the recent-servers file, where a line break in it creates a second entry; a mod's dependency path can point outside its folder; and the local-network discovery answer has no rate limit.
- Sol found these sound: the wire types for logins, chat, orders and file names; message size limits; room permissions; zip entries stay in memory, so no file can be written outside its folder; the simulation's scripts cannot reach files, the network or a shell.

### What moves 1 and 2 already do about it

- With typed commands, an order has no free text and no unchecked number. A coordinate is a ninth of a named square.
- With a host, only the host parses a map or rules. Clients receive typed, already-checked data. A hostile map can then hurt one process, not every player.
- That makes the host the place that must not fall over. So each game runs as its own task, a panic inside one ends that game and no other, and loading and each turn have a time and memory budget.
- If lockstep is kept, the second and third points do not apply: every client goes on parsing every map, and the loader below has to hold on all of them.

### Design

- **One loader for everything that comes from outside**: rules, maps, computer-player scripts, styles, key bindings, packages, zip files, sounds, records. It has limits on bytes, lines, nesting, counts, expanded size and decoded length. Inheritance is resolved once into a graph that is checked to have no cycles. It returns typed data or an error with file and line, and never panics. It is fuzzed in CI. No other code reads external text.
- **Downloaded content has fewer powers than installed content.** A map that arrives from a server may define a map, rules for that game, and how its own unit types sound. It may not change key bindings or the game's own prompts. Its paths cannot leave its folder.
- **The connection is protected where it can be.** A server on the public list must offer TLS, and the list is fetched over HTTPS with no redirect to anything less. A server run from inside the game for friends has an address and no certificate, so it stays unprotected; the README says so. Pinning a home server's key on first use, as SSH does, would close that gap, and I would add it only if home-hosted games over the internet turn out to be common.
- **What a server can make a client hold is bounded**: lists of players and rooms, queued turns, files written.
- **Text from other people is always framed.** For a sighted player, chat looks different from a system message. For you, the only signal is what comes before the words. So every string a remote party chose (a login, a chat line, a server or map name) is preceded by a sound the game reserves for that purpose, is stripped of control characters, has a length limit, and never appears inside a system sentence unframed. Otherwise a player named "the game is out of sync" can say so.
- **Names are a type.** A server name, map name or login is a bounded string with no control characters, checked where it enters, and files that store them are structured, not one name a line.
- **Server hardening** as in M6 and M7, plus Sol's points: a limit on connections that have not said hello, bounded queues counted in bytes, a session token per player (which reconnecting needs anyway), map downloads allowed only to players in or invited to that room, one invitation per player per room, and a rate limit on discovery answers.

## 10. Sound in three dimensions

Sound is how you see the map, so this is the game's graphics engine. Everything in this section is decided by your ears, not by me.

### Now

From the reviews, with `audio.rs` and `sound.rs` read by them: a sound's position becomes left-right pan and loudness, and nothing else. A sound behind you is the same sound at half volume, mirrored. There is no height, no filtering, no reverb. Volume changes are applied with no smoothing. In map mode you sit near the south edge of the selected square facing north, with north-south distances halved; in first-person mode you stand on a unit facing the way it faces. Sixteen channels; which sound owns a channel is decided by comparing sound data, so two units of one type can fight over a channel (from the state-machine review).

### What three-dimensional sound could carry

The point is not realism. It is that each of these is information you now get from speech, or do not get at all:

- **Front and back**, properly. In map mode that is north and south of where you stand.
- **Distance**, from more than loudness: a far sound is duller and has more of the place's echo in it. Loudness alone cannot tell "far" from "quiet".
- **Height.** Dragons and flying machines overhead; a unit up on high ground. Today an air unit sounds like a ground unit.
- **Terrain you can hear without asking.** A forest, an open meadow, a mountain pass and the inside of a town each have a different echo. The square's character would be audible the moment you arrive, under whatever else is playing.
- **Which neighbours you can reach.** Sound from the next square arrives clear through an open exit and muffled where there is no path or the exit is blocked. You would hear the shape of the map around you. This has to follow what the player knows: phase 1 takes away the client's knowledge of blockers it has not seen, so an exit sounds open or closed as the seat last observed it, and an exit never observed needs a sound of its own for "unknown".
- **First-person mode worth using.** Standing on a unit and turning becomes a real way to look around.

### What it cannot do, honestly

- **It needs headphones.** On speakers the game keeps left-right placement, so this is a setting, and the speaker mode has to stay good.
- **Front and back are the weak point of headphone rendering.** With a generic model of the head and no head movement, people often hear a front sound as behind. Turning resolves it, which first-person mode allows and map mode does not.
- **Height is weaker still**, and varies most between people.
- **Ears differ.** The model that works for one person sounds wrong to another.
- **Your screen reader's speech is not part of it.** It comes from another program and stays in the middle of your head.

So the rule for the design: **never rely on the three-dimensional cue alone for anything that matters.** Each cue is doubled with a plain one. Behind is also duller. Above is also brighter and slightly higher in pitch. Far is also quieter. Then the game is still fully playable for someone the head model does not fit, and on speakers.

### Design

**A sound scene, with the renderer behind it.** The game stops computing left and right volumes. Each moment it declares a scene: where the listener is and faces, and for each source a sound, a position in metres including height, a loudness, a priority, and who owns it. The audio engine works out the rest. Three things follow:

- Channels are owned by sources, by id, which removes the channel fight.
- Positions are glided on the audio thread between updates, so nothing clicks or jumps.
- The tests record the scene and not the volumes, so the renderer can change without a re-recording.

**Two renderers behind the scene.**

- **Stereo**, for speakers: today's maths, plus smooth volume changes and a duller tone behind (H8). This is the necessary part and it is done whatever else is decided.
- **Binaural**, for headphones, using Steam Audio. Researched, from its own documentation, which I read:
  - It places each source with a model of the head, and can blend smoothly as a source moves.
  - The head model can be loaded from a standard file (SOFA), so the game can offer several and you choose the one that fits your ears.
  - Distance, air absorption and how muffled a source is can be set directly by the game, with no simulation. That is what the map needs: the game already knows whether an exit is open.
  - Its full reverb is computed from scene geometry, at a real processing cost. It also has a parametric reverb that takes three decay times from the game and needs no geometry. Sol pointed this out and I checked the reference page. That is the first thing to try for terrain echo: a preset for each kind of terrain. The page does not say outright that hand-supplied times are supported, so the listening room is where it is proved.
  - It only processes sound; it does not play it. So rodio keeps the output device, and Steam Audio sits between the scene and rodio.
  - It is Apache-licensed, version 4.8.1, with a Rust binding (audionimbus 0.17.0, 9 November 2026) that says of itself it is "subject to breaking changes". It adds one file to ship beside the program, `phonon.dll`, as Tolk's files are shipped now. The binding's build downloads about 180 megabytes once.

**Where you stand.** Map mode keeps its listener position, so most of the selected square is in front of you, east and west squares are to your sides, the north square is far ahead and the south square is behind. That is the arrangement where front-back confusion would hurt, and it is the first thing the trial must test. If it fails, the fallback is to keep map mode's north-south as today's loudness-and-tone and use the head model for left-right, distance and height only.

**A sound check you run yourself.** An item in the options menu plays a sound in front, behind, left, right, above and far, and asks which you heard. It picks the head model that you get right most often, and tells you plainly if none works well, in which case the stereo renderer is the better choice for you. This is the equivalent of a graphics settings screen.

### The simplest alternative

Only the stereo renderer's two fixes: smooth volume and a duller tone behind. That is perhaps a fifth of the work and gives front and back by tone. It gives no height, no terrain, no sense of which neighbours are open, and distance stays loudness. I would do it first in any case, because the scene needs it.

### How to decide: a listening room

Before any of this is built into the game, a small separate program, the listening room. It plays the game's own sounds through each renderer and walks through a numbered script: a footstep circling you; the same sound in front then behind, in random order, and you say which; a dragon passing overhead; a sound receding; a neighbour heard through an open exit and a closed one; the same square with and without terrain echo. It takes an afternoon to build and half an hour of your time, and it answers the only question that matters: what your ears can actually use. It stays in the repository as the place to try every later sound idea.

### Cost and order

1. The sound scene, channel ownership, smooth volume, the duller tone behind. Medium. Changes the recorded sound stream once, when the tests move from volumes to scenes.
2. The listening room and a trial binaural renderer. Small to medium. Your ears decide whether to go on.
3. If yes: the binaural renderer in the game, the sound check, then height, open and closed neighbours, and terrain echo, each tried in the listening room first.

## 11. Game features, by how fundamental they are

All of these are additions. None changes a unit, a cost, or an existing key.

**Missing fundamentals.** A strategy game is expected to have these:

- **Stances as one setting**: aggressive (today's offensive), hold ground (attack what comes in range, do not chase), hold fire, and cautious (today's defensive: flee stronger forces). Hold position is G2's first item.
- **Every question in section 6.**
- **Pause, save and load, surrender, reconnect, watchers, replays you can listen to** from any player's side. All fall out of the host.
- **A ping.** A key that marks the current square for your allies: they hear a positional sound and "Andrew marks c3". Sighted team games have had this for 25 years, and a blind team needs it more.

**Worth having:**

- Automatic use of an ability, switched on per ability, for human players' units as computers' units have now.
- An order given to several production buildings at once, shared between them.
- More recallable groups, and a longer order queue than 10.
- Giving resources to an ally.
- A map checker: `--check-map file.txt` reads a map through the loader and says what is wrong, by line. This is the map editor a blind map-maker can use, and it is nearly free once the loader exists.

**Experiments, each needing your ears before it earns a place:**

- A sweep of the whole map on one key: each known square sounds in turn, panned by column and pitched by row, with a different timbre for yours, enemy and unknown. An overview in two seconds, where today you page through squares.
- The game's own voice as an option: Windows' built-in voices rendered into the game's mixer. That gives exact timing with any screen reader, including JAWS, and lets speech and game sound be balanced against each other. It gives up your own voice and rate settings, so it could only ever be a choice.
- Lowering game sound a little while speech is active, so words are never masked.

**Sounds decoded before they are needed**, not on first use, so the first sword stroke of a game does not hitch.

**The window:** it shows the last thing said as large text, and a plain high-contrast grid of the map, for a sighted helper, a low-vision player, or anyone streaming the game (H7). Drawn as pixels so no screen reader reads it twice.

## 12. Order of work

Each phase ends with something playable. Sizes are my estimates in pull requests of agent work; your listening time is counted separately because it is the scarce resource.

### What checks the work

The recordings are why a change this size is safe to attempt, so the tests are part of the design.

- **Traces are widened first, then kept.** Today they cover positions, hit points, resources and the random generator's state each turn, and not memories or order targets (section 7). Adding the full digest to each line makes them strong evidence that a simulation change meant to be neutral is neutral.
- **The view gets its own literal checks**: what a seat is sent, in what order events arrive, and that a remembered unit is the same remembered unit. Hashing more internal state would not prove those.
- **Transcripts change form once**, in phase 2: from "what was said at each millisecond of a sleeping loop" to "given these events at these times, these utterances and sounds". The new form does not record sleeps, so later changes to timing internals stop breaking it.
- **Invariants checked every turn of the long games**: no resource below zero, nothing refunded that was not paid, nothing both dead and on the map. The refund bug would have failed the second of these.
- **The loader and the command decoder are fuzzed.**
- **Bots against bots through a real server**, in CI, on Windows and Linux. This is the end-to-end test: it needs no scripted keys and exercises the simulation, host, protocol and server together.
- **Your ears**, for everything spoken, with a short numbered script each time. One idea worth a trial before phase 2: NVDA can log every string it is asked to speak, with times. A test could read that log to check what NVDA received and when, without anyone listening. It would run against a separate copy of NVDA, which cannot run beside yours, so it needs a virtual machine or your go-ahead for a time when losing speech briefly is acceptable. I have not tried it.

**Phase 0. Before any structure.** Small. One short listening session.

- The two listening checks still owed from the merged work.
- The small fixes the state-machine review recommended, each with the test that reproduced it: the dropped chat line, the stuck lobby flag, Shift+F7 in the game menu, the map download that says "Downloading" forever, and the refund for an unpaid order. Later phases remove the causes of the first three, but that is weeks of work away, the fixes are a line or a few each, and their tests carry over as regression tests for the new code.
- Three crashes from hostile data that Sol found and I checked, each a few lines: the trigger owner split inside a character, the unlimited resource-type count, and the unchecked zoom coordinate. They matter the day a stranger joins a game.
- Two more that are right under any design: computer memory duration, and the in-place collision test.
- The traces widened with the full digest, so that later claims of "nothing changed" rest on more.
- The listening room for sound (section 10). It is separate from everything else, and half an hour of your time settles the largest open question in the sound design.

**Phase 1. The seam, with everything still running locally.** Large: about six pull requests. One listening session, for step three.

1. **The contracts and a host stepped by hand.** `Command`, `View`, `Event`, and the description of a game's set-up, as types. A host that owns the world and is stepped directly by the game loop and by the test harness. Session set-up, the record check and the score screen move behind it. Lockstep multiplayer is untouched: each client runs its own stepped host, as it runs its own world now.
2. **The neutral move.** The client's reads go to the view, file by file. Commands are typed in the client and at the host, and the four cheats go through commands. While lockstep remains, the wire and the record still carry today's text, produced from the typed command by one function. That adapter is deleted in phase 3, with lockstep. The view carries exactly what the client reads today, including the five things the player should not know, marked by name. Done when the client no longer names `World` and every recording is byte-identical, checked in CI.
3. **The corrections.** Each of the five leaks is closed, one pull request each, with a decision about what you should hear instead and a transcript difference that is expected and read. You listen to these.

**Speech track A. The speech backend.** Medium: about two pull requests. One listening session. Runs alongside phase 1, in `voice.rs` and `tts.rs` only.

- NVDA's synchronous call on its own thread, with the current client file, switched on by a setting until you have heard it. The end of speech from NVDA; the estimate, as now, for everything else. The stop key always reaches the screen reader. A news message that is cut off goes back in the queue.
- Stopping has its own path. A thread blocked in the synchronous call cannot hear a request to stop, and a call that returns late must not end a newer message, so each utterance has an identity.
- The calibration is not in this track. It needs an options-menu item and a saved setting, which are phase 2's files.
- Done when: at your normal speech rate nothing is cut off early and there are no silences between queued messages. Only you can check that.

**Phase 2. The announcer and the event-driven client.** Large: about six pull requests. Two listening sessions. After phase 1, because it changes the same files.

- Categories, subject keys, the log by category, "what did I miss", jump to the last alert, and the speech-rate calibration.
- The client as one loop waiting on events: modes as one enum, non-blocking menus, keys on release, rebinding, key help, typed settings saved when changed, the new questions, the window.
- The new transcript form is designed first and recorded once.
- Done when: you have played a full game and nothing was lost or late, and the "what did I miss" key has nothing surprising in it.

**Phase 3. The host on the server.** Large: about six pull requests. One listening session, and a real game with another person.

- The stream measured with the real types before anything is built on it.
- The game description sent to the server, the presentation contract, the event stream with a position per seat, the story and end-of-game handshakes.
- The server runs the host. Reconnect with a token, heartbeat, pause, speed, removing a player, watchers.
- Then delete lockstep: the digest messages, out-of-sync handling, the per-client world in server games.
- Done when: a game on a server survives pulling one client's network cable for 30 seconds, and a bots-against-bots game runs through a real server in CI on Windows and Linux.

**Phase 4. Rust, not Python.** Large, but in small independent pieces: about ten pull requests. No listening; playing against the computers.

- The loader with its limits, cycle checks and fuzzing. This piece touches only the simulation's loading code, so it can start during phase 1.
- Rules resolved to ids. This is a separate piece: the ids replace strings in order state, in the orders code and in the computer player, so it changes those files and waits for phase 1.
- Quantities with units; orders, actions and entity kinds as enums; the imitated errors removed function by function; the lints and ratchets.
- The inherited bugs in one batch; entities freed; computer players moved behind the view and into their own crate.
- Each piece ends with unchanged traces, or one re-recording that is read.

**Phase 5. Sound.** Medium, then as far as your ears say.

- The sound scene, channel ownership, smooth volume, the duller tone behind. After phase 1, because it changes the files that place sounds.
- If the listening room said yes: the binaural renderer, the sound check, then height, open and closed neighbours, and terrain echo, each tried in the listening room first.

**Phase 6. Features.** As wanted, each small to medium: stances, pings, automatic abilities, the map checker, then the experiments.

### What can run at the same time

Three tracks do not share files, so three agents can work at once from the start: phase 1 in the client's game code and the simulation's interface; speech track A in `voice.rs` and `tts.rs`; and the loader in the simulation's loading code. The listening room is a separate program. Everything else waits for phase 1.

### Your time

Counted in sessions, because it is the scarce resource: one in phase 0, with the listening room; one for the leak corrections; one for the speech backend; two for the announcer; one for the host, plus a real game with someone; then as many for sound as you care to give it. Each comes with a short numbered script, and nothing that changes what you hear is called done until you have reported on it.

## 13. Decisions

Decided by Andrew on 14 November 2026:

1. **Lockstep or host: the host.** It gets more than it gives up and allows more features. The gate in section 5 no longer decides whether to build it. Its three questions remain as checks inside phase 3: the stream is measured with the real types before anything is built on it.
2. **The original's data: fetch it, and replace it before shipping.** The program fetches the data onto the player's machine, as the fetch script does now. The author is not being asked. The plan is to change the maps, sounds and rules before anything is published, so type names move out of the code and into the rules (section 7) early enough for another set of rules to stand in. Appendix D has the public record.
3. **Saved games across updates: accepted.** With saves and replays as command logs, an update to the game invalidates them (section 7). Players are told so.
4. **Three-dimensional sound: the listening room is built early**, in phase 0.

Not asked, so decided as recommended, and open to change:

5. **How far the announcer goes:** the full subsystem in section 6.
6. **Computer players:** fair, and at first weaker, with an explicit handicap setting.
7. **Platforms:** the client stays Windows-only; the server and simulation are also built and tested on Linux.
8. **What goes first:** the seam and the speech backend start together.

## 14. What I would leave alone

- The square-and-exit map, in-square positions, and the movement and collision code.
- The 300 millisecond turn, the integer maths, and every number in `rules.txt`.
- The text formats, as what people write. They gain a strict loader, not a replacement.
- The lobby half of the protocol and the server's connection handling. They were written for the port, are typed, and need hardening, not redesign.
- The copy of Python's random number generator. It is small and tested.
- The fixed limits: 26 columns, 10 resource types, a food limit of 80.
- Moving a running game to another host.
- Play with the original Python game's clients.

## 15. What gets deleted

Deletion is part of done. When each move lands, these go in the same wave:

- With the seam: text commands and their parser on the play path, `Voila` and the entity copies, the world's public mutable fields, the four direct cheat writes.
- With the host: client-side lockstep, the turn digest messages, out-of-sync handling and its speech, the content-hash compatibility check as a correctness mechanism, the separate network and local game paths.
- With the announcer: the seven message kinds, `say_now` and every blocking loop, the character-count guess as the primary clock, the `verbosity` string, the fixed key repeat, the three throttles and two expiry timers that coalescing replaces, the old transcript form.
- With the simulation work: `Abort` and the Python error strings, the address comparison for memories, string type names in code, the random draws kept only for parity (G5), `ai2` as an unlabelled opponent.

## Appendix A. Where each finding lands

Each finding from `docs/open-findings.md`, and what happens to it under the whole design, host included. Seven of the findings marked "dissolves" depend on the host and would become separate fixes under lockstep: M1, M3, M4, M8, M10, M11 and C6.

"Dissolves" means the design removes the cause, so no separate fix is written. "Built" means it is still work, but inside one of the four pieces. "Stands alone" means the design does not help.

**What you hear**

- H1, the guess about the end of speech: built, in the announcer's speech backends and the calibration (section 6).
- H2, messages that stop the game: dissolves. Nothing blocks; in single-player the host pauses for a cut scene by name (sections 5 and 6).
- H3, news dropped while busy: dissolves into the announcer's four rules (section 6).
- H4, modifier keys on press: built, in the client's key handling (section 6).
- H5, questions you cannot ask: built, as short functions over the view (section 6).
- H6, settings lost or only in files: built, as typed settings (section 6).
- H7, the blank window: built (section 11).
- H8, front and back sound the same: built, below the audio seam (section 10).

**Multiplayer**

- M1, a dropped connection forfeits: dissolves into the host keeping the seat (section 5).
- M2, a silently dead connection: built, a heartbeat in the host's connection, small.
- M3, one player stalls the game: dissolves. The host waits for nobody.
- M4, no pause or speed change: dissolves into the host's clock.
- M5, a game dies with its host: stands alone. Still a README note; still not solved.
- M6, names not verified: partly built. The session token for reconnecting is the same mechanism; reserved names remain optional extra work.
- M7, a public server can be overloaded: stands alone, as hardening of the connection layer (section 9).
- M8, nobody can watch: dissolves into a seat with a view.
- M9, two map limits missing: dissolves into the loader's limits (section 9).
- M10, records stop at out-of-sync reports: dissolves into the host's log, which is written for every game.
- M11, two machines never compared: mostly dissolves. A live game no longer depends on it. A replay shared between two machines still does, so the check on a second processor is kept as a CI job.
- M12, leftovers: the memory-address point and the cost of the sync check are both moot once the sync check leaves the play path; the orc-mod test and the stale branches stand alone.

**Gameplay**

- G1, computers play by different rules: dissolves when the computer player sits behind the view (section 8). The memory number is fixed first, in phase 0.
- G2, orders other games have: built, on typed orders (sections 4 and 10).
- G3, bugs kept from the original: built, one batch in phase 4.
- G4, imitated Python errors: built, function by function in phase 4.
- G5, parity scaffolding in the client: dissolves in the client rework of phase 2.

**Code structure**

- C1, orders as text: dissolves into `Command` (section 4).
- C2, the interface reads the whole world: dissolves into `View` (section 4).
- C3, memories told apart by address: left as it is. It mattered for the sync check and for writing the state to disk. The sync check leaves the play path, and section 7 explains why writing the state to disk is not earned yet.
- C4, per-turn work that grows: mostly left alone, on the measurements. The in-place collision test is done in phase 0, entities are freed in phase 4, and the sync check that grew fastest leaves the play path.
- C5, two very large structures: the world's public fields dissolve with the seam; the 70-field interface dissolves in phase 2.
- C6, everything on one thread: dissolves. The simulation is on the host's thread and speech on its own.
- C7, input checked on a timer: dissolves into the event-driven loop.
- C8, Windows only in practice: partly. The server and simulation gain Linux CI; the client stays Windows.

Count: 16 dissolve, 10 are built inside one of the four pieces, 5 stand alone or partly alone (M5, M6, M7, C8 and parts of M12), and 2 are deliberately left (C3, and most of C4).

**The nine from the state-machine review**, which came after the findings doc:

1. Chat and alerts dropped while an item is spoken (reproduced): patched in phase 0; the cause dissolves into the announcer, where "what is on the voice channel" is one value.
2. A late "you have been removed" breaks the lobby (reproduced): patched in phase 0; the cause dissolves in the event-driven client, where a server event is handled once and is not a flag.
3. Shift+F7 in the game menu writes to everyone (reproduced): patched in phase 0; the cause dissolves into the mode enum.
4. A refund for an order never paid for (reproduced at the world's interface): patched in phase 0; made unrepresentable when production orders carry a payment state (section 7).
5. A refused map download says "Downloading" forever: patched in phase 0.
6. A failed resurrection leaves a ghost unit in the update list: built, in phase 4, when an entity gets its id at the moment it gets a place.
7. Two sounds fight over one audio channel: built, in the sound work of section 10, by recording who owns a channel.
8. The world accepts commands from players who have left: dissolves. The host is the one place commands enter, and it knows who is seated.
9. A campaign mission's map resources stay loaded: dissolves when a game's resources belong to the game and are dropped with it.

That review also found the server's states already explicit and recommended leaving it alone, which agrees with section 14.

## Appendix B. Measurements

Run by an Opus agent on 14 November 2026 at `612afd1`. I did not rerun them. The agent's full report, with the source of its test file so the runs can be repeated, is in the thread "Measure sound-rst simulation cost".

**Method.** A throwaway test built a world from the original's rules and a map, filled every seat with a computer player, each in its own alliance so that they fight, and called `World::update()` up to 12,000 times, which is one hour of game time. Seed 1. The computers cycled through the aggressive, `ai2` and easy types. Each call was timed by itself; draining events was outside the timed region and came to under 5 milliseconds for a whole game. The sync check (`World::digest_u64`) was timed separately after each turn. Release build, one thread.

**The machine.** An Intel Core Ultra 9 285H laptop, 20 to 38 percent busy with other programs, test not pinned to a core.

**What else the numbers could be counting, and how that was ruled out.** The first run showed single turns of 100 to 690 milliseconds. The agent played the same seeded game twice: a turn that is really slow must be slow both times, and no slow turn matched. So those were the machine. Medians are reliable. Treat the 99th percentile as within 30 percent, and every maximum as possibly a scheduling hiccup.

**Time for one turn**, median then 99th percentile, in milliseconds:

- `jl1`, 2 players, whole game (it ended at turn 2,558): 0.10, then 0.29.
- `jl7`, 8 players, whole game: 0.91, then 2.4. In the last thousand turns, with 370 units: 1.1, then 3.7.
- `mb8`, 16 players, whole game: 1.2, then 2.7. Only 3 players were left by turn 6,000.
- `js13`, 16 players, whole game: 11.3, then 20. At its worst, turns 3,001 to 4,000: 12.9, then 31.
- `jl7` again in the development build that `cargo run` uses: 1.4, then 3.5. About one and a half times the release build.

**The sync check**, median for one call, in milliseconds, at the end of each game: `jl1` 0.14, `jl7` 0.8, `mb8` 3.1, `js13` 20. On `mb8` that is four and a half times the turn; on `js13` twice. It grows because it hashes every entity ever created and every player ever, with the full copies each one remembers.

**Replaying one hour**, simulation only: `jl1` about 1.3 seconds (projected, since the game ended sooner), `jl7` 11 seconds, `mb8` 16 to 17, `js13` 137. With the sync check on every turn: `jl7` 19 seconds, `mb8` 48, `js13` 319.

**Growth**, entities created against entities still on the map at the end: `jl1` 162 and 61; `jl7` 839 and 503; `mb8` 2,044 and 407; `js13` 10,314 and 2,529.

**Size of the busiest game at its peak** (`js13`, turn 6,000, 12 players left):

- 2,860 things on the map, 440 of them units; 321 queued orders.
- One player perceived between 11 and 561 entities, and remembered between 293 and 1,315. The `ai2` players perceived the most.
- All players together remembered 9,200 entities, each held as a full copy.
- The trace dump of that turn is 215 kilobytes of text. It leaves out what each player remembers, order targets, and the computer players' state, so a real snapshot would be larger.

### The second run: views for every seat

Run by a second Opus agent the same day, after Sol's review. Its report, with the test source, is in the thread "Measure per-seat view stream".

**Method.** A temporary one-line change made every player count as a local human, so the simulation built events and an end-of-turn view for every seat. The same three games were played with the change off and on, and the world's digest matched on every turn, so the change does not alter the game. Each view was written out in a fixed-width byte layout made for the test: 35 bytes for an entity's observable part, 39 if remembered, and for the seat's own entities 12 bytes more plus 8 for each queued order. No compression and no message framing. Each turn's view was compared with the previous turn's, and with the views 10 and 50 turns earlier.

**Time.** Median, then 99th percentile, for one turn with views off and on, in milliseconds:

- `jl1`: 0.10 and 0.28 off; 0.12 and 0.39 on.
- `jl7`: 0.93 and 2.2 off; 0.99 and 2.9 on.
- `js13`: 11.2 and 19 off; 14.1 and 40 on.

Writing out and comparing one seat's view took a median of about 8 microseconds on `jl1`, 5 on `jl7` and 280 on `js13`.

**Bytes for one seat for one turn**, median then 99th percentile:

- The full view: `jl1` 2,846 and 3,361; `jl7` 2,372 and 8,333; `js13` 32,083 and 70,237.
- Every changed entity sent whole: `jl1` 953 and 1,417; `jl7` 289 and 1,004; `js13` 2,560 and 19,203.
- Only the changed fields sent: `jl1` 251 and 383; `jl7` 75 and 277; `js13` 667 and 5,027.
- Events: `jl1` 0 and 123; `jl7` 0 and 350; `js13` 17 and 240.

**Per second for one seat**, changed fields plus events, median then 99th percentile, in kilobytes: `jl1` 0.9 and 1.5; `jl7` 0.3 and 1.9; `js13` 2.3 and 17.

**A client that fell behind.** One message covering 50 turns, with changed entities sent whole, against 50 separate messages: `jl1` 1,379 bytes against 47,862; `jl7` 696 against 16,006; `js13` 5,148 against 129,217.

**What the changes are made of**, by share of bytes, with changed entities sent whole:

- `js13`: 73 percent remembered objects whose only change was their "last seen" time; 21 percent movement; 3 percent moving between seen and remembered; under 1 percent each for hit points, order queues and the seat's own record.
- `jl7`: 53 percent movement; 18 percent "last seen" only; 16 percent other fields such as deposit quantities and cargo; 8 percent order queues.

**How much each seat held** in `js13`: a median of 103 entities perceived and 652 remembered, and at the 99th percentile 744 and 1,556. In a large game it is memory, not perception, that makes a view big.

**What else these numbers could be counting, and what they leave out.**

- The "on" and "off" timings are separate runs on a busy machine, so their difference is noisy, and the agent's own byte-writing between turns may have slowed the following turn. The single-turn maximums were not confirmed as real work.
- The layout is a prototype. A real protocol would have framing and might compress.
- The views are computer players' views. A person scouts and fights differently.
- No seat had an ally. Allies' orders and cargo would add bytes.
- Order menus are not included.
- An event was sized as an id, a position and a small payload, not a full observable state. Events not about an entity were counted and given no bytes. Zoom and point targets were encoded without coordinates. The catch-up comparison covers view changes only, not the events missed.

**Still not measured:** which part of a turn dominates in the 16-player game; anything on a second machine; anything with a person playing.

## Appendix C. Outside facts and sources

Gathered by an Opus agent from primary sources on 14 November 2026. I checked the two NVDA items against the source files myself; the rest I have not re-checked.

**NVDA controller client.** Checked by me.

- The interface definition, with `speakSsml`, `setOnSsmlMarkReachedCallback`, `isSpeaking` and the priority values: https://github.com/nvaccess/nvda/blob/master/nvdaHelper/interfaces/nvdaController/nvdaController.idl. Its comment on `speakSsml` says that with `asynchronous` false it "returns either when the speech sequence is completed or canceled".
- Versions and licence: https://github.com/nvaccess/nvda/blob/master/extras/controllerClient/readme.md. Version 2.0, with `speakSsml`, came in NVDA 2024.1. Version 3.0, with `isSpeaking`, came in NVDA 2026.3. Older NVDA versions answer these calls with error 1717. The licence is LGPL 2.1.
- The readme also warns that NVDA runs on the lock screen, so a program should not send speech while Windows is locked.
- Not re-checked: that the return value for cancelled speech is `ERROR_CANCELLED`, and that a race which could miss a cancel was fixed in May 2026 (https://github.com/nvaccess/nvda/commit/3207d4852). The agent could not confirm which release first carried that fix.

**JAWS.** The interface has `SayString`, `StopSpeech`, `RunScript`, `RunFunction`, `Enable` and `Disable` only: https://github.com/dkager/tolk/blob/master/src/fsapi.h. Tolk's JAWS driver answers "is speaking" with a fixed false. The agent found no official reference from Freedom Scientific.

**Speech libraries.**

- Tolk: no releases, last commit 8 June 2024. https://github.com/dkager/tolk
- SRAL: archived; its author recommends Prism. https://github.com/m1maker/SRAL
- Prism: version 0.18.3 on 30 September 2026, commits to 11 November. It covers NVDA, JAWS, Narrator and others, with braille for NVDA and JAWS, and does not expose NVDA's SSML call. https://github.com/ethindp/prism. Its Rust wrapper is `prismer`, version 0.1.6.
- The `tts` crate: last published version from July 2024.

**UI Automation notifications and AccessKit.** `UiaRaiseNotificationEvent` offers processing hints and no cancel or completion: https://learn.microsoft.com/en-us/windows/win32/api/uiautomationcoreapi/nf-uiautomationcoreapi-uiaraisenotificationevent. AccessKit's Windows adapter raises live-region events only.

**Three-dimensional audio.**

- Steam Audio 4.8.1, Apache 2.0: https://github.com/ValveSoftware/steam-audio. Rust binding audionimbus 0.17.0: https://github.com/MaxenceMaire/audionimbus
- OpenAL Soft 1.25.2, LGPL, with headphone rendering; its Rust bindings are old or unproven.
- rodio 0.23.0, kira 0.12.6 and firewheel 0.15.0 have left-right placement only. fyrox-sound 1.0.1 has headphone rendering. Synthizer is archived.
- The player comparison: https://github.com/kcat/openal-soft/discussions/1305

**The original game.** Licence: https://github.com/soundmud/soundrts/blob/master/LICENSE.txt. The sentence about the rest: https://github.com/soundmud/soundrts/blob/master/README.txt. Last commit 7 May 2025; last release 1.3.8.1 on 14 September 2024.

## Appendix D. The original's data: what the public record shows

Researched: the original's licence covers "the Python source code" only, and its README says "The license for the rest is unclear at the moment". The rest is the sounds, maps, rules and translations. The project's last commit was in May 2025 and its last release in September 2024. A second search looked for anything the author or the community has said in public. What it found:

- **The author has kept the data outside the licence on purpose, since 2013.** The sentence about "the rest" was added in a commit that also narrowed the licence file to "the Python source code". In 2019 he added "Feel free to experiment on the code base, but don't feel obliged to contribute back." Nothing he has written grants or refuses reuse of the data.
- **The sounds and maps have several authors.** The author's own site in 2008 credited the sound pack to Bryan Smart and Ken Downey. Map files name about a dozen authors, and 8 of the 89 maps carry notices such as "Copyright Masackru, all rights reserved".
- **The community behaves as you describe.** All 41 forks of the repository carry the data, at least five standalone projects ship it, and nobody has objected in public. The author bundled one community mod, CrazyMod, in official releases.
- **Not searched:** audiogames.net refused automated reading, and the game's mailing list could not be searched. Forum quotes the agent saw came from search snippets and are unverified. You can read both; it could not.

So the informal understanding is real as behaviour and absent as a written permission. What that means for this project, as general information and not legal advice:

- Copyright covers a particular expression, not a game's design or rules. Squares joined by exits, two resources, a tech tree, selection by type, and the file formats are free to keep. Unit names such as "peasant" and "footman" are generic words, and a table of costs is close to fact. The recordings, the map files as written, the manual and the translations are other people's work.
- Three routes, which combine. Ship the program and fetch the data onto the player's machine from the author's own repository, as the fetch script does now and as OpenMW and OpenRA do; that redistributes nothing and works today. Ask the author, whose address is in the manual's credits; a yes covers what is his and not Bryan Smart's sounds or the maps with their own notices. Or write your own content: new names, rules, maps and sounds under an open licence.
- I recommend the first now, the second in parallel, and the third as content work when the game has its own identity. Section 7's move of type names out of the code and into the rules is what lets another set of rules and sounds stand in. I would give the game its own name in any case.
