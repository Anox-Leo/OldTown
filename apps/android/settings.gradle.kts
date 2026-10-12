// Appli Android d'OldTown (étape 3). Le cœur Rust (crates/ot-ffi) est
// compilé par la tâche `cargoNdk` de :app ; voir apps/android/CLAUDE.md.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "oldtown"
include(":app")
