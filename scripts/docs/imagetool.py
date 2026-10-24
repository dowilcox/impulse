#!/usr/bin/env python3
"""imagetool.swift for systems without Core Graphics (Linux): composes a
documentation screenshot from the headless snapshot's windows with Pillow.
Run by capture.py:

    imagetool.py spec.json

The spec is imagetool.swift's: a base window with overlays (rounded,
shadowed), an optional crop, then rounded corners, a hairline edge and a
shadow on a transparent margin; or finished images set side by side.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter


def rounded_mask(size: tuple[int, int], radius: float) -> Image.Image:
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius=max(0, radius), fill=255)
    return mask


def draw(canvas: Image.Image, image: Image.Image, x: float, y: float, radius: float, shadow: bool,
         scale: float) -> None:
    """`image` at (x, y) from the top-left, optionally rounded and shadowed
    (the shadow follows the image's own alpha)."""
    image = image.convert("RGBA")
    mask = ImageChops.multiply(image.getchannel("A"), rounded_mask(image.size, radius))
    position = (round(x), round(y))
    if shadow:
        blur = 44 * scale
        offset = round(14 * scale)
        layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
        alpha = Image.new("L", canvas.size, 0)
        alpha.paste(mask.point(lambda a: a * 0.45), (position[0], position[1] + offset))
        # Core Graphics' blur is about twice a Gaussian's radius.
        alpha = alpha.filter(ImageFilter.GaussianBlur(blur / 2))
        layer.putalpha(alpha)
        canvas.alpha_composite(layer)
    piece = Image.new("RGBA", image.size, (0, 0, 0, 0))
    piece.paste(image, (0, 0), mask)
    canvas.alpha_composite(piece, position)


def save(image: Image.Image, out: str, scale: float) -> None:
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    # 144 dpi so viewers that honor it show the image at point size.
    image.save(out, dpi=(72 * scale, 72 * scale))


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: imagetool.py spec.json")
    spec = json.loads(Path(sys.argv[1]).read_text())
    s = spec["scale"]

    if "beside" in spec:
        images = [Image.open(p).convert("RGBA") for p in spec["beside"]]
        gap = round(spec.get("gap", 24) * s)
        width = sum(i.width for i in images) + gap * (len(images) - 1)
        height = max((i.height for i in images), default=0)
        canvas = Image.new("RGBA", (width, height), (0, 0, 0, 0))
        x = 0
        for image in images:
            canvas.alpha_composite(image, (x, height - image.height))
            x += image.width + gap
        save(canvas, spec["out"], s)
        return

    # 1. The base window with overlays composited at their positions.
    base = Image.open(spec["base"]).convert("RGBA")
    full = base.copy()
    for overlay in spec.get("overlays", []):
        draw(full, Image.open(overlay["path"]), overlay["x"] * s, overlay["y"] * s,
             overlay.get("radius", 0) * s, overlay.get("shadow", False), s)
    image = full

    # 2. Crop.
    if spec.get("crop"):
        c = spec["crop"]
        box = (round(c["x"] * s), round(c["y"] * s), round((c["x"] + c["w"]) * s), round((c["y"] + c["h"]) * s))
        image = image.crop(box)

    # 3. Rounded corners, hairline edge and shadow, on a transparent margin.
    radius = spec.get("radius", 0) * s
    shadow = spec.get("shadow", False)
    margin = round(56 * s) if shadow else 0
    out = Image.new("RGBA", (image.width + 2 * margin, image.height + 2 * margin), (0, 0, 0, 0))
    draw(out, image, margin, margin, radius, shadow, s)
    if spec.get("border", True):
        edge = Image.new("RGBA", out.size, (0, 0, 0, 0))
        inset = 0.5 * s
        ImageDraw.Draw(edge).rounded_rectangle(
            (margin + inset, margin + inset, margin + image.width - inset, margin + image.height - inset),
            radius=max(0, radius - inset), outline=(255, 255, 255, round(0.12 * 255)), width=max(1, round(s)))
        out.alpha_composite(edge)
    save(out, spec["out"], s)


if __name__ == "__main__":
    main()
