#!/usr/bin/env python3
"""Synthesize Impulse's notification sound: assets/sounds/notification.wav.

A soft two-note chime rising a fifth (E5, then B5 130 ms later), each note a
sine with a few quiet overtones that die away faster than it does, so it
starts round and ends pure. Each note fades in over 8 ms (no click) and
decays exponentially; the last 120 ms taper to silence. The peak is
normalized to -3 dBFS. 44.1 kHz, 16-bit, mono, 0.8 s.

Standard library only, and deterministic: running it again writes the same
bytes. The app embeds the file (impulse-app/src/notification_sound.rs), so
rebuild after changing it.

    python3 scripts/sounds/make-notification-sound.py [output.wav]
"""

import math
import struct
import sys
import wave
from pathlib import Path

RATE = 44_100
DURATION = 0.80  # seconds
PEAK_DBFS = -3.0
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


def render() -> list:
    count = round(RATE * DURATION)
    samples = [sample(i / RATE) for i in range(count)]
    peak = max(abs(s) for s in samples)
    gain = 10 ** (PEAK_DBFS / 20) / peak
    return [round(s * gain * 32767) for s in samples]


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "assets/sounds/notification.wav"
    frames = render()
    out.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(out), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(struct.pack(f"<{len(frames)}h", *frames))
    print(f"{out}: {len(frames)} frames, {len(frames) / RATE:.3f} s")


if __name__ == "__main__":
    main()
