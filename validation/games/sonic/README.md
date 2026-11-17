# Sonic the Hedgehog (UK) (64K) (2025), GX4000 cartridge

`Sonic the Hedgehog (UK) (64K) (2025) [Original].cpr`: a 512K cartridge (32 banks), the
2025 GX4000 conversion. It exercises most of the ASIC: hardware sprites, the soft scroll,
the screen split (score bar), the programmable raster interrupt and the 4096-colour palette.

    cargo run --release --example run_csl -- validation/games/sonic/sonic.csl /tmp/sonic

`sonic.csl` boots the cartridge on a GX4000 (`cpc_model 6`), screenshots the disclaimer and
the title, presses fire on joystick 0 (`keyboard_write`, line 9 bit 4) to start the game,
then holds right (line 9 bit 3) into the first Motobug and loses a life.

What bemu shows (last run): the "NOT PRODUCED BY OR UNDER LICENSE FROM SEGA" screen, the
title (winged emblem, clouds, water), Green Hill Zone act 1 with the ring counter and the
lives (x3), the scroll as Sonic runs right, the Motobug, the lives going to x2 and the
"GREEN HILL ZONE ACT 1" card on the respawn. No visible glitch.

To check against AMSpiriT Lite (not done yet: it needs the AppImage, see
`../../tools/README.md`), play the same script there (`--script sonic.csl`)
and compare the shots taken at the same times. The game is animated: the disclaimer and
the title are the screens that must match pixel for pixel (`../../tools/demo_compare.py`
measures that, it reads the other emulator's shots as `.bmp`).
