"""Measures how bright each sound file is, for picking positional sounds.

Headphone rendering places a sound in front or behind mostly with cues above
4 to 5 kHz, so a positional sound needs real energy up there. For each file
this prints the sample rate, the channels, the length, the share of energy
above 4 kHz, and the frequency below which 99 percent of the energy lies.

Usage: python -I scripts/measure-brightness.py FILE [FILE ...]
Needs ffmpeg on the path and numpy. Reads files only; plays nothing.
"""

import json
import subprocess
import sys

import numpy as np


def probe(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0",
         "-show_entries", "stream=sample_rate,channels:format=duration",
         "-of", "json", path],
        capture_output=True, check=True, text=True).stdout
    info = json.loads(out)
    stream = info["streams"][0]
    return int(stream["sample_rate"]), int(stream["channels"]), float(info["format"]["duration"])


def samples(path, rate):
    # Mixed down to one channel at the file's own rate, as 32-bit floats.
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(rate),
         "-f", "f32le", "-"],
        capture_output=True, check=True).stdout
    return np.frombuffer(raw, dtype="<f4").astype(np.float64)


def spectrum(signal, rate, size=4096):
    # Power summed over Hann-windowed frames, so long files cost little memory.
    if len(signal) < size:
        signal = np.pad(signal, (0, size - len(signal)))
    window = np.hanning(size)
    power = np.zeros(size // 2 + 1)
    for start in range(0, len(signal) - size + 1, size // 2):
        frame = signal[start:start + size]
        # Without the frame's offset, so a drifting DC level does not leak
        # into the low bins through the window.
        power += np.abs(np.fft.rfft((frame - frame.mean()) * window)) ** 2
    return np.fft.rfftfreq(size, 1 / rate), power


def measure(path):
    rate, channels, seconds = probe(path)
    freqs, power = spectrum(samples(path, rate), rate)
    # Below 20 Hz is DC offset and subsonic rumble, not sound anyone hears.
    power = np.where(freqs < 20, 0.0, power)
    total = power.sum()
    if total == 0:
        return rate, channels, seconds, 0.0, 0.0
    above = power[freqs >= 4000].sum() / total
    cumulative = np.cumsum(power) / total
    edge = freqs[np.searchsorted(cumulative, 0.99)]
    return rate, channels, seconds, above, edge


def main(paths):
    for path in paths:
        rate, channels, seconds, above, edge = measure(path)
        print(f"{path}\t{rate} Hz\t{channels} ch\t{seconds:.2f} s\t"
              f"above 4 kHz {above * 100:.1f}%\t99% below {edge / 1000:.1f} kHz")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
