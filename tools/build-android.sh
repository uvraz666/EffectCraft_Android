#!/usr/bin/env bash
# Build the EffectCraft Android app (arm64 phones): the Rust library with cargo-ndk, then the APK.
#
#   tools/build-android.sh            # debug APK
#   tools/build-android.sh release    # release APK (signed with the debug key; use your own for a store)
#
# Needs: Rust 1.95+, JDK 17, the Android SDK (ANDROID_HOME) and NDK r27+ (ANDROID_NDK_HOME), and
# either Gradle 8.9+ or the wrapper (`cd android && gradle wrapper --gradle-version 8.9`).
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-debug}"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to your NDK (Android Studio > SDK Manager > SDK Tools > NDK)}"

rustup target add aarch64-linux-android
command -v cargo-ndk >/dev/null || cargo install cargo-ndk --locked

cargo_flags=()
[ "$mode" = release ] && cargo_flags+=(--release)

# No --locked: the first build adds the Android-only crates (jni, android_logger) to Cargo.lock.
# The workspace's release profile (thin LTO, one codegen unit) needs hours and a lot of memory on a CI
# runner; these overrides keep the build fast and small enough. Set EC_FULL_OPT=1 to use the profile as is.
if [ -z "${EC_FULL_OPT:-}" ]; then
  export CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_DEBUG=0
fi
export CARGO_INCREMENTAL=0

cargo ndk -t arm64-v8a --platform 29 -o android/app/src/main/jniLibs build "${cargo_flags[@]}" -p effectcraft-android

# cpal's Android backend (oboe) links the shared C++ runtime; make sure it ships in the APK.
libs=android/app/src/main/jniLibs/arm64-v8a
if [ ! -f "$libs/libc++_shared.so" ]; then
  cxx="$(find "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" -path '*aarch64-linux-android/libc++_shared.so' | head -n 1)"
  [ -n "$cxx" ] && cp "$cxx" "$libs/" && echo "copied $cxx"
fi
ls -la "$libs"

cd android
gradle_cmd=./gradlew
[ -x "$gradle_cmd" ] || gradle_cmd=gradle
if [ "$mode" = release ]; then "$gradle_cmd" assembleRelease; else "$gradle_cmd" assembleDebug; fi
echo
find app/build/outputs/apk -name '*.apk' -print
