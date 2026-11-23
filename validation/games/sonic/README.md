# Sonic the Hedgehog (UK) (64K) (2025), GX4000 cartridge

`Sonic the Hedgehog (UK) (64K) (2025) [Original].cpr`: a 512K cartridge (32 banks), the
2025 GX4000 conversion. It uses most of the ASIC: hardware sprites (multiplexed: moved while
the beam is lower, to draw more objects than there are sprites), the soft scroll, a palette
gradient written line after line, the 4096 colours.

Compared with AMSpiriT Lite 1.16.0 (the Linux AppImage of amspirit.fr, run headless under
`xvfb-run`): **the same pictures**, but for a palette write that lands about a microsecond apart
(see below).

## Scripts

- `sonic.csl` (CSL 1.6): boots the cartridge on a GX4000, screenshots the disclaimer and the
  title, `joystick 0 fire1` starts the game, `joystick 0 right` runs into the first Motobug.

      cargo run --release --example run_csl -- validation/games/sonic/sonic.csl /tmp/sonic

- `rings.csl` (CSL 1.6): the ring route. Sonic runs right and jumps every second: kills the
  Motobug, takes rings, loses ten on the spikes (they scatter: multiplexed sprites), takes some
  again. 80 screenshots. Its `wait`s are the emulated times AMSpiriT played
  `rings_amspirit.csl` (the same route in CSL 1.2, `keyboard_write` line 9 for the joystick):
  AMSpiriT stretches every `wait` by one or two frames, so the same script would press the
  joystick at other frames and the two games would part. `../../tools/amspirit_replay.py`
  makes the bemu script from AMSpiriT's `--debug-script` log.
- `amspirit_1.16.0/`: AMSpiriT's shots of 14 of the 80 frames.

      cargo test --release --test sonic_amspirit -- --ignored     # bemu against them
      ../../tools/amspirit_compare.py OUR_SHOTS amspirit_1.16.0 --mark /tmp/marked

## Results (last run)

- Disclaimer and title: identical. The rest of the title animates, a shot taken at another
  time is another frame: compared on a burst of frames, every AMSpiriT frame has an identical
  bemu frame.
- Ring route: 56 of the 80 shots identical, the others differ by 3 to 48 pixels. The scroll,
  Sonic, the Motobug, the rings and the HUD are always the same; `examples/probe_sprite_mux.rs`
  shows the multiplexing (frames 335-465 after the first gameplay shot: the HUD's sprites 2-3
  moved from y=24 to y=65 at line 59, the scattered rings), and those shots are identical but
  for 0 to 4 pixels.
- What differs: a few pixels of one line, where a pen changes colour a few pixels earlier in
  one emulator, later in the other (both ways): the sky's gradient at the left edge (x 0-20), a
  pen inside Sonic, a digit of the ring counter. A palette write about 1 us apart, to be
  explained (the interrupt that writes it, or when the ASIC applies it).
- Colours: AMSpiriT draws the levels from 5 to 245, bemu from 0 to 255, and its picture is one
  pixel to the left: a difference of rendering, not of the machine.
