#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  /// Ignored files in a working tree, checked against `git check-ignore`.
  @Suite(.serialized)
  struct IgnoredFilesTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func listsIgnoredFilesAndWholeFolders() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit([".gitignore": ".env\nnode_modules/\n*.log\n", "a.txt": "a\n"])
      #expect(GitOperations.ignoredEntries(root: repo.root).isEmpty)

      try repo.write(".env", "SECRET=1\n")
      try repo.write("node_modules/pkg/index.js", "x\n")
      try repo.write("web/debug.log", "x\n")
      try repo.write("notes.txt", "untracked, not ignored\n")
      let entries = GitOperations.ignoredEntries(root: repo.root)
      #expect(entries.sorted() == [".env", "node_modules/", "web/debug.log"])
      // The oracle: git itself says each one is ignored, and the untracked
      // file that isn't ignored stays out.
      for entry in entries {
        #expect(try repo.git("check-ignore", entry) == entry)
      }
      #expect(throws: (any Error).self) { try repo.git("check-ignore", "notes.txt") }
    }

    @Test func listsRepositoriesInsideTheCheckout() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "a\n"])
      for path in ["clone", "deps/inner/lib"] {
        let nested = TempRepo(root: repo.root + "/" + path)
        try FileManager.default.createDirectory(atPath: nested.root, withIntermediateDirectories: true)
        try nested.git("init", "-q", "-b", "main")
        try nested.commit(["x.txt": "x\n"])
      }
      try repo.write("plain/untracked.txt", "x\n")
      #expect(GitOperations.nestedRepositories(root: repo.root)?.sorted() == ["clone/", "deps/inner/lib/"])
    }

    @Test func listsCheckedOutSubmodulesAndCommittedRepositories() throws {
      let lib = try TempRepo.create()
      let repo = try TempRepo.create()
      let task = repo.root + "-task"
      defer {
        [lib, repo].forEach { $0.destroy() }
        try? FileManager.default.removeItem(atPath: task)
      }
      try lib.commit(["lib.txt": "lib\n"])
      try repo.commit(["a.txt": "a\n"])
      try repo.git("-c", "protocol.file.allow=always", "submodule", "add", "-q", lib.root, "sub")
      try repo.git("commit", "-q", "-m", "add sub")
      // A task: in a linked worktree, a submodule's repository lives in the
      // worktree's own git folder, which removing the worktree deletes.
      try repo.git("worktree", "add", "-q", "-b", "task", task)
      #expect(GitOperations.nestedRepositories(root: task) == [], "not checked out: an empty folder")
      let worktree = TempRepo(root: task)
      try worktree.git("-c", "protocol.file.allow=always", "submodule", "update", "-q", "--init")
      try TempRepo(root: task + "/sub").commit(["unpushed.txt": "work\n"], message: "unpushed")
      #expect(GitOperations.nestedRepositories(root: task) == ["sub/"])

      // A repository committed as a gitlink, with no .gitmodules.
      let embedded = TempRepo(root: repo.root + "/embedded")
      try FileManager.default.createDirectory(atPath: embedded.root, withIntermediateDirectories: true)
      try embedded.git("init", "-q", "-b", "main")
      try embedded.commit(["x.txt": "x\n"])
      try repo.git("add", "embedded")
      try repo.git("commit", "-q", "-m", "embed")
      #expect(GitOperations.nestedRepositories(root: repo.root)?.sorted() == ["embedded/", "sub/"])
    }

    @Test func aConfigurationThatCantBeReadIsntReadAround() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "a\n"])
      #expect(GitCLI.restrictedEnvironment(root: repo.root, base: TempRepo.gitOverrides) != nil)
      try "[broken\n".write(toFile: repo.root + "/.git/config", atomically: true, encoding: .utf8)
      #expect(GitCLI.restrictedEnvironment(root: repo.root, base: TempRepo.gitOverrides) == nil)
      #expect(GitOperations.nestedRepositories(root: repo.root) == nil, "not knowing isn't \"none\"")
      #expect(GitOperations.ignoredEntryList(root: repo.root) == nil)
    }

    /// A repository whose own configuration names programs (an fsmonitor
    /// hook, a clean filter), as a downloaded one can.
    private func hostileRepository() throws -> (repo: TempRepo, ran: String) {
      let repo = try TempRepo.create()
      let ran = repo.root + "-ran"
      let hook = repo.root + "-hook.sh"
      try "#!/bin/sh\necho fsmonitor >> '\(ran)'\nexit 1\n".write(toFile: hook, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: hook)
      try repo.write(".gitattributes", "*.txt filter=evil\n")
      try repo.commit(["a.txt": "a\n"])
      try repo.git("config", "filter.evil.clean", "echo filter >> '\(ran)'; cat")
      try repo.git("config", "core.fsmonitor", hook)
      return (repo, ran)
    }

    /// What ran since the last call; the file's timestamp moved first, so
    /// git status has to look at its content (through the clean filter).
    private func whatRan(_ repo: TempRepo, _ ran: String, _ age: TimeInterval, _ body: () -> Void) throws -> [String] {
      try? FileManager.default.removeItem(atPath: ran)
      try FileManager.default.setAttributes(
        [.modificationDate: Date(timeIntervalSinceNow: -age)], ofItemAtPath: repo.root + "/a.txt")
      body()
      let text = (try? String(contentsOfFile: ran, encoding: .utf8)) ?? ""
      return Set(text.split(separator: "\n").map(String.init)).sorted()
    }

    @Test func readsInAFolderThatIsntTrustedRunNothingTheRepositoryNames() throws {
      let (repo, ran) = try hostileRepository()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: ran)
        try? FileManager.default.removeItem(atPath: repo.root + "-hook.sh")
      }
      #expect(try whatRan(repo, ran, 3600) { _ = GitOperations.ignoredEntries(root: repo.root) }.isEmpty)
      #expect(try whatRan(repo, ran, 7200) { _ = GitOperations.nestedRepositories(root: repo.root) }.isEmpty)
      #expect(try whatRan(repo, ran, 10800) { _ = FileIndex.files(root: repo.root) }.isEmpty)
      // The oracle: in a trusted folder git does run them.
      #expect(
        try whatRan(repo, ran, 14400) { _ = GitOperations.ignoredEntries(root: repo.root, trusted: true) }
          == ["filter", "fsmonitor"])
    }

    @Test func aMergeCheckInAFolderThatIsntTrustedRunsNoMergeDriver() throws {
      let repo = try TempRepo.create()
      let ran = repo.root + "-merge-ran"
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: ran)
      }
      try repo.commit([".gitattributes": "*.txt merge=evil\n", "a.txt": "1\n2\n3\n"], message: "base")
      try repo.git("config", "merge.evil.driver", "echo driver >> '\(ran)'; false")
      try repo.git("switch", "-q", "-c", "side")
      try repo.commit(["a.txt": "1\n2\nside\n"], message: "side")
      try repo.git("switch", "-q", "main")
      try repo.commit(["a.txt": "main\n2\n3\n"], message: "main")

      #expect(!GitOperations.isMerged("side", into: "main", root: repo.root))
      #expect(!FileManager.default.fileExists(atPath: ran))
      // The oracle: trusted, git does run it.
      _ = GitOperations.isMerged("side", into: "main", root: repo.root, trusted: true)
      #expect(FileManager.default.fileExists(atPath: ran))
    }
  }
#endif
