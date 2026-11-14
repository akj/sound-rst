# What the SoundRTS Ultimate Version fork offers us

A review of [tuohai/soundrts-ultimate-version](https://github.com/tuohai/soundrts-ultimate-version) for bugs it fixed that our port still has, interface ideas worth copying, and rulesets we could adopt. This is evidence for later. We have not acted on any of it.

Written 14 November 2026 against fork commit `c01d5da` (version 1.5.2.0), the original at `f3bd983` (1.3.8.1), and our `f44b677`. The interface ideas below are adjusted for #28 (the status questions, F12) and #29 (nothing said stops the game), which merged since.

## How far each item was checked

Three Opus agents read the code of all three trees. One also loaded the fork's rules, maps and AI scripts through our port's parser, using throwaway probe programs outside this repository. I re-read our Rust for bugs 1 and 2 and for the menu quirk. Nothing was played or heard.

The fork mostly treats 1.3.8.1 as correct. Most of its fixes repair its own rewrite, so it adds few bugs to `open-findings.md`.

## Bugs our port still has

**1. A resurrected unit gets its owner's upgrades a second time.** Checked in our Rust. The corpse keeps a copy of the unit with its upgrade bonuses already applied (`crates/world/src/unit.rs`, `create_corpse`). Resurrection gives that copy back (`crates/world/src/orders.rs`, `resurrect`), and `player_add` re-applies every upgrade, so additive bonuses stack again on each resurrection. The original has the same bug (`worldorders.py` `execute_resurrection`, `worldupgrade.py` `effect_bonus`). The fork fixed it in 1.4.2.1 by recording which upgrade levels each unit already has. Simpler for us: reset the unit's stats from its type before re-applying. A unit converted with `a_conversion` takes the same path and keeps its old owner's bonuses on top of the new owner's. Changes gameplay.

**2. Upgrading a unit drops it from the selected group.** Checked in our Rust. `upgrade_to` deletes the unit and creates a new one with a new id (`orders.rs`, `Keyword::UpgradeTo`), and `update_group` (`crates/client/src/game/selection.rs`) keeps only ids that still exist. This affects archers to dark archers, keep to castle, towers, and walls and gates. The fork sends `upgrade_to,<new id>` before deleting and moves the selection to the new id (release note 1.4.3.9). Changes only the interface.

**3. "Load all" can load a transport into itself.** Read only. The loop over the square's units does not skip the transport itself. Every base transport has `transport_volume 99`, so only a mod can trigger it. The fork fixed it in 1.4.2.0.

**Suspected, not fixed by the fork either:** upgrading a tower, wall or gate with units inside it may lose those units. Deleting the old building takes the units inside away from their owner, and nothing blocks the upgrade while units are inside. Read only, not reproduced.

## Interface ideas worth copying

All are implemented in the fork's code. Sizes are estimates.

| Idea | Size | Relates to |
|---|---|---|
| Paste (and select, copy, cut) in text fields; today a joining player types the server address by hand | small | joining |
| Escape skips the rest of a cut scene; skipped lines stay in the story log. Since #29 any key skips one line, so a ten-line scene still takes ten presses | small | H2 |
| Copy the message under review to the clipboard | small | H3 |
| Step through objectives one at a time, with counts like "1/3" | small to medium | H5 |
| A list of the selected unit's combat statistics | medium | H5 |
| Keys that select idle soldiers; the filter already exists, only bindings are missing | small | |
| Each player's faction in the player list, and "which faction am I" after a random pick; #28's F12 says ally, enemy and defeated, but not faction | small | H5 |
| An ally ping: send your square to allies as a positioned alert | medium | |

**Menu quirk, from the original.** A menu that remembers a choice inserts a second copy of it at the top (`crates/client/src/menu.rs`, `append`), so a letter key searches from there, not from the top. The fork remembers the position instead.

**Rejected:**

- A second voice for battle news. It competes with the screen reader.
- Hotkeys that change with a mode.
- A key-remapping menu of about 2,200 lines.
- Speech-engine settings. The screen reader owns those.
- Scores, achievements and cards.
- Features for sighted players.

## Rulesets and mods

None of the fork's rules, maps or AI scripts load correctly in our port. This was run through our parser.

- **Base rules.** The fork renamed the core keywords instead of adding new ones: `damage`, `armor`, `range` and `cooldown` became the `mdg`/`rdg` and `mdf`/`rdf` families, and `food_cost` became `population_cost`. Its base rules give 434 warnings over 101 properties, so units load with no attack and no defense.
- **Maps.** Its maps fail to load, because of `house` and `x,y` squares.
- **AoE2.** Over our 262,144-byte rules limit.
- **Orc, teleportation, flyingtownhall.** The original already ships these.
- **Blitzkrieg.** The only near miss: its unknown keywords are mostly the renamed stats, so a one-off conversion could work.
- **Licence.** The fork's data has no stated licence. Copying any of it needs tuohai's permission.

**Random map generation** (`soundrts/randommap.py`) writes ordinary map files from menu choices and a seed. The core is a self-contained, medium-sized job. If we build it, the server should generate the map and send it through the existing map transfer. Water, terrain, treasure and the new victory modes depend on fork-only rules.

## AI

Most of the fork's AI work fixed its own rewrite. The original already keeps several production buildings busy. Worth considering:

- **A seed-sweep benchmark (small).** It measures first-attack time and time stuck at the population limit over many seeds, like the fork's `bench_ai_features.py` and `bench_jl1_sweep.py`. Our headless soak test could report the same numbers, and that would also measure G1.
- **Attack the enemy start squares when no enemy has ever been seen (small).** It matters only if the explorer dies early.
- **Send workers to the resource a farm is short of (small).** This would get the computer unstuck when it sits at its population limit. It is unverified whether that happens on the original's maps. Measure first.

The fork's `guard` mode, which shoots units in range and never walks, is a reference for G2.

## Multiplayer

The fork's rollback netcode is a different design and does not fit ours. Speech for events that are later undone cannot be taken back.

**The useful lesson is about late watchers.** Before rollback, the fork let a watcher join a game already in progress by sending the turn history and replaying it at full speed. Sending the whole game state was not needed. That is the same machinery as reconnecting (M1), so M8 and M1 can be built together. Pitfalls the fork hit:

- The watcher must not use up unit ids or random numbers.
- Keep audio muted while catching up, and say "you are spectating" once, after catching up.
- Start the watcher's view on a real player's start square.
- Keep sending live turns after the watcher catches up.

**For M10, the game record.** The fork's replay bugs came from several orders on one line and from timestamps that went backwards. Write one order per line, timed by turn number.
