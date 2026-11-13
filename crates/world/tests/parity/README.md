# Parity with the original game

`../parity.rs` checks that the Rust simulation reproduces what the original
Python code computes:

- `static_*`: the squares, exits and objects of a built map, in full.
- `trace_*`: a scripted game (`scenarios/*.txt`), one line per update: the
  turn, a hash of the dump of every player and entity, and the dump's first
  line (time and random generator state).

The cases are listed in `cases.txt`. The expected files in `expected/` come
from the original code. To regenerate them, create the Python 3.11 venv once
(`uv venv vendor/.venv-ref --python 3.11`), then run from
`vendor/soundrts-upstream`:

    ..\.venv-ref\Scripts\python.exe ..\..\crates\world\tests\parity\dump_reference.py --full ..\..\target\parity-reference

When a trace differs, the test writes the Rust dump to
`target/parity-actual/<case>.txt`; compare it with
`target/parity-reference/<case>.txt` to find the first difference.
