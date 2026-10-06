// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ImpulseApp",
    platforms: [
        .macOS(.v26)
    ],
    dependencies: [
        // 0.x minors may break; Package.resolved (committed) pins the
        // exact version, and release builds use it as is.
        .package(url: "https://github.com/LebJe/TOMLKit.git", .upToNextMinor(from: "0.6.0")),
        // Pinned to an exact gfm-branch revision (a branch ref would drift).
        .package(
            url: "https://github.com/apple/swift-cmark.git",
            revision: "7898f1b3e4befeecee56cb4a3bc8eebd2cb63219"),
    ],
    targets: [
        .systemLibrary(
            name: "CImpulseFFI",
            path: "CImpulseFFI"
        ),
        // Vendored static libgit2 (built by scripts/build-libgit2.sh; local
        // git operations only — no HTTPS/SSH, so no OpenSSL).
        .systemLibrary(
            name: "Clibgit2",
            path: "Clibgit2"
        ),
        // Git layer ported from impulse-core/src/git.rs on top of libgit2.
        .target(
            name: "ImpulseGit",
            dependencies: [
                "Clibgit2",
                "ImpulseKit",
            ],
            path: "Sources/ImpulseGit",
            swiftSettings: [
                .swiftLanguageMode(.v5),
                .unsafeFlags(["-Xcc", "-I.libgit2/1.9.1/include"]),
            ],
            linkerSettings: [
                .unsafeFlags(["-L", ".libgit2/1.9.1/lib"]),
            ]
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
        // LSP client ported from impulse-core/src/lsp.rs: server process
        // management, JSON-RPC framing, registry, document cache.
        .target(
            name: "ImpulseLSP",
            dependencies: [
                "ImpulseKit",
            ],
            path: "Sources/ImpulseLSP",
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
        // The app <-> `impulse` CLI protocol (Foundation only, no other deps).
        .target(
            name: "ImpulseProtocol",
            path: "Sources/ImpulseProtocol"
        ),
        // The `impulse` command-line tool bundled with the app.
        .executableTarget(
            name: "impulse",
            dependencies: ["ImpulseProtocol"],
            path: "Sources/ImpulseCLI"
        ),
        .executableTarget(
            name: "ImpulseApp",
            dependencies: [
                "CImpulseFFI",
                "ImpulseKit",
                "ImpulseGit",
                "ImpulseLSP",
                "ImpulseProtocol",
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
                .unsafeFlags(["-Xcc", "-I.libgit2/1.9.1/include"]),
            ],
            linkerSettings: [
                .unsafeFlags(["-L", "../target/release"]),
                .linkedLibrary("impulse_ffi"),
                // z + iconv are required by the vendored libgit2 (Clibgit2).
                // resolv/Security were only needed by the old Rust git2 +
                // vendored-OpenSSL stack and are gone with it.
                .linkedLibrary("z"),
                .linkedLibrary("iconv"),
            ]
        ),
        .testTarget(
            name: "ImpulseLSPTests",
            dependencies: [
                "ImpulseLSP",
            ],
            path: "Tests/ImpulseLSPTests",
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
        .testTarget(
            name: "ImpulseGitTests",
            dependencies: [
                "ImpulseGit",
            ],
            path: "Tests/ImpulseGitTests",
            resources: [
                .copy("Fixtures"),
            ],
            swiftSettings: [
                .swiftLanguageMode(.v5),
                .unsafeFlags(["-Xcc", "-I.libgit2/1.9.1/include"]),
            ]
        ),
        .testTarget(
            name: "ImpulseKitTests",
            dependencies: [
                "ImpulseKit",
                "ImpulseProtocol",
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
