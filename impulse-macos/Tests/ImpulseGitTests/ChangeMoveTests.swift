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

    /// A clone made inside the checkout, with a commit and untracked work.
    private func nestedRepository(in repo: TempRepo, at path: String) throws {
      let nested = TempRepo(root: repo.root + "/" + path)
      try FileManager.default.createDirectory(atPath: nested.root, withIntermediateDirectories: true)
      try nested.git("init", "-q", "-b", "main")
      try nested.commit(["lib.txt": "lib\n"], message: "lib")
      try nested.write("unpushed.txt", "work\n")
    }

    @Test func aNestedRepositoryStaysWhereItIs() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      try nestedRepository(in: repo, at: "vendor/lib")

      let moved = try ChangeMove.move(paths: nil, from: repo.root, to: target).get()
      #expect(moved.nestedRepositories == ["vendor/lib"])
      #expect(!moved.changed.contains("vendor/lib"))
      #expect(moved.count == 7, "everything else moved")
      #expect(repo.exists("vendor/lib/.git"))
      #expect(try repo.read("vendor/lib/lib.txt") == "lib\n")
      #expect(try repo.read("vendor/lib/unpushed.txt") == "work\n")
      #expect(!FileManager.default.fileExists(atPath: target + "/vendor/lib"))
    }

    @Test func onlyANestedRepositoryCantMove() throws {
      let repo = try TempRepo.create()
      let target = repo.root + "-task"
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      try repo.commit(["a.txt": "a\n"], message: "first")
      try repo.git("worktree", "add", "-q", "--detach", target, "HEAD")
      try nestedRepository(in: repo, at: "clone")
      guard case .failure = ChangeMove.move(paths: nil, from: repo.root, to: target) else {
        Issue.record("expected a refusal")
        return
      }
      #expect(repo.exists("clone/.git"))
      #expect(try repo.read("clone/unpushed.txt") == "work\n")
    }

    @Test func filesEditedSinceTheSnapshotAreFound() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      let snapshot = try SafetySnapshots.create(reason: "test", root: repo.root).get()
      try repo.write("a.txt", "edited again\n")
      try FileManager.default.removeItem(atPath: repo.root + "/src/d.txt")
      try repo.write("c.txt", "back\n")
      let edited = ChangeMove.editedSince(
        snapshot, changed: ["a.txt", "b.txt", "e.txt", "new.txt", "src/d.txt"], deleted: ["c.txt", "old.txt"],
        root: repo.root)
      #expect(edited.sorted() == ["a.txt", "c.txt", "src/d.txt"])
    }

    @Test func undoLeavesFilesEditedDuringTheMoveAlone() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      let moved = try ChangeMove.move(paths: nil, from: repo.root, to: target).get()
      #expect(moved.editedDuring.isEmpty)
      // As if a.txt had been edited while the move ran (so it stayed).
      try repo.write("a.txt", "kept edit\n")
      let partly = ChangeMove.Moved(
        snapshot: moved.snapshot, changed: moved.changed, deleted: moved.deleted, editedDuring: ["a.txt"])
      _ = try ChangeMove.undo(partly, source: repo.root).get()
      #expect(try repo.read("a.txt") == "kept edit\n")
      #expect(try repo.read("src/d.txt") == "new and untracked\n", "the rest came back")
    }

    @Test func undoLeavesFilesChangedSinceTheMoveAlone() throws {
      let (repo, target) = try setUp()
      defer {
        repo.destroy()
        try? FileManager.default.removeItem(atPath: target)
      }
      let moved = try ChangeMove.move(paths: nil, from: repo.root, to: target).get()
      // After the move: one file edited again, one new file made again.
      try repo.write("a.txt", "edited after the move\n")
      try repo.write("src/d.txt", "made again\n")

      let left = try ChangeMove.undo(moved, source: repo.root).get()
      #expect(left.sorted() == ["a.txt", "src/d.txt"])
      #expect(try repo.read("a.txt") == "edited after the move\n")
      #expect(try repo.read("src/d.txt") == "made again\n")
      #expect(try repo.read("b.txt") == "b staged\n", "the rest came back")
      #expect(!repo.exists("c.txt"))
    }

    @Test func removingAFileNeverTakesAFolder() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.write("folder/inside.txt", "x\n")
      #expect(!ChangeMove.removeFile(repo.root + "/folder"))
      #expect(repo.exists("folder/inside.txt"))
      #expect(ChangeMove.removeFile(repo.root + "/folder/inside.txt"))
      #expect(!repo.exists("folder/inside.txt"))
    }
  }
#endif
