#!/usr/bin/env bash
# The app on an Android emulator (the CI's android-smoke job): installed, given a disc as "Open
# with" would, started by itself; screenshots in portrait and landscape, the log, no crash.
#
#   bemu-droid/tests/smoke.sh APK DISC OUT_DIR
#
# Needs adb and a booted emulator (or phone). The APK must be debuggable (the debug one): the disc
# is put in the app's own files, which a shell cannot read for another app on recent Androids.
set -euo pipefail

apk="$1"
disc="$2"
out="$3"
package=org.bemu.droid
mkdir -p "$out"

adb wait-for-device
adb install -r -g "$apk"
adb logcat -c

# the disc in the app's files, opened as a file manager's "Open with" would
name=$(basename "$disc")
adb push "$disc" "/data/local/tmp/$name"
adb shell run-as "$package" mkdir -p files
adb shell run-as "$package" cp "/data/local/tmp/$name" "files/$name"
adb shell am start -W -n "$package/.MainActivity" -a android.intent.action.VIEW \
    -d "file:///data/data/$package/files/$name" -t application/octet-stream

alive() { adb shell pidof "$package" >/dev/null; }

# loaded, its autorun typed, the program started
for _ in $(seq 1 60); do
    adb logcat -d -s bemu:I | grep -q "load $name:" && break
    sleep 1
done
sleep 20
alive || { echo "the app is not running"; adb logcat -d > "$out/logcat.txt"; exit 1; }
adb exec-out screencap -p > "$out/portrait.png"

# landscape: the layout changes, the emulation goes on
adb shell settings put system accelerometer_rotation 0
adb shell settings put system user_rotation 1
sleep 8
alive || { echo "the app died on rotation"; adb logcat -d > "$out/logcat.txt"; exit 1; }
adb exec-out screencap -p > "$out/landscape.png"

# the background (the machine saved), then back
adb shell input keyevent KEYCODE_HOME
sleep 3
adb shell am start -W -n "$package/.MainActivity"
sleep 5
alive || { echo "the app died coming back"; adb logcat -d > "$out/logcat.txt"; exit 1; }

adb logcat -d > "$out/logcat.txt"
echo "--- the app's log"
grep -E " (bemu|AndroidRuntime) " "$out/logcat.txt" | tail -n 40 || true
if grep -E "FATAL EXCEPTION|panicked at" "$out/logcat.txt"; then
    echo "the app crashed"
    exit 1
fi
grep -q "load $name: ok:" "$out/logcat.txt" || { echo "the disc was not loaded"; exit 1; }
grep -q "drawing with" "$out/logcat.txt" || { echo "nothing was drawn"; exit 1; }
echo "smoke test passed"
