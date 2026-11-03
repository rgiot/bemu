#!/bin/sh
# RulezCharge on the M4, through the host's network: browse the SD folder, download
# software from cpcrulez.fr. To try it by hand:
#
#   examples/rulezcharge.sh [--trace] [--no-rz] [SD_FOLDER]
#
# - The SD card is SD_FOLDER (./m4sd by default, created if missing).
# - RulezCharge (inner://rulezcharge, slot 5) starts by itself: |RZ is typed, then ESC past
#   its dark start screen, which shows the SD browser and its keys.
# - Then: ESC (Download/search), a name (e.g. batman), RETURN; the results come from
#   cpcrulez.fr. Pick one with the arrows and RETURN: it downloads into SD_FOLDER, named as
#   the site names it, then "Continue" goes back to the list. Ctrl+H shows the other keys.
# - On a .dsk, RETURN and "Reset + Open .DSK" enter the image (the M4 reads a DSK as a directory
#   of its AMSDOS files, read only) and reset the CPC: CAT lists the image, RUN" runs from it,
#   |CD,".." leaves it.
# - --no-rz doesn't start RulezCharge: BASIC lists the SD card (CAT), for |DIR, |CD,"NAME",
#   RUN"NAME"... by hand; |RZ starts RulezCharge later.
# - --trace prints the M4 commands RulezCharge sends (BEMU_M4_TRACE).
set -e
typed='|rz\(RET)\(ESC)'
while [ -n "$1" ]; do
    case "$1" in
        --trace) export BEMU_M4_TRACE=1 ;;
        --no-rz) typed='cat\(RET)' ;;
        *) break ;;
    esac
    shift
done
sd="${1:-./m4sd}"
mkdir -p "$sd"
sd="$(cd "$sd" && pwd)"
cd "$(dirname "$0")/.."
exec cargo run --release -- --window --m4 "$sd" --rom 5=inner://rulezcharge --type "$typed"
