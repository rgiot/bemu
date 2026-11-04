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
import pathlib
import re
import sys

import numpy as np
from PIL import Image

from _work import REFS


def blocks(x, h, w):
    x = x[: h // 4 * 4, : w // 8 * 8]
    return x.reshape(h // 4, 4, w // 8, 8, 3).mean((1, 3))


def scores(results, crtc="0", refs=None):
    """The metric of each test of a run: {test: (fraction of blocks that differ, capture name)},
    the best capture of each SSM code."""
    best = {}
    # REFS_DIR: another set of references, e.g. ams_shaker_refs.py's (CRTC 1-4)
    if refs is None:
        refs = pathlib.Path(os.environ["REFS_DIR"]) if "REFS_DIR" in os.environ else REFS
    for f in sorted(glob.glob(str(results) + "/**/*_CRTC" + crtc + "_*.png", recursive=True)):
        n = os.path.basename(f)[:-4]
        base = re.sub(r"_\d+$", "", n)
        r = next((p for p in (refs / f"{base}.webp", refs / f"{base}.png") if p.exists()), None)
        if r is None:
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
    return best


def main():
    results = sys.argv[1]
    top = int(sys.argv[2]) if len(sys.argv) > 2 else 25
    best = scores(results, os.environ.get("CRTC", "0"))
    rows = sorted(best.values(), reverse=True)
    for m, n in rows[:top]:
        print(f"{m * 100:5.1f} {n}")
    print("mean", np.mean([r[0] for r in rows]) * 100, len(rows))


if __name__ == "__main__":
    main()
