# R0ENLARGE: when does an R0 write stop enlarging a line? (CRTC 0)

`R0ENL.BIN` on `r0enlarge.dsk` (`RUN"R0ENL"`, source `r0enlarge.asm`, `basm r0enlarge.asm`).
On a normal frame a line far from the VSYNC gets R0=#7F written 1 us later each frame (64 frames
cover a line), then R0=63 about 11 us after. A write in time enlarges the line (C0 goes past 63, then
counts to 255 once R0 is back: about 190 us more in the frame, read from the passes of a 10 us loop to
the next VSYNC); one too late changes nothing. Row 1 uses `OUT (C),C` (effect on its 3rd us), row 2
`OUTI` (5th us); `E` = enlarged, one character per us of delay. The time of a trial does not depend on
the delay and nothing is printed between trials, so the VSYNC polling loops keep their phase.

Results on a CRTC 0 (the sweep starts 12 us before the expected boundary: column n = a delay of n us;
the `E.E.E` runs before the solid block come from the restore write, the block is the useful part):

| | `OUT (C),C`: solid `E` block | `OUTI`: solid `E` block |
|---|---|---|
| AMSpiriT Lite 1.16 | columns 13..19 | columns 5..17 |
| amsrust 2026-11-09 | columns 14..17 | columns 6..16 |

So on amsrust the last write that still enlarges the line is 2 us earlier than on AMSpiriT for
`OUT (C),C` (17 against 19) and 1 us earlier for `OUTI` (16 against 17); the first useful write is 1 us
later in both (the restore write decides that one). `probe_r0_effect` (examples) shows where amsrust
takes the register inside the instruction: `OUT (C),C` 2 us 9 ticks after its start (the 3rd us),
`OUTI` 4 us 5 ticks (the 5th). A first sweep that started too late to show the beginning of the `OUTI`
window gave a wrong impression that `OUT (C),r` agreed. ACCC 13.6.1 (CRTC 0, 2): an R0 taking effect
on C0=R0 is still in time, one character later too late. The line states of Overflow's and Madness'
R0 writes (see validation/demos) are not explained by this: they write with `OUT (C),r` and AMSpiriT
ends there the lines amsrust enlarges (Overflow), or enlarges those a threshold had to protect (Madness).

To photograph on a real CPC: the solid blocks of the two rows, against the table.
