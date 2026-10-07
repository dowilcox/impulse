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
  }
#endif
