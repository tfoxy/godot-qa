"""Write the tiny, deterministic source images and PCM16 tone used by asset trials."""

from math import pi, sin
from pathlib import Path
import struct
import zlib


ROOT = Path(__file__).resolve().parents[2]
ASSET_DIR = ROOT / "assets" / "asset_trial"


def write_png(path: Path, candidate: bool) -> None:
    width = height = 64
    rows = []
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            dx, dy = x - 31.5, y - 31.5
            radius = (dx * dx + dy * dy) ** 0.5
            inside = radius <= (26 if candidate else 24)
            ring = radius >= (18 if candidate else 17)
            if not inside:
                rgba = (0, 0, 0, 0)
            elif candidate and ring:
                rgba = (255, 244, 112, 255)
            elif candidate:
                rgba = (242, 153, 35, 255)
            elif ring:
                rgba = (244, 224, 102, 255)
            else:
                rgba = (210, 135, 28, 255)
            row.extend(rgba)
        rows.append(bytes(row))

    def chunk(kind: bytes, payload: bytes) -> bytes:
        body = kind + payload
        return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    raw = b"".join(rows)
    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(raw, level=9))
    data += chunk(b"IEND", b"")
    path.write_bytes(data)


def write_wav(path: Path) -> None:
    rate, seconds = 44100, 0.8
    samples = []
    for index in range(round(rate * seconds)):
        time = index / rate
        envelope = min(1.0, time / 0.015) * max(0.0, 1.0 - time / seconds)
        sample = int(0.58 * envelope * sin(2.0 * pi * 660.0 * time) * 32767)
        samples.append(sample)
    pcm = struct.pack("<" + "h" * len(samples), *samples)
    path.write_bytes(b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVE")
    with path.open("ab") as stream:
        stream.write(b"fmt " + struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16))
        stream.write(b"data" + struct.pack("<I", len(pcm)) + pcm)


ASSET_DIR.mkdir(parents=True, exist_ok=True)
write_png(ASSET_DIR / "coin_original.png", candidate=False)
write_png(ASSET_DIR / "coin_trial.png", candidate=True)
write_wav(ASSET_DIR / "pickup_trial.wav")
