# Validation tools (Shaker suite, CPCSyntaxError oracle, demos)

Scripts used to find and fix emulation bugs against the Shaker test suite
(<https://shaker.logonsystem.eu>), the AMSpiriT reference screenshots and
CPCSyntaxError used as a second, headless oracle (study only: never copy its
code). Everything downloaded or generated goes to a work directory outside the
repository: `$SHAKER_WORK` (default `~/shaker_work`).

Run the scripts directly (`./shaker_compare.py ...`, not `python3 -I`: they
import `_work.py` next to them). Needs python3 with numpy and Pillow.

## One-time setup

    ./setup_shaker_work.py            # bundle (csl_load stripped) + tests.json + CRTC 0 references
    ./build_cpcse_oracle.sh           # optional: builds CPCSyntaxError headless (needs cmake, g++)
    pdftotext -layout ../../doc/ACCC1.9-EN.pdf $SHAKER_WORK/accc.txt   # optional, to grep the compendium

## The loop that worked

1. Run one test (about a minute) or a whole module (A ~8 min, B ~45, C ~9):

       ./shaker_one.sh A O 1      # module A, menu key 'O' (AO tests), tag 1
       ./shaker_one.sh B - 1      # the whole module B

   It builds `run_shaker_suite`, plays the (single-key) script, prints the worst
   tests with `shaker_compare.py`. The *menu key is not the test id*: in module A
   `6` runs A7 and `7` runs A6; the keys are the `key_output 'X'` lines of
   `$SHAKER_WORK/bundle/MODULE_X/SHAKER-X-0.CSL`. Some tests depend on the state
   left by the previous ones, so a fresh run can differ from the full run the
   references come from.
2. Look at a test: `./shaker_side.py AO_CRTC0_H:res/.../AO_CRTC0_H.png` writes
   `side.png` (ours left, reference right).
3. Trace the CRTC register writes and compare them with the oracle:

       AMSRUST_TRACE_REG=1 ./shaker_one.sh A O 2        # lines "W R4<-38 C0= C4= C9=" in res_AO_2.log
       cd $SHAKER_WORK/cpcse/build
       CPCSE_TRACE_REG="0,1,4,5,6,9,12,13" CPCSE_TRACE_REG_MAX=3000000 \
         ./cpcse --csl $SHAKER_WORK/bundle_AO/MODULE_A/SHAKER-A-0.CSL --out OUT \
                 --disk-dir $SHAKER_WORK/bundle_AO --crtc 0 --emu-name CPCSE </dev/null 2> cpcse.log
       ./trace_compare.py res_AO_2.log cpcse.log <index where the test starts>

   `three_way.py` compares ours / CPCSE / reference per test (CPCSE named its shots
   `CPCSE_0_<hex SSM code>.bmp`). Other CPCSE traces: CPCSE_TRACE_ACK (interrupt
   acknowledge with the R52 value), CPCSE_TRACE_HSYNC, CPCSE_VSYNC_TRACE.
4. Demos: `validation/demos/*/` hold a disc and a CSL script each;
   `cargo run --release --example run_csl -- validation/demos/X/X.csl OUTDIR`,
   then `demo_compare.py OURS CPCSE_SHOTS` (animated demos differ by timing, a
   static scene that differs is a bug).

## CRTC differential test (bemu, AMSpiriT Lite, CPCSE)

`crtc_diff.py` times VSYNC from inside the machine (PPI port B bit 0: four
lengths and periods) for 27 register sets on each CRTC type, in the three
emulators at once, and prints the sets where they disagree:

    cargo build --release
    ./crtc_diff.py                            # CRTC 0-4, ~45 min
    ./crtc_diff.py --types 2 --only R0=       # a subset
    ./crtc_diff.py --emus bemu,cpcse          # without AMSpiriT

AMSpiriT Lite is the Linux AppImage (`--appimage-extract` into
`$SHAKER_WORK/amspirit`), run headless under `xvfb-run` through its web API;
CPCSE needs `build_cpcse_oracle.sh`. Known disagreements, last run: CRTC 3's
VSYNC width (AMSpiriT always 16 lines, CPCSE and bemu follow R3h), interlace on
CRTC 2/3/4 (not emulated by bemu) and CRTC 2 R0=31 (AMSpiriT lets one ghost
VSYNC out of ~3 through, CPCSE and bemu none).

## Metric

4x8-pixel blocks whose summed channel difference is above 90, reference rows
halved (the site's images are 542 rows: each CPC line doubled). A pixel metric is
dominated by the webp dithering. `_N` suffixed captures are second captures of an
SSM code: the best counts.

## Known results (CRTC 0, last measured)

Module A 2.8 %, B ~5.5 % (truncated run), C 7.6 %; BU (R0=0) 5.4 %. What is left is
mostly *not* a CRTC/GA bug:
- RAM power-on noise (AO_E/F ~12 %), a different disc version for A7 (the
  references show R7 screens, shaker27.dsk prints R6, CPCSE agrees with us);
- the monitor: AMSpiriT has a line AFC that pulls the picture gradually when a
  second HSYNC appears (AT, AY: -10 px plateau, ~30 %/line ramp), a vertical
  placement that depends on history (0/1/2 lines, fresh runs of CPCSE and ours
  give none), and the free-run placement of R0=0/R0=3 frames (B4_H, BCAPS_B1).
  `src/monitor.rs` has none of this yet.

## Debug switches

- `AMSRUST_TRACE_REG=1`: every CRTC register write with C0/C4/C9 (stderr; each
  write appears several times, `trace_compare.py` collapses them).
- Examples worth knowing: `probe_frame_writes`, `probe_rows`, `probe_line_stats`,
  `probe_line_pixels`, `probe_trace`, `dump_ram`, `disasm_bin`, `run_csl`,
  `run_shaker_suite`.
- `tests/instruction_timings.rs` checks every opcode's duration against the
  ACCC chapter 26 table (it found two Z80 timing bugs).
