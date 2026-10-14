#!/usr/bin/env python3
"""Count the pixels of a Rhino-Raster screenshot that differ from AMSpiriT's.

  rhino_compare.py shot.png [reference.png]

`shot` is the 768x272 picture of `amsrust` (validation/demos/rhino/rhino.csl
with examples/run_csl); the reference defaults to the AMSpiriT Lite 1.13.4
capture next to the script (768x542: two lines per scanline). The picture
starts one line earlier on amsrust, so the comparison tries a vertical shift of
-2..2 lines and keeps the best. About 770 pixels (0.4%) differ since the HALT
fix, 3900 before it: the demo's pen switches are timed to the microsecond, a
microsecond is 16 mode 2 pixels.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image

here = Path(__file__).resolve().parent
shot = Path(sys.argv[1])
reference = Path(sys.argv[2]) if len(sys.argv) > 2 else here.parent / "demos" / "rhino" / "amspirit_1.13.4_crtc1.png"

ams = np.asarray(Image.open(reference).convert("RGB").resize((768, 271), Image.NEAREST)).astype(int)
ours = np.asarray(Image.open(shot).convert("RGB")).astype(int)
best = None
for dy in range(-2, 3):
    rolled = np.roll(ours, dy, 0)
    h = min(ams.shape[0], rolled.shape[0])
    wrong = int((np.abs(ams[10:h - 10] - rolled[10:h - 10]).sum(2) > 60).sum())
    if best is None or wrong < best[0]:
        best = (wrong, dy)
print(f"{best[0]} pixels differ (vertical shift {best[1]})")
