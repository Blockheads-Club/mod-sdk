#!/usr/bin/env sh
# Builds server/mod.jar with plain javac (a Gradle build is in build.gradle.kts).
# Needs a JDK 8 or newer and the SDK jar: libs/blockheads-mod-sdk.jar, or the path in $BHMOD_SDK_JAR.
set -eu
cd "$(dirname "$0")"
sdk="${BHMOD_SDK_JAR:-libs/blockheads-mod-sdk.jar}"
if [ ! -f "$sdk" ]; then
  echo "missing $sdk: download blockheads-mod-sdk.jar from the mod-sdk release (or run scripts/build-java.sh there) and put it in libs/" >&2
  exit 1
fi
rm -rf build
mkdir -p build/classes server
javac -nowarn --release 8 -encoding UTF-8 -cp "$sdk" -d build/classes $(find src -name '*.java')
printf 'Manifest-Version: 1.0\nBhmod-Main: {{javapkg}}.Main\n' > build/manifest.mf
jar cfm server/mod.jar build/manifest.mf -C build/classes .
echo "built server/mod.jar"
