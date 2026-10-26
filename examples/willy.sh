#!/bin/sh
# Willy (PulkoMandy's adapter) with an OPL3LPT and, given a SoundFont, an S2P. To try it by hand:
#
#   examples/willy.sh [--s2p FILE.sf2] [DSK]
#
# - The OPL3LPT is at &FEBC. With no DSK, examples/willy-opl3.bas runs (--basic): it plays the
#   OPL3 tone of the AdLib tutorial (440 Hz on channel 1) until a key is pressed.
# - --s2p FILE.sf2 plugs an S2P at &FEAC too, its instruments the SoundFont's (any General
#   MIDI .sf2, e.g. GeneralUser GS or FluidR3_GM), and examples/willy-s2p.bas runs instead: it
#   detects the S2P (&80, then &3F to the control: &C0 and its &FE answer), plays a scale on a
#   piano, then the OPL3's tone.
# - With a DSK (vgmplay, sa2play, a SymbOS disc...), the disc is inserted instead and nothing
#   is typed: run its player as usual. The OPL3 player pack (PulkoMandy's vgmplay) runs on
#   UniDOS from a folder: examples/unidos.sh --willy FOLDER.
set -e
soundfont=""
if [ "$1" = "--s2p" ]; then
    soundfont="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
    shift 2
fi
disc=""
if [ -n "$1" ]; then
    disc="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
fi
cd "$(dirname "$0")/.."

if [ -n "$disc" ]; then
    set -- --dsk "$disc"
elif [ -n "$soundfont" ]; then
    set -- --basic examples/willy-s2p.bas
else
    set -- --basic examples/willy-opl3.bas
fi
if [ -n "$soundfont" ]; then
    set -- "$@" --willy-s2p "$soundfont"
fi
exec cargo run --release -- --window --willy-opl3 "$@"
