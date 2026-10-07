// The registry's configuration: each file gets its own project's
// .impulse/lsp.json, and edits to it or to the global lsp.json apply
// without a restart.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseLSP

  struct ConfigReloadTests {
    private func makeDir(_ path: String) throws {
      try FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
    }

    private func write(_ text: String, to path: String) throws {
      try makeDir((path as NSString).deletingLastPathComponent)
      try text.write(toFile: path, atomically: true, encoding: .utf8)
    }

    private func makeRoot() throws -> String {
      let root = NSTemporaryDirectory() + "impulse-lsp-config-" + UUID().uuidString
      try makeDir(root)
      return root
    }

    @Test func projectConfigFolderIsTheNearestOneInTheRepository() throws {
      let root = try makeRoot()
      defer { try? FileManager.default.removeItem(atPath: root) }
      // A config above the repository doesn't apply to it.
      try write("{}", to: root + "/.impulse/lsp.json")
      try makeDir(root + "/repo/.git")
      try makeDir(root + "/repo/src/deep")
      #expect(LSPConfig.projectConfigFolder(forDirectory: root + "/repo/src/deep") == nil)

      try write("{}", to: root + "/repo/.impulse-lsp.json")
      #expect(LSPConfig.projectConfigFolder(forDirectory: root + "/repo/src/deep") == root + "/repo")

      try write("{}", to: root + "/repo/src/.impulse/lsp.json")
      #expect(LSPConfig.projectConfigFolder(forDirectory: root + "/repo/src/deep") == root + "/repo/src")
      // Outside any repository, the walk goes up until it finds one.
      try makeDir(root + "/loose/a")
      #expect(LSPConfig.projectConfigFolder(forDirectory: root + "/loose/a") == root)
    }

    @Test func eachProjectGetsItsOwnConfigAndEditsApply() throws {
      let root = try makeRoot()
      defer { try? FileManager.default.removeItem(atPath: root) }
      try makeDir(root + "/one/.git")
      try makeDir(root + "/two/.git")
      try write(#"{"language_servers": {"rust": ["clangd"]}}"#, to: root + "/one/.impulse/lsp.json")
      let global = root + "/global/lsp.json"
      let registry = LSPRegistry(rootUri: FileURI.fromPath(root)!, globalConfigPath: global)
      registry.configRecheckInterval = 0
      let fileOne = FileURI.fromPath(root + "/one/src/main.rs")!
      let fileTwo = FileURI.fromPath(root + "/two/src/main.rs")!

      #expect(registry.config(forFileUri: fileOne).languageServers["rust"] == ["clangd"])
      #expect(registry.config(forFileUri: fileTwo).languageServers["rust"] == ["rust-analyzer"])

      // The project file changes: the next look picks it up.
      try write(#"{"language_servers": {"rust": ["pyright"]}}"#, to: root + "/one/.impulse/lsp.json")
      #expect(registry.config(forFileUri: fileOne).languageServers["rust"] == ["pyright"])

      // A new global config adds a server, which a project may then use.
      try write(
        #"{"servers": {"gopls": {"command": "gopls"}}, "language_servers": {"go": ["gopls"]}}"#, to: global)
      #expect(registry.config(forFileUri: fileTwo).servers["gopls"]?.command == "gopls")
      #expect(registry.hasServers(languageId: "go"))
      try write(#"{"language_servers": {"rust": ["gopls"]}}"#, to: root + "/one/.impulse/lsp.json")
      #expect(registry.config(forFileUri: fileOne).languageServers["rust"] == ["gopls"])
    }

    @Test func resolvedConfigIsCachedUntilTheRecheckInterval() throws {
      let root = try makeRoot()
      defer { try? FileManager.default.removeItem(atPath: root) }
      try makeDir(root + "/one/.git")
      try write(#"{"language_servers": {"rust": ["clangd"]}}"#, to: root + "/one/.impulse/lsp.json")
      let registry = LSPRegistry(rootUri: FileURI.fromPath(root)!, globalConfigPath: nil)
      registry.configRecheckInterval = 3600
      let file = FileURI.fromPath(root + "/one/main.rs")!
      #expect(registry.config(forFileUri: file).languageServers["rust"] == ["clangd"])
      try write(#"{"language_servers": {"rust": ["pyright"]}}"#, to: root + "/one/.impulse/lsp.json")
      #expect(registry.config(forFileUri: file).languageServers["rust"] == ["clangd"])
    }
  }
#endif
