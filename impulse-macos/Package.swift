// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpulseApp",
    platforms: [
        .macOS(.v14)
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
            linkerSettings: [
                .unsafeFlags(["-L", "../target/release"]),
                .linkedLibrary("impulse_ffi"),
                .linkedLibrary("resolv"),
                .linkedLibrary("z"),
                .linkedLibrary("iconv"),
                .linkedFramework("Security"),
            ]
        ),
    ]
)
