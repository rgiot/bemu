#!/usr/bin/env bash
# Builds libbemu_droid.so for the app's ABIs into app/src/main/jniLibs (what the Gradle build
# runs before packaging; see ../README.md).
#
#   bemu-droid/rust/build-android.sh [--debug] [ABI...]     (ABIs: arm64-v8a x86_64 armeabi-v7a x86)
#
# Needs cargo-ndk (cargo install cargo-ndk), the Rust targets of the ABIs (rustup target add
# aarch64-linux-android x86_64-linux-android) and the NDK (ANDROID_NDK_HOME, or ANDROID_HOME's ndk/).
#
# hxcfe-sys 0.3.4 (HFE images) casts strings to *mut i8 where its bindings take c_char, which is
# u8 on ARM: the crates (hxcfe too) do not build there. Until a fixed release, the build uses copies
# with the casts fixed (target/patched), patched in with `--config patch.crates-io...`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
profile=release
abis=()
for arg in "$@"; do
    case "$arg" in
        --debug) profile=debug ;;
        *) abis+=("$arg") ;;
    esac
done
[ ${#abis[@]} -eq 0 ] && abis=(arm64-v8a x86_64)

if [ -z "${ANDROID_HOME:-}" ]; then
    # Gradle's SDK location when the environment has none
    sdk=$(sed -n 's/^sdk.dir=//p' "$here/../local.properties" 2>/dev/null | head -n 1)
    [ -n "$sdk" ] && export ANDROID_HOME="$sdk"
fi
# The NDK CI tests (newer ones, e.g. Homebrew's, break the bindgen builds), when installed
pinned="${ANDROID_HOME:-}/ndk/27.2.12479018"
[ -d "$pinned" ] && export ANDROID_NDK_HOME="$pinned"
if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/ndk" ]; then
    ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$(ls "$ANDROID_HOME/ndk" | sort -V | tail -n 1)"
    export ANDROID_NDK_HOME
fi

cd "$here"
target_dir="${CARGO_TARGET_DIR:-$here/target}"
cargo fetch --quiet

# hxcfe-sys and hxcfe, their c_char casts fixed
version=0.3.4
patches=()
for crate in hxcfe-sys hxcfe; do
    patched="$target_dir/patched/$crate"
    if [ ! -f "$patched/.fixed-$version" ]; then
        source=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 2 -type d -name "$crate-$version" | head -n 1)
        [ -n "$source" ] || { echo "$crate $version not found in cargo's registry" >&2; exit 1; }
        rm -rf "$patched"
        mkdir -p "$(dirname "$patched")"
        cp -r "$source" "$patched"
        # hxcfe-sys writes the casts in the code its build script generates; hxcfe in its sources
        for file in "$patched/build.rs" $(find "$patched/src" -name '*.rs'); do
            [ -f "$file" ] && perl -pi -e 's/\*mut i8\b/*mut ::std::os::raw::c_char/g' "$file"
        done
        touch "$patched/.fixed-$version"
    fi
    patches+=(--config "patch.crates-io.$crate.path=\"$patched\"")
done

flags=()
[ "$profile" = release ] && flags+=(--release)
targets=()
for abi in "${abis[@]}"; do targets+=(-t "$abi"); done

cargo ndk "${targets[@]}" -P 26 -o "$here/../app/src/main/jniLibs" build "${flags[@]}" "${patches[@]}"

# Only the app's library (cargo-ndk copies the cdylibs of dependencies too), and the C++ runtime
# that the C++ code of dependencies links to.
sysroot=$(find "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" -maxdepth 2 -type d -name sysroot | head -n 1)
strip="$(dirname "$sysroot")/bin/llvm-strip"
[ -x "$strip" ] || { echo "NDK llvm-strip not found: $strip" >&2; exit 1; }
for abi in "${abis[@]}"; do
    out="$here/../app/src/main/jniLibs/$abi"
    find "$out" -name '*.so' ! -name libbemu_droid.so -delete
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        armeabi-v7a) triple=arm-linux-androideabi ;;
        x86_64) triple=x86_64-linux-android ;;
        x86) triple=i686-linux-android ;;
    esac
    symbols="$target_dir/android-symbols/$abi"
    mkdir -p "$symbols"
    cp "$out/libbemu_droid.so" "$symbols/libbemu_droid.so"
    "$strip" --strip-unneeded "$out/libbemu_droid.so"
    cp "$sysroot/usr/lib/$triple/libc++_shared.so" "$out/"
done
