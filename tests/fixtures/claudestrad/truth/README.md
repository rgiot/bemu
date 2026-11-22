# Real-machine ground truth

Text read by hand (by Claude, from the photos in `../photos/<case>/`) off the screens of the user's
real machines. `<CASE>_<machine>.txt` holds the lines the real machine's screen shows, one per
line; `tests/claudestrad.rs` requires each of them (spaces collapsed) to be a line of what bemu
prints for the same machine.

Machines: `crtc0_m4`, `crtc1`, `crtc2`, `crtc2_m4`: the CRTC type of a 6128, with (`_m4`) or
without the M4 board (which delays the interrupt by one tick). A line is written only when its
digits were legible; a photo is a photo of a CRT: check a surprising value against the photo
before believing it or the emulator.
