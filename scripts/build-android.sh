#!/usr/bin/env bash
set -euo pipefail

# Local ARM64 Android build script for FigCraft (V2)
NDK_VERSION="${NDK_VERSION:-27.2.12479018}"
ANDROID_API="${ANDROID_API:-29}"

SDK_DIR="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [[ -z "$SDK_DIR" ]]; then
  echo "Error: ANDROID_HOME or ANDROID_SDK_ROOT is not set." >&2
  exit 1
fi

HOST_TAG="linux-x86_64"
if [[ "$(uname -s)" == "Darwin" ]]; then
  HOST_TAG="darwin-x86_64"
  if [[ -d "$SDK_DIR/ndk/${NDK_VERSION}/toolchains/llvm/prebuilt/darwin-arm64" ]]; then HOST_TAG="darwin-arm64"; fi
fi

NDK="$SDK_DIR/ndk/${NDK_VERSION}"
BIN="$NDK/toolchains/llvm/prebuilt/${HOST_TAG}/bin"
SYSROOT_LIB="$NDK/toolchains/llvm/prebuilt/${HOST_TAG}/sysroot/usr/lib/aarch64-linux-android"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/aarch64-linux-android${ANDROID_API}-clang"
export CC_aarch64_linux_android="$BIN/aarch64-linux-android${ANDROID_API}-clang"
export CXX_aarch64_linux_android="$BIN/aarch64-linux-android${ANDROID_API}-clang++"
export AR_aarch64_linux_android="$BIN/llvm-ar"

echo "[1/3] Running host unit tests..."
cargo test -p figcraft-android

echo "[2/3] Cross-compiling libfigcraft_android.so (aarch64-linux-android)..."
cargo build -p figcraft-android --lib --target aarch64-linux-android --release

JNI_OUT="android/app/src/main/jniLibs/arm64-v8a"
mkdir -p "$JNI_OUT"
cp target/aarch64-linux-android/release/libfigcraft_android.so "$JNI_OUT/"
cp "$SYSROOT_LIB/libc++_shared.so" "$JNI_OUT/"

echo "[3/3] Packaging APK via Gradle..."
gradle -p android :app:assembleDebug

echo "Done! APK path: android/app/build/outputs/apk/debug/app-debug.apk"
