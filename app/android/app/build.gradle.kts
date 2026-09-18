import java.util.Base64

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// The demo is a separate app, not a mode of this one. Same package means the
// demo installs over the real app and lands in its private directory, which is
// where the real database lives — so it takes a package of its own and sits
// beside the real app instead of replacing it.
//
// Keyed on the same PACC_DEMO flag the Dart side reads, so no build can be a
// demo to Dart and the real app to Android. Flutter hands its --dart-define
// values to Gradle as one comma-separated list of base64 "KEY=VALUE" strings.
val isDemo: Boolean = project.findProperty("dart-defines")
    ?.toString()
    ?.split(",")
    ?.filter { it.isNotEmpty() }
    ?.any { String(Base64.getDecoder().decode(it)) == "PACC_DEMO=true" }
    ?: false

android {
    namespace = "com.tannerhathaway.app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.tannerhathaway.app"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName

        if (isDemo) applicationIdSuffix = ".demo"
        manifestPlaceholders["appLabel"] = if (isDemo) "Raven Demo" else "Raven Budgeting"
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
