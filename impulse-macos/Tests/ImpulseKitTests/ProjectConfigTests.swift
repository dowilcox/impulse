#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ProjectConfigTests {
    @Test func parsesActionsScriptsAndCopyList() throws {
      let text = """
        [scripts]
        setup = "npm ci"
        archive = ""

        [worktrees]
        copy = ["local.config", "config/*.yml"]

        [[actions]]
        name = "Dev server"
        command = "npm run dev"
        cwd = "web"
        open = "right"

        [[actions]]
        name = "Tests"
        command = "npm test"

        [[actions]]
        name = ""
        command = "ignored"
        """
      let config = try ProjectConfig.parse(text).get()
      #expect(config.actions.map(\.name) == ["Dev server", "Tests"])
      #expect(config.actions[0].cwd == "web")
      #expect(config.actions[0].open == "right")
      #expect(config.setupScript == "npm ci")
      #expect(config.archiveScript == nil, "empty scripts are dropped")
      #expect(config.worktreeCopy == ["local.config", "config/*.yml"])
      #expect(config.commands == ["npm run dev", "npm test", "npm ci"])
    }

    @Test func emptyAndBrokenFiles() {
      #expect((try? ProjectConfig.parse("").get()) == ProjectConfig())
      guard case .failure = ProjectConfig.parse("[[actions]\nname = ") else {
        Issue.record("broken TOML should fail")
        return
      }
    }

    @Test func trustIsPerContent() throws {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent("proj-\(UUID().uuidString)")
      try FileManager.default.createDirectory(
        at: root.appendingPathComponent(".impulse"), withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: root) }
      #expect(ProjectConfig.load(root: root.path) == nil)
      let file = root.appendingPathComponent(".impulse/project.toml")
      try "[[actions]]\nname = \"A\"\ncommand = \"echo a\"\n".write(to: file, atomically: true, encoding: .utf8)
      let first = try #require(ProjectConfig.load(root: root.path))
      var trust = ProjectTrust()
      #expect(!trust.isTrusted(root: root.path, digest: first.digest))
      trust.trust(root: root.path, digest: first.digest)
      #expect(trust.isTrusted(root: root.path, digest: first.digest))
      try "[[actions]]\nname = \"A\"\ncommand = \"echo b\"\n".write(to: file, atomically: true, encoding: .utf8)
      let edited = try #require(ProjectConfig.load(root: root.path))
      #expect(!trust.isTrusted(root: root.path, digest: edited.digest), "editing asks again")
    }

    @Test func trustCanBeRevokedByFolderOrAll() {
      var trust = ProjectTrust(trusted: [
        "/Users/me/Code/app": "a", "/Users/me/Code/api": "b", "/Users/me/Code/apps": "c", "/Users/me/Other/x": "d",
      ])
      trust.revoke(within: "/Users/me/Code/app")
      #expect(trust.trusted.keys.sorted() == ["/Users/me/Code/api", "/Users/me/Code/apps", "/Users/me/Other/x"])
      trust.revoke(within: "/Users/me/Code/")
      #expect(trust.trusted.keys.sorted() == ["/Users/me/Other/x"])
      trust.revokeAll()
      #expect(trust.trusted.isEmpty)
    }
  }
#endif
