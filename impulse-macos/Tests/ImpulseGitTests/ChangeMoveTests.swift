#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  /// Moving uncommitted work into another checkout on the same commit
  /// (Move Changes to New Task). The git CLI is the oracle.
  @Suite(.serialized)
  struct ChangeMoveTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// A repository with every kind of uncommitted change, and a second
    /// worktree on the same commit.
    private func setUp() throws -> (repo: TempRepo, target: String) {
      let repo = try TempRepo.create()
      try repo.commit(
        ["a.txt": "a\n", "b.txt": "b\n", "c.txt": "c\n", "old.txt": "moving\n", ".gitignore": ".env\n"], message: "first")
      try repo.write("a.txt", "a changed\n")
      try repo.write("b.txt", "b staged\n")
      try repo.git("add", "b.txt")
      try repo.git("rm", "-q", "c.txt")
      try repo.git("mv", "old.txt", "new.txt")
      try repo.write("src/d.txt", "new and untracked\n")
      try repo.write("e.txt", "new and staged\n")
      try repo.git("add", "e.txt")
      try repo.write(".env", "SECRET=1\n")
      let target = repo.root + "-task"
      try repo.git("worktree", "add", "-q", "--detach", target, "HEAD")
      return (repo, target)
    }

    /// `git status --porcelain` lines, trimmed (unstaged " M" reads "M",
    /// staged "M " reads "M  ").
    private func status(_ root: String) throws -> [String] {
      try TempRepo(root: root).git("status", "--porcelain", "--untracked-files=all").split(separator: "\n")
        .map { $0.trimmingCharacters(in: .whitespaces) }.sorted()
    }

    @Test func everythingMovesUnstagedAndTheSourceIsClean() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      let before = try status(repo.root)

      let moved = try ChangeMove.move(paths: nil, from: repo.root, to: target).get()
      #expect(moved.count == 7)
      let inTarget = try status(target)
      #expect(inTarget == ["?? e.txt", "?? new.txt", "?? src/d.txt", "D c.txt", "D old.txt", "M a.txt", "M b.txt"])
      #expect(try String(contentsOfFile: target + "/src/d.txt", encoding: .utf8) == "new and untracked\n")
      #expect(try status(repo.root).isEmpty)
      #expect(repo.exists(".env"), "ignored files stay")
      #expect(!repo.exists("src"), "folders left empty go too")

      _ = try ChangeMove.undo(moved, source: repo.root).get()
      #expect(try status(repo.root) == before)
    }

    @Test func onlyTheChosenFilesMove() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      let moved = try ChangeMove.move(paths: ["a.txt", "old.txt", "new.txt", "src/d.txt"], from: repo.root, to: target).get()
      #expect(moved.changed.sorted() == ["a.txt", "new.txt", "src/d.txt"])
      #expect(moved.deleted == ["old.txt"])
      let inTarget = try status(target)
      let inSource = try status(repo.root)
      #expect(inTarget == ["?? new.txt", "?? src/d.txt", "D old.txt", "M a.txt"])
      #expect(inSource == ["A  e.txt", "D  c.txt", "M  b.txt"])
    }

    @Test func theTargetHasToBeOnTheSameCommit() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      try TempRepo(root: target).git("commit", "-q", "--allow-empty", "-m", "moved on")
      guard case .failure = ChangeMove.move(paths: nil, from: repo.root, to: target) else {
        Issue.record("expected a refusal")
        return
      }
      let untouched = try status(repo.root)
      #expect(untouched.count == 6, "nothing was touched (the rename is one line)")
    }
  }
#endif
