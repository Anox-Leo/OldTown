import java.io.File
import java.util.Properties
import javax.inject.Inject
import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Racine du dépôt (apps/android/app → ../../..) : le cœur Rust est dans crates/.
val repoRoot: File = rootProject.projectDir.parentFile.parentFile

// Réglages locaux, jamais versionnés (local.properties) :
//   cargo.dir=/chemin/vers/.cargo/bin   (si cargo n'est pas dans ~/.cargo/bin)
//   oldtown.abis=arm64-v8a              (téléphone seul : build plus rapide)
//   oldtown.signing.storeFile=…, .storePassword, .keyAlias, .keyPassword
val localProps = Properties().apply {
    val f = rootProject.file("local.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
val cargoBinDir: String = localProps.getProperty("cargo.dir")
    ?: File(System.getProperty("user.home"), ".cargo/bin").path
val rustAbis: List<String> = (localProps.getProperty("oldtown.abis") ?: "arm64-v8a,x86_64")
    .split(',').map { it.trim() }.filter { it.isNotEmpty() }

android {
    namespace = "dev.oldtown.android"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "dev.oldtown.android"
        minSdk = 33
        targetSdk = 37
        // majeur × 10000 + mineur × 100 + correctif ; versionName identique
        // à la version du workspace Cargo et de l'extension.
        versionCode = 10000
        versionName = "1.0.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += rustAbis }
    }

    // Clé de signature personnelle hors dépôt (private/, note de l'étape 3).
    val signing = localProps.getProperty("oldtown.signing.storeFile")?.let { path ->
        signingConfigs.create("personal") {
            storeFile = File(path)
            storePassword = localProps.getProperty("oldtown.signing.storePassword")
            keyAlias = localProps.getProperty("oldtown.signing.keyAlias")
            keyPassword = localProps.getProperty("oldtown.signing.keyPassword")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signing
        }
    }

    buildFeatures {
        compose = true
    }

    packaging {
        // Une seule copie de la bibliothèque Rust par ABI.
        jniLibs.useLegacyPackaging = false
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    lint {
        abortOnError = true
        warningsAsErrors = false
        // Versions des dépendances : mises à jour par Dependabot, pas par lint.
        disable += setOf("GradleDependency", "NewerVersionAvailable", "AndroidGradlePluginVersion")
        // Ni ChromeOS ni tablettes visés : un téléphone (x86_64 sert à l'émulateur).
        disable += "ChromeOsAbiSupport"
    }
}

kotlin {
    jvmToolchain(21)
}

/** Lance une commande cargo depuis la racine du dépôt. */
abstract class CargoTask @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
    @get:Internal abstract val workDir: DirectoryProperty
    @get:Input abstract val cargoDir: Property<String>

    /** Sources Rust : la tâche est à jour tant qu'elles ne changent pas. */
    @get:InputDirectory @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val crates: DirectoryProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val lockFile: org.gradle.api.file.RegularFileProperty

    protected fun cargo(args: List<String>, env: Map<String, String> = emptyMap()) {
        val dir = cargoDir.get()
        val cargo = File(dir, if (hostIsWindows()) "cargo.exe" else "cargo")
        check(cargo.exists()) {
            "cargo introuvable dans $dir : installer Rust (rustup), ou régler cargo.dir dans local.properties"
        }
        exec.exec {
            workingDir = this@CargoTask.workDir.get().asFile
            executable = cargo.path
            args(args)
            environment("PATH", dir + File.pathSeparator + System.getenv("PATH"))
            env.forEach { (k, v) -> environment(k, v) }
        }
    }


    companion object {
        /** Système de la machine de build (pas une entrée de tâche). */
        fun hostIsWindows() = System.getProperty("os.name").lowercase().contains("windows")
    }
}

/** Compile ot-ffi pour Android (cargo-ndk), dans `outputDir/<abi>/libot_ffi.so`. */
abstract class CargoNdkTask @Inject constructor(exec: ExecOperations) : CargoTask(exec) {
    @get:Input abstract val abis: ListProperty<String>
    @get:Input abstract val ndkDir: Property<String>
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        // Le NDK n'est pas téléchargé par AGP pour cette tâche : message clair
        // plutôt qu'une erreur du cache de configuration.
        val ndk = File(ndkDir.get())
        if (!ndk.isDirectory) {
            throw org.gradle.api.GradleException(
                "NDK ${ndk.name} absent ($ndk). Android Studio : Settings → Android SDK → SDK Tools → " +
                    "Show Package Details → NDK (Side by side) ${ndk.name}, ou : sdkmanager \"ndk;${ndk.name}\"",
            )
        }
        val out = outputDir.get().asFile
        out.deleteRecursively()
        val targets = abis.get().flatMap { listOf("-t", it) }
        cargo(
            listOf("ndk") + targets + listOf("-o", out.path, "build", "-p", "ot-ffi", "--profile", "android", "--locked"),
            mapOf("ANDROID_NDK_HOME" to ndk.path),
        )
    }
}

/** Génère les liaisons Kotlin d'ot-ffi avec uniffi-bindgen (jamais versionnées). */
abstract class UniffiTask @Inject constructor(exec: ExecOperations) : CargoTask(exec) {
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        // Bibliothèque de l'hôte, non dépouillée : elle porte les métadonnées UniFFI.
        cargo(listOf("build", "-p", "ot-ffi", "--locked"))
        val os = System.getProperty("os.name").lowercase()
        val lib = when {
            os.contains("mac") -> "libot_ffi.dylib"
            CargoTask.hostIsWindows() -> "ot_ffi.dll"
            else -> "libot_ffi.so"
        }
        val library = File(workDir.get().asFile, "target/debug/$lib")
        cargo(
            listOf(
                "run", "-p", "ot-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen", "--locked", "--",
                "generate", "--library", library.path, "--language", "kotlin",
                "--out-dir", out.path, "--no-format",
            ),
        )
    }
}

val cargoNdk = tasks.register<CargoNdkTask>("cargoNdk") {
    group = "rust"
    description = "Compile le cœur Rust (ot-ffi) pour Android."
    workDir.set(repoRoot)
    cargoDir.set(cargoBinDir)
    crates.set(File(repoRoot, "crates"))
    lockFile.set(File(repoRoot, "Cargo.lock"))
    abis.set(rustAbis)
    // Chemin calculé sans exiger que le NDK soit installé à la configuration.
    ndkDir.set(androidComponents.sdkComponents.sdkDirectory.map { it.dir("ndk/${android.ndkVersion}").asFile.path })
    outputDir.set(layout.buildDirectory.dir("rust/jniLibs"))
}

val uniffiBindings = tasks.register<UniffiTask>("uniffiBindings") {
    group = "rust"
    description = "Génère les liaisons Kotlin du cœur Rust."
    workDir.set(repoRoot)
    cargoDir.set(cargoBinDir)
    crates.set(File(repoRoot, "crates"))
    lockFile.set(File(repoRoot, "Cargo.lock"))
    outputDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
}

androidComponents {
    onVariants { variant ->
        variant.sources.kotlin?.addGeneratedSourceDirectory(uniffiBindings, UniffiTask::outputDir)
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoNdk, CargoNdkTask::outputDir)
    }
}

dependencies {
    implementation(libs.androidx.core)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.cpp)
    implementation(libs.coroutines.android)
    // Liaisons UniFFI : JNA en AAR (bibliothèques natives Android incluses).
    implementation(libs.jna) { artifact { type = "aar" } }

    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    testImplementation(libs.coroutines.test)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.coroutines.test)
}
