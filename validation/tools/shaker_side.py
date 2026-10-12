#!/usr/bin/env python3
"""Side by side picture (ours | reference) written to side.png.

  shaker_side.py NAME:path/to/ours.png [NAME:ours2.png ...]

NAME is the reference's name (e.g. AO_CRTC0_H). Look at the result with any
image viewer.
"""
import sys

from PIL import Image

from _work import REFS

ims = []
for arg in sys.argv[1:]:
    name, path = arg.split(":", 1)
    a = Image.open(path).convert("RGB")
    b = Image.open(REFS / f"{name}.webp").convert("RGB")
    b = b.resize((b.width, b.height // 2))
    ims += [a, b]
W = ims[0].width * 2 + 10
H = sum(max(ims[i].height, ims[i + 1].height) + 6 for i in range(0, len(ims), 2))
o = Image.new("RGB", (W, H), (128, 0, 128))
y = 0
for i in range(0, len(ims), 2):
    o.paste(ims[i], (0, y))
    o.paste(ims[i + 1], (ims[0].width + 10, y))
    y += ims[i].height + 6
o.save("side.png")
print("side.png")
