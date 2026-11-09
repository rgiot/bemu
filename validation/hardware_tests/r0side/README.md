# R0SIDE: the two sides of an R0 enlargement, one instruction at a time (CRTC 0)

`R0SIDE.BIN` on `r0side.dsk` (`RUN"R0SIDE"`, source `r0side.asm`, `basm r0side.asm`). R0=41 (R2=34): per
frame R0 goes to 61 and back to 41. A line is "enlarged" (E) when the raise is in time and the restore
comes after C0 has passed 41 (C0 then counts to 255: about 190 us more in the frame, read from the passes of
a 10 us loop to the next VSYNC). Mode R sweeps the raise (instruction X, then an `OUT (C),C` restore 20 us
later): its window ends where X stops being in time. Mode S sweeps the restore (X; the raise an `OUT (C),C`
well before). X = `OUT (C),C`, `OUTI`, `OUT (n),A`. Six rows of 64 results (delay 0..63 us), 130 bytes each in
RAM, the control's passes first: #4900 R OUT (C),C, #4A00 R OUTI, #4B00 R OUT (n),A, #4C00 S OUT (C),C,
#4D00 S OUTI, #4E00 S OUT (n),A (E when a word is >= the control word + 10; AMSpiriT:
`GET /api/ram?addr=18688&len=130`; amsrust: `examples/probe_mem script addr_hex 130`).

AMSpiriT Lite 1.16:

    R OUT(C),C   .....E.EE.EEEEEEEEEEEEEEE.E.E.E..................EE.EEEEEEEEEEEE
    R OUTI       .....E..EEEEEEEEEEEEEEEEE......................E..EEEEEEEEEEEEEE
    R OUT(n),A   .........E..E.EEEEEEEEEEEEEEE.E...E..................E.E.EEEEEEE
    S OUT(C),C   ......E.EE.E.E.E.E.E.E.E.E.E.E......E.E.E.E.E.E.E.E.E.............
    S OUTI       ...E..EEEEEEEEEEEEEEEEE.........................................
    S OUT(n),A   .........E.E.E.E.E.E.E.E.E.E.E......E...E...E...E...E...........

amsrust at the time of writing (`b94296b` era rules):

    R OUT(C),C   .....E.EE.EEEEEEEEEEEEEEE.E.E..................E.EE.EEEEEEEEEEEE
    R OUTI       .....E..EEEEEEEEEEEEEEE........................E..EEEEEEEEEEEEEE
    R OUT(n),A   .........E..E.EEEEEEEEEEEEE.E...E...................E.E.EEEEEEEE
    S OUT(C),C   ......E.EE.EEEEEEEEEEEEEE.E.E...................................
    S OUTI       ...E..EEEEEEEEEEEEEEEEE.........................................
    S OUT(n),A   .........E.E.EEEEEEEEEEEEEE.E...E...............................

An experimental CRTC 0 model (C0=R0 latched when C0 reaches R0, the line ends with the next character, an R0
raised above C0 cancels it, a write that makes R0 equal to C0 is not seen) reproduces AMSpiriT's alternating
`E.E.E` of the S rows for `OUT (C),C` and `OUT (n),A` exactly, which the committed rules do not (they give
solid blocks), and the S OUTI row; but its R windows end 2 us before AMSpiriT's (R OUTI `...E..EEEEEEEEEEEEEEEEE`
= AMSpiriT's shifted by 2). It does not yet satisfy Shaker B6 A and Overflow's 20352 us frame.
