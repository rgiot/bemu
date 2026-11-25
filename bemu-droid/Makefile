.PHONY: setup debug

ADB ?= $(firstword $(wildcard $(ANDROID_HOME)/platform-tools/adb $(ANDROID_SDK_ROOT)/platform-tools/adb $(HOME)/Library/Android/sdk/platform-tools/adb $(HOME)/Android/Sdk/platform-tools/adb /opt/homebrew/bin/adb /usr/local/bin/adb) adb)
GRADLEW ?= ./gradlew
BEMU_ABIS ?= arm64-v8a
DEBUG_APK ?= app/build/outputs/apk/debug/app-universal-debug.apk
JAVA21 := $(shell /usr/libexec/java_home --failfast -v 21 2>/dev/null || ls -d /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home 2>/dev/null)
JAVA17 := $(shell /usr/libexec/java_home --failfast -v 17 2>/dev/null || ls -d /opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home 2>/dev/null)
BEMU_JAVA_HOME ?= $(or $(JAVA21),$(JAVA17))
DEBUG_PACKAGE := org.bemu.droid.debug

setup:
	cargo install cargo-ndk

debug:
	@command -v "$(ADB)" >/dev/null || { echo "adb not found. Install platform-tools (https://developer.android.com/tools/releases/platform-tools, or brew install --cask android-platform-tools), then run: make debug ADB=/path/to/platform-tools/adb" >&2; exit 1; }
	@"$(ADB)" get-state >/dev/null || { echo "Connect and authorize one Android device (or set ANDROID_SERIAL)" >&2; exit 1; }
	@test -n "$(BEMU_JAVA_HOME)" || { echo "Gradle needs JDK 17 or 21 (found only newer ones). Run: brew install openjdk@21, then: sudo ln -sfn /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk /Library/Java/JavaVirtualMachines/openjdk-21.jdk (or set BEMU_JAVA_HOME=/path/to/jdk)" >&2; exit 1; }
	JAVA_HOME="$(BEMU_JAVA_HOME)" BEMU_ABIS="$(BEMU_ABIS)" "$(GRADLEW)" --no-daemon assembleDebug -PbemuDebugApplicationIdSuffix=.debug
	@"$(ADB)" install -r "$(DEBUG_APK)"
	@output=$$("$(ADB)" shell am start -W -n "$(DEBUG_PACKAGE)/org.bemu.droid.MainActivity") || { echo "$$output"; exit 1; }; \
		printf '%s\n' "$$output"; \
		printf '%s\n' "$$output" | grep -q '^Status: ok' || { echo "App launch failed" >&2; exit 1; }
	@pid=$$("$(ADB)" shell pidof -s "$(DEBUG_PACKAGE)" | tr -d '\r'); \
		case "$$pid" in ''|*[!0-9]*) echo "App is not running" >&2; exit 1;; esac; \
		echo "Streaming app logcat (PID $$pid); press Ctrl-C to stop."; \
		"$(ADB)" logcat --pid="$$pid" -v threadtime