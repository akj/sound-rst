"""Dump what the original game computes, for the parity tests of the Rust
port (crates/world/tests/parity.rs).

Run from vendor/soundrts-upstream with the Python 3.11 venv:

    cd vendor/soundrts-upstream
    ..\\.venv-ref\\Scripts\\python.exe ..\\..\\crates\\world\\tests\\parity\\dump_reference.py

It rewrites crates/world/tests/parity/expected/ from the cases in
crates/world/tests/parity/cases.txt. A trace is stored as one line per
update: the turn, a hash of its full dump, and the dump's first line. A
built map is stored as the hash of its dump, in static_hashes.txt: the
dump reproduces the map's content. With `--full DIR`, the full dumps are
also written to DIR, to compare with the dumps the Rust test writes when it
fails. With `--only PREFIX`, only the cases whose name starts with PREFIX
are run.
"""
import os
import re
import sys

ARGS = sys.argv[1:]
# The original parses the command line when imported.
sys.argv = ["soundrts"]
sys.path.insert(0, os.getcwd())

from soundrts import msgparts as mp  # noqa: E402
from soundrts.definitions import load_ai, rules  # noqa: E402
from soundrts.lib import group  # noqa: E402
from soundrts.lib.nofloat import PRECISION  # noqa: E402
from soundrts.world import World  # noqa: E402
from soundrts.worldclient import DummyClient, RemoteClient, _Controller  # noqa: E402


def _name(self):
    # clientservermenu.name without importing the client (and pygame).
    if self.login == "ai_easy":
        return mp.QUIET_COMPUTER
    if self.login == "ai_aggressive":
        return mp.AGGRESSIVE_COMPUTER
    return [self.login]


_Controller.name = property(_name)

HERE = os.path.dirname(os.path.abspath(__file__))
MULTIPLAYER_TRIGGERS = [
    ("players", ["no_enemy_player_left"], ["victory"]),
    ("players", ["no_building_left"], ["defeat"]),
    ("computers", ["no_unit_left"], ["defeat"]),
]


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def make_world(map_path, rules_paths, ai_paths, seed, default_triggers):
    rules.load("\n".join(read(p) for p in rules_paths))
    load_ai("\n".join(read(p) for p in ai_paths))
    w = World(list(default_triggers), seed)
    w._parse_map(read(map_path))
    w.square_width = int(w.square_width * PRECISION)
    w._build_map()
    return w


def dump_static(w):
    out = [
        "map title=%s objective=%s players=%s-%s starts=%s computers=%s food=%s"
        % (
            ",".join(map(str, w.title)),
            ",".join(map(str, w.objective)),
            w.nb_players_min,
            w.nb_players_max,
            len(w.players_starts),
            len(w.computers_starts),
            w.global_food_limit,
        )
    ]
    for s in w.squares:
        out.append(
            "square %s id=%s x=%s y=%s high=%d water=%d ground=%d air=%d terrain=%s speed=%s cover=%s"
            % (
                s.name,
                s.id,
                s.x,
                s.y,
                s.high_ground,
                s.is_water,
                s.is_ground,
                s.is_air,
                s.type_name,
                ",".join(map(str, s.terrain_speed)),
                ",".join(map(str, s.terrain_cover)),
            )
        )
        for e in s.exits:
            out.append(
                "  exit %s %s %s %s %s other=%s portal=%d"
                % (e.id, e.type_name, e.x, e.y, e.o, e._other_side_id, e.is_a_portal)
            )
        for o in s.objects:
            out.append("  object %s %s %s %s qty=%s" % (o.id, o.type_name, o.x, o.y, o.qty))
    # save_map groups exits by iterating a set, so its line order varies.
    path = os.path.join(HERE, "save_map.tmp")
    w.save_map(path)
    with open(path) as f:
        saved = f.read().splitlines()
    os.remove(path)
    out.extend("save " + line for line in sorted(saved))
    return out


def fingerprint(r):
    h = 0xCBF29CE484222325
    mask = 0xFFFFFFFFFFFFFFFF
    for x in r.getstate()[1]:
        h = ((h ^ x) * 0x100000001B3) & mask
    return "%016x" % h


ACTIONS = {
    "Action": "Action",
    "MoveAction": "MoveAction",
    "MoveXYAction": "MoveXYAction",
    "AttackAction": "AttackAction",
}


def obj_line(o):
    hp = getattr(o, "hp", "-")
    mana = getattr(o, "mana", "-")
    player = o.player.id if o.player is not None else "-"
    orders = ",".join(x.keyword for x in getattr(o, "orders", []))
    action = getattr(o, "action", None)
    action = ACTIONS[type(action).__name__] if action is not None else "-"
    return "%s %s %s %s %s hp=%s mana=%s qty=%s player=%s orders=%s action=%s" % (
        o.id,
        o.type_name,
        o.x,
        o.y,
        o.o,
        hp,
        mana,
        o.qty,
        player,
        orders,
        action,
    )


def dump_tick(w):
    lines = ["time %s random %s" % (w.time, fingerprint(w.random))]
    for p in w.players:
        lines.append(
            "player %s res=%s food=%s used=%s units=%s upgrades=%s perception=%s memory=%s observed=%s victory=%d defeat=%d"
            % (
                p.id,
                ",".join(map(str, p.resources)),
                p.food,
                p.used_food,
                len(p.units),
                ",".join(p.upgrades),
                len(p.perception),
                len(p.memory),
                len(p.observed_squares),
                p.has_victory,
                p.has_been_defeated,
            )
        )
    for s in w.squares:
        for o in s.objects:
            lines.append(obj_line(o))
            inside = getattr(o, "inside", None)
            if inside is not None:
                for i in inside.objects:
                    lines.append("  in " + obj_line(i))
    return lines


