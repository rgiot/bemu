# R0ENLARGE: when does an R0 write stop enlarging a line? (CRTC 0)

`R0ENL.BIN` on `r0enlarge.dsk` (`RUN"R0ENL"`, source `r0enlarge.asm`, `basm r0enlarge.asm`).
On a normal frame a line far from the VSYNC gets R0=#7F written 1 us later each frame (64 frames
cover a line), then R0=63 about 11 us after. A write in time enlarges the line (C0 goes past 63, then
counts to 255 once R0 is back: about 190 us more in the frame, read from the passes of a 10 us loop to
the next VSYNC); one too late changes nothing. Row 1 uses `OUT (C),C` (effect on its 3rd us), row 2
`OUTI` (5th us); `E` = enlarged, one character per us of delay. The time of a trial does not depend on
the delay and nothing is printed between trials, so the VSYNC polling loops keep their phase.

Results on a CRTC 0, `E` window (delay in us from the first character):

| | `OUT (C),C` | `OUTI` |
|---|---|---|
| AMSpiriT Lite 1.16 | 1..7 | 0..5 |
| amsrust 2026-11-09 | 1..7 | 0..3 |

(each row also has one or two stray `E` around 38-41: not explained, the same in both emulators.)
So `OUT (C),r` agrees to the microsecond; on amsrust an `OUTI` writing R0 lands 2 us later than on
AMSpiriT (4 us after the `OUT (C),r` of the same slot instead of 2). ACCC 13.6.1 (CRTC 0, 2): an R0
taking effect on C0=R0 is still in time, one character later too late.

To photograph on a real CPC: compare the two rows with the table (the windows are 7 and 6 characters
wide on AMSpiriT).
