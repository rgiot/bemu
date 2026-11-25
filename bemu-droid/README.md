# bemu-droid: bemu on Android

The bemu Amstrad CPC emulator as an Android app. The whole emulator stays in Rust. That covers the machine, the
CTM644 monitor drawn by the GPU, the sound, the media and their autorun. The app around it is a thin layer
of Kotlin and Jetpack Compose that shows the surface the emulator draws in, plus the touch controls.

```
bemu-droid/
├── rust/             the native library, libbemu_droid.so (a crate of its own, like web/)
│   ├── src/config.rs   the settings and the machine they make
│   ├── src/machine.rs  media, autorun, touch input, a frame at a time (tested on the host)
│   ├── src/runner.rs   the emulation thread: the machine, the surface, the sound
│   ├── src/gfx.rs      wgpu (Vulkan, else GLES) and amsrust::display::Picture
│   ├── src/audio.rs    AAudio through cpal; the sound paces the emulation
│   ├── src/jni_api.rs  what org.bemu.droid.Native calls
│   └── build-android.sh
└── app/              the Android app (Kotlin, Compose)
```

## What it does

- **Opens discs, tapes, cartridges and snapshots**: `.dsk`, `.hfe`, `.cdt`/`.tzx`, `.cpr` and `.sna`, plus zip, 7z
  and tar.gz archives that hold them. You can open them with the Open button, with "Open with" from a file
  manager, or by sharing from another app. A cartridge switches the machine to a 6128+.
- **Autorun** (on by default). Once the machine has booted, the medium starts by itself, as you would start it:
  - A disc gets `RUN"` of its menu or loader, or `|CPM` for a CP/M system disc.
  - A tape gets `|TAPE` then `RUN"`.
  - A cartridge boots on its own.

  This is `amsrust::autorun`, which the desktop also has (`--autorun`).
- **The CTM644** (on by default): the colour monitor's picture is drawn by the GPU, using the same CRT model as the desktop
  and the web page. It needs Vulkan with compute shaders. Without them the picture is shown raw. If the phone is too
  slow, turn it off in the settings to get the raw picture, which is the cheapest.
- **M4 board, RulezCharge, UniDOS** (all on by default). `|RZ`, or the toolbar's `|RZ` button, starts RulezCharge. It
  searches cpcrulez.fr and downloads games and demos onto the M4's SD card. On a `.dsk`, "Reset + Open .DSK" runs it.
  The SD card is the app's `m4sd` folder (`Android/data/org.bemu.droid/files/m4sd`), which you can see over USB.
  UniDOS sits above AMSDOS, and the M4's ROM goes in slot 127 so that drive A stays the floppy.
- **The screen follows the phone's orientation**, and the emulation keeps running when it turns.
  - In portrait, the picture is at the top and the joystick is at the bottom.
  - In landscape, the picture is in the middle, the stick is on the left and the fire buttons are on the right.
- **The CPC keyboard** (the Keyboard button) is drawn over the picture, translucent, because a phone screen has no
  room for both.
  - You can hold several keys at once.
  - SHIFT and CTRL latch: they stay down until the next key.
  - `Fn` shows the function keys (f0–f9) and the cursor keys.
- **Hardware keyboards and gamepads**: a keyboard's keys map to the CPC's keys. A gamepad's d-pad and A/B buttons map to
  the joystick.
- **Performance diagnostics**: Settings > Performance diagnostics shows the selected graphics adapter and backend,
  emulation and draw rates, and average CPU-side emulation, draw submission and pacing times. Let it run for a few
  seconds, then use Copy to share the measurements. Draw attempts may not result in a presented frame, and draw
  submission time is CPU-side and does not measure GPU execution time.
- **Live settings**: changing the monitor, sound, autorun, or touch vibration preserves the running CPC. Changing
  the model, language, or expansion settings restarts it. Autorun changes apply the next time a medium is started.

## Building

You need:
- Rust, with the targets `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`
- The Android SDK with the NDK (`sdkmanager "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"`)
- JDK 17 or 21 (the Gradle wrapper does not support every newer JDK)
- cpclib checked out next to the repository (`../rust.cpcdemotools`), as for the desktop build

```sh
cd bemu-droid
echo "sdk.dir=$ANDROID_HOME" > local.properties
./gradlew assembleDebug        # or assembleRelease
```

`app/build/outputs/apk/` then holds the APK.

For a debuggable app on a USB-connected, authorized phone, run:

```sh
cd bemu-droid
make debug
```

This builds the APK with an optimized Rust core, installs it as `org.bemu.droid.debug` alongside the release app,
launches it and streams its logcat output until Ctrl-C. It has separate settings, saved state and app-owned files;
load the same workload in the debug app when comparing performance. It does not uninstall or replace the release.
To save the stream, run `make debug 2>&1 | tee bemu-debug.log` and stop it with Ctrl-C. Logs can contain information
about the device and files opened; review them before sharing.
The target builds `arm64-v8a` by default; override `BEMU_ABIS` for another device. If needed, set `ADB` to the
platform-tools executable and `ANDROID_SERIAL` to select a device. The build still requires the SDK, NDK, Rust
Android target and cargo-ndk listed above. Device CPU profiling uses the same procedure below with package
`org.bemu.droid.debug` and the matching local symbols in `rust/target/android-symbols/`.

- Gradle runs `rust/build-android.sh` first, which puts `libbemu_droid.so` (always optimised, since a debug emulator is
  too slow) in `app/src/main/jniLibs`. Two options change this:
  - `BEMU_ABIS="arm64-v8a"` limits the ABIs, which makes the build quicker.
  - `-PskipCargo` skips the Rust build when the libraries are already there.
