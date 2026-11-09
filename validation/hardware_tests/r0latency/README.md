# R0LATENCY: when does an R0 write stop enlarging a line, per instruction (CRTC 0)

`R0LAT.BIN` on `r0latency.dsk` (`RUN"R0LAT"`, source `r0latency.asm`, `basm r0latency.asm`). Like
R0ENLARGE (see its README) with R0=41 and R2=34, so that the data of an `OUT (n),A` (A = the data = the
port's high byte, #3D / #29: A15=A14=0, A9..A8=01) reaches the CRTC: R0 goes to 61 at a time that rises
1 us a frame, and back to 41 8 to 11 us later (the same instruction twice). A write in time enlarges the
line. Three rows: `OUT (C),C` (11 us between the writes), `OUTI` (11), `OUT (n),A` (7). `E` = enlarged,
one character per us of delay. The results are in RAM, 65 words each (the control passes first), at
#4900, #4A00 and #4B00; read them through a debugger (AMSpiriT: `GET /api/ram?addr=18688&len=130`,
bytes little endian words, E when the word is at least 10 above the control word).

Results (CRTC 0, delays 0..63):

| | `OUT (C),C` | `OUTI` | `OUT (n),A` |
|---|---|---|---|
| AMSpiriT Lite 1.16 | `EEEEEEE.E.E.E` .. `EEE.EEEEEEE.E.E` (39..) | `EEEEEEEEE` .. `EEEEEEEEEEEEEE` (35..48) | `......EEE` .. `EEE` (47..49) |
| amsrust 2026-11-09 | `.EEEEEEE.E.E` .. `E.EEE.EEEEEEE.E` (39..) | `EEEEEEE` .. `EEEEEEEEEEEEEE` (35..48) | one `E` at 47 |

(AMSpiriT, full rows: `EEEEEEE.E.E.E...........................EEE.EEEEEEE.E.E.........`,
`EEEEEEEEE..........................EEEEEEEEEEEEEE...............`,
`......EEE.......................................EEE.............`; amsrust:
`.EEEEEEE.E.E...........................E.EEE.EEEEEEE.E..........`,
`EEEEEEE............................EEEEEEEEEEEEEE...............`,
`...............................................E................`.)

amsrust and AMSpiriT agree on the OUTI window 35..48; they differ on the first window of OUTI (AMSpiriT
2 us longer) and on the fine structure of `OUT (C),C`, and `OUT (n),A` enlarges the line over 3 delays
on AMSpiriT, almost never on amsrust. The windows repeat about every 41-44 us, not exactly 42: the structure
is not understood yet.
