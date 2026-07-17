import com.google.protobuf.gradle.id
import com.google.protobuf.gradle.proto

plugins {
    id("com.android.library")
    id("com.google.protobuf")
    id("maven-publish")
}

group = "org.linguamesh"
version = "0.1.0-alpha.1"

android {
    namespace = "org.linguamesh.core"
    compileSdk = 36
    buildToolsVersion = "36.0.0"
    ndkVersion = "28.2.13676358"

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
        externalNativeBuild {
            cmake {
                arguments += listOf(
                    "-DANDROID_STL=c++_static",
                    "-DLINGUAMESH_CORE_ROOT=${rootProject.projectDir.resolve("../..").canonicalPath}",
                )
                cppFlags += listOf("-std=c++20", "-Wall", "-Wextra", "-Werror")
            }
        }
        ndk {
            abiFilters += setOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
    }

    buildFeatures {
        buildConfig = false
        prefab = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    publishing {
        singleVariant("release") {
            withSourcesJar()
        }
    }

    sourceSets {
        named("main") {
            proto.srcDir("../../../contracts/proto")
        }
    }
}

dependencies {
    api("com.google.protobuf:protobuf-javalite:4.31.1")
    testImplementation("junit:junit:4.13.2")
}

protobuf {
    protoc {
        artifact = "com.google.protobuf:protoc:4.31.1"
    }
    generateProtoTasks {
        all().configureEach {
            builtins {
                id("java") {
                    option("lite")
                }
            }
        }
    }
}

publishing {
    publications {
        register<MavenPublication>("release") {
            groupId = "org.linguamesh"
            artifactId = "linguamesh-core-android"
            version = project.version.toString()
            afterEvaluate {
                from(components["release"])
            }
            pom {
                name = "LinguaMesh Core Android"
                description = "Kotlin and JNI wrapper for the LinguaMesh native core"
                url = "https://github.com/getio0909/linguamesh-core"
                licenses {
                    license {
                        name = "MIT License"
                        url = "https://opensource.org/license/mit"
                    }
                }
            }
        }
    }
}
