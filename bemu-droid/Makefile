.PHONY: setup debug profile

ADB ?= $(firstword $(wildcard $(ANDROID_HOME)/platform-tools/adb $(ANDROID_SDK_ROOT)/platform-tools/adb $(HOME)/Library/Android/sdk/platform-tools/adb $(HOME)/Android/Sdk/platform-tools/adb /opt/homebrew/bin/adb /usr/local/bin/adb) adb)
GRADLEW ?= ./gradlew
BEMU_ABIS ?= arm64-v8a
DEBUG_APK ?= app/build/outputs/apk/debug/app-universal-debug.apk
DEBUG_PACKAGE := org.bemu.droid.debug

SDK_ROOT ?= $(or $(ANDROID_HOME),$(HOME)/Library/Android/sdk)
CMDLINE_ZIP ?= commandlinetools-mac-11076708_latest.zip
NDK_VERSION := 27.2.12479018

setup:
	mkdir -p "$(SDK_ROOT)/cmdline-tools"
	test -x "$(SDK_ROOT)/cmdline-tools/latest/bin/sdkmanager" || { \
		curl -fL -o /tmp/bemu-cmdline.zip "https://dl.google.com/android/repository/$(CMDLINE_ZIP)" && \
		rm -rf "$(SDK_ROOT)/cmdline-tools/cmdline-tools" && \
		unzip -q -o /tmp/bemu-cmdline.zip -d "$(SDK_ROOT)/cmdline-tools" && \
		mv "$(SDK_ROOT)/cmdline-tools/cmdline-tools" "$(SDK_ROOT)/cmdline-tools/latest"; }
	yes | "$(SDK_ROOT)/cmdline-tools/latest/bin/sdkmanager" --sdk_root="$(SDK_ROOT)" --licenses >/dev/null || true
	"$(SDK_ROOT)/cmdline-tools/latest/bin/sdkmanager" --sdk_root="$(SDK_ROOT)" "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;$(NDK_VERSION)"
	rustup target add aarch64-linux-android x86_64-linux-android
	cargo install cargo-ndk
	echo "sdk.dir=$(SDK_ROOT)" > local.properties

debug:
	@command -v "$(ADB)" >/dev/null || { echo "adb not found. Install platform-tools (https://developer.android.com/tools/releases/platform-tools, or brew install --cask android-platform-tools), then run: make debug ADB=/path/to/platform-tools/adb" >&2; exit 1; }
	@"$(ADB)" get-state >/dev/null || { echo "Connect and authorize one Android device (or set ANDROID_SERIAL)" >&2; exit 1; }
	BEMU_ABIS="$(BEMU_ABIS)" "$(GRADLEW)" --no-daemon assembleDebug -PbemuDebugApplicationIdSuffix=.debug
	@"$(ADB)" install -r "$(DEBUG_APK)"
	@output=$$("$(ADB)" shell am start -W -n "$(DEBUG_PACKAGE)/org.bemu.droid.MainActivity") || { echo "$$output"; exit 1; }; \
		printf '%s\n' "$$output"; \
		printf '%s\n' "$$output" | grep -q '^Status: ok' || { echo "App launch failed" >&2; exit 1; }
	@pid=$$("$(ADB)" shell pidof -s "$(DEBUG_PACKAGE)" | tr -d '\r'); \
		case "$$pid" in ''|*[!0-9]*) echo "App is not running" >&2; exit 1;; esac; \
		echo "Streaming app logcat (PID $$pid); press Ctrl-C to stop."; \
		"$(ADB)" logcat --pid="$$pid" -v threadtime
# Records the running debug app with simpleperf (start it with `make debug`, unlock the phone and
# run the demo to measure first); the report is profile.txt, the raw data profile.data.
PROFILE_SECONDS ?= 15
SYMBOLS := rust/target/android-symbols/$(BEMU_ABIS)
SIMPLEPERF := $(lastword $(sort $(wildcard $(SDK_ROOT)/ndk/*/simpleperf)))

profile:
	@command -v "$(ADB)" >/dev/null || { echo "adb not found; set ADB" >&2; exit 1; }
	"$(ADB)" shell simpleperf record --app "$(DEBUG_PACKAGE)" -g -f 1000 --duration $(PROFILE_SECONDS) -o /data/local/tmp/bemu.data
	"$(ADB)" pull /data/local/tmp/bemu.data profile.data
	"$(SIMPLEPERF)/bin/darwin/x86_64/simpleperf" report -i profile.data --symfs "$(SYMBOLS)" --sort comm,dso,symbol -n > profile.txt
	@head -40 profile.txt
