#!/usr/bin/env python3
"""A snapshot that sends the CRTC the HSYNC pattern of the HBL demo's panel lines (R3=2, R2=17/31/50
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
emit(3, 0x11, r2b, r2a)          # LD DE,nn : E=r2a D=r2b
emit(3, 0x21, 8, r2c)            # L=8 H=r2c
emit(3, 0xfd, 0x2e, 200)         # LD IYL,200
emit(2, 0x06, 0xbc); emit(2, 0x3e, 3); out_r(A)      # select R3
emit(2, 0x06, 0xbd)
loop = len(code)
body = T.us
def at(n):                        # pad so that the next instruction starts n us after the loop start
    assert n - (T.us - body) >= 0, (n, T.us - body)
    nops(n - (T.us - body))
emit(2, 0x3e, 2)                  # LD A,2
out_r(A)                          # R3 = 2         (starts at 2)
emit(2, 0x06, 0xbc); emit(2, 0x3e, 2); out_r(A)      # select R2     (ends at 16)
emit(2, 0x06, 0xbd)
at(18); out_r(E)                  # R2 = r2a   (start 18)
at(28); out_r(D)                  # R2 = r2b   (start 28, 10 later)
at(39); out_r(H)                  # R2 = r2c   (11 later)
emit(2, 0x06, 0xbc); emit(2, 0x3e, 3); out_r(A)      # select R3
emit(2, 0x06, 0xbd)
at(55); out_r(L)                  # R3 = 8
emit(2, 0xfd, 0x2d)               # DEC IYL
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
