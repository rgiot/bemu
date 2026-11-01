# R0WRAP: a real-CPC test for the CRTC 0 last line state

## The question

On a CRTC 0, the "last line of the frame" state (C4==R4 and C9==R9) is evaluated
while C0 is 0 or 1 (ACCC 10.3.1.2). When R0 is lowered under C0 in the middle of a
line, C0 runs on to 255 and wraps to 0 *without* ending the line, then counts up
to R0. Is the state evaluated again when C0 passes 0 and 1 after that wrap?

amsrust says yes. AMSpiriT draws Overflow Preview part 3 as if it were not.
Only a real CPC can settle it.

## What the program does

It runs 4 times, with interrupts off:

1. it waits for a VSYNC;
2. on the frame's last line (C4=38, C9=7), it writes:
   - R0=3 at C0~22, so C0 overflows to 255 and wraps;
   - R4=0 at C0~36, after C0=1;
   - R0=63 at C0~90, so C0 still has to wrap;
   - R4=38 again once the long line is over;
3. it counts, in passes of a 10 usec loop, how long the next VSYNC takes, and
   prints it on a `TEST` line.

Then it prints a `CONTROL` count for an untouched frame.

| result | meaning |
|---|---|
| `TEST` about 1500 | the state is NOT evaluated again after the wrap: the frame restarts |
| `TEST` about 6000 | the state IS evaluated again (R4=0 then): C4 overflows to 127 first |
| `CONTROL` about 2000 | the timing loop works on this machine |

amsrust today prints `TEST 06082` four times and `CONTROL 01996`.

## Steps on the real CPC

1. Use a CPC with a **CRTC 0**: the result means nothing on other CRTC types.
   Note the exact machine and CRTC: model (464/664/6128), CRTC type, and the
   chip markings if you can open it.
2. Put `r0wrap.dsk` on your disc emulator (or write it to a real 3" disc).
3. Switch the CPC off and on (do not run another program first).
4. Type `RUN"R0WRAP"` and press RETURN.
5. Wait about 1 second for 6 lines of numbers.
6. Take a sharp photo of the whole screen (all 4 `TEST` lines and the `CONTROL`
   line must be readable).
7. Do steps 3 to 6 **three times**, switching off between runs.
8. Send the 3 photos with the machine and CRTC details.

If the 4 `TEST` lines of one run differ from each other, or if no number at all
appears after 10 seconds, say so: that is a result too.

## Rebuilding

    basm r0wrap.asm      # writes R0WRAP.BIN on r0wrap.dsk
    cargo run --release --example run_csl -- validation/hardware_tests/r0wrap/r0wrap.csl /tmp/out
