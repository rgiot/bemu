# validation/fuzz

Differential fuzzing and timing probes (see the memory notes of the project for the story).

* `gen_crtc_fuzz.py`, `compare_lines.py`, `shrink.py`: random CRTC register programs run in amsrust and in
  CPCSyntaxError (`$CPCSE`), compared line by line (C4, C9, VSYNC, length). `FUZZ_LOG=1`, `FUZZ_EVENTS=[...]`,
  `FUZZ_NO_R0=1`, `FUZZ_IVM=1`, `FUZZ_TINY_R0=1`. CPCSyntaxError is NOT ground truth.
* `gen_r0_probe.py`, `gen_r7_probe.py`: Z80 programs leaving a result at #8000 (`examples/probe_sna_ram`,
  `cpcse_ram.py`, or AMSpiriT `/api/ram?addr=32768&len=1&view=cpu`).
* `gen_pull_probe.py`, `gen_pull2_probe.py`, `gen_mode_probe.py`, `gen_hsync_probe.py`, `gen_ink_probe.py`:
  striped screens with extra HSYNC pulses (per line) to see how the monitor reacts. Picture compared with
  AMSpiriT screenshots (`/api/screenshot`). `examples/probe_hsync_lines` prints the pulses, mode changes and
  (RGB=1) colours of each line in amsrust; pick a `phase` for which the pulses are where wanted.

Findings against AMSpiriT (single extra pulse, 200 lines of stripes):
* a pulse BEFORE the line sync does not pull the picture (removed from the monitor model);
* a pulse the locked line oscillator does not listen to does not blank the beam either (monitor model);
* with a main sync as narrow as the extra pulse, a restart happens 3 + w/2 us after the pulse start for
  w = 3..5 us (6 us for w >= 6), and 6.5 / 7.5 us for w = 1 / 2; amsrust restarts at the end of its C-HSYNC
  window. NOT applied: it makes Shaker AY_F worse (cut pulses), so the rule must depend on how the pulse ends.
* HBL panel (solved): a narrow (R3l=2) extra HSYNC pulse is 1 us EARLIER than amsrust had it: the black box behind
  the "B" started 15 px late. `gen_mode_probe.py` (mode 1 written before a 2 us pulse) showed the mode edge 16 px
  earlier in AMSpiriT; Shaker AR also improves (4.2 % -> 2.6 %). The Crtc starts such a pulse one character
  early (R3l=2) - unless R3 is raised on the character R2 names (HBL's main sync is begun with R3=2 and widened
  at once by an R3 write: it keeps its place and the Gate Array counts it from the write).
* Waving screens (`gen_wave_probe.py`, R2 from a table, one entry per line; What a Cool Weekend part 2): the
  monitor's line oscillator is a PLL, not a hard lock. A sync that is `e` ticks off pulls the start of the line by
  3/8 of `e`, at most 12 ticks a line (step 16 px: 6,4,2,2,1 px; sine: smooth). Early capture 48, late capture
  36 ticks (B0_B has late pulses of 37-80 that AMSpiriT ignores). A late pulse of 1-2 ticks must be ignored
  (applying that jitter broke Shaker A). Probes match AMSpiriT within 2-3 px on average.

Second HSYNC far ahead of the line sync (`gen_pull3_probe.py`, `pull_measure.py`; Onescreen Colonies #1):
* the probe synchronises once and loops on exactly one frame, the extra pulse on chosen lines only (a table),
  CRTC 1, line sync at C0=50; phase 24. Compare each emulator with its own baseline (`X=-`).
* AMSpiriT Lite 1.16: a pulse 14 to 28 characters ahead pulls the picture right by 3, 6, 10, 13 px (1, 2,
  3, 4 us wide), half of it 30 ahead, nothing from 33; the beam covers 3/8 of the way each line (1/16 px
  steps, floored), from the line after, both when the pulses come and when they go. amsrust had nothing
  there: `MONITOR_AHEAD_PULL` in src/monitor.rs.
* Pulses that already run during the frame flyback (right after the VSYNC, as gen_pull_probe.py does) do not
  move the picture in AMSpiriT; when they stop, it dips by as much for 7 lines and jumps back. Not modelled,
  and why gen_pull_probe.py saw no pull ahead of the line sync.
* AMSpiriT 1.16 also differs from the Shaker-fitted rules nearer the line sync (8-10 characters ahead: no
  pull; a 1 us pulse after it: -4, not -10): left as they are (Shaker's references).

Gaps in the far second sync, and the line sync's width (`gen_pull3_probe.py` with several ranges,
`gen_pull4_probe.py`: R3 rewritten every line too; Onescreen Colonies #1):
* AMSpiriT Lite 1.16: a single line without the far pulse changes nothing; a gap of g lines takes the
  picture down g-1 steps (3/8 of the way back each), back up through the same states from the gap's last
  line, then 3/8 of the first step down above where it was, and the pull keeps that much more (4, 3, 5 for
  a 2 us pulse 29 characters ahead); after 8 lines without it the picture is back to 0. `AheadPull` in
  src/monitor.rs reproduces every row of 22 probe pictures but a few rows of the tails (1 pixel).
* A line sync 1 us narrower (R3 5 instead of 6) for L lines, no other pulse: AMSpiriT moves the picture one
  line later than amsrust, 1, 2, 5, 6, 7, 8 pixels (amsrust: 3, 5, 6, 7, 8), and for L <= 8 straight back
  when the width comes back (amsrust: 3/8 a line); for L = 1, +4 for one line. Not modelled yet (the
  line oscillator's response to the width was fitted on long runs, Shaker D3).

Ink writes (`gen_inkwrite_probe.py`): pen 1 set dark at the start of each line, light in the middle by
OUT (C),r, OUT (n),A or OUTI (all writing in the same microsecond), mode 0, 1 or 2. AMSpiriT Lite 1.16: the
three instructions reach the same pixel; the light colour starts 8 ticks into the character in modes 0 and
1, 7 in mode 2, i.e. a pixel takes the ink of its first tick (the 40007's write timing, then the pixel
grid). The Gate Array (`pixel_start_ring` in src/ga.rs) now does the same.

