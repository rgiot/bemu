#!/usr/bin/env python3
"""Block-mismatch metric of a Shaker run against the AMSpiriT references.

  shaker_compare.py <results_dir> [N]

Prints the worst N tests (default 25) and the mean. The metric compares 4x8
pixel blocks (a block differs when its mean colour moves by more than 90 on the
summed channels): a pixel metric is dominated by the dithering/blur of the
references' webp. The references are 542 rows (each CPC line doubled), so every
second row is compared. A `_N` suffix on a result name is a second capture of
the same SSM code: the best match counts.
"""
import glob
import os
import re
import sys

import numpy as np
from PIL import Image

from _work import REFS


def blocks(x, h, w):
    x = x[: h // 4 * 4, : w // 8 * 8]
    return x.reshape(h // 4, 4, w // 8, 8, 3).mean((1, 3))


def main():
    results = sys.argv[1]
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    best = {}
    for f in sorted(glob.glob(results + "/**/*_CRTC" + os.environ.get("CRTC", "0") + "_*.png", recursive=True)):
        n = os.path.basename(f)[:-4]
        base = re.sub(r"_\d+$", "", n)
        r = REFS / f"{base}.webp"
        if not r.exists():
            continue
        try:
            Image.open(r).load()
        except Exception:
            continue
        a = np.array(Image.open(f).convert("RGB")).astype(int)
        b = np.array(Image.open(r).convert("RGB")).astype(int)[0::2]
        h, w = min(len(a), len(b)), min(a.shape[1], b.shape[1])
        m = (np.abs(blocks(a, h, w) - blocks(b, h, w)).sum(2) > 90).mean()
        if base not in best or m < best[base][0]:
            best[base] = (m, n)
    rows = sorted(best.values(), reverse=True)
    for m, n in rows[:top]:
        print(f"{m * 100:5.1f} {n}")
    print("mean", np.mean([r[0] for r in rows]) * 100, len(rows))


if __name__ == "__main__":
    main()
