#!/usr/bin/env python3
"""A snapshot that writes R0 at a chosen microsecond after a VSYNC rise, restores it
1500 us later, then counts (INC HL every 10 us) up to the next VSYNC rise and stores
the count at #8000: the count depends on whether the line the write landed in
overflowed C0 (+256 us) or not. Ground truth for the write-vs-line-end boundary
when run in a reference emulator.

  gen_r0_probe.py <out.sna> <wait_us> [new_r0=61] [reg=0] [value restored=63]
"""
import os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
out = sys.argv[1]; w = int(sys.argv[2]); new = int(sys.argv[3]) if len(sys.argv) > 3 else 61
reg = int(sys.argv[4]) if len(sys.argv) > 4 else 0
back = int(sys.argv[5]) if len(sys.argv) > 5 else 63
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
wait(w); crtc_write(reg, new); wait(1500); crtc_write(reg, back)
emit(0x01, 0x00, 0xf5); emit(0x21, 0, 0)                  # LD BC,#F500 ; LD HL,0
emit(0xed, 0x78, 0x1f, 0x38, 0xfb)                # wait for VSYNC low
emit(0x23, 0xed, 0x78, 0x1f, 0x30, 0xfa)          # loop: INC HL ; IN A,(C) ; RRA ; JR NC,loop
emit(0x22, 0x00, 0x80)                            # LD (#8000),HL
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
