#!/usr/bin/env python3
"""AMSpiriT Lite references for the Shaker suite, on every CRTC type.

  ams_shaker_refs.py [modules] [crtcs]       e.g.  ams_shaker_refs.py AC 1234

The Shaker site only publishes CRTC 0 references (setup_shaker_work.py). This
plays the suite's own `SHAKER-<module>-<crtc>.CSL` scripts in AMSpiriT Lite
(headless, under xvfb-run, in real time: with --turbo its CSL waits do not
follow the emulated time and screens are skipped) with SSM on: the program asks for its screenshots
itself (ED LL ED HH), AMSpiriT saves them as AMSPIRIT_<crtc>_<HHLL>.png, and
they are renamed `<test id>_CRTC<n>_<subtest>.png` (shakerland_manifest.json)
into $SHAKER_WORK/ref/ams_lite/, where shaker_compare.py finds them:

  REFS_DIR=$SHAKER_WORK/ref/ams_lite CRTC=1 ./shaker_compare.py RESULTS_DIR

The scripts are adapted on the way: their `csl_load` chaining is removed and
the program names follow the disc (the -1..-4 scripts still RUN"SHAKE26x",
shaker27.dsk holds SHAKE27x), and AMSpiriT's waits and key delays are scaled
to its timing (see ams_timing). The same scripts, as the `-0` of each module, make
$SHAKER_WORK/bundle_c<n>, for bemu's side of the comparison:

  BUNDLE_DIR=bundle_c1 ./shaker_one.sh A - 1
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time

from _work import REPO, WORK

AMS = WORK / "amspirit" / "squashfs-root" / "AppRun"
SHAKER = REPO / "validation" / "Shaker_CSL"
OUT = WORK / "ref" / "ams_lite"


def script(module, crtc):
    text = (SHAKER / f"MODULE_{module}" / f"SHAKER-{module}-{crtc}.CSL").read_text(encoding="latin-1")
    lines = [l for l in text.splitlines() if not re.match(r"\s*csl_load", l)]
    text = "\n".join(lines) + "\n"
    return re.sub(r"shaker26\.dsk", "shaker27.dsk", re.sub(r"SHAKE26([A-E])", r"SHAKE27\1", text))


def ams_timing(text):
    """AMSpiriT Lite 1.16 waits half the CSL `wait` and takes twice the `key_delay`
    (its --debug-script trace: wait(10000000) lasts 5 s, a key 280 ms instead of
    140): compensate, so that keys land when the scripts expect them."""
    text = re.sub(r"(?m)^(\s*wait\s+)(\d+)", lambda m: f"{m.group(1)}{int(m.group(2)) * 2}", text)
    return re.sub(r"(?m)^(\s*key_delay\s+)([\d \t]+)",
                  lambda m: m.group(1) + " ".join(str(int(x) // 2) for x in m.group(2).split()), text)


def names(module):
    manifest = json.loads((SHAKER / "shakerland_manifest.json").read_text())
    return {
        int(s["hex"], 16): f"{t['id']}_CRTC{{crtc}}_{s['subTest']}"
        for t in manifest["tests"] if t["module"] == module for s in t["subtests"]
    }


def run(module, crtc):
    work = WORK / f"ams_{module}{crtc}"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    shutil.copy(SHAKER / "shaker27.dsk", work / "shaker27.dsk")
    (work / "run.csl").write_text(ams_timing(script(module, crtc)), encoding="latin-1")
    env = dict(os.environ, SDL_AUDIODRIVER="dummy")
    command = ["xvfb-run", "-a", str(AMS), "--reset", "--no-splash", "--crtc", str(crtc),
               "--model", "6128", "-l", "EN", "--web-port", str(6400 + crtc), "--ssm",
               "--script", "run.csl", "--save-log", "ams.log"]
    process = subprocess.Popen(command, cwd=work, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               start_new_session=True)
    start = time.time()
    # the script ends with the module; AMSpiriT itself stays up
    while time.time() - start < 3 * 3600:
        log = (work / "ams.log").read_text(errors="replace") if (work / "ams.log").exists() else ""
        if "CSL: script completed" in log or process.poll() is not None:
            break
        time.sleep(5)
    time.sleep(2)
    os.killpg(process.pid, 9)
    OUT.mkdir(parents=True, exist_ok=True)
    known, count = names(module), 0
    for png in sorted(work.glob(f"AMSPIRIT_{crtc}_*.png")):
        code = int(png.stem.rsplit("_", 1)[1], 16)
        stem = known.get(code, f"{module}_CRTC{{crtc}}_{code:04X}").format(crtc=crtc)
        shutil.copy(png, OUT / f"{stem}.png")
        count += 1
    print(f"{module} CRTC {crtc}: {count} screenshots in {time.time() - start:.0f} s", flush=True)


def bundle(crtc):
    """$SHAKER_WORK/bundle_c<crtc>: each module's CRTC <crtc> script as its -0."""
    dest = WORK / f"bundle_c{crtc}"
    shutil.rmtree(dest, ignore_errors=True)
    for module in "ABCDE":
        (dest / f"MODULE_{module}").mkdir(parents=True)
        (dest / f"MODULE_{module}" / f"SHAKER-{module}-0.CSL").write_text(script(module, crtc), encoding="latin-1")
        shutil.copy(SHAKER / "shaker27.dsk", dest / f"MODULE_{module}" / "shaker27.dsk")
    shutil.copy(SHAKER / "shaker27.dsk", dest / "shaker27.dsk")


def main():
    modules = sys.argv[1] if len(sys.argv) > 1 else "ABCDE"
    crtcs = sys.argv[2] if len(sys.argv) > 2 else "01234"
    for crtc in crtcs:
        bundle(int(crtc))
    for module in modules:
        for crtc in crtcs:
            run(module, int(crtc))


if __name__ == "__main__":
    main()
