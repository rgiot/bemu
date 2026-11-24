# Notice

## bemu's own code

bemu's source code is licensed under either of the [MIT licence](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. That covers this repository's code:
the emulator, the web package, the Android app (bemu-droid) and their tools. cpclib
([cpcsdk/rust.cpclib](https://github.com/cpcsdk/rust.cpclib)), from the same author, is under the
MIT licence.

## bemu's binaries: GNU GPL, version 3

The desktop executables and the Android app link the **HxC floppy library** (`hxcfe`, `hxcfe-sys`:
HFE images), which is GPL-3.0-only, with parts under the GPL-2.0-or-later (ADFlib, the FAT File IO
Library, libsap). As combined works, **these binaries are distributed under the
[GNU GPL, version 3](LICENSE-GPL-3.0)**. bemu's own code keeps its MIT / Apache-2.0 licence, which
the GPL-3.0 accepts.

Their corresponding source is:
- this repository, at the release's tag;
- [cpcsdk/rust.cpclib](https://github.com/cpcsdk/rust.cpclib) at the commit the release was built
  from;
- the crates at the versions of `Cargo.lock`, `web/Cargo.lock` and `bemu-droid/rust/Cargo.lock`.

`bemu-droid/rust/build-android.sh` patches two casts of `hxcfe` and `hxcfe-sys` for ARM. The
change is in the script.

The web package does not contain HxC: it is MIT / Apache-2.0 plus the third-party components it
lists.

**bemu's binaries must be given away, not sold.** The CPC's ROMs are distributed with Amstrad's
permission for emulators that are not charged for. HxC contains an LZW packer from Parallax
Software (`xml_disk/packer/lzw.c`) whose licence forbids revenue-bearing use.

## What is not bemu's

Everything bemu's binaries contain and is not its author's, with its licence text, is listed in
[third_party/licenses.json](third_party/licenses.json). The desktop window and the web page show it
in Help > Third-party licences, and the Android app in Settings > Licences.
`tools/licenses.py` writes that file from Cargo's dependency graph (the crates linked into each
build) and from [third_party/extra.json](third_party/extra.json) (what Cargo does not know). The CI
checks that it is up to date.

### The ROMs in `roms/`

They are embedded in the executable (`inner://NAME`, `--list-roms`) so that one file runs
everything. Each is distributed unmodified and stays its authors' property, under its own terms.

| Files | What | Whose |
|---|---|---|
| `os464`, `os664`, `os6128`, `os6128_azerty`, `basic464`, `basic664`, `basic6128`, `basic6128_azerty`, `amsdos` | The CPC's firmware, BASIC and AMSDOS | Amstrad and Locomotive Software. Amstrad has allowed their distribution with emulators that are not charged for, keeping its copyright. |
| `m4rom` | The M4 board's ROM, v2.0.8 | Duke (M4Duke); its source is published at <https://github.com/M4Duke/m4rom>. |
| `unidos`, `unidos-nova`, `unidos-albireo`, `unidos-m4`, `unitools` | UniDOS 1.51, its nodes, UniTools | Futurs' |
| `orgams`, `orgext`, `monogams`, `bricbrac`, `orgass` | The Orgams assembler and its ROMs | The Orgams authors |
| `dmp2000` | The EPROM of the Amstrad DMP2000 printer (its characters; the Spanish version, dumped by the CPC community and published on CPCWiki's DMP2000 page) | Amstrad |
| `rulezcharge` | RulezCharge 0.9b2 for the M4 | CPCrulez (<https://cpcrulez.fr>); its licence is unknown. |

Anyone who owns one of these and does not want it here can ask for its removal. Each sits behind
a Cargo feature (`unidos`, `orgams`, `m4`, `m4-wifi`), so a build can also leave it out.

### Libraries under copyleft or special licences

- **HxC floppy library** (`hxcfe`, `hxcfe-sys`): GPL-3.0-only, see above. Besides ADFlib, the FAT File IO
  Library and libsap (GPL-2.0-or-later), it compiles in the following:
  - zlib and minizip (Zlib);
  - Expat (MIT);
  - LZ4 and xxHash (BSD-2-Clause);
  - libspng (BSD-2-Clause);
  - xDMS (public domain);
  - Dave Dunfield's Teledisk decoder (his terms);
  - Parallax Software's LZW packer (non-commercial).
- **nuked-opl3** (Willy's OPL3: the `willy` feature): [GNU LGPL 2.1 or later](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html),
  linked statically. The source offer above lets its users relink it against a modified nuked-opl3.
  A build without the `willy` feature leaves it out.
- **buildid** (MPL-2.0), used unmodified.
- **egui's default fonts** (OFL-1.1, Ubuntu Font Licence) are in the desktop and web builds.
- **webpki-roots** (CDLA-Permissive-2.0) and the Unicode-3.0 crates (ICU4X) are used too.
- **The Android app** also contains the following:
  - LLVM's libc++ (Apache-2.0 WITH LLVM-exception);
  - AndroidX, Jetpack Compose, the Kotlin standard library and kotlinx.coroutines (Apache-2.0).
- The other dependencies are under permissive licences (MIT, Apache-2.0, BSD, ISC, Zlib...). All are
  in `third_party/licenses.json`.

bemu's builds leave out the following parts of cpclib, because some of their licences forbid commercial
use or are not stated. bemu uses cpclib without their features (`Cargo.toml`).
- **The compressors** (`cpclib-crunchers`: Exomizer, Shrinkler, PuCrunch...).
- **cpclib-asm's `inner://` include files** (`cpclib-asm/assets`): Z80 decrunchers by various authors and
  firmware labels. Its `embedded-files` feature puts them in.

## Test data

The tests build their own data (a Dandanator romset, a SoundFont, cpcrulez.fr's replies, Z80
programs, discs). Besides the discs of `validation/`, no third-party romset, disc or tape is bundled
for them.
