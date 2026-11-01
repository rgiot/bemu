#!/bin/sh
# UniDOS (from the ROMs embedded in bemu) on a Nova and an Albireo, the Albireo's USB key a host
# folder. To try it by hand:
#
#   examples/unidos.sh [--willy] FOLDER [RUN [TYPED]]
#
# - FOLDER is the USB key (UniDOS's device UMS:). Without a #BOOT.CFG there, one is added (a
#   single "5": UniDOS boots on the key, so A: is UMS:).
# - RUN, if given, is run once UniDOS has booted. In a folder (DIR/PROG), that folder is made
#   the current one first, as UniDOS's manual does it: LOAD"DIR" then RUN"PROG", so the program
#   finds its own files (examples/unidos.sh uniload_beta UniLoad.app/START).
# - TYPED is typed after RUN, as --type: \(WAIT n) waits n seconds for the program's prompt.
# - --willy plugs Willy with an OPL3LPT too (the OPL3 player pack: examples/willy.sh --pack).
# - By hand, UniDOS's folders: LOAD"DIR" enters DIR (LOAD"UMS:A/B" too), LOAD":" goes back to
#   the drive's root, |DRIVE,"A","UMS:DIR" makes A: that folder.
# - What the CPC writes goes to FOLDER (a game's saves, UniLoad's LAUNCH.RUN), as on the key.
# - The Nova's NVRAM (UniDOS's settings) is kept in FOLDER/../nova.bin.
set -e
willy=""
if [ "$1" = "--willy" ]; then
    willy="--willy-opl3"
    shift
fi
if [ -z "$1" ]; then
    echo "usage: $0 [--willy] FOLDER [RUN [TYPED]]" >&2
    exit 2
fi
folder="$(cd "$1" && pwd)"
if [ ! -e "$folder/#BOOT.CFG" ]; then
    printf '5' > "$folder/#BOOT.CFG"
    echo "added $folder/#BOOT.CFG: UniDOS boots on this USB key" >&2
fi
cd "$(dirname "$0")/.."
run="$2"
typed="$3"
set -- --window --nova "$(dirname "$folder")/nova.bin" --albireo "$folder" \
    --rom 1=inner://unidos-nova --rom 7=inner://unidos --rom 14=inner://unitools \
    --rom 15=inner://amsdos --rom 27=inner://unidos-albireo $willy
if [ -n "$run" ]; then
    case "$run" in
        */*) keys="load\"${run%/*}\"\nrun\"${run##*/}\"\n" ;;
        *) keys="run\"$run\"\n" ;;
    esac
    # after UniDOS's boot scan, which would eat the keys
    set -- "$@" --type "$keys$typed" --type-at 8
fi
exec cargo run --release -- "$@"
