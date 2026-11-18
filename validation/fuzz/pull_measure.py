#!/usr/bin/env python3
"""Pull of the gen_pull3_probe.py pictures: per line, where the first stripe starts, against the
baseline of the same emulator (+ = the picture moved right, in pixels of a 768 wide picture).
  pull_measure.py <dir with w*_x*.png> [--doubled] [--lines a:b]"""
import argparse, glob, os, re
import numpy as np
from PIL import Image

ap = argparse.ArgumentParser()
ap.add_argument('dir'); ap.add_argument('--doubled', action='store_true')
ap.add_argument('--lines', default='40:230')
ap.add_argument('--profile', help='print the per-line pull of this probe (w6_x21)')
a = ap.parse_args()
lo, hi = map(int, a.lines.split(':'))


def edges(path):
    img = np.array(Image.open(path).convert('L')).astype(int)
    if a.doubled:
        img = img[::2]
    img = img[:271]
    out = np.full(len(img), -1)
    for y, row in enumerate(img):
        xs = np.where(row > 60)[0]
        if len(xs):
            out[y] = xs[0]
    return out


base = {w: edges(os.path.join(a.dir, f'w{w}_xbase.png')) for w in (4, 6)}
res = {}
for p in sorted(glob.glob(os.path.join(a.dir, 'w*_x*.png'))):
    m = re.match(r'w(\d+)_x(\d+)\.png', os.path.basename(p))
    if not m:
        continue
    w, x = int(m[1]), int(m[2])
    e = edges(p); b = base[w]
    ok = (e[lo:hi] >= 0) & (b[lo:hi] >= 0)
    d = (e[lo:hi] - b[lo:hi])[ok]
    res[(w, x)] = (np.median(d) if len(d) else None, d.min() if len(d) else None, d.max() if len(d) else None)
    if a.profile == f'w{w}_x{x}':
        print('per line', list(zip(range(len(e)), (e - b).tolist()))[:80])
for w in (4, 6):
    print(f'W={w}: X (chars ahead of the line sync): median pull [min..max]')
    for x in sorted(k[1] for k in res if k[0] == w):
        med, mn, mx = res[(w, x)]
        print(f'  X={x:2d} ({50 - x:2d} ahead): {med:+5.1f} [{mn:+d}..{mx:+d}]')
