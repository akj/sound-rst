# Traces of the simulation

`../traces.rs` runs the simulation on fixed cases and checks that it
computes what `expected/` records:

- `static_*`: the squares, exits and objects of a built map. Only the hash
  of the dump is kept, in `expected/static_hashes.txt`: the dump reproduces
  the map's content, whose license is not stated.
- `trace_*`: a scripted game (`scenarios/*.txt`), one line per update: the
  turn, a hash of the dump of every player and entity, and the dump's first
  line (time and random generator state).
- `trace_*.digest.txt`: the same game, one line per update: the turn and
  `World::digest_u64`, the hash of the whole state.
- `trace_ai_*`: games with computer players (`easy`, `aggressive`, `ai2`),
  against each other or a scripted human (`scenarios/human.txt`).
- `trace_orders` and `trace_spells`: short games that do what no other
  case does: the orders and effects their scenario lists at its top.

The dump has the positions, hit points, mana, resources, owners, the names
of queued orders, the kind of each action and the random generator's state.
It leaves out what players remember, what orders are aimed at, timers and
the computer players' state. The digest covers all of these. The two
answer different questions:

- A change of behaviour changes the digest, and usually the dump too.
- A change in how the state is stored, with the same behaviour, changes
  the digest and leaves the dump the same. Re-record only the digest files
  then, and check that no other file in `expected/` changed.

Multiplayer games and the record check compare the digest, so a change of
the digest files also needs a new `SIM_VERSION` (see Record again).

The cases are listed in `cases.txt`. A player is a login or `ai_<type>`,
then optionally `:<alliance>`. Computers are allied with each other by
default.

A scenario line is `<turn> <player> <command>`. A line `expect <player>
<event>` says that the player must hear, at some turn, an event whose
entity's type name and event text, as `<type> <text>`, start with
`<event>`: `expect 0 priest use_complete,a_conversion`. The test fails if
none comes, so a case that stops doing what it was written for is noticed,
not re-recorded. Hearing events changes neither the dump nor the digest.

When a change of the simulation makes a scenario stop doing what its top
lines say it covers, rewrite the scenario so that it does again, rather
than deleting the case. When something no recorded game does matters to a
change, add it to `scenarios/orders.txt` or `scenarios/spells.txt`, or a new
short case, with an `expect` line when an event shows it.

The files in `expected/` were recorded from the original Python game. Up
to the Git tag `last-python-parity`, the port matched them byte for byte.
Since then they are the port's own. The script that recorded them is in
that tag.

## Run

Run every case:

    cargo test -p soundrts-world --test traces

To run one test, add part of its name, for example `fight_on_jl7`. The
test `games_with_computer_players_match_the_recording_tick_by_tick` runs
every `trace_ai_` case. To run only some of them, set `TRACES_ONLY` to the
start of their name, for example `TRACES_ONLY=jl5`.

## Find the first difference

When a trace differs, the test prints the first line that differs. It also
writes the full dump of the case to `target/traces-actual/<case>.txt`. The
dump has a `tick <n>` line for each update, then the state of every player
and entity after that update. Find that tick in the dump.

When a built map differs, the test prints the cases whose hash differs and
writes their dumps to the same folder.

When only a digest file differs, the dump did not see the change. The test
prints the first turn whose digest differs. The digest has no dump to
compare: find which part of the state differs at that turn with a debugger
or a temporary print of the parts that `impl Hash for World` hashes.

To compare with the previous behaviour, get the dump of the previous
commit too:

1. Check out the previous commit in a second worktree.
2. In the second worktree, change one character in the first line of
   `expected/<case>.txt`. For a built map, change one character of its
   hash in `expected/static_hashes.txt`.
3. Run the same test there. It fails and writes the previous dump to its
   own `target/traces-actual/`.
4. Compare the two dumps.

## Record again

Do this only when the change of the simulation is intended.

1. Run the tests with `UPDATE_EXPECTED=1`. The tests write what the port
   computes to `expected/` and pass. In PowerShell:

       $env:UPDATE_EXPECTED = '1'
       cargo test -p soundrts-world --test traces
       Remove-Item Env:UPDATE_EXPECTED

2. Review the diff of `expected/`. A change that is meant to keep the
   behaviour may change only `*.digest.txt`.
3. Increase `SIM_VERSION` in `crates/world/src/lib.rs`. Players can join the
   same game only with the same version.
4. Run `cargo test -p soundrts-world --lib sim_version`. The test fails and
   prints the new value of `TRACES_HASH`. Set `TRACES_HASH` to that value.

Keep only hashes, scenarios and the first line of each dump in the
repository. Never commit map text or full dumps.
