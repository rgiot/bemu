#!/bin/sh
# UniLoad (Roudoudou's game launcher for UniDOS, uniload_beta.zip) on UniDOS, from the folder
# the archive was unzipped in (STARTUP.BAS, UniLoad.app, GAME). To try it by hand:
#
#   examples/uniload.sh FOLDER
#
# - UniDOS boots on FOLDER (examples/unidos.sh), then UniLoad.app is entered and its START run.
#   STARTUP.BAS's RUN"UniLoad.app/START." is not used: START loads UniLoad.scr by its bare
#   name, from the current folder, and UniDOS's manual doesn't say a RUN with a path makes its
#   folder the current one (LOAD"FOLDER" does).
# - In the menu: the arrows move, TAB sorts, COPY (or Fire 1) starts the game, ESC goes back.
#   A game runs from its GAME/G-... folder, through UniLoad.app/LAUNCH.RUN, which UniLoad
#   writes in FOLDER; the game's own saves go there too.
set -e
if [ -z "$1" ]; then
    echo "usage: $0 FOLDER" >&2
    exit 2
fi
exec "$(dirname "$0")/unidos.sh" "$1" UniLoad.app/START
