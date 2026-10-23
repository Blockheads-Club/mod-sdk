// Gradle build for {{id}}: `gradle jar` writes server/mod.jar.
// For Kotlin add `kotlin("jvm") version "<version>"` to the plugins and put the sources in src/main/kotlin.
plugins {
    java
}

java {
    // Java 8 bytecode runs on every JRE and on Android.
    sourceCompatibility = JavaVersion.VERSION_1_8
    targetCompatibility = JavaVersion.VERSION_1_8
}

dependencies {
    // The host provides the SDK at run time: compile against it, do not bundle it.
    compileOnly(files(System.getenv("BHMOD_SDK_JAR") ?: "libs/blockheads-mod-sdk.jar"))
}

tasks.jar {
    archiveFileName.set("mod.jar")
    destinationDirectory.set(layout.projectDirectory.dir("server"))
    manifest {
        attributes("Bhmod-Main" to "{{javapkg}}.Main")
    }
}
