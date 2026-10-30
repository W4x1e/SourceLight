import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.sourcelight.qrscan"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

// ZXing, Apache-2.0 both: the scanner screen (zxing-android-embedded) and the
// decoder it drives, raised to the current release. This replaces the
// official barcode-scanner plugin, whose Android side is Google's
// closed-source ML Kit.
dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    // ActivityResult, for the scanner screen's answer. Tauri's own Android
    // library uses it but does not pass it on.
    implementation("androidx.activity:activity-ktx:1.7.2")
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    implementation("com.google.zxing:core:3.5.4")
    implementation(project(":tauri-android"))
}
