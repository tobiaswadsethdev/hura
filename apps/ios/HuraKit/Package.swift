// swift-tools-version:6.0

import PackageDescription

let package = Package(
    name: "HuraKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "HuraKit", targets: ["HuraKit"]),
    ],
    targets: [
        .target(name: "HuraKit"),
        .testTarget(name: "HuraKitTests", dependencies: ["HuraKit"]),
    ]
)
