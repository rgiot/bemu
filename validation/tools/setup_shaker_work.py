#!/usr/bin/env python3
"""Prepare the work directory used by the other tools (see _work.py).

  setup_shaker_work.py            bundle + tests.json + CRTC 0 references
  setup_shaker_work.py --no-refs  only the bundle (no network)

The bundle is validation/Shaker_CSL with the `csl_load` lines removed from the
-0 scripts, so that each module is played alone (the suite chains the five
CRTC types through csl_load; we only run CRTC 0).
"""
import json
import re
import shutil
import sys
import urllib.request

from _work import BUNDLE, REFS, REPO, TESTS, WORK

SITE = "https://shaker.logonsystem.eu"
REF_URL = SITE + "/images/amspirit/lite-1.15/{id}_CRTC0_{sub}.webp"


def make_bundle():
    src = REPO / "validation" / "Shaker_CSL"
    if BUNDLE.exists():
        shutil.rmtree(BUNDLE)
    shutil.copytree(src, BUNDLE)
    for m in "ABCD":
        p = BUNDLE / f"MODULE_{m}" / f"SHAKER-{m}-0.CSL"
        lines = p.read_text(encoding="latin-1").splitlines()
        p.write_text("\n".join(l for l in lines if not re.match(r"\s*csl_load", l)) + "\n", encoding="latin-1")
    print("bundle:", BUNDLE)


def fetch(url, dest):
    if dest.exists() and dest.stat().st_size > 0:
        return True
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            dest.write_bytes(r.read())
        return True
    except Exception as e:  # missing reference for this subtest
        print("  no", url.rsplit("/", 1)[-1], e)
        return False


def fetch_refs():
    WORK.mkdir(parents=True, exist_ok=True)
    REFS.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(SITE + "/api/tests", timeout=30) as r:
        TESTS.write_bytes(r.read())
    tests = json.loads(TESTS.read_text())
    n = 0
    for t in tests:
        for s in t["subtests"]:
            if 0 in s["crtcs"]:
                name = f"{t['id']}_CRTC0_{s['subTest']}"
                n += fetch(REF_URL.format(id=t["id"], sub=s["subTest"]), REFS / f"{name}.webp")
    print(f"{n} references in {REFS}")


if __name__ == "__main__":
    WORK.mkdir(parents=True, exist_ok=True)
    make_bundle()
    if "--no-refs" not in sys.argv:
        fetch_refs()
