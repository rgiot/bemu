# Kevin Thacker's CPC "Acid tests"

The binaries here are from Kevin Thacker's (arnoldemu) "Acid tests", a suite that checks how
closely an emulator follows real Amstrad hardware. They ship with the Arnold emulator, and
emu-info (`tests/cpc/acid`) mirrors them. They are copied unmodified. Their sources carry this
notice:

    ;; (c) Copyright Kevin Thacker 2015-2016
    ;; This code is part of the Arnold emulator distribution.
    ;; This code is free to distribute without modification
    ;; this copyright header must be retained.

| File | Source | What it tests |
|---|---|---|
| `cpctest.bin` | `cpc/cpctest.asm` | Gate Array: ROM/mode register, interrupt timing against HSYNC |
| `ppi.bin` | `ppi/ppi.asm` | 8255 PPI: modes, latches, port reads and writes |
| `psg.bin` | `psg/psg.asm` | AY-3-8912: register masks, its bus seen through PPI port A |
| `cpu.bin` | `z80tests/cpu.asm` | Z80: OUT (C),0, interrupts after EI/DD/FD, LD A,I P/V |
| `inout.bin` | `z80tests/inout.asm` | Z80: the flags of IN |
| `crtc3_4_status.bin` | `crtc/crtc3_4_status.asm` | The CPC+'s ASIC CRTC: its status registers |

Kevin ran them on the machines described in the suite's `desc.txt` (a 6128 with each CRTC type,
a 6128+, a GX4000 and others). `tests/acid.rs` runs them.