def resolve(w, players, pi, cmd):
    me = players[pi]
    other = players[1 - pi] if len(players) > 1 else None

    def units(p, t):
        return [u for u in p.units if u.type_name == t]

    def repl(m):
        parts = m.group(1).split(":")
        k = parts[0]
        if k == "u":
            return units(me, parts[1])[int(parts[2])].id
        if k == "eu":
            return units(other, parts[1])[int(parts[2])].id
        if k == "here":
            return units(me, parts[1])[int(parts[2])].place.id
        if k == "hn":
            return units(me, parts[1])[int(parts[2])].place.name
        if k == "there":
            return units(other, parts[1])[int(parts[2])].place.id
        if k == "at":
            objs = [o for o in w.grid[parts[1]].objects if o.type_name == parts[2]]
            return objs[int(parts[3])].id
        if k == "mead":
            objs = [o for o in me.units[0].place.objects if o.type_name == "meadow"]
            return objs[int(parts[1])].id
        if k == "loc":
            objs = [o for o in me.units[0].place.objects if o.type_name == parts[1]]
            return objs[int(parts[2])].id
        if k == "ids":
            return group.encode([u.id for u in units(me, parts[1])])
        if k == "sq":
            return w.grid[parts[1]].id
        raise ValueError(k)

    try:
        return re.sub(r"\{([^}]*)\}", repl, cmd)
    except (IndexError, AttributeError):
        return None


def controllers(spec):
    # login or ai_<type>, then optionally :<alliance>
    result = []
    for s in spec.split(","):
        s, _, alliance = s.partition(":")
        if s.startswith("ai_"):
            c = DummyClient(s[3:])
        else:
            c = RemoteClient(s)
        if alliance:
            c.alliance = alliance
        result.append(c)
    return result


def run_trace(case, upstream):
    w = make_world(
        os.path.join(upstream, case["map"]),
        [os.path.join(upstream, p) for p in case["rules"]],
        [os.path.join(upstream, p) for p in case["ai"]],
        case["seed"],
        MULTIPLAYER_TRIGGERS if case["triggers"] == "multiplayer" else [],
    )
    clients = controllers(case["players"])
    w.populate_map(clients, random_starts=True, equivalents=True)
    players = [c.player for c in clients]
    scenario = {}
    for line in read(os.path.join(HERE, case["scenario"])).splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        tick, pi, cmd = line.split(" ", 2)
        scenario.setdefault(int(tick), []).append((int(pi), cmd))
    full = []
    summary = []
    for t in range(case["ticks"]):
        for pi, cmd in scenario.get(t, []):
            r = resolve(w, players, pi, cmd)
            if r is not None and players[pi] in w.players:
                players[pi].execute_command(r)
        w.update()
        lines = dump_tick(w)
        full.append("tick %s" % t)
        full.extend(lines)
        summary.append("tick %s %s %s" % (t, fnv64("\n".join(lines)), lines[0]))
    return summary, full


def fnv64(text):
    h = 0xCBF29CE484222325
    for b in text.encode("utf-8"):
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "%016x" % h


def parse_cases():
    cases = []
    for line in read(os.path.join(HERE, "cases.txt")).splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = dict(f.split("=", 1) for f in line.split())
        case = {
            "name": fields["name"],
            "kind": fields["kind"],
            "map": fields["map"],
            "rules": fields.get("rules", "res/rules.txt").split(","),
            "ai": fields.get("ai", "res/ai.txt").split(","),
            "seed": int(fields.get("seed", "0")),
            "triggers": fields.get("triggers", "none"),
            "players": fields.get("players", ""),
            "scenario": fields.get("scenario", ""),
            "ticks": int(fields.get("ticks", "0")),
        }
        cases.append(case)
    return cases


def main():
    upstream = os.getcwd()
    full_dir = ARGS[ARGS.index("--full") + 1] if "--full" in ARGS else None
    only = ARGS[ARGS.index("--only") + 1] if "--only" in ARGS else ""
    os.makedirs(os.path.join(HERE, "expected"), exist_ok=True)
    hashes_path = os.path.join(HERE, "expected", "static_hashes.txt")
    static_hashes = {}
    if os.path.exists(hashes_path):
        for line in read(hashes_path).splitlines():
            if line.strip():
                name, h = line.split()
                static_hashes[name] = h
    cases = parse_cases()
    for case in cases:
        if not case["name"].startswith(only):
            continue
        if case["kind"] == "static":
            w = make_world(
                os.path.join(upstream, case["map"]),
                [os.path.join(upstream, p) for p in case["rules"]],
                [os.path.join(upstream, p) for p in case["ai"]],
                case["seed"],
                [],
            )
            full = dump_static(w)
            # The dump reproduces the map, so only its hash is kept.
            static_hashes[case["name"]] = fnv64("\n".join(full))
        else:
            lines, full = run_trace(case, upstream)
            with open(os.path.join(HERE, "expected", case["name"] + ".txt"), "w", newline="\n") as f:
                f.write("\n".join(lines) + "\n")
        if full_dir:
            os.makedirs(full_dir, exist_ok=True)
            with open(os.path.join(full_dir, case["name"] + ".txt"), "w", newline="\n") as f:
                f.write("\n".join(full) + "\n")
        print(case["name"], len(full))
    with open(hashes_path, "w", newline="\n") as f:
        for case in cases:
            if case["name"] in static_hashes:
                f.write("%s %s\n" % (case["name"], static_hashes[case["name"]]))


if __name__ == "__main__":
    main()
