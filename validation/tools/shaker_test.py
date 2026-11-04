#!/usr/bin/env python3
"""Shaker regression test: does a version of bemu score worse than another one?

    ./shaker_test.py                         the working tree (uncommitted changes included) vs master
    ./shaker_test.py --base v1 --head HEAD   two commits (branches, tags, SHAs)
    ./shaker_test.py --modules A C           some modules only (default A B C)

Each version is built in a temporary git worktree (your checkout is never switched; with no
--head, the working tree itself is built), plays the Shaker modules (CSL scripts, CRTC 0) and
the two versions' screens are compared pixel for pixel, then scored against the AMSpiriT
references (shaker_compare.py's metric: the share of 4x8-pixel blocks that differ). The modules
run in parallel. Prints the screens that changed, each module's mean and every test whose score
changed, and exits with 1 if a test got worse by more than --threshold (percentage points).
Without the references (they couldn't be downloaded), any changed screen exits with 1.

The results of a commit are kept in $SHAKER_WORK/regress/<sha>/ and reused (the base is played
once); --fresh plays them again. The first run sets up $SHAKER_WORK (setup_shaker_work.py: the
references are downloaded). Times: module A ~8 min, B ~45, C ~9.
"""
import argparse
import concurrent.futures
import os
import shutil
import subprocess
import sys

import numpy as np
from PIL import Image

from _work import BUNDLE, REFS, REPO, WORK
from shaker_compare import scores

HERE = os.path.dirname(os.path.abspath(__file__))
REGRESS = WORK / "regress"
# one target directory for every version: the dependencies are only built once
TARGET = REGRESS / "target"
RUNNER = "run_shaker_suite"


def git(*args, cwd=REPO):
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def build(source, destination):
    """Builds the Shaker runner from `source` and copies it to `destination`."""
    print(f"  building in {source}", flush=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(TARGET))
    subprocess.run(["cargo", "build", "--release", "--example", RUNNER], cwd=source, env=env, check=True)
    shutil.copy2(TARGET / "release" / "examples" / RUNNER, destination)


def runner_for(ref, fresh):
    """The results directory of `ref` (None: the working tree), with its runner built."""
    if ref is None:
        directory = REGRESS / "worktree"
        shutil.rmtree(directory, ignore_errors=True)
        directory.mkdir(parents=True)
        build(REPO, directory / RUNNER)
        return directory
    sha = git("rev-parse", "--verify", ref + "^{commit}")
    directory = REGRESS / sha[:12]
    if fresh:
        shutil.rmtree(directory, ignore_errors=True)
    if (directory / RUNNER).exists():
        return directory
    directory.mkdir(parents=True, exist_ok=True)
    # next to the repository: Cargo.toml finds cpclib at ../rust.cpcdemotools
    tree = REPO.parent / f".bemu-shaker-{sha[:12]}"
    shutil.rmtree(tree, ignore_errors=True)
    git("worktree", "add", "--detach", str(tree), sha)
    try:
        build(tree, directory / RUNNER)
    finally:
        git("worktree", "remove", "--force", str(tree))
    return directory


def play(directory, module):
    """Plays `module` with the runner of `directory`, unless already done: its results dir."""
    results = directory / module
    done = directory / f"{module}.done"
    if done.exists():
        return results
    shutil.rmtree(results, ignore_errors=True)
    results.mkdir()
    with open(directory / f"{module}.log", "w") as log:
        subprocess.run([str(directory / RUNNER), str(BUNDLE), str(results), module], stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    done.touch()
    return results


def changed_screens(before, after):
    """The captures (relative paths) whose pixels differ between two runs, or that one lacks."""
    names = lambda root: {p.relative_to(root) for p in root.rglob("*.png")}
    a, b = names(before), names(after)
    moved = [str(n) + (" (missing in head)" if n not in b else " (new in head)") for n in sorted(a ^ b)]
    for name in sorted(a & b):
        if not np.array_equal(np.asarray(Image.open(before / name)), np.asarray(Image.open(after / name))):
            moved.append(str(name))
    return moved


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", default="master", help="the reference version (default master)")
    parser.add_argument("--head", help="the version tested (default: the working tree)")
    parser.add_argument("--modules", nargs="+", default=["A", "B", "C"])
    parser.add_argument("--threshold", type=float, default=0.1,
                        help="worse by more than this (percentage points) fails (default 0.1)")
    parser.add_argument("--fresh", action="store_true", help="play cached commits again")
    args = parser.parse_args()

    if not BUNDLE.exists() or not any(REFS.glob("*")):
        print("setting up", WORK, flush=True)
        setup = subprocess.run([sys.executable, os.path.join(HERE, "setup_shaker_work.py")])
        if setup.returncode != 0:
            print("the references couldn't be fetched: the screens are only compared between the versions")
    have_refs = any(REFS.glob("*"))

    print(f"base {args.base}", flush=True)
    base = runner_for(args.base, args.fresh)
    print(f"head {args.head or 'working tree'}", flush=True)
    head = runner_for(args.head, args.fresh)

    jobs = [(side, directory, module) for side, directory in (("base", base), ("head", head))
            for module in args.modules]
    print(f"playing {len(jobs)} module runs (cached ones are skipped)", flush=True)
    results = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
        futures = {pool.submit(play, directory, module): (side, module) for side, directory, module in jobs}
        for future in concurrent.futures.as_completed(futures):
            side, module = futures[future]
            results[side, module] = future.result()
            print(f"  {side} {module} played", flush=True)

    worse = []
    changed = []
    for module in args.modules:
        moved = changed_screens(results["base", module], results["head", module])
        changed += moved
        print(f"\nmodule {module}: {len(moved)} screen(s) differ between the versions")
        for name in moved:
            print(f"  {name}")
        if not have_refs:
            continue
        before = scores(results["base", module])
        after = scores(results["head", module])
        mean = lambda s: np.mean([m for m, _ in s.values()]) * 100 if s else float("nan")
        print(f"module {module} against the references: mean {mean(before):.2f} % -> {mean(after):.2f} % "
              f"({len(before)} -> {len(after)} tests scored)")
        for test in sorted(set(before) | set(after)):
            b = before[test][0] * 100 if test in before else None
            a = after[test][0] * 100 if test in after else None
            if b is None or a is None:
                print(f"  {test}: {'missing' if a is None else 'new'} in head")
                if a is None:
                    worse.append(test)
                continue
            if abs(a - b) > 1e-9:
                tag = "WORSE" if a - b > args.threshold else "better" if a < b else "worse (within threshold)"
                print(f"  {test}: {b:5.1f} -> {a:5.1f}  {tag}")
                if a - b > args.threshold:
                    worse.append(test)

    print()
    if not have_refs:
        if changed:
            print(f"{len(changed)} screen(s) changed, no references to tell better from worse: "
                  f"look at them (results in {head})")
            sys.exit(1)
        print("no change")
        return
    if worse:
        print(f"REGRESSION: {len(worse)} test(s) worse: {' '.join(worse)}")
        print(f"look at one: ./shaker_side.py <capture>:<reference> (results in {head})")
        sys.exit(1)
    print("no regression")


if __name__ == "__main__":
    main()
