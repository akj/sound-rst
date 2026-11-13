"""Drive the original SoundRTS client with scripted key presses and record
everything it says, for the interface parity tests of the Rust port.

The original runs in-process, headless and deterministic: a virtual clock,
a synchronous world, a fake speech engine, fake mixer channels and a fake
event queue. Nothing is spoken, played, shown or sent over the network.
README.md in this directory states the exact semantics.

Run from the repository root with the venv vendor/.venv-ui:

    vendor\\.venv-ui\\Scripts\\python.exe crates\\client\\tests\\parity\\drive_reference.py
        regenerate every expected/<scenario>.txt
    ... drive_reference.py menus_main training_jl1
        regenerate only these scenarios
    ... drive_reference.py --check
        run every scenario twice from fresh scratch roots and compare the
        two runs byte for byte with each other and with expected/
    ... drive_reference.py --perturb
        run every scenario with two other orders of the hash-based sets and
        compare with expected/ (finds output that depends on set order)
    ... drive_reference.py --perturb-random
        run every scenario with two other seeds for the client's global
        random module and compare the voice (not the positional sounds)
    --full DIR    also write the full positional-sound streams to DIR

Each scenario runs in a child process (`--child`), with its current
directory at a fresh scratch game root under vendor/ui-scratch/.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
UPSTREAM = os.path.join(REPO, "vendor", "soundrts-upstream")
SCRATCH = os.path.join(REPO, "vendor", "ui-scratch")
SCENARIOS = os.path.join(HERE, "scenarios")
EXPECTED = os.path.join(HERE, "expected")

# Committed alongside the transcript when the full positional-sound stream
# is smaller than this; otherwise only its per-step hashes are committed.
FULL_SOUNDS_LIMIT = 256 * 1024

SETTINGS = ("language", "login", "mods", "soundpacks", "seed", "limit")
STEP_KINDS = ("key", "type", "wait", "wait_turn", "wait_turns", "wait_game_end", "interrupt_after")


def fnv64(text):
    h = 0xCBF29CE484222325
    for b in text.encode("utf-8"):
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "%016x" % h


def parse_scenario(path):
    settings = {
        "language": "en",
        "login": "player",
        "mods": "",
        "soundpacks": "",
        "seed": "0",
        "limit": "600",
    }
    steps = []
    with open(path, encoding="utf-8") as f:
        for n, raw in enumerate(f, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            words = line.split()
            if words[0] in SETTINGS:
                if steps:
                    raise ValueError("%s:%d: setting after a step" % (path, n))
                settings[words[0]] = " ".join(words[1:])
            elif words[0] in STEP_KINDS:
                steps.append(" ".join(words))
            else:
                raise ValueError("%s:%d: unknown line: %s" % (path, n, line))
    return settings, steps


# ---------------------------------------------------------------------------
# parent: scratch roots, child processes, comparisons


def scenario_names():
    return sorted(n[:-4] for n in os.listdir(SCENARIOS) if n.endswith(".txt"))


def make_scratch_root(tag):
    import shutil

    root = os.path.join(SCRATCH, tag)
    if os.path.exists(root):
        shutil.rmtree(root)
    os.makedirs(root)
    for d in ("res", "cfg", "single", "mods"):
        shutil.copytree(os.path.join(UPSTREAM, d), os.path.join(root, d))
    os.makedirs(os.path.join(root, "user"))
    return root


def run_child(name, out_path, sounds_path, tag, perturb=0, random_offset=0):
    import subprocess

    root = make_scratch_root(tag)
    env = dict(os.environ)
    env["PYTHONHASHSEED"] = str(perturb)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env["SDL_VIDEODRIVER"] = "dummy"
    env["SDL_AUDIODRIVER"] = "dummy"
    env["PYGAME_HIDE_SUPPORT_PROMPT"] = "1"
    cmd = [
        sys.executable,
        "-B",
        os.path.abspath(__file__),
        "--child",
        name,
        "--out",
        out_path,
        "--sounds",
        sounds_path,
        "--perturb",
        str(perturb),
        "--random-offset",
        str(random_offset),
    ]
    with open(os.path.join(root, "driver.log"), "w", encoding="utf-8") as log:
        p = subprocess.run(cmd, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
    return p.returncode, root


def write_expected(name, tmp_out, tmp_sounds, full_dir):
    import shutil

    shutil.copyfile(tmp_out, os.path.join(EXPECTED, name + ".txt"))
    committed_sounds = os.path.join(EXPECTED, name + ".sounds.txt")
    if os.path.getsize(tmp_sounds) <= FULL_SOUNDS_LIMIT:
        shutil.copyfile(tmp_sounds, committed_sounds)
    elif os.path.exists(committed_sounds):
        os.remove(committed_sounds)
    if full_dir:
        os.makedirs(full_dir, exist_ok=True)
        shutil.copyfile(tmp_sounds, os.path.join(full_dir, name + ".sounds.txt"))


def read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def voice_only(path):
    """The transcript without the positional-sound summaries and lengths."""
    lines = read_bytes(path).decode("utf-8").splitlines()
    return [l for l in lines if not l.startswith("length ") and " sounds " not in l]


def summary_line(name, path):
    lines = read_bytes(path).decode("utf-8").splitlines()
    voice = sum(1 for l in lines if " voice " in l and " voice cut" not in l)
    steps = sum(1 for l in lines if " step " in l)
    last = lines[-1].split(" ", 1)
    return "%s: %d steps, %d voice lines, virtual %s ms, %s" % (name, steps, voice, last[0][1:], last[1])


MODES = {
    # mode: (variants (run name, hash perturbation, client random offset), voice only?)
    "--check": ([("run1", 0, 0), ("run2", 0, 0)], False),
    "--perturb": ([("perturb1", 1, 0), ("perturb2", 2, 0)], False),
    "--perturb-random": ([("random1", 0, 1), ("random2", 0, 2)], True),
}


def compare(name, mode, tmp):
    variants, voice_only_cmp = MODES[mode]
    outs = []
    for run, perturb, offset in variants:
        d = os.path.join(tmp, run)
        os.makedirs(d, exist_ok=True)
        out = os.path.join(d, name + ".txt")
        sounds = os.path.join(d, name + ".sounds.txt")
        code, _ = run_child(name, out, sounds, "%s-%s" % (name, run), perturb, offset)
        outs.append((run, code, out, sounds))
    ref = os.path.join(EXPECTED, name + ".txt")
    ref_sounds = os.path.join(EXPECTED, name + ".sounds.txt")
    failures = 0
    verdicts = []
    for run, code, out, sounds in outs:
        if voice_only_cmp:
            ok = code == 0 and voice_only(out) == voice_only(ref)
        else:
            ok = code == 0 and read_bytes(out) == read_bytes(ref)
            if ok and os.path.exists(ref_sounds):
                ok = read_bytes(sounds) == read_bytes(ref_sounds)
        verdicts.append("%s=%s" % (run, "same as expected" if ok else "DIFFERENT(exit %d)" % code))
        failures += not ok
    if voice_only_cmp:
        same = voice_only(outs[0][2]) == voice_only(outs[1][2])
    else:
        same = read_bytes(outs[0][2]) == read_bytes(outs[1][2]) and read_bytes(outs[0][3]) == read_bytes(
            outs[1][3]
        )
    failures += not same
    what = "voice transcripts" if voice_only_cmp else "transcripts and full sound streams"
    print(
        "%s: %s; %s vs %s %s: %s"
        % (name, ", ".join(verdicts), outs[0][0], outs[1][0], what, "byte-identical" if same else "DIFFERENT")
    )
    return failures


def main_parent(args):
    full_dir = None
    if "--full" in args:
        i = args.index("--full")
        full_dir = os.path.abspath(args[i + 1])
        del args[i : i + 2]
    modes = [a for a in args if a in MODES]
    names = [a for a in args if not a.startswith("--")] or scenario_names()
    tmp = os.path.join(REPO, "target", "ui-parity")
    os.makedirs(EXPECTED, exist_ok=True)
    failures = 0
    for name in names:
        if modes:
            for mode in modes:
                failures += compare(name, mode, tmp)
            continue
        d = os.path.join(tmp, "regen")
        os.makedirs(d, exist_ok=True)
        out = os.path.join(d, name + ".txt")
        sounds = os.path.join(d, name + ".sounds.txt")
        code, root = run_child(name, out, sounds, name + "-regen")
        if code != 0:
            print("%s: FAILED (exit %d), see %s" % (name, code, os.path.join(root, "driver.log")))
            failures += 1
            if not os.path.exists(out):
                continue
        write_expected(name, out, sounds, full_dir)
        print(summary_line(name, out))
    sys.exit(1 if failures else 0)

# ---------------------------------------------------------------------------
# child: the guarded, deterministic run of one scenario


NO_OP_THREAD_TARGETS = set()


class ScenarioEnd(SystemExit):
    """Raised at an input point when the script is exhausted and settled."""


class ScenarioAbort(SystemExit):
    """Raised when a limit is exceeded."""


BASE_US = 1_000_000_000_000_000  # the virtual epoch: 1e9 s, in microseconds


class Clock:
    def __init__(self, limit_us):
        self.us = 0
        self.limit_us = limit_us
        self.on_advance = None

    def time(self):
        return (BASE_US + self.us) / 1_000_000

    def advance(self, us):
        if us < 0:
            raise ValueError("negative sleep")
        self.us += us
        if self.us > self.limit_us:
            raise ScenarioAbort("virtual time limit exceeded")
        if self.on_advance is not None:
            self.on_advance()

    def sleep(self, seconds):
        self.advance(round(seconds * 1_000_000))

    @property
    def ms(self):
        return self.us // 1000


def install_guards():
    """No speech, sound, window or network, ever. Runs before pygame and
    before any client module is imported."""
    import socket
    import threading
    import types
    import urllib.request

    os.environ["SDL_VIDEODRIVER"] = "dummy"
    os.environ["SDL_AUDIODRIVER"] = "dummy"
    os.environ["PYGAME_HIDE_SUPPORT_PROMPT"] = "1"
    sys.dont_write_bytecode = True

    blocked = []

    # A stub accessible_output2: the real one talks to the screen reader.
    class _BlockedOutput:
        def __init__(self, *a, **k):
            blocked.append("accessible_output2")
            raise RuntimeError("accessible_output2 is blocked by the parity driver")

    ao2 = types.ModuleType("accessible_output2")
    ao2.__path__ = []
    outputs = types.ModuleType("accessible_output2.outputs")
    outputs.__path__ = []
    auto = types.ModuleType("accessible_output2.outputs.auto")
    auto.Auto = _BlockedOutput
    ao2.outputs = outputs
    outputs.auto = auto
    sys.modules["accessible_output2"] = ao2
    sys.modules["accessible_output2.outputs"] = outputs
    sys.modules["accessible_output2.outputs.auto"] = auto

    # No network.
    def _no_network(*a, **k):
        import traceback

        stack = traceback.extract_stack()[:-1]
        if any(f.name == "_has_ipv6" for f in stack):
            # urllib3 probes IPv6 support at import by creating a socket;
            # refused like any other socket, nothing to report.
            raise OSError("network access is blocked by the parity driver")
        where = " < ".join(
            "%s:%s" % (os.path.basename(f.filename), f.lineno)
            for f in reversed(stack)
            if not f.filename.startswith("<")
        )
        blocked.append("network %r at %s" % (a[:1], where))
        raise OSError("network access is blocked by the parity driver")

    class _NoSocket(socket.socket):
        def __init__(self, *a, **k):
            _no_network()

    socket.socket = _NoSocket
    socket.create_connection = _no_network
    socket.getaddrinfo = _no_network
    socket.gethostbyname = _no_network
    urllib.request.urlopen = _no_network

    # No threads: the order of things must not depend on a scheduler.
    _start = threading.Thread.start

    def _guarded_start(self):
        target = getattr(self, "_target", None)
        if getattr(target, "__func__", None) in NO_OP_THREAD_TARGETS:
            return  # the world runs synchronously (see World.queue_command)
        blocked.append("thread %r" % self)
        raise RuntimeError("threads are blocked by the parity driver: %r" % self)

    threading.Thread.start = _guarded_start
    return blocked


def sdl_drivers():
    import ctypes

    import pygame

    dll = ctypes.CDLL(os.path.join(os.path.dirname(pygame.__file__), "SDL2.dll"))
    dll.SDL_GetCurrentAudioDriver.restype = ctypes.c_char_p
    dll.SDL_GetCurrentVideoDriver.restype = ctypes.c_char_p
    a = dll.SDL_GetCurrentAudioDriver()
    v = dll.SDL_GetCurrentVideoDriver()
    return (v or b"").decode(), (a or b"").decode()


def assert_dummy_drivers():
    import pygame

    video, audio = sdl_drivers()
    if pygame.display.get_driver() != "dummy" or video != "dummy":
        raise SystemExit("ABORT: the video driver is %r, not dummy" % video)
    if not pygame.mixer.get_init() or audio != "dummy":
        raise SystemExit("ABORT: the audio driver is %r, not dummy" % audio)


KEY_UNICODE = {
    "SPACE": " ",
    "RETURN": "\r",
    "KP_ENTER": "\r",
    "TAB": "\t",
    "BACKSPACE": "\x08",
    "ESCAPE": "\x1b",
    "DELETE": "\x7f",
    "KP_PLUS": "+",
    "KP_MINUS": "-",
    "KP_MULTIPLY": "*",
    "KP_DIVIDE": "/",
    "KP_PERIOD": ".",
}
US_PUNCT = {
    "QUOTE": ("'", '"'),
    "BACKQUOTE": ("`", "~"),
    "SEMICOLON": (";", ":"),
    "BACKSLASH": ("\\", "|"),
    "COMMA": (",", "<"),
    "PERIOD": (".", ">"),
    "SLASH": ("/", "?"),
    "MINUS": ("-", "_"),
    "EQUALS": ("=", "+"),
    "LEFTBRACKET": ("[", "{"),
    "RIGHTBRACKET": ("]", "}"),
}
US_SHIFTED_DIGITS = ")!@#$%^&*("
EXTRA_SCANCODES = {"QUOTE": 52, "BACKQUOTE": 53}  # as in soundrts/lib/bindings.py
MODIFIER_FLAGS = {"CTRL": "KMOD_LCTRL", "ALT": "KMOD_LALT", "SHIFT": "KMOD_LSHIFT"}
MODIFIER_KEYS = {
    "LCTRL": "KMOD_LCTRL",
    "RCTRL": "KMOD_RCTRL",
    "LALT": "KMOD_LALT",
    "RALT": "KMOD_RALT",
    "LSHIFT": "KMOD_LSHIFT",
    "RSHIFT": "KMOD_RSHIFT",
}


def key_event(spec):
    """'CTRL SHIFT F2' -> a pygame KEYDOWN event, as SDL2 + pygame 2 deliver
    it on a US layout: key (keycode), mod, unicode, scancode."""
    import pygame

    words = spec.split()
    mods, name = words[:-1], words[-1]
    mod = 0
    for m in mods:
        mod |= getattr(pygame, MODIFIER_FLAGS[m])
    if name in MODIFIER_KEYS:
        mod |= getattr(pygame, MODIFIER_KEYS[name])
    key = getattr(pygame, "K_" + (name.lower() if len(name) == 1 else name))
    if name in EXTRA_SCANCODES:
        scancode = EXTRA_SCANCODES[name]
    else:
        scancode = getattr(pygame, "KSCAN_" + (name.upper() if len(name) == 1 else name))
    shift = "SHIFT" in mods
    ctrl = "CTRL" in mods
    if len(name) == 1 and name.isalpha():
        if ctrl:
            uni = chr(ord(name.lower()) - ord("a") + 1)
        else:
            uni = name.upper() if shift else name.lower()
    elif len(name) == 1 and name.isdigit():
        uni = "" if ctrl else (US_SHIFTED_DIGITS[int(name)] if shift else name)
    elif name in US_PUNCT:
        uni = "" if ctrl else US_PUNCT[name][1 if shift else 0]
    else:
        uni = KEY_UNICODE.get(name, "")
    return pygame.event.Event(pygame.KEYDOWN, key=key, mod=mod, unicode=uni, scancode=scancode, window=None)


TYPE_KEYS = {" ": "SPACE", ".": "PERIOD"}


def expand_steps(steps):
    """`type bob` becomes one key step per character."""
    result = []
    for s in steps:
        words = s.split(" ", 1)
        if words[0] == "type":
            for c in words[1]:
                if c.isalpha() and c.isupper():
                    result.append(("key SHIFT %s" % c.lower(), s))
                elif c.isalnum():
                    result.append(("key %s" % c, s))
                else:
                    result.append(("key %s" % TYPE_KEYS[c], s))
        else:
            result.append((s, s))
    return result


def fmt(x):
    s = "%.3f" % x
    return "0.000" if s == "-0.000" else s


class Driver:
    def __init__(self, name, settings, steps, perturb):
        self.name = name
        self.settings = settings
        self.steps = expand_steps(steps)
        self.perturb = perturb
        self.clock = Clock(int(float(settings["limit"]) * 1_000_000))
        self.voice_lines = []  # the main transcript (voice, titles, steps...)
        self.sound_lines = []  # the positional-sound stream
        self.step_sounds = []  # positional lines of the current step
        self.lengths = {}  # sound name -> length in microseconds
        self.next_step = 0
        self.last_delivery_us = 0
        self.voilas = 0
        self.voilas_at_delivery = 0
        self.turn_at_delivery = None
        self.in_voice_play = False
        self.last_cut_us = None
        self.fake_tts = None
        self.interface = None
        self.queue = []
        self.title = None
        self.ended = False
        self.reads_without_time = 0

    # --- transcript

    def emit(self, text):
        self.voice_lines.append("@%d %s" % (self.clock.ms, text))

    def emit_sound(self, text):
        line = "@%d %s" % (self.clock.ms, text)
        self.sound_lines.append(line)
        self.step_sounds.append(line)

    def close_step_sounds(self):
        if self.step_sounds:
            self.emit("sounds %d %s" % (len(self.step_sounds), fnv64("\n".join(self.step_sounds))))
            self.step_sounds = []

    def voice_stopping(self):
        """Speech in progress is being stopped. A new message stops the
        previous one implicitly; any other stop is recorded as a cut, with
        the number of message parts still queued in the voice channel
        (sound.stop() does not clear them: they play on)."""
        if self.in_voice_play:
            return
        from soundrts.lib.voice import voice

        channel = getattr(voice, "channel", None)
        if channel is None or self.fake_tts is None:
            return
        if self.fake_tts.is_speaking() or channel.c.get_busy():
            if self.last_cut_us != self.clock.us:
                self.last_cut_us = self.clock.us
                self.emit("voice cut queue=%d" % len(channel._queue))

    # --- steps

    def voice_is_idle(self):
        from soundrts.lib.voice import voice

        return not voice.channel.get_busy() and not voice._unsaid_exists()

    def settled(self, point):
        if any(e.type == self.KEYDOWN for e in self.queue):
            return False
        if not self.voice_is_idle():
            return False
        if point == "game":
            i = self.interface
            if not i._srv_queue.empty():
                return False
            if self.voilas <= self.voilas_at_delivery:
                return False
        return True

    def step_ready(self, step, point):
        kind, _, arg = step.partition(" ")
        if kind == "key":
            return point != "voice" and self.settled(point)
        if kind == "interrupt_after":
            delay, _, _ = arg.partition(" ")
            return self.clock.us >= self.last_delivery_us + round(float(delay) * 1_000_000)
        if kind == "wait":
            return self.clock.us >= self.last_delivery_us + round(float(arg) * 1_000_000)
        if kind == "wait_turn":
            return self.interface is not None and self.interface.world.turn >= int(arg)
        if kind == "wait_turns":
            # counted from the turn of the previous step's delivery, or from
            # turn 0 if no game was running then
            return self.interface is not None and self.interface.world.turn >= (
                self.turn_at_delivery or 0
            ) + int(arg)
        if kind == "wait_game_end":
            return self.interface is None
        raise ValueError(step)

    def deliver(self, step, original):
        kind, _, arg = step.partition(" ")
        self.close_step_sounds()
        self.next_step += 1
        label = original if original == step else "%s (%s)" % (step, original)
        self.emit("step %d %s" % (self.next_step, label))
        self.sound_lines.append("@%d step %d" % (self.clock.ms, self.next_step))
        self.last_delivery_us = self.clock.us
        self.voilas_at_delivery = self.voilas
        self.turn_at_delivery = self.interface.world.turn if self.interface is not None else None
        if kind == "key":
            self.queue.append(key_event(arg))
        elif kind == "interrupt_after":
            self.queue.append(key_event(arg.split(" ", 2)[2]))

    def on_time(self):
        """Called after every sleep: a timed key (interrupt_after) arrives in
        the event queue as soon as it is due, whether the client is reading
        input or not (it may be discarded, after an alert for example)."""
        if self.ended or self.next_step >= len(self.steps):
            return
        step, original = self.steps[self.next_step]
        if step.startswith("interrupt_after") and self.step_ready(step, None):
            self.deliver(step, original)

    def on_input(self, point):
        """Called at every input point, before the client reads the queue."""
        if self.ended:
            return
        while self.next_step < len(self.steps):
            step, original = self.steps[self.next_step]
            if not self.step_ready(step, point):
                return
            self.deliver(step, original)
            if step.startswith(("key", "interrupt_after")):
                return
        if point != "voice" and self.settled(point):
            self.close_step_sounds()
            self.emit("end")
            self.ended = True
            raise ScenarioEnd(0)

    # --- the fake event queue

    def install_events(self):
        import pygame

        self.KEYDOWN = pygame.KEYDOWN
        codes = {}

        def point_of_caller():
            return codes.get(sys._getframe(2).f_code)

        def matches(e, types):
            if types is None:
                return True
            if isinstance(types, int):
                return e.type == types
            return e.type in types

        def poll():
            point = point_of_caller()
            if point == "text":
                # input_string() busy-polls: each iteration is 1 ms.
                self.clock.advance(1000)
            if point:
                self.on_input(point)
            self.watchdog()
            if self.queue:
                return self.queue.pop(0)
            return pygame.event.Event(pygame.NOEVENT)

        def get(eventtype=None, pump=True, exclude=None):
            point = point_of_caller()
            if point:
                self.on_input(point)
            self.watchdog()
            result = [e for e in self.queue if matches(e, eventtype)]
            self.queue = [e for e in self.queue if not matches(e, eventtype)]
            return result

        def clear(eventtype=None, pump=True):
            self.queue = [e for e in self.queue if not matches(e, eventtype)]

        def post(e):
            self.queue.append(e)
            return True

        def peek(eventtype=None, pump=True):
            if eventtype is None:
                return self.queue[0] if self.queue else pygame.event.Event(pygame.NOEVENT)
            return any(matches(e, eventtype) for e in self.queue)

        def pump():
            pass

        def wait(timeout=0):
            raise RuntimeError("pygame.event.wait is not modelled")

        pygame.event.poll = poll
        pygame.event.get = get
        pygame.event.clear = clear
        pygame.event.post = post
        pygame.event.peek = peek
        pygame.event.pump = pump
        pygame.event.wait = wait
        return codes

    def watchdog(self):
        if self.clock.us == getattr(self, "_watch_us", None):
            self.reads_without_time += 1
            if self.reads_without_time > 1_000_000:
                raise ScenarioAbort("input read a million times without time passing")
        else:
            self._watch_us = self.clock.us
            self.reads_without_time = 0


class FakeChannel:
    """A mixer channel on the virtual clock: a sound is busy for exactly its
    length (rounded to the microsecond), times (loops + 1), or forever for
    loops == -1."""

    def __init__(self, driver, n):
        self.driver = driver
        self.n = n
        self.sound = None
        self.end_us = 0
        self.volume = (1.0, 1.0)
        self.endevent = 0

    def length_us(self, sound):
        us = round(sound.get_length() * 1_000_000)
        name = getattr(sound, "name", "?")
        known = self.driver.lengths.setdefault(name, us)
        if known != us:
            self.driver.lengths[name + "@" + str(getattr(sound, "mod_name", "?"))] = us
        return us

    def play(self, sound, loops=0, maxtime=0, fade_ms=0):
        self.sound = sound
        if loops == -1:
            self.end_us = None
        else:
            self.end_us = self.driver.clock.us + self.length_us(sound) * (loops + 1)
        if loops == -1:
            self.length_us(sound)

    def get_busy(self):
        return self.sound is not None and (self.end_us is None or self.driver.clock.us < self.end_us)

    def stop(self):
        if self.n == 0:
            self.driver.voice_stopping()
        self.sound = None

    def fadeout(self, ms):
        if self.get_busy():
            end = self.driver.clock.us + ms * 1000
            self.end_us = end if self.end_us is None else min(self.end_us, end)

    def get_sound(self):
        return self.sound if self.get_busy() else None

    def get_queue(self):
        return None

    def queue(self, sound):
        raise RuntimeError("Channel.queue is not modelled")

    def set_volume(self, left, right=None):
        self.volume = (left, left if right is None else right)

    def get_volume(self):
        return self.volume[0]

    def set_endevent(self, t=0):
        self.endevent = t

    def get_endevent(self):
        return self.endevent

    def pause(self):
        pass

    def unpause(self):
        pass


class FakeTTS:
    """soundrts.lib.tts without threads and without speech. Upstream's rule
    for when speech ends: len(text) * wait_delay_per_character, plus
    nb_digits * (tts_digit_coefficient - 1) * wait_delay_per_character.
    Here each term is rounded to the microsecond."""

    def __init__(self, driver):
        self.driver = driver
        self.end_us = None
        self.delay = 0.1

    def init(self, wait_delay_per_character):
        self.delay = wait_delay_per_character

    def speak(self, text):
        from soundrts import parameters

        assert isinstance(text, str)
        nb_digits = sum(c.isdigit() for c in text)
        digit_bonus = parameters.d.get("tts_digit_coefficient", 1) - 1
        self.end_us = (
            self.driver.clock.us
            + round(len(text) * self.delay * 1_000_000)
            + round(nb_digits * digit_bonus * self.delay * 1_000_000)
        )

    def stop(self):
        self.driver.voice_stopping()
        self.end_us = None

    def is_speaking(self):
        return self.end_us is not None and self.end_us > self.driver.clock.us


def install_mixer(driver):
    import pygame

    channels = {}
    state = {"num": 8}

    def Channel(n):
        if n not in channels:
            channels[n] = FakeChannel(driver, n)
        return channels[n]

    def set_num_channels(n):
        state["num"] = n

    def get_num_channels():
        return state["num"]

    def stop():
        for c in channels.values():
            c.stop()

    def sound_play(self, loops=0, maxtime=0, fade_ms=0):
        for n in range(1, state["num"]):
            c = Channel(n)
            if not c.get_busy():
                c.play(self, loops)
                return c
        return None

    pygame.mixer.Channel = Channel
    pygame.mixer.set_num_channels = set_num_channels
    pygame.mixer.get_num_channels = get_num_channels
    pygame.mixer.set_reserved = lambda n: n
    pygame.mixer.stop = stop
    pygame.mixer.pause = lambda: None
    pygame.mixer.unpause = lambda: None
    pygame.mixer.fadeout = lambda ms: stop()
    pygame.mixer.find_channel = lambda force=False: None
    return sound_play


def id_key(o):
    i = getattr(o, "id", None)
    return int(i) if i is not None else 0


class IdSortedSet(set):
    """A set whose iteration order is ascending world id."""

    def __iter__(self):
        return iter(sorted(set.__iter__(self), key=id_key))


class IdSortedView:
    """A live view of a world set (the world mutates it in place), iterated
    in ascending world id."""

    def __init__(self, s):
        self._s = s

    def __iter__(self):
        return iter(sorted(self._s, key=id_key))

    def __contains__(self, o):
        return o in self._s

    def __len__(self):
        return len(self._s)

    def __bool__(self):
        return bool(self._s)

    def union(self, *others):
        return IdSortedSet(set(self._s).union(*others))

    def copy(self):
        return IdSortedSet(self._s)


def deterministic_hashes(perturb, classes):
    """Hash world objects by a creation counter instead of their address, so
    that the iteration order of the remaining hash-based sets is the same on
    every run. With perturb != 0 the hashes are scrambled differently, to
    find output that depends on such an order."""
    counter = [0]
    mult = 0x9E3779B97F4A7C15

    def _hash(self):
        d = self.__dict__
        h = d.get("_parity_hash")
        if h is None:
            counter[0] += 1
            h = counter[0]
            if perturb:
                h = ((h * mult) ^ (perturb * 0xBF58476D1CE4E5B9)) & 0x0FFFFFFFFFFFFFFF
            d["_parity_hash"] = h
        return h

    for c in classes:
        c.__hash__ = _hash


# ---------------------------------------------------------------------------
# THE PORT'S SCOPE. The Rust port has no server item (hosting is in its
# multiplayer menu), replays, save/restore or documentation item yet, so the
# original's menus are cut down to what the port has (README, "The port's
# scope"). Each patch goes away when its feature is ported.


def restrict_to_port_scope():
    from soundrts import clientgame, clientmain
    from soundrts import msgparts as mp
    from soundrts.clientmenu import CLOSE_MENU, Menu
    from soundrts.lib.resource import res

    def main_menu():
        # without server, observe a recorded game, documentation
        Menu(
            [clientmain.app_title() + ","] + mp.MAKE_A_SELECTION,
            [
                [mp.SINGLE_PLAYER, clientmain.single_player_menu, mp.SINGLE_PLAYER_EXPLANATION],
                [mp.MULTIPLAYER2, clientmain.multiplayer_menu, mp.MULTIPLAYER2_EXPLANATION],
                [mp.OPTIONS, clientmain.options_menu, mp.OPTIONS_EXPLANATION],
                [mp.QUIT2, CLOSE_MENU, mp.QUIT2_EXPLANATION],
            ],
        ).loop()

    def single_player_menu():
        # without restore
        Menu(
            mp.MAKE_A_SELECTION,
            [(c.title, c) for c in res.campaigns()]
            + [
                (mp.START_A_GAME_ON, clientmain.TrainingMenu().run),
                (mp.BACK, CLOSE_MENU),
            ],
        ).loop()

    # set_and_launch_mod/soundpack and main() find these by module lookup
    clientmain.main_menu = main_menu
    clientmain.single_player_menu = single_player_menu
    # the game menu without save
    clientgame.GameInterface.can_save = lambda self: False


def main_child(args):
    name = args[args.index("--child") + 1]
    out_path = args[args.index("--out") + 1]
    sounds_path = args[args.index("--sounds") + 1]
    perturb = int(args[args.index("--perturb") + 1])
    random_offset = int(args[args.index("--random-offset") + 1])
    settings, steps = parse_scenario(os.path.join(SCENARIOS, name + ".txt"))
    root = os.getcwd()
    if not os.path.abspath(root).startswith(os.path.abspath(SCRATCH)):
        raise SystemExit("ABORT: the current directory must be a scratch root")

    blocked = install_guards()

    import time

    driver = Driver(name, settings, steps, perturb)
    real_perf = time.perf_counter
    wall_start = real_perf()
    time.time = driver.clock.time
    time.sleep = driver.clock.sleep
    driver.clock.on_advance = driver.on_time

    # The scratch root's language and configuration.
    with open(os.path.join(root, "cfg", "language.txt"), "w") as f:
        f.write(settings["language"])
    sys.argv = ["soundrts"]  # soundrts/options.py parses the command line
    sys.path.insert(0, UPSTREAM)

    import pygame

    pygame.mixer.pre_init(44100, -16, 2, 1024)  # as soundrts/lib/sound.py
    pygame.init()
    pygame.display.init()
    assert_dummy_drivers()

    # Reinitialising with another driver is refused from now on.
    _pg_init = pygame.init
    _display_init = pygame.display.init

    def _forced(f):
        def g(*a, **k):
            os.environ["SDL_VIDEODRIVER"] = "dummy"
            os.environ["SDL_AUDIODRIVER"] = "dummy"
            r = f(*a, **k)
            assert_dummy_drivers()
            return r

        return g

    pygame.init = _forced(_pg_init)
    pygame.display.init = _forced(_display_init)
    sound_play = install_mixer(driver)
    codes = driver.install_events()

    titles = []

    def set_caption(title, icontitle=None):
        if title != driver.title:
            driver.title = title
            driver.emit("title %s" % title)

    pygame.display.set_caption = set_caption

    import soundrts.lib

    fake_tts = FakeTTS(driver)
    driver.fake_tts = fake_tts
    import types

    tts_module = types.ModuleType("soundrts.lib.tts")
    tts_module.init = fake_tts.init
    tts_module.speak = fake_tts.speak
    tts_module.stop = fake_tts.stop
    tts_module.is_speaking = fake_tts.is_speaking
    sys.modules["soundrts.lib.tts"] = tts_module
    soundrts.lib.tts = tts_module

    # Directory listings and the set of mod names come in a sorted order
    # (upstream: os.walk order, and a set of strings, whose order changes
    # with the hash seed; the mods menu lists them in that order).
    from soundrts.lib import package

    for cls in (package.FolderPackage, package.ZipPackage):
        for m in ("dirnames", "filenames"):
            f = getattr(cls, m)
            setattr(cls, m, lambda self, f=f: sorted(f(self)))

    def mods(self):
        mod_names = set()
        for p in reversed(self):
            sub = p.subpackage("mods")
            if sub is not None:
                mod_names.update(sub.dirnames())
        return [self.mod(name) for name in sorted(mod_names)]

    package.PackageStack.mods = mods

    from soundrts import config

    config.load()
    config.login = settings["login"]
    config.mods = settings["mods"]
    config.soundpacks = settings["soundpacks"]
    config.save()

    import webbrowser

    def _no_browser(*a, **k):
        blocked.append("webbrowser")
        raise RuntimeError("webbrowser is blocked by the parity driver")

    webbrowser.open = _no_browser

    from soundrts import clientmain

    os.environ["SDL_VIDEODRIVER"] = "dummy"  # clientmedia sets "windib"
    from soundrts import clientgame, clientmenu, game, world, worldentity, worldplayerbase, worldroom
    from soundrts.clientversion import revision_checker
    from soundrts.lib import sound as soundlib
    from soundrts.lib.message import Message
    from soundrts.lib.sound import DEFAULT_VOLUME
    from soundrts.lib.voice import _Voice, voice
    from soundrts.lib.voicechannel import VoiceChannel

    revision_checker.start_if_needed = lambda: None  # update check and stats
    restrict_to_port_scope()
    from soundrts.lib import sound_cache

    sound_cache.Sound.play = sound_play  # only used by the map editor

    deterministic_hashes(
        perturb, [worldentity.Entity, worldroom._Space, worldplayerbase.Player]
    )

    # Swallowed exceptions are recorded: they change what is said.
    import logging
    import re

    class _ErrorRecorder(logging.Handler):
        def emit(self, record):
            if record.levelno >= logging.ERROR:
                msg = record.getMessage()
                if record.exc_info and record.exc_info[1] is not None:
                    e = record.exc_info[1]
                    msg += " (%s: %s)" % (type(e).__name__, e)
                msg = re.sub(r"0x[0-9a-fA-F]+", "0x?", msg).replace("\n", " ")
                driver.emit("exception %s" % msg)

    logging.getLogger().addHandler(_ErrorRecorder())

    # The world runs synchronously.
    def queue_command(self, player, order):
        try:
            if player is None:
                order()
            else:
                player.execute_command(order)
        except SystemExit:
            raise
        except:
            from soundrts.lib.log import exception

            exception("")

    def loop(self):
        pass

    world.World.queue_command = queue_command
    world.World.loop = loop
    NO_OP_THREAD_TARGETS.add(loop)

    # A fixed seed for the world; the global random module (client-side
    # noises only) is seeded too.
    import random

    seed = int(settings["seed"])
    for cls in (game.TrainingGame, game.MissionGame):
        _init = cls.__init__

        def _seeded(self, *a, _init=_init, **k):
            _init(self, *a, **k)
            self.seed = seed

        cls.__init__ = _seeded

    # Input points.
    codes[clientmenu.Menu._get_choice_from_static_menu.__code__] = "menu"
    codes[clientmenu.Menu.step.__code__] = "menu"
    codes[clientmenu.input_string.__code__] = "text"
    codes[clientgame.GameInterface._process_events.__code__] = "game"
    codes[_Voice._key_hit.__code__] = "voice"

    # The interface: track it, count world updates, order its sets by id.
    # Diagnostic only: PARITY_NO_NORMALIZE=1 keeps upstream's set orders
    # (with --perturb, shows that the normalisation is needed).
    normalize = not os.environ.get("PARITY_NO_NORMALIZE")
    sorted_set = IdSortedSet if normalize else set
    _gi_init = clientgame.GameInterface.__init__

    def gi_init(self, *a, **k):
        _gi_init(self, *a, **k)
        self.scout_info = sorted_set()
        driver.interface = self

    clientgame.GameInterface.__init__ = gi_init

    _srv_voila = clientgame.GameInterface.srv_voila

    def srv_voila(self, t, memory, perception, scouted, scouted_before, collision_debug):
        def view(s):
            return IdSortedView(s) if normalize and isinstance(s, (set, frozenset)) else s

        if not normalize:
            _srv_voila(self, t, memory, perception, scouted, scouted_before, collision_debug)
            driver.voilas += 1
            return
        _srv_voila(
            self,
            t,
            IdSortedSet(memory),
            IdSortedSet(perception),
            view(scouted),
            view(scouted_before),
            collision_debug,
        )
        driver.voilas += 1

    clientgame.GameInterface.srv_voila = srv_voila

    _scout_info_if_needed = clientgame.GameInterface.scout_info_if_needed

    def scout_info_if_needed(self):
        _scout_info_if_needed(self)
        if type(self.scout_info) is set:
            self.scout_info = sorted_set(self.scout_info)

    clientgame.GameInterface.scout_info_if_needed = scout_info_if_needed

    from soundrts import msgparts as mp

    def units_alert(self, units, msg_end, brief=True):
        # upstream iterates a set of squares; here: ascending square id
        places = sorted_set(x[1] for x in units if x[1] is not None)
        for place in places:
            units_in_place = [x[0] for x in units if x[1] is place]
            s = self.summary(units_in_place, brief=brief)
            if s:
                voice.info(s + msg_end + mp.AT + place.title)
        while units:
            units.pop()

    clientgame.GameInterface.units_alert = units_alert

    _run_game = clientgame.GameInterface.run_game

    def run_game(self, *a, **k):
        try:
            return _run_game(self, *a, **k)
        finally:
            if driver.interface is self:
                driver.interface = None

    clientgame.GameInterface.run_game = run_game

    # Recording the voice.
    kind = ["?"]

    def tag(method, label):
        f = getattr(_Voice, method)

        def g(self, *a, **k):
            previous = kind[0]
            kind[0] = label
            try:
                return f(self, *a, **k)
            finally:
                kind[0] = previous

        setattr(_Voice, method, g)

    for m in ("item", "alert", "important", "confirmation", "menu"):
        tag(m, m)

    _update = _Voice.update

    def update(self):
        # a message from the queue (info) or a replay from the history
        previous = kind[0]
        kind[0] = "history" if self.history else "info"
        try:
            return _update(self)
        finally:
            kind[0] = previous

    _Voice.update = update

    _info = _Voice.info

    def info(self, *a, **k):
        previous = kind[0]
        kind[0] = "info"
        try:
            return _info(self, *a, **k)
        finally:
            kind[0] = previous

    _Voice.info = info

    _play = VoiceChannel.play

    def play(self, msg):
        parts = []
        for p in msg.translate_and_collapse():
            if isinstance(p, str):
                parts.append(json_str(p))
            else:
                parts.append("[%s]" % getattr(p, "name", "?"))
        line = "voice %s %s" % (kind[0], " ".join(parts))
        if (msg.lv, msg.rv) != (DEFAULT_VOLUME, DEFAULT_VOLUME):
            line += " vol=%s,%s" % (fmt(msg.lv), fmt(msg.rv))
        driver.emit(line)
        driver.in_voice_play = True
        try:
            return _play(self, msg)
        finally:
            driver.in_voice_play = False

    VoiceChannel.play = play

    # Recording the positional and stereo sounds.
    psounds = soundlib.psounds
    SM = type(psounds)

    def sname(s):
        return getattr(s, "name", None) if s is not None else None

    _pplay = SM.play

    def pplay(self, s, v, x, y, priority, limit=0, ambient=False):
        r = _pplay(self, s, v, x, y, priority, limit, ambient)
        if s is not None:
            started = r is not None
            gains = ""
            if started:
                gains = " gains=%s,%s" % tuple(fmt(g) for g in r.previous_vol)
            driver.emit_sound(
                "pos %s v=%s x=%s y=%s prio=%s limit=%s%s %s%s"
                % (
                    sname(s),
                    fmt(v),
                    fmt(x),
                    fmt(y),
                    priority,
                    fmt(limit),
                    " ambient" if ambient else "",
                    "started" if started else "skipped",
                    gains,
                )
            )
        return r

    SM.play = pplay

    _ploop = SM.play_loop

    def ploop(self, s, v, x, y, priority, limit=0, ambient=False):
        r = _ploop(self, s, v, x, y, priority, limit, ambient)
        if s is not None:
            started = r is not None and r.is_playing()
            gains = ""
            if started:
                gains = " gains=%s,%s" % tuple(fmt(g) for g in r.previous_vol)
            driver.emit_sound(
                "loop %s v=%s x=%s y=%s prio=%s %s%s"
                % (sname(s), fmt(v), fmt(x), fmt(y), priority, "started" if started else "pending", gains)
            )
        return r

    SM.play_loop = ploop

    _pstereo = SM.play_stereo

    def pstereo(self, s, vol=1, limit=0):
        if s is None:
            return _pstereo(self, s, vol, limit)
        before = self._start_time.get(s)
        r = _pstereo(self, s, vol, limit)
        played = self._start_time.get(s) != before
        if isinstance(vol, tuple):
            v = "%s,%s" % (fmt(vol[0]), fmt(vol[1]))
        else:
            v = fmt(vol)
        driver.emit_sound(
            "stereo %s vol=%s limit=%s %s" % (sname(s), v, fmt(limit), "started" if played else "skipped")
        )
        return r

    SM.play_stereo = pstereo

    random.seed(seed + random_offset)
    status = 0
    try:
        clientmain.main()
        if driver.next_step < len(driver.steps):
            driver.emit("abort the client quit before the end of the script")
            status = 2
        elif not driver.ended:
            driver.close_step_sounds()
            driver.emit("end")
    except ScenarioEnd:
        pass
    except ScenarioAbort as e:
        driver.close_step_sounds()
        driver.emit("abort %s" % e)
        status = 2
    except SystemExit:
        # the client exits by raising SystemExit (after choosing a mod or
        # a soundpack, for example)
        driver.close_step_sounds()
        if driver.next_step < len(driver.steps):
            driver.emit("abort the client exited before the end of the script")
            status = 2
        else:
            driver.emit("exit")
    finally:
        try:
            pygame.quit()
        except Exception:
            pass

    header = [
        "soundrts-ui-transcript 1",
        "scenario %s" % name,
        "settings %s"
        % " ".join("%s=%s" % (k, settings[k]) for k in SETTINGS),
    ]
    for k in sorted(driver.lengths, key=lambda s: (len(s), s)):
        header.append("length %s %d" % (k, driver.lengths[k]))
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(header + driver.voice_lines) + "\n")
    with open(sounds_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(driver.sound_lines) + "\n")
    print(
        "scenario %s: %d steps, virtual %.3f s, wall %.1f s, blocked: %s"
        % (name, driver.next_step, driver.clock.us / 1e6, real_perf() - wall_start, blocked or "nothing")
    )
    if blocked:
        status = 3
    sys.exit(status)


def json_str(s):
    import json

    return json.dumps(s, ensure_ascii=False)


if __name__ == "__main__":
    if "--child" in sys.argv:
        main_child(sys.argv[1:])
    else:
        main_parent(sys.argv[1:])
