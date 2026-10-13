#!/usr/bin/env python3
"""R2 (HSYNC position) taken from a 256 byte table, one entry per line, on a striped mode 1 screen:
what the monitor does with a moving HSYNC (the waving screens of 'What a Cool Weekend').
  gen_wave_probe.py <out.sna> <pattern> [phase=20]
pattern: step:<a>:<b>:<n>  (a for n lines then b for n lines), sine:<centre>:<amp>:<period>, ramp:<a>:<b>:<n>"""
import math, struct, sys
out = sys.argv[1]; pat = sys.argv[2].split(':'); phase = int(sys.argv[3]) if len(sys.argv) > 3 else 20
tab = [46] * 256
if pat[0] == 'step':
    a, b, n = int(pat[1]), int(pat[2]), int(pat[3])
    tab = [a if (i // n) % 2 == 0 else b for i in range(256)]
elif pat[0] == 'sine':
    c, amp, per = int(pat[1]), float(pat[2]), float(pat[3])
    tab = [int(round(c + amp * math.sin(2 * math.pi * i / per))) for i in range(256)]
elif pat[0] == 'ramp':
    a, b, n = int(pat[1]), int(pat[2]), int(pat[3])
    tab = [a + (b - a) * (i % n) // n for i in range(256)]
code = bytearray()
class T: us = 0
def emit(us, *b): code.extend(b); T.us += us
def nops(n):
    for _ in range(n): emit(1, 0)
emit(1, 0xf3)
outer = len(code)
emit(3, 0x01, 0x00, 0xf5)
emit(8, 0xed, 0x78, 0x1f, 0x38, 0xfb)
emit(8, 0xed, 0x78, 0x1f, 0x30, 0xfb)
nops(phase)
emit(3, 0x21, 0x00, 0x50)               # LD HL,#5000 (table)
emit(3, 0xfd, 0x2e, 200)                # LD IYL,200
emit(3, 0x01, 0x02, 0xbc); emit(4, 0xed, 0x49)   # BC=#BC02 ; OUT (C),C  : select R2
emit(2, 0x06, 0xbd)                     # LD B,#BD
loop = len(code); body = T.us
emit(2, 0x7e)                           # LD A,(HL)
emit(1, 0x2c)                           # INC L
emit(4, 0xed, 0x79)                     # OUT (C),A
emit(2, 0xfd, 0x2d)                     # DEC IYL
pad = 64 - 3 - (T.us - body)
assert pad >= 0
nops(pad)
emit(3, 0xc2, 0, 0)
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + loop)
emit(3, 0xc3, 0, 0); code[-2:] = struct.pack('<H', 0x4000 + outer)
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
mem[0x5000:0x5100] = bytes(tab)
for i in range(0xc000, 0x10000, 2): mem[i] = 0xff; mem[i + 1] = 0x00
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x0b, 0x0b, 0x0b] + [0x0b] * 12 + [0x14])
hdr[0x40] = 0x8d; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
