// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ImpulseApp",
    platforms: [
        .macOS(.v26)
    ],
    dependencies: [
        .package(url: "https://github.com/LebJe/TOMLKit.git", from: "0.6.0"),
        // Pinned to an exact gfm-branch revision (Package.resolved is not
        // committed in this repo, so a bare branch ref would drift).
        .package(
            url: "https://github.com/apple/swift-cmark.git",
            revision: "7898f1b3e4befeecee56cb4a3bc8eebd2cb63219"),
    ],
    targets: [
        .systemLibrary(
            name: "CImpulseFFI",
            path: "CImpulseFFI"
        ),
        // Pure logic ported from the Rust backend (impulse-core / impulse-editor).
        // Foundation-only: no AppKit, no FFI, so it stays headless-testable.
        .target(
            name: "ImpulseKit",
            dependencies: [
                .product(name: "TOMLKit", package: "TOMLKit"),
                .product(name: "cmark-gfm", package: "swift-cmark"),
                .product(name: "cmark-gfm-extensions", package: "swift-cmark"),
            ],
            path: "Sources/ImpulseKit",
            resources: [
                .copy("Resources"),
            ],
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
        .executableTarget(
            name: "ImpulseApp",
            dependencies: [
                "CImpulseFFI",
                "ImpulseKit",
            ],
            path: "Sources/ImpulseApp",
            resources: [
                .copy("Resources/monaco"),
                .copy("Resources/icons"),
            ],
            swiftSettings: [
                // Stay in Swift 5 language mode to avoid the strict
                // concurrency regressions that Swift 6 mode introduces
                // in existing AppKit delegate code (nonisolated deinit
                // touching non-Sendable stored properties, etc.).
                .swiftLanguageMode(.v5),
            ],
            linkerSettings: [
                .unsafeFlags(["-L", "../target/release"]),
                .linkedLibrary("impulse_ffi"),
                .linkedLibrary("resolv"),
                .linkedLibrary("z"),
                .linkedLibrary("iconv"),
                .linkedFramework("Security"),
            ]
        ),
        .testTarget(
            name: "ImpulseKitTests",
            dependencies: [
                "ImpulseKit",
            ],
            path: "Tests/ImpulseKitTests",
            resources: [
                .copy("Fixtures"),
            ],
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
        .testTarget(
            name: "ImpulseAppTests",
            dependencies: [
                "ImpulseApp",
            ],
            path: "Tests/ImpulseAppTests",
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
    ]
)
