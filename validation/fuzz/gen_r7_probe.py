#!/usr/bin/env python3
"""R7 written with the value of C4 at a chosen microsecond of a line (ACCC 16.4.1.1: blocked
at C0=0/1, triggered at once otherwise). After a VSYNC rise R7 is set to 99, then
`gen_r7_probe.py <out.sna> <wait_us> [row=1]` writes R7=C4(30+row...) `wait_us` after the rise
(row lines of 64 us after it, C4 stepping every 8 lines, so wait_us ~ 8*64*row + k) and reads
the VSYNC bit 7 us later; the byte lands at #8000 (1 = VSYNC seen). Sweep wait_us over a
line to see the blocked window; compare reference emulators' sweeps (their phase differs).
"""
import struct, sys
out = sys.argv[1]; w = int(sys.argv[2]); row = int(sys.argv[3]) if len(sys.argv) > 3 else 1
code = bytearray()
def emit(*b): code.extend(b)
def wait(us):
    while us >= 6:
        n = min((us - 1) // 4, 255)
        emit(0x06, n, 0x10, 0xfe); us -= 4 * n + 1
    code.extend([0] * us)
def crtc_write(r, v):
    emit(0x01, r, 0xbc, 0xed, 0x49); emit(0x01, v, 0xbd, 0xed, 0x49)
emit(0xf3); emit(0x01, 0x00, 0xf5)
emit(0xed, 0x78, 0x1f, 0x38, 0xfb); emit(0xed, 0x78, 0x1f, 0x30, 0xfb)
crtc_write(7, 99)
emit(0x01, 0x00, 0xf5)
wait(w - 40)                                   # the write itself is a few us after the end of the wait
emit(0x01, 7, 0xbc, 0xed, 0x49)                # select R7
emit(0x01, 30 + row, 0xbd)                     # BC=#BD1F
emit(0xed, 0x49)                               # write R7 = C4 of that row
emit(0x01, 0x00, 0xf5)
wait(1)
emit(0xed, 0x78, 0x1f, 0x3e, 0x00, 0x17)       # IN A,(C) ; RRA ; LD A,0 ; RLA  -> A = vsync bit
emit(0x32, 0x00, 0x80)                         # LD (#8000),A
emit(0x18, 0xfe)
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x04, 0x15, 0x1c, 0x18, 0x1d, 0x0c, 0x05, 0x0d, 0x16, 0x06, 0x17, 0x1e, 0x00, 0x1f, 0x0e, 0x14])
hdr[0x40] = 0x8c; hdr[0x41] = 0xc0; hdr[0x42] = 0
hdr[0x43:0x43 + 18] = bytes([63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
