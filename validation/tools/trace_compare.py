#!/usr/bin/env python3
"""Align the CRTC register writes of amsrust and of CPCSyntaxError.

  trace_compare.py ours.log cpcse.log [start_index]

ours.log: stderr of a run with AMSRUST_TRACE_REG=1 (lines `W R<r><-<v> C0= C4= C9=`;
the bus holds a write for several ticks, so repeats are collapsed).
cpcse.log: stderr of cpcse with CPCSE_TRACE_REG="0,1,4,5,6,9,12,13" (the same
registers are kept on our side). Boot differs between the emulators, so the
first lines never match: pass the index where the test's own writes begin.
Prints the first divergence of (register, value, C4, C9) with its context.
"""
import re
import sys

REGS = (0, 1, 4, 5, 6, 9, 12, 13)


def ours(path):
    out, prev, run = [], None, 0
    for l in open(path, errors="ignore"):
        m = re.match(r"W R(\d+)<-(\d+) C0=(\d+) C4=(\d+) C9=(\d+)", l)
        if not m:
            continue
        t = tuple(map(int, m.groups()))
        if (prev and prev[0] == t[0] and prev[1] == t[1] and prev[3] == t[3] and prev[4] == t[4]
                and 0 <= t[2] - prev[2] <= 1 and run < 14):
            prev, run = t, run + 1
            continue
        out.append(t)
        prev, run = t, 1
    return [t for t in out if t[0] in REGS]


def cpcse(path):
    out = []
    for l in open(path, errors="ignore"):
        m = re.match(r"f\d+\s+R(\d+)\s*<-\s*(\d+)\s+C0=\s*(\d+) C4=\s*(\d+) C9=(\d+)", l)
        if m:
            out.append(tuple(map(int, m.groups())))
    return out


def main():
    o, c = ours(sys.argv[1]), cpcse(sys.argv[2])
    start = int(sys.argv[3]) if len(sys.argv) > 3 else 0
    print(len(o), "writes (ours),", len(c), "(cpcse)")
    n, i = min(len(o), len(c)), start
    while i < n and o[i][:2] == c[i][:2] and o[i][3:] == c[i][3:]:
        i += 1
    print("first divergence at", i)
    for k in range(max(0, i - 6), min(n, i + 12)):
        print(k, "ours", o[k], "cpcse", c[k])


if __name__ == "__main__":
    main()
