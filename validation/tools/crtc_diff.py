#!/usr/bin/env python3
"""Differential CRTC test: bemu against AMSpiriT Lite and CPCSyntaxError.

A small Z80 program programs the CRTC from a register table, lets a few
frames pass, then times VSYNC (PPI port B bit 0) from the inside: four
VSYNC lengths and their periods, counted in iterations of a 15 us polling
loop (timeout ~0.9 s). bemu and AMSpiriT run the bytes through their debug
API (POST /api/ram with exec); CPCSE from a generated .sna, its RAM dumped
at the end.

  crtc_diff.py [--types 0,1,2,3,4] [--only NAME] [--emus bemu,ams,cpcse]

Needs `cargo build --release`, AMSpiriT Lite's extracted AppImage in
$SHAKER_WORK/amspirit/squashfs-root (run under xvfb-run) and, for cpcse,
build_cpcse_oracle.sh. A line is "ok" when every emulator agrees within one
loop iteration per value; crtc_diff.json keeps everything.
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request

from _work import REPO, WORK

BEMU = str(REPO / "target" / "release" / "amsrust")
AMS = str(WORK / "amspirit" / "squashfs-root" / "AppRun")
CPCSE = str(WORK / "cpcse" / "build" / "cpcse")
PROG, TABLE, RESULT = 0x4000, 0x4800, 0x9000
MARKER = TABLE - 1  # the run's number: the program copies it to RESULT when done
LOOP_US = 15
BASE = {0: 63, 1: 40, 2: 46, 3: 0x8E, 4: 38, 5: 0, 6: 25, 7: 30, 8: 0, 9: 7, 12: 0x30, 13: 0}

# name -> registers changed from BASE
CONFIGS = {
    "default": {},
    "R9=31 R4=7": {9: 31, 4: 7, 7: 2, 6: 6},
    "R9=15 R4=19": {9: 15, 4: 19, 7: 15, 6: 12},
    "R9=3 R4=77": {9: 3, 4: 77, 7: 60, 6: 50},
    "R9=0 R4=127 R5=4": {9: 0, 4: 127, 5: 4, 7: 100, 6: 50},
    "R9=1 R4=127": {9: 1, 4: 127, 7: 100, 6: 50},
    "R9=0 R4=127 R5=0": {9: 0, 4: 127, 7: 100, 6: 50},
    "R4=0 R9=31 R5=8": {4: 0, 9: 31, 5: 8, 7: 0, 6: 0},
    "R5=1": {5: 1},
    "R5=4": {5: 4},
    "R5=16": {5: 16},
    "R5=31": {5: 31},
    "R3=&1E (vsync 1)": {3: 0x1E},
    "R3=&4E (vsync 4)": {3: 0x4E},
    "R3=&0E (vsync 0)": {3: 0x0E},
    "R3=&FE (vsync 15)": {3: 0xFE},
    "R3=&4E R9=15": {3: 0x4E, 9: 15, 4: 19, 7: 15, 6: 12},
    "R7=0": {7: 0},
    "R7=38 (=R4)": {7: 38},
    "R7=39 (>R4, none)": {7: 39},
    "R7=127": {7: 127},
    "R0=31": {0: 31, 1: 20, 2: 24},
    "R0=127": {0: 127, 1: 40, 2: 90},
    "R0=1": {0: 1, 1: 1, 2: 1, 3: 0x81},
    "R8=1 (interlace)": {8: 1},
    "R8=3 (interlace video)": {8: 3},
    "R9=6 R4=43 (7-line rows)": {9: 6, 4: 43, 7: 34},
}


class Asm:
    """Just enough of a Z80 assembler for this program."""

    def __init__(self, origin):
        self.origin, self.items = origin, []

    def b(self, *bytes_):
        self.items.append(("b", bytes(bytes_)))

    def label(self, name):
        self.items.append(("l", name))

    def jr(self, opcode, name):  # 0x18 jr, 0x20 nz, 0x28 z, 0x30 nc, 0x38 c
        self.items.append(("jr", opcode, name))

    def w(self, opcode, value_or_label):  # opcode followed by a 16-bit operand
        self.items.append(("w", opcode, value_or_label))

    def build(self):
        size = {"b": lambda i: len(i[1]), "l": lambda i: 0, "jr": lambda i: 2, "w": lambda i: len(i[1]) + 2}
        labels, at = {}, self.origin
        for item in self.items:
            if item[0] == "l":
                labels[item[1]] = at
            at += size[item[0]](item)
        out, at = bytearray(), self.origin
        for item in self.items:
            if item[0] == "b":
                out += item[1]
            elif item[0] == "jr":
                offset = labels[item[2]] - (at + 2)
                assert -128 <= offset < 128, item
                out += bytes([item[1], offset & 0xFF])
            elif item[0] == "w":
                value = labels.get(item[2], item[2]) if isinstance(item[2], str) else item[2]
                out += item[1] + bytes([value & 0xFF, value >> 8])
            at += size[item[0]](item)
        return bytes(out)


def program():
    a = Asm(PROG)
    a.b(0xF3)                                  # di
    a.w(b"\x21", TABLE)                        # ld hl,table
    a.label("regs")
    a.b(0x7E, 0xFE, 0xFF)                      # ld a,(hl) : cp #ff
    a.jr(0x28, "regs_done")
    a.w(b"\x01", 0xBC00)                       # ld bc,#bc00
    a.b(0xED, 0x79, 0x23, 0x7E, 0x04, 0xED, 0x79, 0x23)  # out (c),a : inc hl : ld a,(hl) : inc b : out (c),a : inc hl
    a.jr(0x18, "regs")
    a.label("regs_done")
    a.w(b"\x01", 0xF500)                       # ld bc,#f500
    a.w(b"\xDD\x21", RESULT + 1)               # ld ix,results
    # settle: 3 x (wait high, wait low)
    a.b(0x3E, 3, 0xF5)                         # ld a,3 : push af
    a.label("settle")
    a.w(b"\xCD", "while_low")                  # call while_low (waits for the VSYNC to rise)
    a.w(b"\xCD", "while_high")
    a.b(0xF1, 0x3D, 0xF5)                      # pop af : dec a : push af
    a.jr(0x20, "settle")
    a.b(0xF1)                                  # pop af
    a.w(b"\xCD", "while_low")                  # aligned on a VSYNC start
    a.b(0x3E, 4, 0xF5)
    a.label("measure")
    a.w(b"\xCD", "while_high")                 # VSYNC length
    a.b(0xDD, 0x75, 0, 0xDD, 0x74, 1, 0xDD, 0x23, 0xDD, 0x23)  # ld (ix+0),l : ld (ix+1),h : inc ix x2
    a.w(b"\xCD", "while_low")                  # gap to the next VSYNC
    a.b(0xDD, 0x75, 0, 0xDD, 0x74, 1, 0xDD, 0x23, 0xDD, 0x23)
    a.b(0xF1, 0x3D, 0xF5)
    a.jr(0x20, "measure")
    a.b(0xF1)                                  # pop af
    a.w(b"\x3A", MARKER)                      # ld a,(marker)
    a.w(b"\x32", RESULT)                       # ld (results),a
    a.label("forever")
    a.jr(0x18, "forever")
    # count 15 us iterations while bit 0 is high (exit on low or timeout)
    for name, exit_jr in (("while_high", 0x30), ("while_low", 0x38)):
        a.label(name)
        a.w(b"\x21", 0)                        # ld hl,0
        a.label(name + "_l")
        a.b(0xED, 0x78, 0x1F)                  # in a,(c) : rra       4+1
        a.jr(exit_jr, name + "_x")             # jr nc/c,exit          2
        a.b(0x23, 0x7C, 0xFE, 0xF0)            # inc hl : ld a,h : cp #f0   2+1+2
        a.jr(0x38, name + "_l")                # jr c,loop             3  => 15 us
        a.label(name + "_x")
        a.b(0xC9)                              # ret
    return a.build()


def table(regs):
    out = bytearray()
    for r in sorted(regs):
        out += bytes([r, regs[r]])
    return bytes(out + b"\xFF")


def api(port, path, body=None, timeout=10):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, method="POST" if body is not None else "GET")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read())


def start(emulator, crtc, port):
    env = dict(os.environ, SDL_AUDIODRIVER="dummy")
    if emulator == "bemu":
        command = [BEMU, "--debug-server", "--debug-port", str(port), "--crtc", str(crtc)]
    else:
        command = ["xvfb-run", "-a", AMS, "--crtc", str(crtc), "--turbo", "--no-splash", "--web-port", str(port), "--model", "6128", "-l", "EN", "--reset"]
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env,
                               start_new_session=True)
    for _ in range(120):
        try:
            api(port, "/api/ping")
            break
        except Exception:
            time.sleep(0.5)
    time.sleep(3)  # the firmware boots
    return process


RUNS = [0]


def run(port, regs, budget):
    prog = program()
    RUNS[0] = RUNS[0] % 250 + 1
    marker = RUNS[0]
    api(port, "/api/ram", {"addr": RESULT, "data": "00" * 20})
    api(port, "/api/ram", {"addr": MARKER, "data": bytes([marker]).hex() + table(regs).hex()})
    end = time.time() + budget
    # AMSpiriT applies RAM writes on its own thread, and may jump before the
    # data lands: write everything, wait until applied, only then exec.
    def settle(answer):
        seq = answer.get("seq")
        while seq is not None and time.time() < end:
            if api(port, "/api/ping")["emu"].get("ram_apply_seq", seq) >= seq:
                break
            time.sleep(0.05)
    settle(api(port, "/api/ram", {"addr": PROG, "data": prog.hex()}))
    settle(api(port, "/api/ram", {"addr": PROG, "data": prog.hex(), "exec": True}))
    while time.time() < end:
        raw = bytes.fromhex(api(port, f"/api/ram?addr={RESULT}&len=17")["hex"])
        if raw[0] == marker:
            counts = [raw[1 + 2 * i] | raw[2 + 2 * i] << 8 for i in range(8)]
            return [(counts[i], counts[i + 1]) for i in range(0, 8, 2)]
        time.sleep(0.2)
    return None


def sna(regs):
    h = bytearray(256)
    h[0:8] = b"MV - SNA"
    h[0x10] = 1
    h[0x21:0x23] = (0xC000).to_bytes(2, "little")   # SP
    h[0x23:0x25] = PROG.to_bytes(2, "little")      # PC
    h[0x25] = 1                                     # IM 1
    h[0x2F:0x40] = bytes([0x54] * 17)
    h[0x40] = 0x8D                                  # mode 1, ROMs off
    for r, v in BASE.items():
        h[0x43 + r] = v
    h[0x59] = 0x82                                  # PPI: A out, B in, C out
    h[0x6B] = 64
    ram = bytearray(65536)
    prog = program()
    ram[PROG:PROG + len(prog)] = prog
    t = bytes([0x5A]) + table(regs)
    ram[MARKER:MARKER + len(t)] = t
    return bytes(h + ram)


def run_cpcse(crtc, regs, frames=600):
    with tempfile.TemporaryDirectory() as d:
        s, r = os.path.join(d, "t.sna"), os.path.join(d, "ram.bin")
        open(s, "wb").write(sna(regs))
        subprocess.run([CPCSE, "--shot", os.path.join(d, "s.bmp"), "--sna", s, "--crtc", str(crtc), "--frames", str(frames), "--dumpram", r, "--model", "cpc6128"],
                       cwd=os.path.dirname(CPCSE),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
        if not os.path.exists(r):
            return None
        raw = open(r, "rb").read()[RESULT:RESULT + 17]
    if raw[0] != 0x5A:
        return None
    counts = [raw[1 + 2 * i] | raw[2 + 2 * i] << 8 for i in range(8)]
    return [(counts[i], counts[i + 1]) for i in range(0, 8, 2)]


def describe(result):
    if result is None:
        return "no result (hung)"
    return " ".join(f"{h * LOOP_US}us/{(h + l) * LOOP_US}us" for h, l in result)


def same(results):
    values = list(results.values())
    if any(v is None for v in values):
        return all(v is None for v in values)
    # 1 iteration of slack per value: the loop samples the edges
    return all(abs(x - y) <= 1 for other in values[1:] for pa, pb in zip(values[0], other) for x, y in zip(pa, pb))


PORTS = {"bemu": 6310, "ams": 6320}


def main():
    arg = lambda name, default: sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default
    types = [int(t) for t in arg("--types", "0,1,2,3,4").split(",")]
    only = arg("--only", None)
    emus = arg("--emus", "bemu,ams,cpcse").split(",")
    configs = {k: v for k, v in CONFIGS.items() if only is None or only in k}
    report = []
    for crtc in types:
        procs = {emu: start(emu, crtc, PORTS[emu] + crtc) for emu in emus if emu in PORTS}
        try:
            for name, changes in configs.items():
                regs = {**BASE, **changes}
                results = {}
                for emu in emus:
                    results[emu] = run_cpcse(crtc, regs) if emu == "cpcse" else run(PORTS[emu] + crtc, regs, 25)
                ok = same(results)
                lines = [f"CRTC {crtc} {'ok  ' if ok else 'DIFF'} {name:28} {emus[0]:5}: {describe(results[emus[0]])}"]
                if not ok:
                    lines += [f"{'':40} {emu:5}: {describe(results[emu])}" for emu in emus[1:]]
                print("\n".join(lines), flush=True)
                report.append({"crtc": crtc, "config": name, "regs": regs, "results": results, "same": ok})
        finally:
            # its own process groups only (xvfb-run and AMSpiriT under it), not
            # another AMSpiriT running on the machine
            for process in procs.values():
                os.killpg(process.pid, 9)
            time.sleep(1)
    with open(os.path.join(WORK, "crtc_diff.json"), "w") as out:
        json.dump(report, out, indent=1)


if __name__ == "__main__":
    main()
