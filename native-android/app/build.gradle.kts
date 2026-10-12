import java.time.Instant
import groovy.json.JsonSlurper

plugins {
    id("com.android.application")
    kotlin("android")
    kotlin("kapt")
    kotlin("plugin.serialization")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Read the same public client configuration as the Google Services plugin, without
// requiring a household Firebase configuration for the isolated device-test package.
val firebaseFile = file("google-services.json")
val firebaseConfig = if (firebaseFile.exists()) JsonSlurper().parse(firebaseFile) as Map<*, *> else emptyMap<String, Any>()
val firebaseProject = firebaseConfig["project_info"] as? Map<*, *> ?: emptyMap<String, Any>()
val firebaseClient = (firebaseConfig["client"] as? List<*>)?.map { it as Map<*, *> }?.firstOrNull {
    val info = it["client_info"] as Map<*, *>
    (info["android_client_info"] as Map<*, *>)["package_name"] == "com.prestosupuesto.nativeapp"
}
val firebaseInfo = firebaseClient?.get("client_info") as? Map<*, *>
val firebaseKey = (firebaseClient?.get("api_key") as? List<*>)?.firstOrNull() as? Map<*, *>
fun quotedFirebase(value: Any?) = "\"${value?.toString().orEmpty()}\""

android {
    namespace = "com.presto.nativeapp"
    compileSdk = 36
    defaultConfig {
        applicationId = "com.prestosupuesto.nativeapp"
        manifestPlaceholders["appLabel"] = "Presto Native"
        manifestPlaceholders["loginScheme"] = "prestosupuesto-native"
        minSdk = 24
        targetSdk = 36
        buildConfigField("boolean", "FCM_ENABLED", (firebaseClient != null).toString())
        buildConfigField("String", "FCM_APP_ID", quotedFirebase(firebaseInfo?.get("mobilesdk_app_id")))
        buildConfigField("String", "FCM_API_KEY", quotedFirebase(firebaseKey?.get("current_key")))
        buildConfigField("String", "FCM_PROJECT_ID", quotedFirebase(firebaseProject["project_id"]))
        buildConfigField("String", "FCM_SENDER_ID", quotedFirebase(firebaseProject["project_number"]))
        versionCode = 1
        versionName = "2.0.0-native"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField(
            "String",
            "API_URL",
            "\"${providers.gradleProperty("financeApiUrl").getOrElse("https://athena.timdumol.com")}\"",
        )
        buildConfigField(
            "String",
            "BUILD_TIMESTAMP",
            "\"${providers.environmentVariable("PRESTO_BUILD_TIMESTAMP").getOrElse(Instant.now().toString())}\"",
        )
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        isCoreLibraryDesugaringEnabled = true
    }
    kotlinOptions { jvmTarget = "17" }
    buildTypes {
        release { isMinifyEnabled = false }
        create("deviceTest") {
            initWith(getByName("debug"))
            applicationIdSuffix = ".devicetest"
            buildConfigField("boolean", "FCM_ENABLED", "false")
            versionNameSuffix = "-device-test"
            matchingFallbacks += "debug"
            manifestPlaceholders["appLabel"] = "Presto Test"
            manifestPlaceholders["loginScheme"] = "prestosupuesto-native-test"
        }
    }
    // Connected tests may uninstall their target package during cleanup.
    testBuildType = "deviceTest"
    sourceSets.getByName("deviceTest").manifest.srcFile("src/debug/AndroidManifest.xml")
    packaging { resources.excludes += "/META-INF/{AL2.0,LGPL2.1}" }
}

kapt {
    arguments {
        arg("room.schemaLocation", "$projectDir/schemas")
        arg("room.incremental", "true")
    }
}

dependencies {
    implementation(project(":core"))
    implementation("com.google.firebase:firebase-messaging:25.0.1")
    implementation(platform("androidx.compose:compose-bom:2025.01.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("androidx.activity:activity-compose:1.10.1")
    // Firebase transitively brings an old Fragment; keep Activity Result compatible.
    implementation("androidx.fragment:fragment:1.8.6")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.8.7")
    implementation("androidx.browser:browser:1.8.0")
    implementation("androidx.room:room-runtime:2.7.2")
    implementation("androidx.room:room-ktx:2.7.2")
    kapt("androidx.room:room-compiler:2.7.2")
    implementation("androidx.work:work-runtime-ktx:2.9.1")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.0")
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
    testImplementation(kotlin("test-junit"))
    androidTestImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
    androidTestImplementation(kotlin("test-junit"))
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.01.00"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// Include Compose tooling and its test host in the isolated debug-like variant.
configurations.named("deviceTestImplementation") {
    extendsFrom(configurations.getByName("debugImplementation"))
}
