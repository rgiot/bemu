#!/usr/bin/env python3
"""The real-CPC test kit: a DSK of the probe programs of validation/fuzz, to run on a real CPC.

  make_kit.py [out.dsk] [--phases 23,24,25]

Each probe (validation/fuzz/gen_*_probe.py) is a snapshot that sets up the CRTC, the Gate Array and a
striped screen, then runs one frame-long loop that rewrites a CRTC register on every line. Here every
snapshot becomes a standalone binary: a short prologue (di, the Gate Array's inks and mode, the CRTC's
registers, the screen, the PPI) then the probe's own code at &4000, exactly as the snapshot would start it.
`RUN"ALT24"` on a CPC (any model, any CRTC) shows the probe; the picture is what to capture.

The probes' timing is exact to the microsecond (the loop starts a chosen number of microseconds, the
`phase`, after the VSYNC: 24 matches AMSpiriT Lite and bemu); a real CPC may need another one, so every
probe comes with phases 23, 24 and 25 (the last two digits of the name).

The PAL1 and PAL2 programs show the 27 inks as 16 horizontal bands (mode 0), to calibrate colours.
"""
import argparse
import os
import subprocess
import struct
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FUZZ = os.path.join(HERE, "..", "fuzz")

# (name, generator, arguments after <out> and <phase>)
PROBES = [
    # the line sync's width (R3 rewritten on every line), no other pulse: 6 us normally
    ("ALT", "gen_pull4_probe.py", ["6", "-", "-", "100:40:6/5"]),          # 6/5/6/5... for 40 lines
    ("W5", "gen_pull4_probe.py", ["6", "-", "-", "100:40:5"]),             # R3=5 for 40 lines
    ("W4", "gen_pull4_probe.py", ["6", "-", "-", "100:40:4"]),             # R3=4 for 40 lines
    ("W3", "gen_pull4_probe.py", ["6", "-", "-", "100:40:3"]),             # R3=3 for 40 lines
    ("WAVE", "gen_pull4_probe.py", ["6", "-", "-", "100:96:6/5/4/3/4/5"]),   # a wave of widths
    ("SHORT", "gen_pull4_probe.py", ["6", "-", "-", "20:1:5,60:2:5,100:3:5,140:8:5"]),  # runs of 1, 2, 3, 8 lines
    # a second HSYNC (4 and 2 us wide) every line, moving one character nearer the line sync every 6 lines
    ("PUL4", "gen_pull4_probe.py", ["4", "sweep:42:14:6", "-"]),
    ("PUL2", "gen_pull4_probe.py", ["2", "sweep:42:14:6", "-"]),
]
INKWRITE = ("INK1", "gen_inkwrite_probe.py", ["30", "0", "1"])  # a palette write in the middle of a mode 1 line


def sna_probe(generator, phase, arguments, directory):
    out = os.path.join(directory, "probe.sna")
    # gen_pull4_probe.py <out> <phase> <We> <X|-> <extra lines> [<narrow lines>]
    # gen_inkwrite_probe.py <out> <phase> <write_us> <shift> <mode>
    command = [sys.executable, "-I", os.path.join(FUZZ, generator), out, str(phase), *arguments]
    subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    return open(out, "rb").read()


class Code:
    def __init__(self):
        self.bytes = bytearray()

    def emit(self, *b):
        self.bytes.extend(b)

    def out_a(self, value):  # ld a,n ; out (c),a
        self.emit(0x3E, value & 255, 0xED, 0x79)

    def crtc(self, register, value):
        self.emit(0x01, register, 0xBC, 0xED, 0x49)  # ld bc,#bc<r> ; out (c),c : select
        self.emit(0x01, 0x00, 0xBD, 0x3E, value & 255, 0xED, 0x79)  # ld bc,#bd00 ; ld a,v ; out (c),a

    def fill(self, address, length, value):
        """RAM from `address`, `length` bytes of `value`."""
        self.emit(0x21, address & 255, address >> 8, 0x36, value)  # ld hl,address ; ld (hl),value
        if length > 1:
            self.emit(0x11, (address + 1) & 255, (address + 1) >> 8)  # ld de,address+1
            self.emit(0x01, (length - 1) & 255, (length - 1) >> 8, 0xED, 0xB0)  # ld bc,length-1 ; ldir


