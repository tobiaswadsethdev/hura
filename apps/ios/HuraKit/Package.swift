// swift-tools-version:6.0

import PackageDescription

let package = Package(
    name: "HuraKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "HuraKit", targets: ["HuraKit"]),
    ],
    targets: [
        // hura-core's tracker client, built from crates/hura-mobile by
        // Tools/build-core.sh.
        .binaryTarget(name: "HuraCoreFFI", path: "Build/HuraCore.xcframework"),
        .target(name: "HuraKit", dependencies: ["HuraCoreFFI"]),
        .testTarget(name: "HuraKitTests", dependencies: ["HuraKit"]),
    ]
)
