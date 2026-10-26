#!/bin/sh
# UniDOS (from the ROMs embedded in bemu) on a Nova and an Albireo, the Albireo's USB key a host
# folder. To try it by hand:
#
#   examples/unidos.sh [--willy] FOLDER [RUN]
#
# - FOLDER is the USB key (UniDOS's device UMS:). Without a #BOOT.CFG there, one is added (a
#   single "5": UniDOS boots on the key, so A: is UMS:).
# - RUN, if given, is typed as RUN"RUN" once UniDOS has booted: UniLoad's folder starts with
#   STARTUP (examples/unidos.sh uniload_beta STARTUP).
# - --willy plugs Willy with an OPL3LPT too (the OPL3 player pack's PLAYVGM).
# - A program that loads its files from its own folder (PLAYVGM's PlayVGM.COD, UniLoad's
#   START and UniLoad.scr) needs that folder as the current one: how UniDOS sets it is still to
#   be found here, so put such a program at the root of FOLDER (PLAYVGM.BAS, PLAYVGM.COD and
#   the .VGM files: examples/unidos.sh --willy FOLDER PLAYVGM, then a .VGM's name).
# - The Nova's NVRAM (UniDOS's settings) is kept in FOLDER/../nova.bin.
set -e
willy=""
if [ "$1" = "--willy" ]; then
    willy="--willy-opl3"
    shift
fi
if [ -z "$1" ]; then
    echo "usage: $0 [--willy] FOLDER [RUN]" >&2
    exit 2
fi
folder="$(cd "$1" && pwd)"
if [ ! -e "$folder/#BOOT.CFG" ]; then
    printf '5' > "$folder/#BOOT.CFG"
    echo "added $folder/#BOOT.CFG: UniDOS boots on this USB key" >&2
fi
cd "$(dirname "$0")/.."
run="$2"
set -- --window --nova "$(dirname "$folder")/nova.bin" --albireo "$folder" \
    --rom 1=inner://unidos-nova --rom 7=inner://unidos --rom 14=inner://unitools \
    --rom 15=inner://amsdos --rom 27=inner://unidos-albireo $willy
if [ -n "$run" ]; then
    # after UniDOS's boot scan, which would eat the keys
    set -- "$@" --type "run\"$run\"\n" --type-at 8
fi
exec cargo run --release -- "$@"
