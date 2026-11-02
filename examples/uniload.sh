#!/bin/sh
# UniLoad (Roudoudou's game launcher for UniDOS, uniload_beta.zip) on UniDOS. To try it by hand:
#
#   examples/uniload.sh [FOLDER]
#
# - FOLDER is where uniload_beta.zip was unzipped (STARTUP.BAS, UniLoad.app, GAME). Without it,
#   the archive is downloaded from roudoudou.com (60 MB, once) into examples/downloads/uniload.
# - UniDOS boots on FOLDER (examples/unidos.sh), then UniLoad.app is entered and its START run.
#   STARTUP.BAS's RUN"UniLoad.app/START." is not used: START loads UniLoad.scr by its bare
#   name, from the current folder, and UniDOS's manual doesn't say a RUN with a path makes its
#   folder the current one (LOAD"FOLDER" does).
# - In the menu: the arrows move, TAB sorts, COPY (or Fire 1) starts the game, ESC goes back.
#   A game runs from its GAME/G-... folder, through UniLoad.app/LAUNCH.RUN, which UniLoad
#   writes in FOLDER; the game's own saves go there too.
set -e
. "$(dirname "$0")/download.sh"
folder="$1"
if [ -z "$folder" ]; then
    folder="$(download_archive uniload https://roudoudou.com/ACE-DL/uniload_beta.zip)"
fi
exec "$(dirname "$0")/unidos.sh" "$folder" UniLoad.app/START
