# Real-CPC test kit

`REALKIT.dsk` holds small programs (`RUN"ALT24"`, `RUN"PUL424"`...) that make a CPC's monitor
show how it follows the sync pulses, and how the Gate Array times a palette write. bemu and AMSpiriT
Lite disagree about some of this (and neither is the real thing): a photograph of a real CPC's screen
for each program tells which of them is right, and what to model.

`make_kit.py` builds the disc from the probes of `validation/fuzz/` (`./make_kit.py [out.dsk]`);
`baseline.sh [crtc]` plays every program in bemu (and in AMSpiriT Lite with `AMSPIRIT=...`) and writes
the pictures to `bemu/` and `amspirit/` (CRTC 1 here: `*_crtc1.png`), to compare the photographs with.

## What to do

1. Put `REALKIT.dsk` on a real disc (or a Gotek / HxC: it is a standard 40 tracks, 9 sectors
   AMSDOS "data format" DSK) and `RUN"<name>"` on a real CPC. Each program takes over the machine
   (interrupts off) and shows one steady picture; reset to stop it.
2. Photograph the screen, straight on, the camera on a tripod, the whole tube in the frame (as the
   photographs of the Shaker site). A fixed exposure of about 1/25 s or longer (a shorter one shows
   the beam's scan as a band). One photograph per program, named `<program>_<crtc>_<ga>.jpg`
   (for example `ALT24_crtc1_ga40010.jpg`).
3. The probes' timing is exact to the microsecond: each program exists with three phases (the number
   at the end of its name: `ALT23`, `ALT24`, `ALT25`). 24 is the one that matches AMSpiriT and bemu. If a
   picture looks nothing like what bemu shows (the stripes all in one block, nothing moving at the
   expected lines), try the other phases and tell me which one gives the clean picture.
4. Tell me what the machine is: the CPC model, its CRTC type (the Shaker disc's detection, or a
   Hitachi / UM6845 / MC6845 / Amstrad ASIC chip you can read), its Gate Array (40007, 40008, 40010,
   40226, 40489), its monitor (CTM640, CTM644, GT64, GT65, MM12...) and where the monitor's knobs
   are set.

The programs and the question each one asks:

| Program | What it draws | The question |
|---|---|---|
| `ALT` | 40 lines whose line sync (R3) alternates 6, 5, 6, 5 µs | Does the picture shift by the mean (steady, AMSpiriT: 4 px) or wobble line by line (bemu)? |
| `W5`, `W4`, `W3` | 40 lines with a line sync of 5, 4, 3 µs | How far does the picture move (AMSpiriT: 8, 16, 24 px) and how many lines does it take to get there and to come back? |
| `WAVE` | 96 lines whose sync width goes 6 5 4 3 4 5 | The shape of the wave: smooth or stepped? |
| `SHORT` | runs of 1, 2, 3 and 8 lines with a 5 µs line sync | AMSpiriT moves the picture 4 px on one line for the run of 1, 1 px for the run of 2, and back at once; what does the real monitor do? |
| `PUL4`, `PUL2` | a second HSYNC (4 and 2 µs wide) on every line, moving away from the line sync every 6 lines: 8 µs ahead of it at the top, 36 µs at the bottom | How far does the picture move as a function of the pulse's position and width (AMSpiriT: 3, 6, 10, 13 px for 1 to 4 µs, between 12 and 28 µs ahead, nothing from 33)? |
| `INK1` | a mode 1 screen whose ink is changed in the middle of every line | At which pixel the new colour starts: the Gate Array's write timing, per chip. |
| `PAL1`, `PAL2` | the 27 inks as 16 horizontal bands (mode 0) | The colours and brightness of each ink, on the colour tube (and on a GT64 / GT65 / MM12: how the monochrome tubes show them). |

## More photographs that would help

* **The knobs** of a CTM monitor (rear): the BASIC `Ready` screen (just boot the CPC) with H-hold at
  its two ends and its middle, then the same for V-hold; the brightness knob at its two ends and middle.
  bemu now has `--hhold`, `--vhold` (-100 to 100) and `--brightness`: it says where the picture starts
  tearing or rolling, which is a guess.
* **`PAL1` and `PAL2` on a green (GT64 / GT65) and on a grey (MM12) monitor.**
* **The Shaker tests** `AT/F`, `AP/F` and `D4/A` on CRTC 3 and 4, `AY/A` on CRTC 1 (the Shaker site's
  photographs are missing or off for them).

## About the pictures of `bemu/` and `amspirit/`

They are the programs run from this disc on a CRTC 1 (`./baseline.sh 1`; the other CRTC types:
`./baseline.sh 0`, `2`...). Two programs, `PUL4` and `INK1`, differ by a few pixels from the same
probe started from a snapshot: bemu's alignment of the Z80 and the Gate Array depends on how the
machine got there, a real CPC's is fixed by the Gate Array. Do not expect a pixel-for-pixel match with
a photograph: the monitor's geometry and the camera blur the picture. What matters is the shape.
