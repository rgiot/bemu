#!/usr/bin/env python3
"""A bemu CSL script replaying what AMSpiriT Lite did, at the same emulated times.

AMSpiriT stretches each `wait` a little (tens of ms: it acts on frame boundaries), so the same
script presses keys at other frames in the two emulators and a game soon diverges. Its log,
written with `--debug-script --save-log ams.log`, gives the emulated microseconds between the
instructions (`CSL:6: +339454 keyboard_write(...)`); this script turns them into `wait`s.

  amspirit_replay.py ams.log out.csl [--cart 'game.cpr'] [--shift US]

`--cart` starts the script with a GX4000 and that cartridge (else the header is `reset` alone);
`--shift` moves everything after the reset by US microseconds. A `keyboard_write` that only
holds line 9 becomes `joystick 0 ...` (CSL 1.6).
"""
import argparse
import re

JOY = (("up", 0), ("down", 1), ("left", 2), ("right", 3), ("fire2", 4), ("fire1", 5), ("fire3", 6))

parser = argparse.ArgumentParser()
parser.add_argument("log")
parser.add_argument("out")
parser.add_argument("--cart")
parser.add_argument("--shift", type=int, default=0)
args = parser.parse_args()

lines = ["csl_version 1.6"]
if args.cart:
    lines += ["cpc_model 6", f"rom_config C 0 '{args.cart}'"]
lines.append("reset")
shift = args.shift
for line in open(args.log, encoding="utf-8", errors="replace"):
    m = re.search(r"CSL:\d+: \+(\d+) (\w+)\((.*)\)\s*$", line)
    if not m:
        continue
    delta, name, params = int(m.group(1)), m.group(2), m.group(3)
    if name in ("csl_version", "reset", "wait", "cpc_model", "rom_config"):
        continue
    delta, shift = delta + shift, 0
    if delta:
        lines.append(f"wait {delta}")
    if name == "keyboard_write":
        rows = [int(v) for v in params.split(",")]
        if all(r == 255 for r in rows[:9]) and rows[9] & 0x80:
            held = [word for word, bit in JOY if not rows[9] >> bit & 1]
            lines.append(" ".join(["joystick 0"] + held))
        else:
            lines.append("keyboard_write " + ",".join(map(str, rows)))
    elif name in ("screenshot_name", "key_output", "screenshot_dir"):
        lines.append(f"{name} {params}")
    elif name in ("screenshot", "snapshot"):
        lines.append(f"{name} {params}".strip())
    else:
        raise SystemExit(f"amspirit_replay: {name} is not handled")
open(args.out, "w").write("\n".join(lines) + "\n")
