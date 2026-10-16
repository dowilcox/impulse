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

    @Test func aRestrictedFolderStaysRestrictedInsideATrustedOne() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let code = dir.appendingPathComponent("Code").path
      let task = code + "/app/src"
      let trust = WorkspaceTrust(file: nil)
      trust.trust(code)
      trust.restrict(task)
      #expect(!trust.isTrusted(task))
      #expect(!trust.isTrusted(task + "/main.rs"))
      #expect(trust.trustingFolder(task) == nil)
      #expect(trust.isTrusted(code + "/app"), "only the restricted folder")
      #expect(trust.isTrusted(code + "/other"))

      // Trusting the folder above again doesn't lift it.
      trust.trust(code)
      #expect(!trust.isTrusted(task))
      // Trusting it does.
      trust.trust(task)
      #expect(trust.isTrusted(task))
      #expect(trust.restrictedFolders.isEmpty)
    }

    @Test func aFolderInsideARestrictedOneCanBeTrusted() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let code = dir.appendingPathComponent("Code").path
      let trust = WorkspaceTrust(file: nil)
      trust.trust(code)
      trust.restrict(code + "/app")
      trust.trust(code + "/app/src")
      #expect(trust.isTrusted(code + "/app/src"))
      #expect(!trust.isTrusted(code + "/app"))
      #expect(trust.revoke(code + "/app/src") == nil, "restricted again, not trusted from above")
    }

    @Test func restrictionsAreKeptBetweenLaunches() throws {
      let dir = try scratch()
      defer { try? FileManager.default.removeItem(at: dir) }
      let file = dir.appendingPathComponent("data/trusted-folders.json")
      let code = dir.appendingPathComponent("Code").path
      let first = WorkspaceTrust(file: file)
      first.trust(code)
      first.restrict(code + "/other")
      let second = WorkspaceTrust(file: file)
      #expect(second.isTrusted(code + "/app"))
      #expect(!second.isTrusted(code + "/other"))
      second.revokeAll()
      #expect(WorkspaceTrust(file: file).restrictedFolders.isEmpty)
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
