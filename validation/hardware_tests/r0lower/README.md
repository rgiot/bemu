# R0LOWER

CRTC 0: R0 (41) lowered to 21 under C0; which write times leave the line counting on to 255 (E).
Rows of 64 delays, OUT (C),C / OUTI / OUT (n),A (RAM #4900/#4A00/#4B00, E = word >= control+10).

AMSpiriT Lite 1.13.4 (CRTC 0):
```
OUT(C),C  ..................E.EE.EEEEEEEEEEEEEE.E.E.E..................EE.
OUTI      ...............E..EEEEEEEEEEEEEEEEE......................E..EEEE
OUT(n),A  ....................E.E.EEEEEEEEEEEEEEE.E...E..................E
```
amsrust:
```
OUT(C),C  ..................EE.EEEEEEEEEEEEEEE.E.E.E..................EE.E
OUTI      ...............E..EEEEEEEEEEEEEEEEE......................E..EEEE
OUT(n),A  ....................E.E.EEEEEEEEEEEEEEE.E...E..................E
```
The lowering side agrees except for one-sample edge jitter in the first row; the raise side (R0SIDE)
is where the fitted rule still differs.
