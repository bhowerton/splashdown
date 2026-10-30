// swift-tools-version:5.9
// Static library exposing Apple's on-device speech and language models to
// Rust through a small C ABI. Built by ../build.rs.
import PackageDescription

let package = Package(
    name: "SplashdownAppleShim",
    platforms: [.macOS(.v13)],
    products: [
        .library(name: "SplashdownAppleShim", type: .static, targets: ["SplashdownAppleShim"]),
    ],
    targets: [
        .target(
            name: "SplashdownAppleShim",
            swiftSettings: [
                // FoundationModels only exists on macOS 26+. Don't let the
                // compiler record a hard dependency on it; build.rs weak-
                // links it so the app still launches on older systems.
                .unsafeFlags(["-Xfrontend", "-disable-autolink-framework", "-Xfrontend", "FoundationModels"]),
            ]
        ),
    ]
)
