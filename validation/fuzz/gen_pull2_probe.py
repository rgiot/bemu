#!/usr/bin/env python3
"""HSYNC probe with a main pulse of another width (mw): every line the CRTC gets extra HSYNC pulses at C0=x1[,x2] (R2 rewritten ahead
of them) before the main one at C0=46, all of width w (R3=#80|w), on 200 lines of a striped mode 1
screen. The pictures of reference emulators show how the monitor reacts (shift of the picture, gaps).
  gen_pull2_probe.py <out.sna> <phase_us> <w> <x1> <mw>
The phase slides the whole pattern along the line; pick one for which probe_hsync_lines (ours)
prints the expected pulses, the other emulator's phase differs by a few us at most."""
import struct, sys
out = sys.argv[1]; phase = int(sys.argv[2]); w = int(sys.argv[3])
xs = [int(v) for v in sys.argv[4:5]]
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
# one extra pulse of width w at C0=x1, then the main one (C0=46) of width mw
mw = int(sys.argv[5]); x1 = xs[0]
emit(3, 0x11, x1, 46)             # LD DE: E=x1 D=46
emit(3, 0x21, 0x80 | mw, 0x80 | w)  # L=#80|w H=#80|mw  (HL: H=mw L=w)
code[-2:] = bytes([0x80 | w, 0x80 | mw])
emit(3, 0xfd, 0x2e, 200)
emit(2, 0x3e, 3); emit(2, 0x06, 0xbc); out_r(A)             # select R3
emit(2, 0x06, 0xbd)
loop = len(code); body = T.us
def at(n):
    assert n - (T.us - body) >= 0, (n, T.us - body)
    nops(n - (T.us - body))
at(0); out_r(5)                   # R3 = #80|w (L)
emit(2, 0x06, 0xbc); emit(2, 0x3e, 2); out_r(A)            # select R2
emit(2, 0x06, 0xbd)
at(14); out_r(E)                  # R2 = x1
at(max(x1 + w + 3 - 2, 26)); out_r(D)   # R2 = 46
emit(2, 0x06, 0xbc); emit(2, 0x3e, 3); out_r(A)            # select R3
emit(2, 0x06, 0xbd)
out_r(4)                          # R3 = #80|mw (H)
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
