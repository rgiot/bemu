#!/usr/bin/env python3
"""The inventory of what bemu's binaries contain and is not bemu's own, with the licence texts.

    tools/licenses.py            writes third_party/licenses.json
    tools/licenses.py --check    fails when third_party/licenses.json is not what it would write

The Help > Third-party licences window (src/ui/licences.rs) and the Android app's Licences screen
read that file. It lists, for each build (desktop, web, android):

- the Rust crates linked into it: the normal dependencies of the desktop crate, of web/ and of
  bemu-droid/rust, as Cargo resolves them for each build's targets. Build and dev dependencies and
  proc-macros run at build time only and are left out, and so are the packages of this repository
  and of cpclib (../rust.cpcdemotools), which are bemu's author's;
- what Cargo does not know (third_party/extra.json): the ROMs, the C libraries compiled inside
  hxcfe-sys, the Android runtime and libraries.

Each licence text is a file of the package (LICENSE*, COPYING*, NOTICE*...), else the standard text
of its SPDX licence in third_party/spdx/. A package with neither is an error: its licence must be
looked at before it ships.

Needs cargo and cpclib next to the repository (as the build does). Plain Python 3, no packages.
"""

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "third_party" / "licenses.json"
EXTRA = ROOT / "third_party" / "extra.json"
SPDX = ROOT / "third_party" / "spdx"

# the builds: (name, the crate's folder, its targets)
BUILDS = [
    ("desktop", ROOT, ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-pc-windows-msvc"]),
    ("web", ROOT / "web", ["wasm32-unknown-unknown"]),
    ("android", ROOT / "bemu-droid" / "rust", ["aarch64-linux-android", "x86_64-linux-android"]),
]

LICENCE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)([-._].*)?$", re.IGNORECASE)


def owned(package):
    """A package of this repository or of cpclib: a path dependency (the registry's have a source)."""
    return package.get("source") is None


def metadata(folder):
    command = ["cargo", "metadata", "--format-version", "1"]
    result = subprocess.run(command, cwd=folder, capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def linked_packages(folder, target, meta):
    """The packages linked into the crate's binary for `target`: its normal dependencies,
    transitively, as `cargo tree` resolves them (the features of each target, which `cargo
    metadata` does not tell apart); proc-macros and what they use run in the compiler only."""
    command = ["cargo", "tree", "-e", "normal,no-proc-macro", "--target", target, "--prefix", "none",
               "--format", "{p}"]
    result = subprocess.run(command, cwd=folder, capture_output=True, text=True, check=True)
    packages = {(p["name"], p["version"]): p for p in meta["packages"]}
    linked = {}
    for line in result.stdout.splitlines():
        match = re.match(r"^(\S+) v(\S+)", line)
        if match:
            package = packages[(match.group(1), match.group(2))]
            linked[package["id"]] = package
    return list(linked.values())


def normalise(expression):
    """`MIT/Apache-2.0` (the old Cargo way) as `MIT OR Apache-2.0`."""
    expression = (expression or "").strip()
    return re.sub(r"\s*/\s*", " OR ", expression)


def spdx_ids(expression):
    words = re.findall(r"[A-Za-z0-9.+-]+", expression)
    return [w for w in words if w not in ("OR", "AND", "WITH")] if expression else []


def spdx_text(identifier):
    path = SPDX / f"{identifier}.txt"
    return path.read_text(encoding="utf-8") if path.exists() else None


def read_text(path):
    return path.read_bytes().decode("utf-8", errors="replace")


def clean(text):
    return "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").strip().split("\n")) + "\n"


def package_texts(package):
    """The licence files of a package, else the SPDX texts of its licence; [] when there are none."""
    folder = Path(package["manifest_path"]).parent
    files = sorted(p for p in folder.iterdir() if p.is_file() and LICENCE_FILE.match(p.name))
    if package.get("license_file"):
        named = folder / package["license_file"]
        if named.is_file() and named not in files:
            files.append(named)
    if files:
        return [(f.name, read_text(f)) for f in files]
    ids = spdx_ids(normalise(package.get("license")))
    texts = [(f"{i} (SPDX)", spdx_text(i)) for i in ids if not i.startswith("LLVM-exception")]
    if not ids or any(text is None for _, text in texts):
        return []
    return texts


