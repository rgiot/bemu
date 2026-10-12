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
