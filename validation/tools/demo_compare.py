#!/usr/bin/env python3
"""Block distance between amsrust and CPCSyntaxError screenshots of a demo.

  demo_compare.py <ours_dir> <cpcse_dir>

Both directories hold the shots of the same CSL script (validation/demos/*/):
ours `*.png`, cpcse `*.bmp`, matched by sorted order (= same emulated time).
Animated demos differ by timing; a static scene that differs is a real bug.
"""
import glob
import sys

import numpy as np
from PIL import Image


def blk(a, h, w):
    a = a[: h // 4 * 4, : w // 8 * 8]
    return a.reshape(h // 4, 4, w // 8, 8, 3).mean((1, 3))


ours = sorted(glob.glob(sys.argv[1] + "/*.png"))
cse = sorted(glob.glob(sys.argv[2] + "/*.bmp"))
res = []
for o, c in zip(ours, cse):
    a = np.array(Image.open(c).convert("RGB")).astype(float)
    if a.shape[0] > 300:
        a = a[0::2]
    b = np.array(Image.open(o).convert("RGB")).astype(float)
    h, w = min(len(a), len(b)), min(a.shape[1], b.shape[1])
    res.append(round((np.abs(blk(a[:h], h, w) - blk(b[:h], h, w)).sum(2) > 90).mean() * 100))
print(len(ours), len(cse), res)
