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
      #expect(ProjectConfig.load(root: root.path, commonGitDirectory: nil) == nil)
      let file = root.appendingPathComponent(".impulse/project.toml")
      try "[[actions]]\nname = \"A\"\ncommand = \"echo a\"\n".write(to: file, atomically: true, encoding: .utf8)
      let first = try #require(ProjectConfig.load(root: root.path, commonGitDirectory: nil)?.committed)
      var trust = ProjectTrust()
      #expect(!trust.isTrusted(root: root.path, digest: first.digest))
      trust.trust(root: root.path, digest: first.digest)
      #expect(trust.isTrusted(root: root.path, digest: first.digest))
      try "[[actions]]\nname = \"A\"\ncommand = \"echo b\"\n".write(to: file, atomically: true, encoding: .utf8)
      let edited = try #require(ProjectConfig.load(root: root.path, commonGitDirectory: nil)?.committed)
      #expect(!trust.isTrusted(root: root.path, digest: edited.digest), "editing asks again")
    }

    @Test func localFileWinsKeyByKey() throws {
      let committed = """
        [scripts]
        setup = "npm ci"
        archive = "docker compose down"

        [worktrees]
        copy = [".env"]

        [[actions]]
        name = "dev"
        command = "npm run dev"

        [[actions]]
        name = "test"
        command = "npm test"
        """
      let local = """
        [scripts]
        archive = ""

        [worktrees]
        copy = [".env", "config/*.local.json"]

        [[actions]]
        name = "test"
        command = "npm run test:run"

        [[actions]]
        name = "lint"
        command = "npm run lint"
        """
      let config = ProjectConfig.resolve([
        try ProjectConfig.parseLayer(committed).get(), try ProjectConfig.parseLayer(local).get(),
      ])
      #expect(config.setupScript == "npm ci", "a key the local file doesn't set comes from the committed one")
      #expect(config.archiveScript == nil, "an empty local script clears the committed one")
      #expect(config.worktreeCopy == [".env", "config/*.local.json"], "a local list replaces the committed one")
      #expect(config.actions.map(\.name) == ["dev", "test", "lint"])
      #expect(config.actions[1].command == "npm run test:run", "a local action replaces one of the same name")
    }

    @Test func loadsBothFilesAndNamesTheBrokenOne() throws {
      let base = FileManager.default.temporaryDirectory.appendingPathComponent("proj-\(UUID().uuidString)")
      let root = base.appendingPathComponent("repo")
      let common = root.appendingPathComponent(".git")
      try FileManager.default.createDirectory(
        at: root.appendingPathComponent(".impulse"), withIntermediateDirectories: true)
      try FileManager.default.createDirectory(at: common.appendingPathComponent("impulse"), withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: base) }

      let localPath = ProjectConfig.localPath(commonGitDirectory: common.path)
      #expect(localPath == common.appendingPathComponent("impulse/project.toml").path)
      try "[scripts]\nsetup = \"composer install\"\n".write(toFile: localPath, atomically: true, encoding: .utf8)
      let onlyLocal = try #require(ProjectConfig.load(root: root.path, commonGitDirectory: common.path))
      #expect(onlyLocal.committed == nil)
      #expect(onlyLocal.local?.commands == ["composer install"])
      #expect(try onlyLocal.config.get().setupScript == "composer install")
      #expect(onlyLocal.sources.map(\.path) == [localPath])

      let committedPath = root.appendingPathComponent(ProjectConfig.relativePath).path
      try "[[actions]]\nname = \"dev\"\ncommand = \"npm run dev\"\n".write(
        toFile: committedPath, atomically: true, encoding: .utf8)
      let both = try #require(ProjectConfig.load(root: root.path, commonGitDirectory: common.path))
      #expect(both.sources.map(\.path) == [committedPath, localPath])
      #expect(try both.config.get().commands == ["npm run dev", "composer install"])

      try "[scripts\n".write(toFile: localPath, atomically: true, encoding: .utf8)
      let broken = try #require(ProjectConfig.load(root: root.path, commonGitDirectory: common.path))
      guard case .failure(.invalid(let message)) = broken.config else {
        Issue.record("a broken local file fails the whole config")
        return
      }
      #expect(message.hasPrefix(".git/impulse/project.toml: "))
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
