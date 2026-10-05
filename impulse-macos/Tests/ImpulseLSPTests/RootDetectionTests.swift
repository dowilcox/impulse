// Workspace-root detection tests (ported from the Rust
// `root_detection_tests` module in lsp.rs) plus config/override and
// managed-status coverage.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  private func makeTempDir() throws -> String {
    let dir = NSTemporaryDirectory() + "impulse-lsp-tests-" + UUID().uuidString
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    return dir
  }

  struct RootDetectionTests {
    // Ported from Rust: marker_root_beats_outer_git_root.
    @Test func markerRootBeatsOuterGitRoot() throws {
      let repo = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: repo) }
      try FileManager.default.createDirectory(
        atPath: repo + "/.git", withIntermediateDirectories: true)
      let package = repo + "/packages/app"
      try FileManager.default.createDirectory(
        atPath: package + "/src", withIntermediateDirectories: true)
      try "{}".write(toFile: package + "/package.json", atomically: true, encoding: .utf8)
      let file = package + "/src/main.ts"
      try "export {};\n".write(toFile: file, atomically: true, encoding: .utf8)

      let root = LSPConfig.detectProjectRoot(
        fileUri: FileURI.fromPath(file)!, markers: ["package.json"])

      #expect(root == FileURI.fromPath(package))
    }

    // Ported from Rust: git_root_used_when_no_marker_exists.
    @Test func gitRootUsedWhenNoMarkerExists() throws {
      let repo = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: repo) }
      try FileManager.default.createDirectory(
        atPath: repo + "/.git", withIntermediateDirectories: true)
      try FileManager.default.createDirectory(
        atPath: repo + "/src", withIntermediateDirectories: true)
      let file = repo + "/src/main.rs"
      try "fn main() {}\n".write(toFile: file, atomically: true, encoding: .utf8)

      let root = LSPConfig.detectProjectRoot(
        fileUri: FileURI.fromPath(file)!, markers: ["go.mod"])

      #expect(root == FileURI.fromPath(repo))
    }

    @Test func innermostMarkerWins() throws {
      let outer = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: outer) }
      try "".write(toFile: outer + "/Cargo.toml", atomically: true, encoding: .utf8)
      let inner = outer + "/crates/sub"
      try FileManager.default.createDirectory(atPath: inner, withIntermediateDirectories: true)
      try "".write(toFile: inner + "/Cargo.toml", atomically: true, encoding: .utf8)
      let file = inner + "/lib.rs"
      try "".write(toFile: file, atomically: true, encoding: .utf8)

      let root = LSPConfig.detectProjectRoot(
        fileUri: FileURI.fromPath(file)!, markers: ["Cargo.toml"])

      #expect(root == FileURI.fromPath(inner))
    }

    @Test func nonFileUriReturnsNil() {
      #expect(
        LSPConfig.detectProjectRoot(fileUri: "https://example.com/a.ts", markers: ["package.json"])
          == nil)
    }
  }

  struct ConfigTests {
    @Test func defaultTablesMatchRust() {
      let cfg = LSPConfig.defaultConfig()
      #expect(cfg.servers.count == 18)
      #expect(cfg.servers["rust-analyzer"]?.command == "rust-analyzer")
      #expect(cfg.servers["rust-analyzer"]?.args == [])
      #expect(cfg.servers["pyright"]?.command == "pyright-langserver")
      #expect(cfg.servers["pyright"]?.args == ["--stdio"])
      #expect(cfg.servers["bash-language-server"]?.args == ["start"])
      #expect(cfg.servers["graphql-lsp"]?.args == ["server", "-m", "stream"])
      #expect(
        cfg.languageServers["typescript"] == [
          "typescript-language-server",
          "vscode-eslint-language-server",
          "tailwindcss-language-server",
          "emmet-ls",
        ])
      #expect(cfg.languageServers["rust"] == ["rust-analyzer"])
      #expect(cfg.languageServers["shellscript"] == ["bash-language-server"])
      // The Rust table plus sourcekit-lsp for Swift (and its Package.swift
      // root marker), added after the port.
      #expect(cfg.servers["sourcekit-lsp"]?.command == "sourcekit-lsp")
      #expect(cfg.languageServers["swift"] == ["sourcekit-lsp"])
      #expect(cfg.rootMarkers.count == 18)
      #expect(cfg.rootMarkers.first == "Cargo.toml")
      #expect(cfg.rootMarkers.contains("deno.jsonc"))
    }

    @Test func untrustedProjectConfigCannotDefineServers() throws {
      let dir = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: dir) }
      let path = dir + "/lsp.json"
      let json = """
        {
          "servers": {"evil": {"command": "/bin/evil"}},
          "language_servers": {
            "rust": ["typescript-language-server"],
            "foo": ["evil"]
          },
          "root_markers": ["ok.json", "../bad", "a/b", ""]
        }
        """
      try json.write(toFile: path, atomically: true, encoding: .utf8)

      var cfg = LSPConfig.defaultConfig()
      cfg.applyFile(path, trusted: false)

      #expect(cfg.servers["evil"] == nil)
      // Remapping to an existing server is allowed.
      #expect(cfg.languageServers["rust"] == ["typescript-language-server"])
      // Mapping to an unknown server is filtered to nothing.
      #expect(cfg.languageServers["foo"] == nil)
      // Markers with path separators / ".." / empty are filtered out.
      #expect(cfg.rootMarkers == ["ok.json"])
    }

    @Test func trustedGlobalConfigCanDefineServers() throws {
      let dir = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: dir) }
      let path = dir + "/lsp.json"
      let json = """
        {
          "servers": {"mylang-ls": {"command": "/usr/local/bin/mylang-ls", "args": ["--stdio"]}},
          "language_servers": {"mylang": ["mylang-ls"]}
        }
        """
      try json.write(toFile: path, atomically: true, encoding: .utf8)

      var cfg = LSPConfig.defaultConfig()
      cfg.applyFile(path, trusted: true)

      #expect(cfg.servers["mylang-ls"]?.command == "/usr/local/bin/mylang-ls")
      #expect(cfg.servers["mylang-ls"]?.args == ["--stdio"])
      #expect(cfg.languageServers["mylang"] == ["mylang-ls"])
      // Untouched defaults survive.
      #expect(cfg.servers["rust-analyzer"] != nil)
    }

    @Test func invalidConfigFileIsIgnored() throws {
      let dir = try makeTempDir()
      defer { try? FileManager.default.removeItem(atPath: dir) }
      let path = dir + "/lsp.json"
      try "not json".write(toFile: path, atomically: true, encoding: .utf8)

      var cfg = LSPConfig.defaultConfig()
      cfg.applyFile(path, trusted: true)
      #expect(cfg.servers.count == 18)

      // Structurally invalid overrides are rejected wholesale, like serde.
      try "{\"servers\": {\"x\": {\"args\": []}}}".write(
        toFile: path, atomically: true, encoding: .utf8)
      cfg.applyFile(path, trusted: true)
      #expect(cfg.servers["x"] == nil)
      #expect(cfg.servers.count == 18)
    }
  }

  struct ManagedStatusTests {
    @Test func managedStatusJSONShape() throws {
      let raw = ManagedServers.checkStatusJSON()
      let parsed = try #require(JSONUtil.parse(raw) as? [[String: Any]])
      #expect(parsed.count == 14)
      #expect(parsed[0]["command"] as? String == "typescript-language-server")
      for entry in parsed {
        #expect(entry["command"] is String)
        #expect(entry["installed"] is Bool || entry["installed"] is NSNumber)
        #expect(entry.keys.sorted() == ["command", "installed", "resolvedPath"])
      }
    }

    @Test func systemStatusJSONShape() throws {
      let raw = ManagedServers.systemStatusJSON()
      let parsed = try #require(JSONUtil.parse(raw) as? [[String: Any]])
      #expect(parsed.map { $0["command"] as? String } == ["rust-analyzer", "pyright", "clangd"])
      for entry in parsed {
        #expect(entry.keys.sorted() == ["command", "installed", "resolvedPath"])
      }
    }

    @Test func missingCommandMessagesMatchRust() {
      #expect(
        ManagedServers.missingCommandMessage(
          serverId: "typescript-language-server", command: "typescript-language-server")
          == "LSP server 'typescript-language-server' requires 'typescript-language-server' but it is not installed. Run `impulse --install-lsp-servers` (or `cargo run -p impulse-linux -- --install-lsp-servers`) to install managed web LSP servers."
      )
      #expect(
        ManagedServers.missingCommandMessage(serverId: "rust-analyzer", command: "rust-analyzer")
          == "LSP server 'rust-analyzer' requires 'rust-analyzer' but it is not in PATH. Install it or override `servers.rust-analyzer` in lsp.json."
      )
    }

    @Test func commandResolutionFindsShellInPath() {
      // /bin/sh exists on every macOS system and PATH contains /bin.
      #expect(ManagedServers.resolveLspCommandPath("sh") != nil)
      #expect(ManagedServers.resolveLspCommandPath("/bin/sh") == "/bin/sh")
      #expect(
        ManagedServers.resolveLspCommandPath("definitely-not-a-real-command-xyz") == nil)
    }
  }
#endif
