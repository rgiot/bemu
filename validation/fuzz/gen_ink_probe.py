#!/usr/bin/env python3
"""A snapshot that sends the CRTC the INK edge reference (see gen_hsync_probe.py): pen 3 turns red at a fixed Z80 time of each line (R3=2, R2=17/31/50
written ~10 us apart, R3=8 near the end of the line) on 200 lines of a striped mode 1 screen.
  gen_hsync_probe.py <out.sna> <phase_us> [r2a r2b r2c]
The phase (us added before the 64 us loop starts) slides the pattern along the line; compare the
picture of each emulator for the same phase (they differ by a constant offset)."""
import struct, sys
out = sys.argv[1]; phase = int(sys.argv[2])
r2a, r2b, r2c = (int(x) for x in sys.argv[3:6]) if len(sys.argv) >= 6 else (17, 31, 50)
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
emit(3, 0x11, 0x4c, 0x4b)        # LD DE,nn : E=#4B (white) D=#4C (red)
emit(3, 0xfd, 0x2e, 200)         # LD IYL,200
emit(3, 0x01, 0x03, 0x7f)        # LD BC,#7F03
emit(4, 0xed, 0x49)              # OUT (C),C : select pen 3
emit(2, 0x06, 0x7f)              # LD B,#7F
loop = len(code)
body = T.us
def at(n):
    assert n - (T.us - body) >= 0, (n, T.us - body)
    nops(n - (T.us - body))
at(10); out_r(D)                  # pen 3 red  (start 10)
at(30); out_r(E)                  # pen 3 white (start 30)
emit(2, 0xfd, 0x2d)
at(64 - 3)
emit(3, 0xc2, 0, 0)
j = len(code)
code[j - 2:j] = struct.pack('<H', 0x4000 + loop)
emit(3, 0xc3, 0, 0)
code[-2:] = struct.pack('<H', 0x4000 + outer)
print(out, 'loop', 64, 'us', 'body ends at', T.us - body - 2)
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
