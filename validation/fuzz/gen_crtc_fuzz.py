#!/usr/bin/env python3
"""Generate a random CRTC rupture program as a .sna (differential testing).

  gen_crtc_fuzz.py <seed> <out.sna> [events]

The program waits for the start of a VSYNC (PPI port B bit 0), then performs
`events` CRTC register writes, each after a random wait, from a catalogue of
values that matter to demos (R0/R1/R2/R3/R4/R5/R6/R7/R8/R9/R12/R13). It then
spins. Run it in amsrust (examples/fuzz_lines) and in CPCSyntaxError
(CPCSE_TRACE_LINE) and compare the per-line (C4, C9, VSYNC, length) sequences
(compare_lines.py).
"""
import os
import random
import struct
import sys

seed = int(sys.argv[1]); out = sys.argv[2]
events = int(sys.argv[3]) if len(sys.argv) > 3 else 40
rnd = random.Random(seed)

CATALOGUE = {
    0: [1, 2, 3, 31, 61, 62, 63, 64, 65] if os.environ.get('FUZZ_TINY_R0') else [31, 61, 62, 63, 64, 65],
    1: [0, 1, 20, 30, 40, 41, 63, 64],
    2: [10, 40, 46, 50, 55],
    3: [0x8e, 0x86, 0x06, 0x02, 0x04, 0x88, 0x0e, 0x89],
    4: [0, 1, 7, 10, 30, 38, 39],
    5: [0, 1, 2, 3, 8],
    6: [0, 1, 20, 25, 30, 38, 40],
    7: [0, 5, 20, 30, 35, 38],
    8: [0, 1, 3, 0x10, 0x20, 0x30] if os.environ.get('FUZZ_IVM') else [0, 0x10, 0x20, 0x30],
    9: [0, 1, 3, 6, 7, 8, 9],
    12: [0x30, 0x20, 0x33, 0x10],
    13: [0, 0x28, 0x50, 0xf0],
}
if os.environ.get('FUZZ_NO_R0'):
    del CATALOGUE[0]      # R0 writes landing on the last character are a known CPCSE difference (see README)
# registers that, once rewritten, should come back to a standard value later
STD = {0: 63, 1: 40, 2: 46, 3: 0x8e, 4: 38, 5: 0, 6: 25, 7: 30, 8: 0, 9: 7, 12: 0x30, 13: 0}

code = bytearray()
def emit(*b): code.extend(b)
def nops(n):
    code.extend([0] * n)
def wait(us):
    # LD B,n (2 us) + DJNZ looping n times (4n-1 us) = 4n+1 us, then NOPs
    while us >= 6:
        n = min((us - 1) // 4, 255)
        emit(0x06, n, 0x10, 0xfe)
        us -= 4 * n + 1
    nops(us)
def crtc_write(reg, val):
    emit(0x01, reg, 0xbc, 0xed, 0x49)      # LD BC,#BCrr ; OUT (C),C   (4+... us)
    emit(0x01, val, 0xbd, 0xed, 0x49)      # LD BC,#BDvv ; OUT (C),C

emit(0xf3)                                  # DI
emit(0x01, 0x00, 0xf5)                      # LD BC,#F500
# wait for VSYNC low then high, so that we start at its rising edge
emit(0xed, 0x78, 0x1f, 0x38, 0xfb)          # loop: IN A,(C) ; RRA ; JR C,loop
emit(0xed, 0x78, 0x1f, 0x30, 0xfb)          # loop: IN A,(C) ; RRA ; JR NC,loop

changed = []
log = []
forced = os.environ.get('FUZZ_EVENTS')      # replay a (wait_us, reg, value) list, e.g. a shrunk case
if forced:
    import ast
    for w, reg, val in ast.literal_eval(forced):
        wait(w)
        if reg not in changed: changed.append(reg)
        crtc_write(reg, val)
        log.append((w, reg, val))
else:
    for e in range(events):
        w = rnd.randint(0, 400)
        wait(w)
        reg = rnd.choice(list(CATALOGUE))
        if rnd.random() < 0.35 and changed:
            reg = rnd.choice(changed); val = STD[reg]       # go back to standard
        else:
            val = rnd.choice(CATALOGUE[reg])
            if reg not in changed: changed.append(reg)
        crtc_write(reg, val)
        log.append((w, reg, val))
wait(2000)
for reg in changed:
    crtc_write(reg, STD[reg])
emit(0x18, 0xfe)                            # JR $

org = 0x4000
assert len(code) < 0x7000, len(code)
mem = bytearray(0x10000)
mem[org:org + len(code)] = code

hdr = bytearray(256)
hdr[0:8] = b'MV - SNA'
hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff)      # SP
hdr[0x23:0x25] = struct.pack('<H', org)         # PC
hdr[0x25] = 1                                   # IM 1
hdr[0x2e] = 0x10
pal = [0x14, 0x04, 0x15, 0x1c, 0x18, 0x1d, 0x0c, 0x05, 0x0d, 0x16, 0x06, 0x17, 0x1e, 0x00, 0x1f, 0x0e, 0x14]
hdr[0x2f:0x2f + 17] = bytes(pal)
hdr[0x40] = 0x8c                                # RMR: mode 0, ROMs off
hdr[0x41] = 0xc0
hdr[0x42] = 0
regs = [63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0]
hdr[0x43:0x43 + 18] = bytes(regs)
hdr[0x55] = 0
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82])
hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
if os.environ.get('FUZZ_LOG'):
    print('events (wait_us, reg, value):', log)
print(f'{out}: {len(code)} bytes of code, {events} events')
