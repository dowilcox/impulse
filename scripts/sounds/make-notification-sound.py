#!/usr/bin/env python3
"""Synthesize Impulse's notification sounds into assets/sounds/.

ack.wav       two quick chirps that flick upward, then a short settled blip
              a fourth below the second (the default).
two-tone.wav  a chirp sliding up, answered by one sliding down to rest.
chime.wav     a soft two-note chime rising a fifth (E5, then B5 130 ms
              later), each note a sine with a few quiet overtones that die
              away faster than it does, so it starts round and ends pure.

The chirps are console tones (a sine with a little third harmonic) whose
pitch slides over the first 60 % of each, with 3 ms attacks; the last one
rings down instead of stopping short. Every sound fades in without a click
and tapers to silence, peaks at -3 dBFS at most, and is no louder than the
chime (its loudest 400 ms: -12.3 dBFS RMS). 44.1 kHz, 16-bit, mono.

Standard library only, and deterministic: running it again writes the same
bytes. The app embeds the files (impulse-app/src/notification_sound.rs), so
rebuild after changing them.

    python3 scripts/sounds/make-notification-sound.py [output folder]
"""

import math
import struct
import sys
import wave
from pathlib import Path

RATE = 44_100
DURATION = 0.80  # seconds, the chime's
PEAK_DBFS = -3.0
LOUDEST_RMS_DBFS = -12.3  # the chime's loudest 400 ms
ATTACK = 0.008  # seconds, raised-cosine fade in of each note
TAIL = 0.120  # seconds, raised-cosine fade out at the end

# (frequency Hz, start s, level, decay time constant s)
NOTES = [
    (659.255, 0.000, 1.00, 0.16),  # E5
    (987.767, 0.130, 0.85, 0.24),  # B5
]

# Overtones of each note: (frequency ratio, level, share of the note's decay).
# The slightly stretched fourth partial gives a hint of a struck bar; all of
# them fade faster than the fundamental.
PARTIALS = [
    (1.000, 1.000, 1.00),
    (2.000, 0.220, 0.50),
    (3.000, 0.060, 0.33),
    (4.070, 0.025, 0.22),
]


def sample(t: float) -> float:
    value = 0.0
    for freq, start, level, decay in NOTES:
        local = t - start
        if local < 0:
            continue
        attack = 1.0 if local >= ATTACK else math.sin(0.5 * math.pi * local / ATTACK) ** 2
        for ratio, partial_level, share in PARTIALS:
            envelope = math.exp(-local / (decay * share))
            value += level * partial_level * attack * envelope * math.sin(
                2 * math.pi * freq * ratio * local
            )
    remaining = DURATION - t
    if remaining < TAIL:
        value *= math.sin(0.5 * math.pi * max(remaining, 0.0) / TAIL) ** 2
    return value


def raised(x: float) -> float:
    """0 -> 1 over x in [0, 1], smoothly."""
    x = min(max(x, 0.0), 1.0)
    return math.sin(0.5 * math.pi * x) ** 2


def render() -> list:
    count = round(RATE * DURATION)
    samples = [sample(i / RATE) for i in range(count)]
    peak = max(abs(s) for s in samples)
    gain = 10 ** (PEAK_DBFS / 20) / peak
    return [round(s * gain * 32767) for s in samples]


def chirps(events: list, duration: float, ring: float) -> list:
    """`events`: (start s, length s, from Hz, to Hz, level). Each chirp slides
    from one pitch to the other over its first 60 %, then holds; the last
    one rings down for `ring` seconds instead of stopping short."""
    phases = [0.0] * len(events)
    out = []
    for i in range(round(RATE * duration)):
        t = i / RATE
        value = 0.0
        for n, (start, length, f0, f1, level) in enumerate(events):
            local = t - start
            if local < 0:
                continue
            last = n == len(events) - 1
            if local < length:
                envelope = raised(local / 0.003)
            else:
                envelope = math.exp(-(local - length) / (ring if last else 0.012))
                if envelope < 1e-4:
                    continue
            freq = f0 * (f1 / f0) ** raised(local / (0.6 * length))
            phases[n] += freq / RATE
            edge = 0.08 if last else 0.15
            tone = math.sin(2 * math.pi * phases[n]) + edge * math.sin(6 * math.pi * phases[n])
            value += level * envelope * tone
        out.append(value)
    return finish(out)


def finish(samples: list) -> list:
    """Taper the end to silence, take out any DC, and set the level: a -3
    dBFS peak, unless that would be louder than the chime."""
    count = len(samples)
    tail = round(0.1 * RATE)
    for i in range(tail):
        samples[count - 1 - i] *= raised(i / tail)
    samples[0] = 0.0
    mean = sum(samples) / count
    samples = [s - mean * raised(min(i, count - 1 - i) / 2000) for i, s in enumerate(samples)]
    gain = 10 ** (PEAK_DBFS / 20) / max(abs(s) for s in samples)
    window = round(0.4 * RATE)
    loudest = max(
        math.sqrt(sum(v * v for v in samples[i : i + window]) / window)
        for i in range(0, max(count - window, 1), 441)
    )
    gain = min(gain, 10 ** (LOUDEST_RMS_DBFS / 20) / loudest)
    return [round(s * gain * 32767) for s in samples]


SOUNDS = {
    "ack": lambda: chirps(
        [
            (0.010, 0.045, 1250.0, 1660.0, 1.0),
            (0.090, 0.045, 1660.0, 2220.0, 1.0),
            (0.170, 0.070, 1660.0, 1660.0, 0.9),
        ],
        0.55,
        ring=0.12,
    ),
    "two-tone": lambda: chirps(
        [
            (0.010, 0.075, 1100.0, 1760.0, 1.0),
            (0.120, 0.090, 1980.0, 1320.0, 1.0),
        ],
        0.5,
        ring=0.09,
    ),
    "chime": render,
}


def write(path: Path, frames: list) -> None:
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(struct.pack(f"<{len(frames)}h", *frames))
    print(f"{path}: {len(frames)} frames, {len(frames) / RATE:.3f} s")


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "assets/sounds"
    folder.mkdir(parents=True, exist_ok=True)
    for name, make in SOUNDS.items():
        write(folder / f"{name}.wav", make())


if __name__ == "__main__":
    main()
