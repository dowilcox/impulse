#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct WorkspaceTrustTests {
    private func scratch() throws -> URL {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("trust-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir.appendingPathComponent("Code/app/src"), withIntermediateDirectories: true)
      try FileManager.default.createDirectory(at: dir.appendingPathComponent("Code/other"), withIntermediateDirectories: true)
      try FileManager.default.createDirectory(at: dir.appendingPathComponent("Code/app-2"), withIntermediateDirectories: true)
      return dir
    }

    @Test func aTrustedFolderCoversWhatsInside() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let trust = WorkspaceTrust(file: nil)
      let app = dir.appendingPathComponent("Code/app").path
      #expect(!trust.isTrusted(app))
      trust.trust(app)
      #expect(trust.isTrusted(app))
      #expect(trust.isTrusted(app + "/src"))
      #expect(trust.isTrusted(app + "/src/main.rs"), "files that don't exist yet too")
      #expect(!trust.isTrusted(dir.appendingPathComponent("Code/app-2").path), "a name prefix isn't a folder")
      #expect(!trust.isTrusted(dir.appendingPathComponent("Code").path))
    }

    @Test func symlinksAndTmpAliasesAreTheSameFolder() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let link = dir.appendingPathComponent("link")
      try FileManager.default.createSymbolicLink(
        at: link, withDestinationURL: dir.appendingPathComponent("Code/app"))
      let trust = WorkspaceTrust(file: nil)
      trust.trust(link.path)
      #expect(trust.isTrusted(dir.appendingPathComponent("Code/app/src").path))
      // temporaryDirectory is under /var, which is /private/var.
      #expect(trust.isTrusted("/private" + dir.appendingPathComponent("Code/app").path))
    }

    @Test func revokingAndParentFolders() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let code = dir.appendingPathComponent("Code").path
      let app = code + "/app"
      let trust = WorkspaceTrust(file: nil)
      trust.trust(app)
      trust.trust(code)
      #expect(trust.trustedFolders == [WorkspaceTrust.normalize(code)], "the parent replaces it")
      #expect(trust.revoke(app) == WorkspaceTrust.normalize(code), "still trusted from above")
      #expect(trust.revoke(code) == nil)
      #expect(!trust.isTrusted(app))
    }

    @Test func turnedOffTrustsEverything() {
      let trust = WorkspaceTrust(file: nil)
      #expect(!trust.isTrusted("/somewhere/else"))
      trust.isEnabled = false
      #expect(trust.isTrusted("/somewhere/else"))
    }

    @Test func keepsTheListBetweenLaunches() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let file = dir.appendingPathComponent("data/trusted-folders.json")
      let first = WorkspaceTrust(file: file)
      #expect(!first.existedBefore)
      first.trust(dir.appendingPathComponent("Code/other").path)
      let second = WorkspaceTrust(file: file)
      #expect(second.existedBefore)
      #expect(second.isTrusted(dir.appendingPathComponent("Code/other").path))
      second.revokeAll()
      #expect(!WorkspaceTrust(file: file).isTrusted(dir.appendingPathComponent("Code/other").path))
    }
  }
#endif
