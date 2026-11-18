#!/usr/bin/env python3
"""Second-sync probe with the line sync's width too (Onescreen Colonies #1's gaps): like
gen_pull3_probe.py (line sync at C0=50, an extra HSYNC at C0=X of width We on chosen lines, a
striped mode 1 screen, CRTC 1, one frame long loop), but R3 is also rewritten on every line: We
before the extra pulse, the line sync's width (6, or 5 on chosen lines) after it.
  gen_pull4_probe.py <out.sna> <phase_us> <We> <X|-> <extra lines> [<narrow lines>]
Lines are `first:count,first:count...` of the 200 (`-`: none); the narrow ones have a line sync
of width 5, or `first:count:width` (an R3 of 1 to 6), `first:count:w1/w2/...` (the widths in turn, from
line `first` on: an R3 wave). The phase for AMSpiriT and bemu: 24 (as gen_pull3_probe.py)."""
import struct
import sys

out, phase, we = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
x = None if sys.argv[4] == '-' else int(sys.argv[4])
MAIN = 50


def lines(spec):
    if spec in ('-', ''):
        return set()
    out = set()
    for r in spec.split(','):
        f, c = map(int, r.split(':'))
        out.update(range(f, f + c))
    return out


extra = lines(sys.argv[5]) if len(sys.argv) > 5 else set(range(200))
def widths(spec):
    """line -> the line sync's width (6 if not given)"""
    out = {}
    if spec in ('-', ''):
        return out
    for r in spec.split(','):
        f, c, *w = r.split(':')
        f, c = int(f), int(c)
        cycle = [int(v) for v in w[0].split('/')] if w else [5]
        for i in range(c):
            out[f + i] = cycle[i % len(cycle)]
    return out


narrow = widths(sys.argv[6]) if len(sys.argv) > 6 else {}
# entry i (2 bytes): the R2 of line i+1's extra pulse (written at the start of line i's loop),
# then the width of line i+1's line sync (written after that extra pulse)
TABLE = 0x8000
table = bytearray()
for i in range(201):
    table.append(x if x is not None and (i + 1) in extra else MAIN)
    table.append(0x80 | narrow.get(i + 1, 6))
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
emit(3, 0x01, 0x02, 0xbc)                       # ld bc,#bc02
emit(4, 0xed, 0x49)                             # out (c),c : select R2
emit(2, 0x1e, MAIN)                             # ld e,50
emit(2, 0x26, 0x80 | we)                        # ld h,#80|We : R3 of the extra pulse
frame = len(code)
emit(4, 0xdd, 0x21, TABLE & 0xff, TABLE >> 8)   # ld ix,table
emit(5, 0xdd, 0x56, 0)                          # ld d,(ix+0)
emit(3, 0xfd, 0x2e, 200)                        # ld iyl,200
emit(2, 0x06, 0xbd)                             # ld b,#bd
loop = len(code)
body = T[0]


def at(n):
    assert n >= T[0] - body, (n, T[0] - body)
    nops(n - (T[0] - body))


# loop time 0 is at C0 = 58 (the line sync, at 50, is over)
emit(4, 0xed, 0x51)                              # 0: out (c),d : R2 <- X (or 50)
emit(2, 0x06, 0xbc)                              # 4: ld b,#bc
emit(2, 0x3e, 3)                                 # 6: ld a,3
emit(4, 0xed, 0x79)                              # 8: out (c),a : select R3
emit(2, 0x06, 0xbd)                              # 12: ld b,#bd
emit(4, 0xed, 0x61)                              # 14: out (c),h : R3 <- We (C0=8), the pulse is at C0=X
emit(5, 0xdd, 0x4e, 1)                           # 18: ld c,(ix+1) : the line sync's width
emit(5, 0xdd, 0x56, 2)                           # 23: ld d,(ix+2) : the next line's R2
emit(3, 0xdd, 0x23)                              # 28: inc ix (x2 below)
emit(3, 0xdd, 0x23)                              # 31: inc ix
at(36)
emit(4, 0xed, 0x49)                              # 36: out (c),c : R3 <- 5 or 6 (C0=30)
emit(2, 0x06, 0xbc)                              # 40: ld b,#bc
emit(2, 0x3e, 2)                                 # 42: ld a,2
emit(4, 0xed, 0x79)                              # 44: out (c),a : select R2
emit(2, 0x06, 0xbd)                              # 48: ld b,#bd
emit(4, 0xed, 0x59)                              # 50: out (c),e : R2 <- 50 (C0=44)
emit(2, 0xfd, 0x2d)                              # 54: dec iyl
at(64 - 3)
emit(3, 0xc2, 0, 0)                              # 61: jp nz,loop
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + loop)
setup = 4 + 5 + 3 + 2
emit(2, 0x3e, 111)                               # ld a,111
idle = len(code)
nops(60)
emit(1, 0x3d)                                    # dec a
emit(3, 0xc2, 0, 0)                              # jp nz,idle
j = len(code); code[j - 2:j] = struct.pack('<H', 0x4000 + idle)
pad = 64 - setup - 2 - 3
assert setup + 200 * 64 + 2 + 111 * 64 + pad + 3 == 312 * 64
nops(pad)
emit(3, 0xc3, 0, 0); code[-2:] = struct.pack('<H', 0x4000 + frame)
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
mem[TABLE:TABLE + len(table)] = table
for i in range(0xc000, 0x10000, 2): mem[i] = 0xff; mem[i + 1] = 0x00
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x0b, 0x0b, 0x0b] + [0x0b] * 12 + [0x14])
hdr[0x40] = 0x8d; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, MAIN, 0x86, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
