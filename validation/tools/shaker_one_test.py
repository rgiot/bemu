#!/usr/bin/env python3
"""Build a one-test Shaker bundle (about a minute instead of ~45 for a module).

  shaker_one_test.py MODULE KEY OUTDIR

KEY is the menu key the module's script types (`key_output 'U'`, `'\\(CAP)'`...):
the menu entry, which is NOT the test id ('6' runs A7, '7' runs A6, 'U' runs
AU...). The new bundle has the header of the module script (disc + RUN +
`wait 10000000`) followed by that key's block only. Some tests depend on the
state left by the previous ones (A6 started fresh differs from the full run).
"""
import os
import shutil
import sys

from _work import BUNDLE


def main():
    mod, key, out = sys.argv[1:4]
    src = BUNDLE / f"MODULE_{mod}" / f"SHAKER-{mod}-0.CSL"
    lines = src.read_text(encoding="latin-1").split("\n")
    hdr = []
    start = 0
    for i, l in enumerate(lines):
        hdr.append(l)
        if l.startswith("key_output") and "RUN" in l:
            start = i
            break
    hdr.append("wait 10000000")
    res, inblk = [], False
    for l in lines[start + 1:]:
        p = l.split(";")[0].split()
        if p and p[0] == "key_output":
            if f"'{key}'" in l and not inblk:
                inblk = True
            elif inblk and "' '" not in l:
                break
        if inblk:
            res.append(l)
    os.makedirs(out, exist_ok=True)
    for m in os.listdir(BUNDLE):
        d = os.path.join(out, m)
        if os.path.lexists(d):
            os.remove(d) if os.path.islink(d) else shutil.rmtree(d)
        if m == f"MODULE_{mod}":
            os.makedirs(d)
            for f in os.listdir(BUNDLE / m):
                if f != f"SHAKER-{mod}-0.CSL":
                    os.symlink(BUNDLE / m / f, os.path.join(d, f))
            with open(os.path.join(d, f"SHAKER-{mod}-0.CSL"), "w", encoding="latin-1") as fh:
                fh.write("\n".join(hdr + res) + "\n")
        else:
            os.symlink(BUNDLE / m, d)
    print(f"{len(res)} script lines in {out}")


if __name__ == "__main__":
    main()
