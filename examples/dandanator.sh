#!/bin/sh
# The CPC Dandanator Mini with a romset. To try it by hand:
#
#   examples/dandanator.sh [--writable] [ROMSET.rom]
#
# - The romset boots in place of the firmware: its menu shows, a game starts with its key.
# - With no romset, the test romset of the romset generator (teiram/dandanator-cpc, the source
#   of dandanator-cpc-2.3.jar: Dandanator menu v2.0 with ten games) is downloaded once to
#   target/dandanator_2.0.rom. Make your own with that generator (java -jar
#   dandanator-cpc-2.3.jar) from SNA files.
# - --writable: what the CPC programs into the flash (a game's saves, a romset written by
#   Dandare's loader) goes back to the file. Without it the file is left as it is.
set -e
writable=""
if [ "$1" = "--writable" ]; then
    writable="--dandanator-writable"
    shift
fi
romset=""
if [ -n "$1" ]; then
    romset="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
fi
cd "$(dirname "$0")/.."
if [ -z "$romset" ]; then
    romset="$PWD/target/dandanator_2.0.rom"
    if [ ! -f "$romset" ]; then
        mkdir -p target
        curl -fsSL -o "$romset" \
            https://raw.githubusercontent.com/teiram/dandanator-cpc/master/src/test/resources/romset/dandanator_2.0.rom
    fi
fi
exec cargo run --release -- --window --dandanator "$romset" $writable
