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
        .executableTarget(
            name: "ImpulseApp",
            dependencies: [
                "CImpulseFFI",
                "ImpulseKit",
                "ImpulseGit",
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
