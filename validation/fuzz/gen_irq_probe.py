#!/usr/bin/env python3
"""Interrupt acceptance probe: after a VSYNC rise (polled), `phase` NOPs, EI and a long NOP chain; the
handler stores the return address at #8000/#8001. The address tells at which microsecond of the chain the
interrupt was accepted, so the same snapshot in several emulators measures the interrupt timing.
  gen_irq_probe.py <out.sna> <phase_nops> [skip_frames=0]"""
import struct, sys
out = sys.argv[1]; phase = int(sys.argv[2])
code = bytearray()
code += bytes([0xfb, 0x76, 0xf3, 0x01, 0x00, 0xf5])          # EI ; HALT (eat the pending interrupt) ; DI ; LD BC,#F500
code += bytes([0xed, 0x78, 0x1f, 0x38, 0xfb])                # wait VSYNC low
code += bytes([0xed, 0x78, 0x1f, 0x30, 0xfb])                # wait VSYNC high
code += bytes([0]) * phase
code += bytes([0x01, 0x9d, 0x7f, 0xed, 0x49])                  # RMR with bit 4: clears the interrupt counter and request
code += bytes([0xfb])                                        # EI
chain = len(code)
code += bytes([0]) * 3700
code += bytes([0x18, 0xfe])
org = 0x4000
mem = bytearray(0x10000); mem[org:org + len(code)] = code
# handler at #38 (ROMs off)
# the first interrupt only returns, the second one records
mem[0x38:0x38 + 24] = bytes([0xf5, 0x3a, 0x02, 0x80, 0xb7, 0x20, 0x08, 0x3e, 0x01, 0x32, 0x02, 0x80, 0xf1, 0xfb, 0xc9,
                             0xf1, 0xe1, 0x22, 0x00, 0x80, 0x18, 0xfe, 0, 0])
hdr = bytearray(256); hdr[0:8] = b'MV - SNA'; hdr[0x10] = 1
hdr[0x21:0x23] = struct.pack('<H', 0xbfff); hdr[0x23:0x25] = struct.pack('<H', org); hdr[0x25] = 1
hdr[0x2e] = 0x10
hdr[0x2f:0x2f + 17] = bytes([0x14, 0x0b] * 8 + [0x14])
hdr[0x40] = 0x8d; hdr[0x41] = 0xc0
hdr[0x43:0x43 + 18] = bytes([63, 40, 46, 0x8e, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0, 0, 0, 0, 0])
hdr[0x56:0x5a] = bytes([0, 0, 0, 0x82]); hdr[0x6b:0x6d] = struct.pack('<H', 64)
open(out, 'wb').write(bytes(hdr) + bytes(mem))
print(f'chain starts at {org + chain:04X}')
