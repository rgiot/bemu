#!/bin/bash
# Build CPCSyntaxError (https://github.com/WacKEDmaN/CPCSyntaxError) as a
# headless oracle: STUDY ONLY, never copy its code into amsrust.
# It needs SDL2 headers; if the system has none, build the release tarball into
# a private prefix. Everything lands in $SHAKER_WORK/cpcse.
set -e
WORK="${SHAKER_WORK:-$HOME/shaker_work}"
mkdir -p "$WORK/cpcse" && cd "$WORK/cpcse"
[ -d src ] || git clone -q --depth 1 https://github.com/WacKEDmaN/CPCSyntaxError src
if ! pkg-config --exists sdl2 2>/dev/null; then
  if [ ! -d prefix ]; then
    curl -sL -o sdl.tgz https://github.com/libsdl-org/SDL/releases/download/release-2.32.10/SDL2-2.32.10.tar.gz
    tar xzf sdl.tgz && mkdir -p SDL2-2.32.10/build && cd SDL2-2.32.10/build
    cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$WORK/cpcse/prefix" -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_TEST=OFF
    make -j"$(nproc)" && make install
    cd "$WORK/cpcse"
  fi
  P="$WORK/cpcse/prefix"; EXTRA="-DCMAKE_PREFIX_PATH=$P -DSDL2_DIR=$P/lib/cmake/SDL2"
fi
mkdir -p build && cd build
cmake ../src -DCMAKE_BUILD_TYPE=Release $EXTRA
make -j"$(nproc)" cpcse
echo "built: $WORK/cpcse/build/cpcse (run it from that directory: the ROMs sit next to it)"
cat <<'U'
Usage examples (run from $SHAKER_WORK/cpcse/build):
  # a CSL script, screenshots named by SSM code (CPCSE_0_<hex>.bmp)
  ./cpcse --csl script.csl --out OUT --disk-dir DIR_WITH_DSK --crtc 0 --emu-name CPCSE </dev/null
  # one shot of a program on a disc
  ./cpcse --shot out.bmp --model cpc6128 --crtc 0 --frames N --disk x.dsk --type 'RUN"X"\n'
  # traces (stderr): CRTC writes with their counters, interrupt acks, HSYNC, VSYNC gaps
  CPCSE_TRACE_REG="0,1,4,9" CPCSE_TRACE_REG_MAX=3000000  CPCSE_TRACE_ACK=1
  CPCSE_TRACE_HSYNC=1 CPCSE_TRACE_HSYNC_R2=11  CPCSE_VSYNC_TRACE=1
U