def package_folders(metas):
    """name-version → the folder of a registry package (for extra.json's files taken from a crate)."""
    folders = {}
    for meta in metas:
        for p in meta["packages"]:
            folders[f'{p["name"]}-{p["version"]}'] = Path(p["manifest_path"]).parent
    return folders


def inventory():
    components, texts, missing, metas = {}, {}, [], []

    def keep(text):
        text = clean(text)
        key = hashlib.sha256(text.encode()).hexdigest()[:16]
        texts[key] = text
        return key

    for build, folder, targets in BUILDS:
        meta = metadata(folder)
        metas.append(meta)
        for target in targets:
            for package in linked_packages(folder, target, meta):
                if owned(package):
                    continue
                key = ("crate", package["name"], package["version"])
                component = components.get(key)
                if component is None:
                    found = package_texts(package)
                    if not found:
                        missing.append(f'{package["name"]} {package["version"]} ({package.get("license")})')
                    component = {
                        "kind": "crate",
                        "name": package["name"],
                        "version": package["version"],
                        "license": normalise(package.get("license")) or "see the text",
                        "owner": ", ".join(package.get("authors") or []) or None,
                        "url": package.get("repository") or package.get("homepage")
                        or f'https://crates.io/crates/{package["name"]}',
                        "builds": [],
                        "texts": [{"name": name, "text": keep(text)} for name, text in found],
                    }
                    components[key] = component
                if build not in component["builds"]:
                    component["builds"].append(build)

    folders = package_folders(metas)
    for entry in json.loads(EXTRA.read_text(encoding="utf-8"))["components"]:
        found = []
        for source in entry.pop("text_files", []):
            if "crate" in source:
                base = folders.get(source["crate"])
                if base is None:
                    missing.append(f'{entry["name"]}: crate {source["crate"]} not in the builds')
                    continue
                path = base / source["path"]
            else:
                path = ROOT / source["path"]
            if not path.is_file():
                missing.append(f'{entry["name"]}: {path} missing')
                continue
            found.append((source.get("title", path.name), read_text(path)))
        for identifier in entry.pop("spdx_texts", []):
            text = spdx_text(identifier)
            if text is None:
                missing.append(f'{entry["name"]}: no third_party/spdx/{identifier}.txt')
                continue
            found.append((f"{identifier} (SPDX)", text))
        if "text" in entry:
            found.append(("Terms", entry.pop("text")))
        if not found:
            missing.append(f'{entry["name"]}: no text')
        entry["texts"] = [{"name": name, "text": keep(text)} for name, text in found]
        entry.setdefault("version", "")
        components[(entry["kind"], entry["name"], entry["version"])] = entry

    order = {"rom": 0, "c-library": 1, "crate": 2, "android": 3}
    listed = sorted(
        components.values(),
        key=lambda c: (order.get(c["kind"], 9), c["name"].lower(), c.get("version", "")),
    )
    for component in listed:
        component["builds"] = sorted(component["builds"], key=["desktop", "web", "android"].index)
    used = {t["text"] for c in listed for t in c["texts"]}
    return {
        "note": "Generated by tools/licenses.py: do not edit (third_party/extra.json is the hand-written part).",
        "components": listed,
        "texts": {k: v for k, v in sorted(texts.items()) if k in used},
    }, missing


def main():
    data, missing = inventory()
    if missing:
        print("No licence text for:\n  " + "\n  ".join(sorted(set(missing))), file=sys.stderr)
        print("Add the SPDX text to third_party/spdx/ or the component to third_party/extra.json.", file=sys.stderr)
        sys.exit(1)
    text = json.dumps(data, indent=1, ensure_ascii=False, sort_keys=False) + "\n"
    if "--check" in sys.argv:
        if not OUT.exists() or OUT.read_text(encoding="utf-8") != text:
            print("third_party/licenses.json is out of date: run tools/licenses.py", file=sys.stderr)
            sys.exit(1)
        print(f"third_party/licenses.json is up to date ({len(data['components'])} components)")
        return
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)}: {len(data['components'])} components, {len(data['texts'])} texts")


if __name__ == "__main__":
    main()
