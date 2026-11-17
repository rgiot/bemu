#!/usr/bin/env python3
"""Compare bemu's screenshots with AMSpiriT Lite's, pixel for pixel, at the CPC's colour levels.

  amspirit_compare.py BEMU_DIR AMSPIRIT_DIR [--mark OUT_DIR]

Shots are matched by file name. AMSpiriT's are 542 lines (each CPC line twice), its levels go
from 5 to 245 and its picture sits one pixel to the left of bemu's: the comparison takes every
other line, maps both to the 16 levels of a CPC+ colour channel and shifts by that pixel. Prints,
per shot, the differing pixels and where; `--mark` writes bemu's shot with them in red.
"""
import argparse
import glob
import os

import numpy as np
from PIL import Image


def bemu_levels(path):
    return np.rint(np.array(Image.open(path).convert("RGB"))[:271, 1:].astype(float) / 17).astype(int)


def amspirit_levels(path):
    shot = np.array(Image.open(path).convert("RGB"))[::2][:271, :-1].astype(float)
    return np.clip(np.rint((shot - 5) / 240 * 15), 0, 15).astype(int)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("bemu")
    parser.add_argument("amspirit")
    parser.add_argument("--mark")
    args = parser.parse_args()
    total = same = 0
    for path in sorted(glob.glob(os.path.join(args.amspirit, "*.png"))):
        name = os.path.basename(path)
        ours = os.path.join(args.bemu, name)
        if not os.path.exists(ours):
            continue
        a, b = bemu_levels(ours), amspirit_levels(path)
        differ = np.abs(a - b).max(2) > 0
        total += 1
        if not differ.any():
            same += 1
            print(f"{name:20s} identical")
            continue
        ys, xs = np.where(differ)
        print(f"{name:20s} {differ.sum():6d} pixels  x {xs.min()}-{xs.max()}  lines {ys.min()}-{ys.max()}")
        if args.mark:
            os.makedirs(args.mark, exist_ok=True)
            marked = np.array(Image.open(ours).convert("RGB"))
            marked[:271, 1:][differ] = (255, 0, 0)
            Image.fromarray(marked).save(os.path.join(args.mark, name))
    print(f"{same} of {total} identical")


main()
