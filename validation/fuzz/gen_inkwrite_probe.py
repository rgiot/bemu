#!/usr/bin/env python3
"""Ink write probe: which pixel a palette write reaches, by instruction. A mode 1 screen of pen 1;
every line, pen 1's ink is set to a dark colour at the start of the line (OUT (C),E) then to a light
one in the middle with the instruction under test: OUT (C),r on lines 0-49, OUT (n),A on lines
50-99, OUTI on lines 100-149 (started 2 us earlier: it writes on its 5th microsecond, the others on
their 3rd), so that all three write in the same microsecond; lines 150-199 shift the OUT (C),r
write by `shift` microseconds (default 0) for reference. The x where the light colour starts, per
group, tells the latency (validation/fuzz/inkwrite_measure.py).
Synchronised once, then a loop of exactly one frame.
  gen_inkwrite_probe.py <out.sna> <phase_us> [write_us [shift_us [mode]]]
The screen is in `mode` (default 1), all of pen 1."""
import struct
import sys

out, phase = sys.argv[1], int(sys.argv[2])
write_us = int(sys.argv[3]) if len(sys.argv) > 3 else 30
DARK, LIGHT = 0x44, 0x4b   # inks 4 (dark blue) and 11 (bright white)
TABLE = 0x8000
code = bytearray()
T = [0]


def emit(us, *b):
    code.extend(b)
    T[0] += us


def nops(n):
    for _ in range(n):
        emit(1, 0)


emit(1, 0xf3)                                   # di
emit(3, 0x01, 0x00, 0xf5)                       # ld bc,#f500
emit(8, 0xed, 0x78, 0x1f, 0x38, 0xfb)           # wait for the VSYNC
emit(8, 0xed, 0x78, 0x1f, 0x30, 0xfb)           # and its end
nops(phase)
emit(3, 0x01, 0x01, 0x7f)                       # ld bc,#7f01
emit(4, 0xed, 0x49)                             # out (c),c : select pen 1
emit(2, 0x1e, DARK)                             # ld e,dark
emit(2, 0x16, LIGHT)                            # ld d,light
frame = len(code)


def line_loop(count, body):
    """`count` lines of 64 us; `body(at)` emits the line, `at(n)` pads to microsecond n."""
    emit(3, 0xfd, 0x2e, count)                  # ld iyl,count
    loop = len(code)
    start = T[0]

    def at(n):
        assert n >= T[0] - start, (n, T[0] - start)
        nops(n - (T[0] - start))
    body(at)
    at(64 - 5)
    emit(2, 0xfd, 0x2d)                         # dec iyl
    emit(3, 0x20, 0)                            # jr nz,loop (taken: 3 us, not taken: 2)
    code[-1] = (loop - len(code)) & 0xff
    T[0] -= 1                                   # the last jr is not taken
    return 3 - 1                                # ld iyl, and the jr not taken


def g_outc(shift):
    def body(at):
        emit(2, 0x06, 0x7f)                     # ld b,#7f
        emit(4, 0xed, 0x59)                     # out (c),e : dark
        at(write_us - 2 + shift)
        emit(4, 0xed, 0x51)                     # out (c),d : light (written on its 3rd us)
    return body


def g_outn(at):
    emit(2, 0x06, 0x7f)
    emit(4, 0xed, 0x59)                         # out (c),e : dark
    at(write_us - 4)
    emit(2, 0x3e, LIGHT)                        # ld a,light
    emit(3, 0xd3, 0xff)                         # out (#ff),a : port #4Bff, written on its 3rd us


def g_outi(at):
    emit(2, 0x06, 0x7f)
    emit(4, 0xed, 0x59)                         # out (c),e : dark
    emit(3, 0x21, TABLE & 0xff, TABLE >> 8)     # ld hl,table (light)
    emit(2, 0x06, 0x80)                         # ld b,#80 : OUTI decrements it first
    at(write_us - 4)
    emit(5, 0xed, 0xa3)                         # outi : written on its 5th us


extra = 0
shift = int(sys.argv[4]) if len(sys.argv) > 4 else 0
for body in (g_outc(0), g_outn, g_outi, g_outc(shift)):
    extra += line_loop(50, body)
# 200 lines done (each loop: 3 us of setup/exit not counted): the other 112 lines, then again
emit(2, 0x3e, 111)                              # ld a,111
idle = len(code)
nops(60)
emit(1, 0x3d)                                   # dec a
emit(3, 0xc2, 0, 0)                             # jp nz,idle
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + idle)
# a frame is 312 lines: 200 + the loops' extra + ld a + 111 lines + pad + jp
pad = 64 - extra - 2 - 3
assert 200 * 64 + extra + 2 + 111 * 64 + pad + 3 == 312 * 64
nops(pad)
emit(3, 0xc3, 0, 0); code[-2:] = struct.pack('<H', 0x4000 + frame)
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
mem[TABLE] = LIGHT
mode = int(sys.argv[5]) if len(sys.argv) > 5 else 1
for i in range(0xc000, 0x10000):
    mem[i] = (0xc0, 0xf0, 0xff)[mode]           # every pixel of the byte in pen 1
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x01
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x04] + [0x14] * 14 + [0x14])
hdr[0x40] = 0x8c | mode; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
