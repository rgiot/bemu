#!/bin/bash
# baseline.sh [crtc] : the kit's pictures as bemu and AMSpiriT Lite show them (CRTC `crtc`, default 1),
# in bemu/ and amspirit/ next to the DSK, to compare the real CPC's captures with.
# Needs target/release/examples/run_csl (cargo build --release --example run_csl) and, for AMSpiriT,
# $AMSPIRIT (its AppRun) and xvfb-run.
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
CRTC="${1:-1}"
DSK="$HERE/REALKIT.dsk"
RUN_CSL="$HERE/../../target/release/examples/run_csl"
NAMES=$(python3 - "$DSK" <<'PY'
import sys
d = open(sys.argv[1], 'rb').read()
raw = b''.join(d[256 + 256 + 512 * s: 256 + 256 + 512 * (s + 1)] for s in range(4))
print(' '.join(sorted({raw[i + 1:i + 9].decode().strip() for i in range(0, 2048, 32) if raw[i] == 0 and raw[i + 12] == 0})))
PY
)
mkdir -p "$HERE/bemu" "$HERE/amspirit"
work="$(mktemp -d)"
cp "$DSK" "$work/kit.dsk"
for name in $NAMES; do
  cat > "$work/$name.csl" <<CSL
csl_version 1.0
crtc_select $CRTC
reset
wait 3000000
disk_insert 'kit.dsk'
key_delay 70000 70000 400000
key_output 'RUN"$name"\(RET)'
wait 8000000
screenshot_name '$name'
screenshot
CSL
  (cd "$work" && "$RUN_CSL" "$name.csl" out >/dev/null 2>&1 && cp "out/$name.png" "$HERE/bemu/${name}_crtc$CRTC.png") &
  while [ "$(jobs -r | wc -l)" -ge 3 ]; do sleep 1; done
done
wait
if [ -n "$AMSPIRIT" ]; then
  for name in ${AMS_NAMES:-$NAMES}; do
    sed 's/wait 8000000/wait 4000000/; s/wait 3000000/wait 1500000/' "$work/$name.csl" > "$work/a_$name.csl"
    (cd "$work" && SDL_AUDIODRIVER=dummy timeout 180 xvfb-run -a "$AMSPIRIT" --reset --no-splash --turbo -m 6128 \
       --crtc "$CRTC" -l EN --script "a_$name.csl" -X 3000 kit.dsk >/dev/null 2>&1; \
       cp "$name.png" "$HERE/amspirit/${name}_crtc$CRTC.png" 2>/dev/null || true) &
    while [ "$(jobs -r | wc -l)" -ge 3 ]; do sleep 1; done
  done
  wait
fi
rm -rf "$work"
ls "$HERE/bemu" | wc -l
