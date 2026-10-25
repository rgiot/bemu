#!/bin/sh
# RulezCharge on the M4, through the host's network: browse the SD folder, download
# software from cpcrulez.fr. To try it by hand:
#
#   examples/rulezcharge.sh [--trace] [SD_FOLDER]
#
# - The SD card is SD_FOLDER (./m4sd by default, created if missing).
# - RulezCharge (inner://rulezcharge, slot 5) starts by itself: |RZ is typed, then ESC past
#   its dark start screen, which shows the SD browser and its keys.
# - Then: ESC (Download/search), a name, RETURN; pick an entry with the arrows and RETURN.
#   The file lands in SD_FOLDER; back in the SD browser, RETURN on a .DSK opens it.
# - --trace prints the M4 commands RulezCharge sends (BEMU_M4_TRACE).
set -e
if [ "$1" = "--trace" ]; then
    export BEMU_M4_TRACE=1
    shift
fi
sd="${1:-./m4sd}"
mkdir -p "$sd"
sd="$(cd "$sd" && pwd)"
cd "$(dirname "$0")/.."
exec cargo run --release -- --window --m4 "$sd" --rom 5=inner://rulezcharge --type '|rz\(RET)\(ESC)'
