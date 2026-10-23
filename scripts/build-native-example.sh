#!/usr/bin/env bash
# Builds the native example mod sdk/native/hello-c with the Android NDK and packs it as
# target/native-examples/native-hello-<version>.bhmod (deterministic like bhmod pack). Prints "id version sha256 size".
# Needs: the NDK (ANDROID_NDK_HOME or ~/Android/Sdk/ndk/<latest>). The library is armeabi-v7a, ARM mode, no C runtime.
# Usage: scripts/build-native-example.sh [DIR]    (DIR: a native mod folder with src/*.c, default sdk/native/hello-c)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${1:-$root/sdk/native/hello-c}"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
ndk="${ANDROID_NDK_HOME:-$(ls -d "$sdk"/ndk/* | sort -V | tail -1)}"
cc="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/armv7a-linux-androideabi24-clang"
out="${OUT:-$root/target/native-examples}"
id="$(sed -n 's/^id = "\(.*\)"/\1/p' "$dir/mod.toml" | head -1)"
ver="$(sed -n 's/^version = "\(.*\)"/\1/p' "$dir/mod.toml" | head -1)"
lib="lib$(echo "$id" | tr - _).so"
work="$out/$id"
rm -rf "$work" && mkdir -p "$work/client/native/armeabi-v7a"
cp "$dir/mod.toml" "$dir/LICENSE" "$work/"
cp -r "$dir/client/." "$work/client/"
"$cc" -marm -O2 -fPIC -shared -nostdlib -ffreestanding -fno-stack-protector -fvisibility=hidden \
  -fno-asynchronous-unwind-tables -Wall -Werror -ffile-prefix-map="$root"=/mod-sdk \
  -Wl,--build-id=none -Wl,-soname,"$lib" -Wl,--no-undefined -Wl,-z,max-page-size=4096 \
  -I "$root/crates/bhmod-native/include" "$dir"/src/*.c -o "$work/client/native/armeabi-v7a/$lib"
# Only bhmod_native_init / bhmod_native_shutdown are exported.
"$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip" --strip-unneeded "$work/client/native/armeabi-v7a/$lib"
bhmod="${BHMOD:-$root/target/debug/bhmod}"
[ -x "$bhmod" ] || cargo build --locked -q -p bhmod --manifest-path "$root/Cargo.toml"
"$bhmod" pack "$work" -o "$out/$id-$ver.bhmod" > /dev/null
echo "$id $ver $(sha256sum "$out/$id-$ver.bhmod" | cut -d' ' -f1) $(stat -c %s "$out/$id-$ver.bhmod")"
