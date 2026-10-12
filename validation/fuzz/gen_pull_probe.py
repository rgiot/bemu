#!/usr/bin/env python3
"""HSYNC pull probe: every line the CRTC gets extra HSYNC pulses at C0=x1[,x2] (R2 rewritten ahead
of them) before the main one at C0=46, all of width w (R3=#80|w), on 200 lines of a striped mode 1
screen. The pictures of reference emulators show how the monitor reacts (shift of the picture, gaps).
  gen_pull_probe.py <out.sna> <phase_us> <w> <x1> [x2]
The phase slides the whole pattern along the line; pick one for which probe_hsync_lines (ours)
prints the expected pulses, the other emulator's phase differs by a few us at most."""
import struct, sys
out = sys.argv[1]; phase = int(sys.argv[2]); w = int(sys.argv[3])
xs = [int(v) for v in sys.argv[4:]]
A, D, E, H, L = 7, 2, 3, 4, 5
code = bytearray()
class T: us = 0
def emit(us, *b): code.extend(b); T.us += us
def nops(n):
    for _ in range(n): emit(1, 0)
def out_r(reg): emit(4, 0xed, 0x41 + 8 * reg)
emit(1, 0xf3)
outer = len(code)
emit(3, 0x01, 0x00, 0xf5)
emit(8, 0xed, 0x78, 0x1f, 0x38, 0xfb)
emit(8, 0xed, 0x78, 0x1f, 0x30, 0xfb)
nops(phase)
vals = xs + [46]
regs = [E, D, H][:len(vals)] if len(xs) == 2 else [E, D][:len(vals)]
# values: E=x1, D=x2 (or 46), H=46
emit(3, 0x11, xs[1] if len(xs) == 2 else 46, xs[0])        # LD DE: E=x1, D=x2|46
emit(3, 0x21, 0, 46)                                        # H=46
emit(3, 0xfd, 0x2e, 200)
emit(3, 0x01, 0x03, 0xbc); emit(4, 0xed, 0x41 + 8 * 0)[:0] if False else None
emit(2, 0x3e, 3); emit(2, 0x06, 0xbc); out_r(A)             # select R3
emit(2, 0x06, 0xbd); emit(2, 0x3e, 0x80 | w); out_r(A)      # R3 = #80|w
emit(2, 0x06, 0xbc); emit(2, 0x3e, 2); out_r(A)             # select R2
emit(2, 0x06, 0xbd)
loop = len(code); body = T.us
def at(n):
    assert n - (T.us - body) >= 0, (n, T.us - body)
    nops(n - (T.us - body))
# write times (loop-relative): each write goes between the previous pulse's end and the next pulse
prev_end = 0
times = []
bounds = xs + [46]
lo = 0
for i, x in enumerate(bounds):
    hi = x                                   # must be written before C0 reaches x
    times.append((lo + hi) // 2 - 2)
    lo = x + w
for i, ts in enumerate(times):
    at(max(ts, T.us - body)); out_r([E, D, H][i] if len(xs) == 2 else [E, D][i])
emit(2, 0xfd, 0x2d)
at(64 - 3)
emit(3, 0xc2, 0, 0)
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + loop)
emit(3, 0xc3, 0, 0); code[-2:] = struct.pack('<H', 0x4000 + outer)
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
for i in range(0xc000, 0x10000, 2): mem[i] = 0xff; mem[i + 1] = 0x00
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x0b, 0x0b, 0x0b] + [0x0b] * 12 + [0x14])
hdr[0x40] = 0x8d; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