def setup(sna, code):
    """The machine's state of the snapshot, as code: inks, mode, CRTC, PPI."""
    code.emit(0xF3)  # di
    code.emit(0x31, 0xFF, 0xBF)  # ld sp,#bfff
    code.emit(0x01, 0x00, 0x7F)  # ld bc,#7f00 (the Gate Array)
    for pen in range(17):
        code.out_a(pen if pen < 16 else 0x10)
        code.out_a(0x40 | (sna[0x2F + pen] & 0x1F))
    code.out_a(sna[0x40])  # mode, ROMs
    code.out_a(sna[0x41])  # RAM
    for register in range(18):
        code.crtc(register, sna[0x43 + register])
    code.emit(0x01, sna[0x42], 0xBC, 0xED, 0x49)  # the selected CRTC register
    code.emit(0x01, 0x00, 0xF7, 0x3E, sna[0x59], 0xED, 0x79)  # the PPI's control word


def screen(mem, code):
    """The screen RAM (&C000-&FFFF) of the snapshot: a pattern of period 1 or 2, else runs of 80 bytes."""
    ram = mem[0xC000:0x10000]
    for period in (1, 2):
        if all(ram[i] == ram[i % period] for i in range(len(ram))):
            code.emit(0x21, 0x00, 0xC0, 0x36, ram[0])
            if period == 2:
                code.emit(0x23, 0x36, ram[1], 0x2B)  # inc hl ; ld (hl),b ; dec hl
            code.emit(0x11, period, 0xC0)  # ld de,#c000+period
            n = 0x4000 - period
            code.emit(0x01, n & 255, n >> 8, 0xED, 0xB0)
            return
    i = 0
    while i < len(ram):
        j = i
        while j < len(ram) and ram[j] == ram[i]:
            j += 1
        # runs of a non-zero byte, cut at 80 bytes (a CRTC line)
        k = i
        while k < j and ram[i] != 0:
            n = min(80, j - k)
            code.fill(0xC000 + k, n, ram[k])
            k += n
        i = j


def probe_binary(sna):
    """The standalone program of a probe snapshot: (load address, bytes)."""
    mem = sna[256:]
    org = 0x4000
    # the probe's code: from &4000 to its last non-zero byte (it ends with a JP)
    end = max(i for i in range(org, org + 0x1000) if mem[i]) + 1
    table = bytes(mem[0x8000:max([i for i in range(0x8000, 0xC000) if mem[i]] + [0x7FFF]) + 1])
    load = 0x3800
    code = Code()
    setup(sna, code)
    screen(mem, code)
    if table:
        source = org + (end - org)
        code.emit(0x21, source & 255, source >> 8, 0x11, 0x00, 0x80)
        code.emit(0x01, len(table) & 255, len(table) >> 8, 0xED, 0xB0)  # ldir
    code.emit(0xC3, org & 255, org >> 8)  # jp &4000: the probe
    assert len(code.bytes) <= org - load, len(code.bytes)
    body = bytes(code.bytes).ljust(org - load, b"\x00") + bytes(mem[org:end]) + table
    return load, body


# ---- the colour bands (PAL1, PAL2): mode 0, 16 pens of 16 inks, horizontal bands of 12 lines
# the hardware colour of each of the 27 inks of the firmware
HARDWARE = [
    0x14, 0x04, 0x15, 0x1C, 0x18, 0x1D, 0x0C, 0x05, 0x0D, 0x16, 0x06, 0x17, 0x1E, 0x00,
    0x1F, 0x0E, 0x07, 0x0F, 0x12, 0x02, 0x13, 0x1A, 0x19, 0x1B, 0x0A, 0x03, 0x0B,
]


def mode0_byte(pen):
    """A byte of two pixels of `pen` in mode 0: bits 7 5 3 1 (left pixel) and 6 4 2 0 (right) hold its bits 0..3."""
    value = 0
    for n in range(4):
        bit = (pen >> n) & 1
        value |= bit << (7 - 2 * n)
        value |= bit << (6 - 2 * n)
    return value


