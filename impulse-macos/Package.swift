// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpulseApp",
    platforms: [
        .macOS(.v13)
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.2.0"),
    ],
    targets: [
        .systemLibrary(
            name: "CImpulseFFI",
            path: "CImpulseFFI"
        ),
        .executableTarget(
            name: "ImpulseApp",
            dependencies: [
                "SwiftTerm",
                "CImpulseFFI",
            ],
            path: "Sources/ImpulseApp",
            resources: [
                .copy("Resources/monaco"),
            ],
            linkerSettings: [
                .unsafeFlags(["-L", "../target/release"]),
                .unsafeFlags(["-L", "/opt/homebrew/opt/openssl@3/lib"]),
                .linkedLibrary("impulse_ffi"),
                .linkedLibrary("resolv"),
                .linkedLibrary("z"),
                .linkedLibrary("iconv"),
                .linkedLibrary("ssl"),
                .linkedLibrary("crypto"),
                .linkedFramework("Security"),
            ]
        ),
    ]
)
