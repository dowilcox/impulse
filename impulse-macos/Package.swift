// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ImpulseApp",
    platforms: [
        .macOS(.v26)
    ],
    dependencies: [
    ],
    targets: [
        .systemLibrary(
            name: "CImpulseFFI",
            path: "CImpulseFFI"
        ),
        .executableTarget(
            name: "ImpulseApp",
            dependencies: [
                "CImpulseFFI",
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
