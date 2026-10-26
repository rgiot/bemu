# Notice

bemu's own code is licensed under either of the [MIT licence](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. What follows is not bemu's own and
is not covered by that licence.

## The ROMs in `roms/`

They are embedded in the executable (`inner://NAME`, `--list-roms`) so that one file runs
everything. Each stays its authors' property, under its own terms.

| Files | What | Whose |
|---|---|---|
| `os464`, `os664`, `os6128`, `os6128_azerty`, `basic464`, `basic664`, `basic6128`, `basic6128_azerty`, `amsdos` | The CPC's firmware, BASIC and AMSDOS | Amstrad and Locomotive Software. Amstrad has allowed their distribution with emulators, keeping its copyright. |
| `m4rom` | The M4 board's ROM, v2.0.8 | Duke (M4Duke); its source is published at <https://github.com/M4Duke/m4rom>. |
| `unidos`, `unidos-nova`, `unidos-albireo`, `unidos-m4`, `unitools` | UniDOS 1.51, its nodes, UniTools | Futurs' |
| `orgams`, `orgext`, `monogams`, `bricbrac`, `orgass` | The Orgams assembler and its ROMs | The Orgams authors |
| `rulezcharge` | RulezCharge 0.9b2 for the M4 | CPCrulez (<https://cpcrulez.fr>); its licence is unknown. |

Anyone who owns one of these and does not want it here can ask for its removal; each sits behind
a Cargo feature (`unidos`, `orgams`, `m4`, `m4-wifi`), so a build can also leave it out.

## Libraries under other licences

- **nuked-opl3** (Willy's OPL3: the `willy` feature) is under the
  [GNU LGPL 2.1 or later](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html). It is linked
  statically: a binary built with it must let its users relink it against a modified
  nuked-opl3 (for example by providing bemu's object files or its source), or be built without
  the `willy` feature (`cargo build --no-default-features --features ...`).
- The other dependencies are under permissive licences (MIT, Apache-2.0, BSD, Zlib...): see
  `cargo tree` and each crate's licence.

## Test data

The tests build their own data (a Dandanator romset, a SoundFont, cpcrulez.fr's replies, Z80
programs). No third-party romset, disc or tape is bundled for them.
