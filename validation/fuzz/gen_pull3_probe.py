#!/usr/bin/env python3
"""Second-sync probe (Onescreen Colonies #1's screen splitting): every line has its line sync at
C0=50 and an extra HSYNC at C0=X, both of width W (R3=#80|W), on 200 lines of a striped mode 1
screen; R2 is selected once and rewritten twice a line (X after the line sync, 50 after the extra
pulse), so the extra pulse can sit anywhere from 2 characters after the line sync to 2 before it.
`X=-` leaves R2 at 50 (the baseline). The shift of each line against the baseline is the monitor's
pull (validation/fuzz/pull_measure.py).
  gen_pull3_probe.py <out.sna> <phase_us> <W> <X|-> [first_line count | first:count,first:count...]
With first_line and count, only those of the 200 lines get the extra pulse (the others keep R2=50):
the picture before, during and after them shows how the line oscillator reacts.
The phase for AMSpiriT and bemu (CRTC 1): 24 (R2 written at C0=58)."""
import struct, sys

out, phase, w = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
x = None if sys.argv[4] == '-' else int(sys.argv[4])
# the lines with the extra pulse: `first count`, or several ranges `first:count,first:count`
if len(sys.argv) > 6:
    ranges = [(int(sys.argv[5]), int(sys.argv[6]))]
elif len(sys.argv) > 5:
    ranges = [tuple(map(int, r.split(':'))) for r in sys.argv[5].split(',')]
else:
    ranges = [(0, 200)]
TABLE = 0x8000
table = bytes((x if x is not None and any(f <= i < f + c for f, c in ranges) else 50)
              for i in range(201))
MAIN = 50
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
emit(8, 0xed, 0x78, 0x1f, 0x30, 0xfb)           # and its end: synchronised once, then a loop of
nops(phase)                                     # exactly one frame (312 lines) keeps the phase
emit(3, 0x01, 0x02, 0xbc)                       # ld bc,#bc02
emit(4, 0xed, 0x49)                             # out (c),c : select R2
emit(2, 0x06, 0xbd)                             # ld b,#bd
emit(2, 0x1e, MAIN)                             # ld e,50
frame = len(code)
frame_t = T[0]
emit(3, 0xfd, 0x2e, 200)                        # ld iyl,200
emit(3, 0x21, TABLE & 0xff, TABLE >> 8)         # ld hl,table: R2 of the extra pulse, line by line
emit(2, 0x56)                                   # ld d,(hl)
emit(2, 0x23)                                   # inc hl
loop = len(code)
body = T[0]


def at(n):
    assert n >= T[0] - body, (n, T[0] - body)
    nops(n - (T[0] - body))


# loop time 0 is at C0 = MAIN + W + 2 (after the line sync, the phase chosen for it)
xr = x if x is not None else MAIN + 20
since = (xr - (MAIN + w + 2)) % 64              # loop time at which C0 = X
t_main = min(since + w + 2, 64 - 12)
emit(4, 0xed, 0x51)                              # out (c),d : R2 <- X
emit(2, 0x56)                                    # ld d,(hl) : the next line's
emit(2, 0x23)                                    # inc hl
at(max(t_main, T[0] - body)); emit(4, 0xed, 0x59)  # out (c),e : R2 <- 50
emit(2, 0xfd, 0x2d)                              # dec iyl
at(64 - 3)
emit(3, 0xc2, 0, 0)                              # jp nz,loop
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + loop)
# the other 112 lines: R2 stays 50
idle_t = T[0]
emit(2, 0x3e, 111)                               # ld a,111
idle = len(code); idle_body = T[0]
nops(60)
emit(1, 0x3d)                                    # dec a
emit(3, 0xc2, 0, 0)                              # jp nz,idle
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + idle)
nops(49)
emit(3, 0xc3, 0, 0); code[-2:] = struct.pack('<H', 0x4000 + frame)
# 10 + 200 lines + 2 + 111 lines + 49 + 3 = 312 lines
assert 10 + 200 * 64 + 2 + 111 * 64 + 49 + 3 == 312 * 64
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
mem[TABLE:TABLE + len(table)] = table
for i in range(0xc000, 0x10000, 2): mem[i] = 0xff; mem[i + 1] = 0x00
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x0b, 0x0b, 0x0b] + [0x0b] * 12 + [0x14])
hdr[0x40] = 0x8d; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, MAIN, 0x80 | w, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
