// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "LinguaMeshCore",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "LinguaMeshCore", targets: ["LinguaMeshCore"]),
    ],
    targets: [
        .binaryTarget(
            name: "CLinguaMeshCore",
            path: "Artifacts/LinguaMeshCore.xcframework"
        ),
        .target(
            name: "LinguaMeshCore",
            dependencies: ["CLinguaMeshCore"]
        ),
        .testTarget(
            name: "LinguaMeshCoreTests",
            dependencies: ["LinguaMeshCore"]
        ),
    ]
)
