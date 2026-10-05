# Module B "BR" reference screenshots (SHAKER KILLER / R52 interrupt tests)

Captured 2026-10-05 from **AMSpiriT Lite 1.13.4** (the reference emulator,
not amsrust) running the real `shaker27.dsk`, to get ground-truth data for
the open ACCC §27.3.2 "R52 bit 5" interrupt-spacing question - see
`project_crtc_timing_core_rewrite` memory. These are the `BR` module's
SSM-triggered screenshots (manifest: `INTERRUPT DELAY FROM R2 (SHAKER
KILLER)`, hex codes 0x01F0-0x01F5 for subtests A-F; `F` - "Otir Zone" - is
a decorative "for fun" screen, not reproduced here).

- `BR_A_z80aintb.png` - general Z80 interrupt-acceptance timing table
  across various instructions.
- `BR_B_r52reset.png` - **the key one**: "R52 BIT 5 RESET (R2=46/R3=8/
  CLOSE END HSYNC) FROM R52=#1F to #20". Measures inter-interrupt timing
  (`FROM LAST INTER` / `NEXT INTER`) across three sub-cases (EI/NOP,
  EI/CP(HL), EI/EX(SP),HL) - every measured value exactly matches its own
  `(Exp:...)` expectation, confirming this is self-verified-correct
  reference data, not just a raw unverified trace.
- `BR_C_inter_test_im1.png` - HSYNC (C0=R2) to IM1-interrupt delay vs. R3,
  plus VSYNC duration and RST#38 entry latency figures.
- `BR_D_inter_test_im2.png` - same shape, IM2.
- `BR_E_r52inc.png` - "R52INC IN HSYNC": R0=1/R3=1 and R0=0/R3=1 edge
  cases both calculate R52=0x34 for CRTC0, matching their own `(Exp:...)`.

## How these were generated

`amspirit_shaker_b_through_br.csl` in this directory is
`MODULE_B/SHAKER-B-0.CSL` lines 1-253 (boot + the full B1 test + the 'R'
menu selection reaching BR) with the `disk_insert` line removed - the
disc was instead preloaded via AMSpiriT's own CLI positional argument.
Reaching BR this way (not jumping to the 'R' key directly from a fresh
boot) is necessary: the test menu is **not** a free-access hotkey list -
pressing 'R' before B1 has run produces nothing (confirmed by trying it).

```
Amspirit-Lite-SDL-ImGui-*.AppImage validation/Shaker_CSL/shaker27.dsk \
    --crtc 0 --turbo --ssm --ssm-both -v \
    --script amspirit_shaker_b_through_br.csl \
    --save-log amspirit_shaker_br.log
```

`--turbo` (no 50Hz pacing) is what makes this tractable at all: a prior
*amsrust* attempt at the full module (not turbo, not AMSpiriT) spent 5.8
real hours just reaching the end of B1 alone. AMSpiriT in turbo mode
reaches BR (through the whole B1 sequence first) in well under a minute.
SSM-triggered screenshots land in AMSpiriT's current working directory
as `AMSPIRIT_<crtc>_<hex code>.png`/`.sna`.

## What's still needed to close the R52 question

`examples/debug_r52_bit5.rs` (repo root) reproduces the exact R2=46/R3=8
CRTC0 setup in amsrust and logs every R52 reset, split by which of the
two reset paths fired (the unconditional "R52==52" path vs. the VSYNC-
boundary bit-5-dependent path currently gated the *opposite* way from
the ACCC's literal wording - see the code comment at `GateArray::
manage_hsync_end`). It confirms the VSYNC-boundary path does fire
(alternating bit5-set/bit5-clear roughly every other VSYNC in this
setup), but doesn't yet measure inter-interrupt *timing* the way
`BR_B_r52reset.png` does, so it can't yet say which polarity actually
reproduces AMSpiriT's `(Exp:#0504)`-style numbers. That replication -
matching `BR_B_r52reset.png`'s exact EI/NOP, EI/CP(HL), EI/EX(SP),HL
measurement sequence and comparing against both polarities - is the
concrete next step before touching `manage_hsync_end`'s `if self.r52.
bit(5)` condition either way.
