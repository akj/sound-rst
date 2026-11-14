# Traces of the simulation

`../traces.rs` runs the simulation on fixed cases and checks that it
computes what `expected/` records:

- `static_*`: the squares, exits and objects of a built map. Only the hash
  of the dump is kept, in `expected/static_hashes.txt`: the dump reproduces
  the map's content, whose license is not stated.
- `trace_*`: a scripted game (`scenarios/*.txt`), one line per update: the
  turn, a hash of the dump of every player and entity, and the dump's first
  line (time and random generator state).
- `trace_ai_*`: games with computer players (`easy`, `aggressive`, `ai2`),
  against each other or a scripted human (`scenarios/human.txt`).

The cases are listed in `cases.txt`. A player is a login or `ai_<type>`,
then optionally `:<alliance>`. Computers are allied with each other by
default.

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

2. Review the diff of `expected/`.
3. Increase `SIM_VERSION` in `crates/world/src/lib.rs`. Players can join the
   same game only with the same version.
4. Run `cargo test -p soundrts-world --lib sim_version`. The test fails and
   prints the new value of `TRACES_HASH`. Set `TRACES_HASH` to that value.

Keep only hashes, scenarios and the first line of each dump in the
repository. Never commit map text or full dumps.
