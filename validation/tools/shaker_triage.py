#!/usr/bin/env python3
"""bemu against AMSpiriT Lite and CPCSE on the Shaker suite, any CRTC type.

  shaker_triage.py <bemu_results_dir> <cpcse_shots_dir> <crtc> [N]

bemu_results_dir: run_shaker_suite's output for one module (<test>_CRTC<n>_<sub>.png);
cpcse_shots_dir: `cpcse --csl ... --emu-name CPCSE` shots of the same script
(CPCSE_<n>_<hex SSM code>.bmp); the AMSpiriT Lite references come from
ams_shaker_refs.py ($SHAKER_WORK/ref/ams_lite). Per test, the block metric of
shaker_compare.py between each pair. The tests are sorted by how far bemu is
from the closer reference; "BEMU" marks those where AMSpiriT and CPCSE agree
(their own distance, ~5 % from picture placement alone, under 10 %) and bemu
is further from both (over twice that plus 8): bugs with two witnesses.
"""
import glob
import json
import os
import re
import sys

import numpy as np
from PIL import Image

from _work import REPO, WORK

REFS = WORK / "ref" / "ams_lite"


def blocks(x, h, w):
    x = x[: h // 4 * 4, : w // 8 * 8]
    return x.reshape(h // 4, 4, w // 8, 8, 3).mean((1, 3))


def image(path):
    """RGB, one row per CPC line (the references double the lines)."""
    a = np.array(Image.open(path).convert("RGB")).astype(int)
    return a[0::2] if len(a) > 300 else a


def distance(a, b):
    h, w = min(len(a), len(b)), min(a.shape[1], b.shape[1])
    return (np.abs(blocks(a, h, w) - blocks(b, h, w)).sum(2) > 90).mean() * 100


def witnessed(row):
    """AMSpiriT and CPCSE agree, bemu differs from both."""
    return row[2] < 10 and min(row[0], row[1]) > 2 * row[2] + 8


def main():
    results, cpcse, crtc = sys.argv[1], sys.argv[2], sys.argv[3]
    top = int(sys.argv[4]) if len(sys.argv) > 4 else 30
    manifest = json.loads((REPO / "validation" / "Shaker_CSL" / "shakerland_manifest.json").read_text())
    hexes = {f"{t['id']}_{s['subTest']}": s["hex"].upper().zfill(4) for t in manifest["tests"] for s in t["subtests"]}
    best = {}
    for f in sorted(glob.glob(f"{results}/*_CRTC{crtc}_*.png")):
        name = os.path.basename(f)[:-4]
        base = re.sub(r"_\d+$", "", name) if not (REFS / f"{name}.png").exists() else name
        test, sub = base.split(f"_CRTC{crtc}_")
        code = hexes.get(f"{test}_{sub}", sub if re.fullmatch(r"[0-9A-F]{4}", sub) else None)
        ams = REFS / f"{base}.png"
        cps = os.path.join(cpcse, f"CPCSE_{crtc}_{code}.bmp") if code else ""
        if not ams.exists() or not os.path.exists(cps):
            continue
        a, b, c = image(f), image(ams), image(cps)
        row = (distance(a, b), distance(a, c), distance(b, c), name)
        if base not in best or min(row[:2]) < min(best[base][:2]):
            best[base] = row
    rows = sorted(best.values(), key=lambda r: -min(r[0], r[1]))
    print(f"{'bemu-ams':>8} {'bemu-cpcse':>10} {'ams-cpcse':>9}")
    for r in rows[:top]:
        flag = "BEMU" if witnessed(r) else ""
        print(f"{r[0]:8.1f} {r[1]:10.1f} {r[2]:9.1f}  {r[3]} {flag}")
    if rows:
        m = np.mean([r[:3] for r in rows], axis=0)
        print(f"mean {m[0]:.2f} {m[1]:.2f} {m[2]:.2f} over {len(rows)} tests; "
              f"{sum(1 for r in rows if witnessed(r))} BEMU")


if __name__ == "__main__":
    main()