def palette_binary(inks):
    """16 inks, as 16 bands of 12 lines, in mode 0 (the border is black)."""
    code = Code()
    code.emit(0xF3, 0x31, 0xFF, 0xBF, 0x01, 0x00, 0x7F)
    for pen in range(16):
        code.out_a(pen)
        code.out_a(0x40 | inks[pen])
    code.out_a(0x10)
    code.out_a(0x40 | 0x14)  # a black border
    code.out_a(0x8C)  # mode 0, ROMs off
    code.out_a(0xC0)
    for register, value in enumerate((63, 40, 46, 0x8E, 38, 0, 25, 30, 0, 7, 0, 0, 0x30, 0)):
        code.crtc(register, value)
    code.emit(0x01, 0x00, 0xF7, 0x3E, 0x82, 0xED, 0x79)
    for line in range(200):
        pen = min(line // 12, 15)
        base = 0xC000 + (line % 8) * 0x800 + (line // 8) * 80
        code.fill(base, 80, mode0_byte(pen))
    code.emit(0x18, 0xFE)  # jr $
    return 0x4000, bytes(code.bytes)


# ---- the DSK
def amsdos_header(name, ext, load, entry, length):
    header = bytearray(128)
    header[1:9] = name.ljust(8).encode()
    header[9:12] = ext.ljust(3).encode()
    header[18] = 2  # binary
    header[21:23] = struct.pack("<H", load)
    header[24:26] = struct.pack("<H", length)
    header[26:28] = struct.pack("<H", entry)
    header[64:67] = struct.pack("<I", length)[:3]
    header[67:69] = struct.pack("<H", sum(header[0:67]))
    return bytes(header)


def build_dsk(files):
    """A standard 40 tracks, 9 sectors DSK (AMSDOS data format) holding `files` (name, load, bytes)."""
    sectors = 40 * 9
    disc = [bytearray([0xE5]) * 512 for _ in range(sectors)]
    directory = []
    block = 2  # blocks 0 and 1 are the directory
    for name, load, body in files:
        content = amsdos_header(name, "BIN", load, load, len(body)) + body
        content += b"\x00" * (-len(content) % 128)
        blocks = []
        for offset in range(0, len(content), 1024):
            chunk = content[offset : offset + 1024].ljust(1024, b"\xE5")
            blocks.append(block)
            for half in range(2):
                disc[block * 2 + half] = bytearray(chunk[half * 512 : (half + 1) * 512])
            block += 1
        assert block * 2 <= sectors, "the disc is full"
        # extents of 16 blocks (128 records)
        for extent in range(0, len(blocks), 16):
            part = blocks[extent : extent + 16]
            records = min(128, (len(content) - extent * 1024 + 127) // 128)
            entry = bytearray(32)
            entry[1:9] = name.ljust(8).encode()
            entry[9:12] = b"BIN"
            entry[12] = extent // 16
            entry[15] = records
            entry[16 : 16 + len(part)] = bytes(part)
            directory.append(bytes(entry))
    assert len(directory) <= 64
    raw = b"".join(directory).ljust(2048, b"\xE5")
    for i in range(4):
        disc[i] = bytearray(raw[i * 512 : (i + 1) * 512])
    header = bytearray(256)
    signature = b"MV - CPC Disk-File\r\nDisk-Info\r\n"
    header[0 : len(signature)] = signature
    header[0x22:0x30] = b"bemu real kit"[:14].ljust(14, b"\x00")
    header[0x30] = 40
    header[0x31] = 1
    header[0x32:0x34] = struct.pack("<H", 0x1300)
    out = bytearray(header)
    for track in range(40):
        info = bytearray(256)
        info[0:13] = b"Track-Info\r\n\x00"
        info[16] = track
        info[17] = 0
        info[20] = 2
        info[21] = 9
        info[22] = 0x4E
        info[23] = 0xE5
        for s in range(9):
            at = 24 + s * 8
            info[at : at + 8] = bytes([track, 0, 0xC1 + s, 2, 0, 0, 0, 2])
        out += info
        for s in range(9):
            out += disc[track * 9 + s]
    return bytes(out)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", nargs="?", default=os.path.join(HERE, "REALKIT.dsk"))
    parser.add_argument("--phases", default="23,24,25")
    args = parser.parse_args()
    files = []
    with tempfile.TemporaryDirectory() as directory:
        for phase in [int(p) for p in args.phases.split(",")]:
            for name, generator, arguments in PROBES + [INKWRITE]:
                # the ink probe's phase is its own (36), the others' is `phase`
                real_phase = phase + 12 if generator == "gen_inkwrite_probe.py" else phase
                sna = sna_probe(generator, real_phase, arguments, directory)
                load, body = probe_binary(sna)
                files.append((f"{name}{phase}", load, body))
    for name, inks in (("PAL1", range(0, 16)), ("PAL2", range(11, 27))):
        load, body = palette_binary([HARDWARE[i] for i in inks])
        files.append((name, load, body))
    dsk = build_dsk(files)
    open(args.out, "wb").write(dsk)
    print(f"{args.out}: {len(files)} files")
    for name, load, body in files:
        print(f"  {name:8s} load &{load:04X}, {len(body)} bytes")


if __name__ == "__main__":
    main()