- The release APK is signed with the key named by `ANDROID_KEYSTORE_FILE`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` when they are set. Otherwise it uses the debug key.
- `cd rust && cargo test --no-default-features` tests the machine on the host. This covers the settings, media,
  autorun and touch input, without the GPU's CRT, whose host build downloads LLVM.

`build-android.sh` works around one problem. hxcfe-sys and hxcfe 0.3.4 (the HFE images) cast strings to `*mut i8`
where C's `char` is `u8` on ARM, so they do not build for arm64. The script builds with copies of both crates
that have the cast fixed, until fixed releases are out.

The CI (`.github/workflows/build.yml`) builds the app and tests it:
- The `android` job runs the Kotlin unit tests and the host tests. It uploads the `bemu-android` artifact:
  - `bemu-droid.apk` (universal);
  - `bemu-droid-arm64.apk` (about half the size, for most phones);
  - `bemu-droid.aab` (for Google Play);
  - `bemu-droid-debug.apk`.
- The `android-smoke` job installs the debug APK on an Android emulator and opens a disc as "Open with" would. It
  checks that the disc starts by itself, in both orientations and back from the background, with no crash. Its
  screenshots and log are the `bemu-android-smoke` artifact. The emulator draws with GLES, so the picture is raw:
  the CTM needs a phone with Vulkan.
- The `android-profile-symbols` artifact contains unstripped Rust libraries for profiling; use the symbols from the
  same CI run as the APK.

### Profiling Android CPU time

Release APKs allow profiling from an authorized Android debugging shell on Android 10 or later; the app is not
debuggable. This allows CPU sampling with minimal instrumentation instead of adding a timer to every emulated tick.
Profiles can include information about the running program and device: only share them with someone you trust.

With the matching APK running on a USB-debugging-enabled phone, download `android-profile-symbols` from its CI run and
unzip it to a directory named `symbols`. On a computer with the Android NDK and `adb` on `PATH`, run:

```sh
mkdir bemu-profile
cd bemu-profile
"$ANDROID_NDK_HOME/simpleperf/app_profiler.py" -p org.bemu.droid \
  -r "-e task-clock:u -f 200 --duration 20 -g" -lib /absolute/path/to/symbols
"$ANDROID_NDK_HOME/simpleperf/report.py" -g > report.txt
```

Keep the emulator visible and running the workload throughout the capture. The report separates sampled CPU time
by thread and function; focus on the `bemu` emulation thread. The `perf.data`, `binary_cache/` and `report.txt`
are saved in the current directory. Repeat under the same workload with CTM enabled and disabled. CPU sampling
does not measure GPU execution or time a thread spends waiting, and the sample percentages are estimates.
`app_profiler.py` may pull system libraries from the phone; the matching unstripped app libraries are required
for useful Rust symbols. Do not mix symbols and APKs from different builds.

### Signing CI releases

Android only updates an installed app when the new APK has the same signing certificate. CI requires a persistent
release key for pushes that publish the development build or a tagged release. Other CI builds can use a temporary
debug key, which is not suitable for updating an installed release.

To create a new key on macOS with Java and the authenticated GitHub CLI installed, run the following locally. The
password and keystore are stored in a private folder and are not printed or added to the repository:

```sh
(
set -eu
umask 077
mkdir -m 700 "$HOME/bemu-android-signing"
openssl rand -hex 32 > "$HOME/bemu-android-signing/password.txt"
keytool -genkeypair -v -keystore "$HOME/bemu-android-signing/release.jks" \
  -storetype PKCS12 -alias bemu -keyalg RSA -keysize 3072 -validity 10000 \
  -dname "CN=bemu Android" \
  -storepass:file "$HOME/bemu-android-signing/password.txt" \
  -keypass:file "$HOME/bemu-android-signing/password.txt"
base64 -i "$HOME/bemu-android-signing/release.jks" | gh secret set ANDROID_KEYSTORE_BASE64 --repo rgiot/bemu
gh secret set ANDROID_KEYSTORE_PASSWORD --repo rgiot/bemu < "$HOME/bemu-android-signing/password.txt"
printf %s bemu | gh secret set ANDROID_KEY_ALIAS --repo rgiot/bemu
gh secret set ANDROID_KEY_PASSWORD --repo rgiot/bemu < "$HOME/bemu-android-signing/password.txt"
)
```

Only generate a key if there is no existing release key to preserve. Keep the keystore and password backed up
securely: GitHub secrets cannot be read back, and losing the key prevents future in-place updates. Never commit
them. The command refuses to overwrite an existing signing directory. If uploading a secret fails, use the existing
files to retry the `gh secret set` commands rather than generating a new key. If a previously installed APK was
signed with a different key, it must be uninstalled before the first
installation with the new key; uninstalling can delete app data, so back up anything important first. Subsequent
release APKs signed with the same key can be installed as updates. Use `bemu-droid.apk` or `bemu-droid-arm64.apk`,
not the temporary-key debug APK.

## Licence

bemu's source code, this app's included, is under the MIT or the Apache-2.0 licence. The APK links
the HxC floppy library (HFE images, GPL-3.0-only), so **the APK is distributed under the GNU GPL,
version 3** (see `../NOTICE.md`). It must be given away, not sold, because of the terms of the CPC's
ROMs and of a part of HxC.

Settings > Licences lists everything in the app that is not bemu's own, with its licence. That
covers the ROMs, the C libraries inside HxC, the Rust crates, LLVM's libc++, AndroidX, Compose and
Kotlin. The list comes from `third_party/licenses.json` (written by `tools/licenses.py`), which the
build copies into the APK's assets along with `NOTICE.md` and the licences.
