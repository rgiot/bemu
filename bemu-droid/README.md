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

## Building

You need:
- Rust, with the targets `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`
- The Android SDK with the NDK (`sdkmanager "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"`)
- JDK 17 or later
- cpclib checked out next to the repository (`../rust.cpcdemotools`), as for the desktop build

```sh
cd bemu-droid
echo "sdk.dir=$ANDROID_HOME" > local.properties
./gradlew assembleDebug        # or assembleRelease
```

`app/build/outputs/apk/` then holds the APK.

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

The CI (`.github/workflows/build.yml`, the `android` job) builds both APKs and uploads them as the `bemu-android`
artifact.
