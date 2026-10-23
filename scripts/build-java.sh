#!/usr/bin/env sh
# Builds the Java SDK (target/java-sdk/blockheads-mod-sdk.jar) and every Java example mod
# (examples/*-java/server/mod.jar). The bhmod-core JVM tests and the conformance suite need the example jars.
# Needs a JDK 8 or newer (javac, jar). Java 8 bytecode, so the jars run on any JRE and on Android through d8.
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/target/java-sdk"
rm -rf "$out"
mkdir -p "$out/classes"
find "$root/sdk/java/src" -name '*.java' > "$out/sources.txt"
javac -nowarn -Xlint:-options --release 8 -encoding UTF-8 -d "$out/classes" @"$out/sources.txt"
jar cf "$out/blockheads-mod-sdk.jar" -C "$out/classes" .
echo "built target/java-sdk/blockheads-mod-sdk.jar"
for dir in "$root"/examples/*-java; do
  name="$(basename "$dir")"
  classes="$out/$name"
  mkdir -p "$classes"
  find "$dir/src" -name '*.java' > "$out/$name.txt"
  javac -nowarn -Xlint:-options --release 8 -encoding UTF-8 -cp "$out/blockheads-mod-sdk.jar" -d "$classes" @"$out/$name.txt"
  printf 'Manifest-Version: 1.0\nBhmod-Main: %s\n' "$(cat "$dir/main-class")" > "$out/$name.mf"
  mkdir -p "$dir/server"
  jar cfm "$dir/server/mod.jar" "$out/$name.mf" -C "$classes" .
  echo "built examples/$name/server/mod.jar"
done
