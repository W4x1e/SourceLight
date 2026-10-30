import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "app.sourcelight.photos"
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

// Android's own MediaStore and image decoders: nothing third-party.
dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    // Reading and writing EXIF, HEIC's included (Apache-2.0, AndroidX).
    implementation("androidx.exifinterface:exifinterface:1.3.7")
    implementation(project(":tauri-android"))
}
