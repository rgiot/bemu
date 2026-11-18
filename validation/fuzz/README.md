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
  when the width comes back (amsrust: 3/8 a line); for L = 1, +4 for one line.
* `gen_pull4_probe.py` widths `first:count:w` and waves `first:count:w1/w2/...` (scratch harness: each
  probe, base and per-row left-edge shift in both emulators). AMSpiriT Lite 1.16's response to the width is
  not one filter: R3 5 for 40 lines ramps 1, 2, 5, 6, 7, 8; R3 4 ramps 1, 2, 4, 7, 9, 11, 14, 16; R3 3 jumps
  to 24 at once; alternate 6/5 lines give a steady 4 (the mean, no ripple); a wave 6..2..6 is followed
  nearly line by line; 2 narrow lines give 1 pixel on one row, 1 narrow line 4. A width change moves the
  middle of the pulse like a position change does, which AMSpiriT follows with the 3/8 PLL (R2 waves
  above): amsrust keeps the same PLL for both (the R3 waves it shows stay smooth), and does NOT copy
  these dynamics. Two amsrust bugs found on the way, fixed:
  - the middle reference stopped at a 2 us C-HSYNC: R3 3 (1 us) settled at 32 ticks, not 24 (the
    1/2 us per R3 rule, which AMSpiriT follows). Shaker D3 D1-D11 (a 1 us line sync) were 1/2 us off
    as a whole for it, and MONITOR_PULL_MIN (a 1 us second sync pulled as much as a 2 us one) made up
    for it in D1-D4: removed, the eleven are now at 0-1.2 % (were 4.6-11.4 %);
  - after a line without a sync (R3 2 on CRTC 1: no C-HSYNC), the next sync, at the end of the next line,
    was taken for a sync 1000 ticks late: the beam restarted (48-tick spikes in an R3 wave, AMSpiriT none).
  Onescreen Colonies #1 narrows the line sync for 2 lines: the few lines around it still differ from
  AMSpiriT (up to 5 pixels), by design.

Real-CPC photos (the Shaker site's, `validation/tools/real_compare.py`) against bemu and AMSpiriT, modules A-D on CRTC 0:
correlation after registration 0.675 for bemu, 0.674 for AMSpiriT: the two are the same distance from the real CPC
on almost every screen (it cannot see a few pixels of shift). Where AMSpiriT is clearly closer: AT/F (0.71 against
0.75), AY/C (0.82 against 0.86), AO/H, CT/D. AT/F and AY/C show it: in the band of lines where R2 or R3 is
rewritten during the HSYNC, the real CPC and AMSpiriT draw slanted stripes (each line a little further), bemu a
checkerboard (the picture alternates between two positions line by line): the same effect as gen_pull4_probe.py with
R3 alternating 6/5 (AMSpiriT a steady 4 px, the mean; bemu 2, 4, 2, 4...). The cause: the line oscillator's phase
detector follows the END of the pulse and subtracts the instant half-width (`sync_adj`), so an alternating width
leaves a +/-8 tick error that the 3/8 PLL passes on. Tried and dropped: a low-pass on `sync_adj` alone (the ripple
stays: the pulse end itself alternates), a start-of-pulse detector with the picture offset taken from the filtered
width (steady plateaus drift; needs the offset and the PLL state separated, not a patch). Left for the real-CPC
ground truth.

Ink writes (`gen_inkwrite_probe.py`): pen 1 set dark at the start of each line, light in the middle by
OUT (C),r, OUT (n),A or OUTI (all writing in the same microsecond), mode 0, 1 or 2. AMSpiriT Lite 1.16: the
three instructions reach the same pixel; the light colour starts 8 ticks into the character in modes 0 and
1, 7 in mode 2, i.e. a pixel takes the ink of its first tick (the 40007's write timing, then the pixel
grid). The Gate Array (`pixel_start_ring` in src/ga.rs) now does the same.

