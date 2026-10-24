#!/usr/bin/env bash
# Builds the Android mod loader: libbhmodloader.so (armeabi-v7a, API 24) and its Java UI (classes.dex), packed as
# target/android-loader/bhmod-android-loader-<version>.zip with loader.json (version, sha256 of each file). The
# Patcher's "Add the mod loader" patch pins that zip by SHA-256.
#
# Needs: rustup target armv7-linux-androideabi, the Android NDK (ANDROID_NDK_HOME or ~/Android/Sdk/ndk/<latest>),
# a JDK (javac) and the SDK build tools (d8) plus a platform android.jar (ANDROID_HOME or ~/Android/Sdk).
# Usage: scripts/build-android-loader.sh [--no-rust] [--jvm]
#   --jvm  also builds the JVM runtime (Java/Kotlin mods on ART): the Rust `jvm` feature and the SDK classes
#          (sdk/java/src) in classes.dex. Off by default; the owner then switches it on with a `jvm-enabled` file.
set -euo pipefail
no_rust=0; jvm=0
for a in "$@"; do
  case "$a" in
    --no-rust) no_rust=1 ;;
    --jvm) jvm=1 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done
root="$(cd "$(dirname "$0")/.." && pwd)"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ndk="${ANDROID_NDK_HOME:-$(ls -d "$sdk"/ndk/* | sort -V | tail -1)}"
tc="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
out="$root/target/android-loader"
crate="$root/crates/bhmod-android"
version="$(grep -m1 '^version' "$root/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')"
mkdir -p "$out"

if [[ $no_rust == 0 ]]; then
  export CC_armv7_linux_androideabi="$tc/armv7a-linux-androideabi24-clang"
  export CXX_armv7_linux_androideabi="$tc/armv7a-linux-androideabi24-clang++"
  export AR_armv7_linux_androideabi="$tc/llvm-ar"
  export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$tc/armv7a-linux-androideabi24-clang"
  # Luau is C++: link libc++ statically (the game ships its own, older libcxx.so).
  export CXXSTDLIB_armv7_linux_androideabi=c++_static
  export RUSTFLAGS="--cfg pulley_assume_llvm_makes_tail_calls -C link-arg=-lc++abi -C link-arg=-Wl,-soname,libbhmodloader.so"
  features=(); [[ $jvm == 1 ]] && features=(--features jvm)
  cargo build --release --locked --target armv7-linux-androideabi -p bhmod-android "${features[@]}"
  "$tc/llvm-strip" --strip-unneeded -o "$out/libbhmodloader.so" "${CARGO_TARGET_DIR:-$root/target}/armv7-linux-androideabi/release/libbhmodloader.so"
fi

platform="$(ls -d "$sdk"/platforms/android-* | sort -V | tail -1)"
bt="$(ls -d "$sdk"/build-tools/* | sort -V | tail -1)"
classes="$out/java-classes"
rm -rf "$classes" && mkdir -p "$classes"
# Stubs of the game's own Java API (com.apportable.*) are compiled against, never shipped.
sources=("$crate/java/src" "$crate/java/stubs")
[[ $jvm == 1 ]] && sources+=("$root/sdk/java/src")
javac -nowarn --release 8 -encoding UTF-8 -cp "$platform/android.jar" -d "$classes" \
  $(find "${sources[@]}" -name '*.java')
stubs="$out/java-stubs"
rm -rf "$stubs" && mkdir -p "$stubs/com" && mv "$classes/com/apportable" "$stubs/com/"
(cd "$stubs" && jar cf ../stubs.jar com)
"$bt/d8" --release --min-api 21 --lib "$platform/android.jar" --classpath "$out/stubs.jar" --output "$out" \
  $(find "$classes" -name '*.class')
rm -rf "$classes" "$stubs" "$out/stubs.jar"

cd "$out"
sha() { sha256sum "$1" | cut -d' ' -f1; }
cat > loader.json <<JSON
{
  "version": "$version",
  "abi": "armeabi-v7a",
  "lib": { "name": "libbhmodloader.so", "sha256": "$(sha libbhmodloader.so)", "size": $(stat -c %s libbhmodloader.so) },
  "dex": { "name": "classes.dex", "sha256": "$(sha classes.dex)", "size": $(stat -c %s classes.dex) },
  "listener": "com.blockheadsclub.modloader.Listener",
  "libName": "bhmodloader"
}
JSON
zip="bhmod-android-loader-$version.zip"
rm -f "$zip"
# Fixed timestamps so the zip (and its pinned SHA-256) is reproducible for the same inputs.
touch -d '2026-01-01 00:00:00 UTC' loader.json libbhmodloader.so classes.dex
zip -X -q -0 "$zip" loader.json libbhmodloader.so classes.dex
echo "$out/$zip $(sha "$zip")"
