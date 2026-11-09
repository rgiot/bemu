# POLLPHASE: where code lands against the CRTC after a VSYNC poll

`POLLPH.BIN` on `pollphase.dsk` (`RUN"POLLPH"`, source `pollphase.asm`, `basm pollphase.asm`). Every
frame: the usual PPI poll for the VSYNC (`IN A,(C)` / `RRA` / `JR`), a fixed wait, then three bands
in the top border, 8 lines apart: the border turns white with `OUT (C),C`, black 10 us later; the
same with `OUT (n),A` and with `OUTI`. The x position of a band is the time since its line's start
(16 pixels of the 768 screenshot a microsecond), so the poll's exit phase and the instruction's effect
are measured together against the CRTC, not against another instruction. The poll is entered 0 to 7 us
later by stretches of 32 frames (the phase also is at `#3F01`), so the screenshots of 0.7 s show the
different exit phases.

Results (CRTC 0, x of the band of `OUT (C),C`; the other two bands are 112 and 176 pixels lower in x):

| | exit phases seen |
|---|---|
| AMSpiriT Lite 1.16 | 585, 617, 649, 681 |
| amsrust 2026-11-09 | 584, 616, 648, 680 |
| CPCSyntaxError 0.14.2 | 567, 599, 631, 663 |

The three bands of one frame are 112 and 176 pixels apart on all of them. amsrust agrees with
AMSpiriT to the pixel (VSYNC poll, `OUT (C),C`, `OUT (n),A`, `OUTI`); CPCSyntaxError puts everything 1 us
earlier.
