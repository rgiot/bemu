#!/bin/bash
# Run one Shaker test (or one module) with the current build and compare it.
#   shaker_one.sh MODULE KEY TAG      one menu entry, e.g.  shaker_one.sh A O 1
#   shaker_one.sh MODULE - TAG        the whole module (45 min for B)
# Results: $SHAKER_WORK/res_<MODULE><KEY>_<TAG>/ ; trace the CRTC writes with
#   AMSRUST_TRACE_REG=1 shaker_one.sh A O 2      (they land in res_..._2.log)
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK="${SHAKER_WORK:-$HOME/shaker_work}"
MOD="$1"; KEY="$2"; TAG="$3"
cd "$REPO"
cargo build --release --example run_shaker_suite 2>&1 | grep -E "^error" -A8 || true
if [ "$KEY" = "-" ]; then
  BUNDLE="$WORK/${BUNDLE_DIR:-bundle}"; NAME="res_${MOD}_${TAG}"
else
  python3 "$HERE/shaker_one_test.py" "$MOD" "$KEY" "$WORK/bundle_${MOD}${KEY//[^A-Za-z0-9]/}${BUNDLE_DIR:+_$BUNDLE_DIR}" >/dev/null
  BUNDLE="$WORK/bundle_${MOD}${KEY//[^A-Za-z0-9]/}${BUNDLE_DIR:+_$BUNDLE_DIR}"; NAME="res_${MOD}${KEY//[^A-Za-z0-9]/}_${TAG}"
fi
rm -rf "$WORK/$NAME"; mkdir -p "$WORK/$NAME"
./target/release/examples/run_shaker_suite "$BUNDLE" "$WORK/$NAME" "$MOD" > "$WORK/$NAME.log" 2>&1
tail -2 "$WORK/$NAME.log" | cut -c1-120
python3 "$HERE/shaker_compare.py" "$WORK/$NAME" 12
