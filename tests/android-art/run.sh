#!/usr/bin/env bash
# Runs hello-java on ART, the JVM of Android, headless on an emulator (no game, no phone):
#   * the SDK classes and a tiny entry class are dexed into smoke.jar and put on app_process' class path,
#   * the example mod's jar gets a classes.dex (d8) next to its class files, like a mod made for Android,
#   * a Rust cdylib adopts the running VM (bhmod_core::jvm::attach_existing) and drives the real mod host.
# Needs: Android SDK (ANDROID_HOME, default ~/Android/Sdk) with the emulator and the API 30 x86_64 google_apis system image
# (the AVD bhmod_m9_api30 is created on first use; an AVD of its own so it never collides with another emulator),
# the NDK, rustup target x86_64-linux-android, a JDK, and scripts/build-java.sh run before. Starts an emulator that it
# stops again (only the process it started). A phone on adb is never touched: everything addresses the emulator by serial.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
sdk="${ANDROID_HOME:-$HOME/Android/Sdk}"
avd="${AVD:-bhmod_m9_api30}"
ndk="${ANDROID_NDK_HOME:-$(ls -d "$sdk"/ndk/* | sort -V | tail -1)}"
tc="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
bt="$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)"
platform="$(ls -d "$sdk"/platforms/android-* | sort -V | tail -1)"
adb="$sdk/platform-tools/adb"
"$root/scripts/build-java.sh" > /dev/null
out="$root/target/android-art"
rm -rf "$out" && mkdir -p "$out/classes"

# 1. dex: SDK classes + entry class.
javac -nowarn --release 8 -cp "$platform/android.jar" -d "$out/classes" \
  $(find "$root/sdk/java/src" -name '*.java') "$root/tests/android-art/java/ArtSmoke.java"
"$bt/d8" --release --min-api 26 --lib "$platform/android.jar" --output "$out/smoke.jar" $(find "$out/classes" -name '*.class')

# 2. the mods: a jar plus classes.dex (d8, compiled against the SDK jar, which stays out of the dex), like a mod made
# for Android. hello-java is the example; the others come from the test fixtures of the host suite.
sdkjar="$root/target/java-sdk/blockheads-mod-sdk.jar"
with_dex() { # jar-dir-with-classes out-jar
  mkdir -p "$out/dex-$$"
  "$bt/d8" --release --min-api 26 --lib "$platform/android.jar" --classpath "$sdkjar" --output "$out/dex-$$" $(find "$1" -name '*.class')
  (cd "$out/dex-$$" && zip -q "$2" classes.dex) && rm -rf "$out/dex-$$"
}
mkdir -p "$out/mods/hello/hello-java/server"
cp "$root/examples/hello-java/mod.toml" "$out/mods/hello/hello-java/"
mkdir -p "$out/hello-classes" && (cd "$out/hello-classes" && unzip -q -o "$root/examples/hello-java/server/mod.jar" -x 'META-INF/*')
cp "$root/examples/hello-java/server/mod.jar" "$out/mods/hello/hello-java/server/mod.jar"
with_dex "$out/hello-classes" "$out/mods/hello/hello-java/server/mod.jar"
printf '[hello-java]\ngrant = ["native", "players:chat"]\n' > "$out/mods/hello/mods.toml"

mkdir -p "$out/fx-classes"
javac -nowarn --release 8 -cp "$sdkjar" -d "$out/fx-classes" $(find "$root/crates/bhmod-core/tests/jvm-fixtures" -name '*.java')
fx_mod() { # scenario id main extra-toml grants
  local d="$out/mods/$1/$2"
  mkdir -p "$d/server" "$d/wit"
  printf 'package test:calc@1.0.0;\ninterface calc {}\n' > "$d/wit/calc.wit"
  printf 'schema = 1\nid = "%s"\nversion = "1.0.0"\nname = { en = "N" }\nauthors = ["a"]\nlicense = "MIT"\napi = "^0.2"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = { runtime = "jvm", path = "server/mod.jar" }\n[permissions]\nnative = true\nchannels = true\n%b' "$2" "$4" > "$d/mod.toml"
  printf 'Manifest-Version: 1.0\nBhmod-Main: %s\n' "$3" > "$out/mf"
  jar cfm "$d/server/mod.jar" "$out/mf" -C "$out/fx-classes" .
  with_dex "$out/fx-classes" "$d/server/mod.jar"
  printf '[%s]\ngrant = ["native", "channels"]\n' "$2" >> "$out/mods/$1/mods.toml"
}
fx_mod mutator mutator fx.Mutator ""
fx_mod services provider fx.Provider '[provides]\nservices = [{ name = "calc", version = "1.0.0", wit = "wit/calc.wit" }]\n'
fx_mod services consumer fx.Consumer '[dependencies]\nprovider = "^1"\n'
fx_mod faulty faulty fx.Faulty ""

# 3. the Rust library for the emulator's ABI.
export CC_x86_64_linux_android="$tc/x86_64-linux-android26-clang" AR_x86_64_linux_android="$tc/llvm-ar"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$tc/x86_64-linux-android26-clang"
(cd "$root/tests/android-art" && CARGO_TARGET_DIR="$out/cargo" cargo build --release --target x86_64-linux-android -q)
lib="$out/cargo/x86_64-linux-android/release/libbhmod_art_smoke.so"

# 4. emulator. Always our own on a fixed port and always addressed by serial: a phone on adb is never touched.
if ! "$sdk/emulator/emulator" -list-avds | grep -qx "$avd"; then
  echo no | "$sdk/cmdline-tools/latest/bin/avdmanager" create avd -n "$avd" -k "system-images;android-30;google_apis;x86_64" -d pixel_5 > /dev/null
fi
port="${EMU_PORT:-5590}"
serial="emulator-$port"
adb() { "$sdk/platform-tools/adb" -s "$serial" "$@"; }
"$sdk/emulator/emulator" -avd "$avd" -port "$port" -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect \
  -read-only > "$out/emulator.log" 2>&1 &
emu=$!
trap 'adb emu kill >/dev/null 2>&1 || true; kill "$emu" 2>/dev/null || true' EXIT
until adb get-state 2>/dev/null | grep -q device; do sleep 2; done
until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; do sleep 2; done
dir=/data/local/tmp/bhart
adb shell "rm -rf $dir && mkdir -p $dir"
adb push -q "$out/smoke.jar" "$lib" "$dir/"
adb push -q "$out/mods" "$dir/"
adb shell "CLASSPATH=$dir/smoke.jar app_process /system/bin ArtSmoke $dir/libbhmod_art_smoke.so $dir/mods" | tee "$out/transcript.txt"

# What ART must have produced (the same as on a desktop JVM).
for want in \
  'hello mods: hello-java Enabled' \
  'hello chat cancelled: true' \
  'hello /hello: Some(["Hello, ALICE!"])' \
  'hello log: chat to p1: Hello, ALICE! You are greeting number 1.' \
  'text: "QUIET"' \
  'services add: Some(["ok 42"])' \
  'services name: Some(["ok ALICE"])' \
  'provider-error: calc refused' \
  'faulty interrupted within 5 s: true' \
  'hard time budget exceeded'; do
  grep -qF -- "$want" "$out/transcript.txt" || { echo "MISSING: $want" >&2; exit 1; }
done
echo "ART smoke test passed"
