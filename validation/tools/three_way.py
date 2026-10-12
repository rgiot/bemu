#!/usr/bin/env python3
"""Ours vs CPCSyntaxError vs the AMSpiriT references, per test.

  three_way.py <our_results_dir> <cpcse_shots_dir> [N]

cpcse_shots_dir holds `CPCSE_0_<hex>.bmp` (the oracle named by SSM code: run
`cpcse --csl <script> --out <dir> --disk-dir <dir with the dsk> --crtc 0
--emu-name CPCSE`). Prints the N worst ours-vs-cpcse tests with the other two
distances (block metric, as in shaker_compare.py) and the means.
"""
import glob
import os
import sys

import numpy as np
from PIL import Image

from _work import REFS, TESTS
import json


def blk(a, h, w):
    a = a[: h // 4 * 4, : w // 8 * 8]
    return a.reshape(h // 4, 4, w // 8, 8, 3).mean((1, 3))


def load(p, half):
    a = np.array(Image.open(p).convert("RGB")).astype(float)
    return a[0::2] if half else a


def main():
    ours_dir, cdir = sys.argv[1], sys.argv[2]
    top = int(sys.argv[3]) if len(sys.argv) > 3 else 30
    hexmap = {}
    for x in json.loads(TESTS.read_text()):
        for s in x["subtests"]:
            if 0 in s["crtcs"]:
                hexmap[f"{x['id']}_CRTC0_{s['subTest']}"] = s["hex"].upper().zfill(4)
    rows = []
    for name, hx in hexmap.items():
        cp, rf = f"{cdir}/CPCSE_0_{hx}.bmp", REFS / f"{name}.webp"
        cands = glob.glob(f"{ours_dir}/*/{name}.png") + glob.glob(f"{ours_dir}/*/{name}_*.png") + glob.glob(f"{ours_dir}/{name}.png")
        if not (os.path.exists(cp) and rf.exists() and cands):
            continue
        try:
            c, r = load(cp, True), load(rf, True)
        except Exception:
            continue
        best = None
        for f in cands:
            o = load(f, False)
            h, w = min(len(o), len(c), len(r)), min(o.shape[1], c.shape[1], r.shape[1])
            d = lambda x, y: (np.abs(blk(x[:h], h, w) - blk(y[:h], h, w)).sum(2) > 90).mean() * 100
            t = (d(o, c), d(o, r), d(c, r))
            if best is None or t[1] < best[1]:
                best = t
        rows.append((best[0], name, best[1], best[2]))
    rows.sort(reverse=True)
    print("ours-vs-cpcse  test  ours-vs-ams  cpcse-vs-ams")
    for r in rows[:top]:
        print("%5.1f %-22s %5.1f %5.1f" % (r[0], r[1], r[2], r[3]))
    print("mean ours-cpcse", np.mean([r[0] for r in rows]), "ours-ams", np.mean([r[2] for r in rows]),
          "cpcse-ams", np.mean([r[3] for r in rows]), len(rows))


if __name__ == "__main__":
    main()
