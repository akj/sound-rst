# Parity with the original game

`../parity.rs` checks that the Rust simulation reproduces what the original
Python code computes:

- `static_*`: the squares, exits and objects of a built map. Only the hash
  of the dump is kept, in `expected/static_hashes.txt`: the dump reproduces
  the map's content, whose license is not stated.
- `trace_*`: a scripted game (`scenarios/*.txt`), one line per update: the
  turn, a hash of the dump of every player and entity, and the dump's first
  line (time and random generator state).
- `trace_ai_*`: games with computer players (`easy`, `aggressive`, `ai2`),
  against each other or a scripted human (`scenarios/human.txt`).

The cases are listed in `cases.txt`. A player is a login or `ai_<type>`,
then optionally `:<alliance>` (computers are allied with each other by
default). The files in `expected/` come from the original code. Keep only
hashes and scenarios in the repository, never map text or full dumps.

To regenerate them, create the Python 3.11 venv once
(`uv venv vendor/.venv-ref --python 3.11`), then run from
`vendor/soundrts-upstream`:

    ..\.venv-ref\Scripts\python.exe ..\..\crates\world\tests\parity\dump_reference.py --full ..\..\target\parity-reference

`--only <prefix>` runs only the cases whose name starts with the prefix.
`--full <dir>` writes the full dumps to the directory.

When a case differs, the test writes the Rust dump to
`target/parity-actual/<case>.txt`; produce the original's with `--only
<case> --full ..\..\target\parity-reference` and compare the two files to
find the first difference. `PARITY_ONLY=<prefix>` narrows the
`games_with_computer_players...` test to some of the `trace_ai_` cases.
