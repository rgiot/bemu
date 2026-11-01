#!/bin/sh
# Willy (PulkoMandy's adapter) with an OPL3LPT and, given a SoundFont, an S2P. To try it by hand:
#
#   examples/willy.sh [--s2p FILE.sf2] [DSK]
#   examples/willy.sh --pack FOLDER [PATH]
#
# - The OPL3LPT is at &FEBC. With no DSK, examples/willy-opl3.bas runs (--basic): it plays the
#   OPL3 tone of the AdLib tutorial (440 Hz on channel 1) until a key is pressed.
# - --s2p FILE.sf2 plugs an S2P at &FEAC too, its instruments the SoundFont's (any General
#   MIDI .sf2, e.g. GeneralUser GS or FluidR3_GM), and examples/willy-s2p.bas runs instead: it
#   detects the S2P (&80, then &3F to the control: &C0 and its &FE answer), plays a scale on a
#   piano, then the OPL3's tone.
# - With a DSK (vgmplay, sa2play, a SymbOS disc...), the disc is inserted instead and nothing
#   is typed: run its player as usual.
# - --pack FOLDER: the OPL3 player pack (OPL3_PlayerPack.zip: PulkoMandy's vgmplay and its
#   music), FOLDER the one it was unzipped in (PLAYVGM in it), on UniDOS (examples/unidos.sh):
#   PLAYVGM's folder is entered and PLAYVGM run. It asks for a VGM or a folder to enter, one at
#   a time: PATH (MUSVZ/WOLF3D/01HORSTW.VGM) is typed for it, a part at each prompt, after the
#   folder's listing (a few seconds, more for a big folder). Without PATH, type at the prompt.
set -e
if [ "$1" = "--pack" ]; then
    if [ -z "$2" ]; then
        echo "usage: $0 --pack FOLDER [PATH]" >&2
        exit 2
    fi
    typed=""
    dir="$2/PLAYVGM"
    rest="$3"
    while [ -n "$rest" ]; do
        part="${rest%%/*}"
        if [ "$part" = "$rest" ]; then rest=""; else rest="${rest#*/}"; fi
        # the player lists the folder before its prompt: about 4 entries a second
        entries=$(ls "$dir" | wc -l)
        typed="$typed\\(WAIT $((4 + entries / 4)))$part\\n"
        dir="$dir/$part"
    done
    exec "$(dirname "$0")/unidos.sh" --willy "$2" PLAYVGM/PLAYVGM "$typed"
fi
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
