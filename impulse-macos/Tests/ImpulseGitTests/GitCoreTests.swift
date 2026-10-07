#if canImport(Testing)
  import Foundation
  import ImpulseKit
  import Testing

  @testable import ImpulseGit

  /// Repo snapshot, scoped diffs, hunk/line staging and write operations,
  /// checked against the git CLI as the oracle.
  @Suite(.serialized)
  struct GitCoreTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    // MARK: Snapshot

    @Test func snapshotSeparatesStagedUnstagedAndUntracked() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\ntwo\nthree\n", "b.txt": "b\n"])
      try repo.write("a.txt", "one\nTWO\nthree\n")
      try repo.git("add", "a.txt")
      try repo.write("a.txt", "one\nTWO\nthree\nfour\n")
      try repo.write("new.txt", "fresh\n")
      try repo.git("rm", "-q", "b.txt")

      let snap = try #require(GitClient.snapshot(forPath: repo.root))
      #expect(snap.branch == "main")
      #expect(!snap.isDetached)
      #expect(snap.staged.map(\.path).sorted() == ["a.txt", "b.txt"])
      #expect(snap.staged.first { $0.path == "b.txt" }?.status == .deleted)
      let stagedA = try #require(snap.staged.first { $0.path == "a.txt" })
      #expect(stagedA.added == 1 && stagedA.removed == 1)
      #expect(snap.unstaged.map(\.path) == ["a.txt"])
      #expect(snap.unstaged.first?.added == 1 && snap.unstaged.first?.removed == 0)
      #expect(snap.untracked.map(\.path) == ["new.txt"])
      #expect(snap.untracked.first?.added == 1)
      #expect(snap.operation == nil)
    }

    @Test func snapshotReportsAheadBehindAndUnborn() throws {
      let origin = try TempRepo.create()
      defer { origin.destroy() }
      try origin.commit(["a.txt": "1\n"])
      let clone = try TempRepo.create()
      defer { clone.destroy() }
      try clone.git("remote", "add", "origin", origin.root)
      try clone.git("fetch", "-q", "origin")
      try clone.git("reset", "-q", "--hard", "origin/main")
      try clone.git("branch", "-q", "--set-upstream-to=origin/main", "main")
      try clone.commit(["b.txt": "2\n"], message: "local")
      try origin.commit(["c.txt": "3\n"], message: "remote")
      try clone.git("fetch", "-q", "origin")

      let snap = try #require(GitClient.snapshot(forPath: clone.root))
      #expect(snap.upstream == "origin/main")
      #expect(snap.ahead == 1)
      #expect(snap.behind == 1)

      let fresh = try TempRepo.create(initialBranch: "trunk")
      defer { fresh.destroy() }
      let unborn = try #require(GitClient.snapshot(forPath: fresh.root))
      #expect(unborn.isUnborn)
      #expect(unborn.branch == "trunk")
      #expect(unborn.headOid == nil)
    }

    @Test func snapshotDetectsMergeConflictState() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "base\n"])
      try repo.git("switch", "-q", "-c", "other")
      try repo.commit(["a.txt": "other\n"])
      try repo.git("switch", "-q", "main")
      try repo.commit(["a.txt": "main\n"])
      _ = try? repo.git("merge", "-q", "other")

      let snap = try #require(GitClient.snapshot(forPath: repo.root))
      #expect(snap.operation == .merge)
      #expect(snap.conflicted.map(\.path) == ["a.txt"])
    }

    // MARK: Scoped diffs

    @Test func scopedDiffsMatchGit() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n2\n3\n"])
      try repo.write("a.txt", "1\nTWO\n3\n")
      try repo.git("add", "a.txt")
      try repo.write("a.txt", "1\nTWO\n3\n4\n")

      let staged = try GitClient.fileDiff(repoPath: repo.root, path: "a.txt", scope: .staged)
      #expect(staged.added == 1 && staged.removed == 1)
      let unstaged = try GitClient.fileDiff(repoPath: repo.root, path: "a.txt", scope: .unstaged)
      #expect(unstaged.added == 1 && unstaged.removed == 0)
      let all = try GitClient.fileDiff(repoPath: repo.root, path: "a.txt", scope: .uncommitted)
      #expect(all.added == 2 && all.removed == 1)
      #expect(all.hunkIds.count == all.hunks.count)

      let files = try GitClient.changedFiles(repoPath: repo.root, scope: .uncommitted)
      #expect(files == [FileChange(path: "a.txt", status: .modified, added: 2, removed: 1)])
    }

    @Test func moreContextMergesHunksButKeepsTheChangeIdentity() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let lines = (1...30).map { "line \($0)" }
      try repo.commit(["a.txt": lines.joined(separator: "\n") + "\n"])
      var edited = lines
      edited[2] = "changed 3"
      edited[20] = "changed 21"
      try repo.write("a.txt", edited.joined(separator: "\n") + "\n")

      let narrow = try GitClient.fileDiff(repoPath: repo.root, path: "a.txt", scope: .unstaged)
      let wide = try GitClient.fileDiff(
        repoPath: repo.root, path: "a.txt", scope: .unstaged, options: DiffOptions(contextLines: 25))
      #expect(narrow.hunks.count == 2)
      #expect(wide.hunks.count == 1)
      #expect(wide.hunks[0].lines.count == 32, "the whole file, both changes")
      #expect(GitClient.changeIdentity(narrow.hunks) == GitClient.changeIdentity(wide.hunks))
      #expect(GitClient.hunkIdentity(wide.hunks[0]) == GitClient.changeIdentity(wide.hunks))
    }

    @Test func branchScopeComparesAgainstMergeBase() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "base\n"])
      try repo.git("switch", "-q", "-c", "feature")
      try repo.commit(["f.txt": "feature\n"], message: "feature work")
      try repo.git("switch", "-q", "main")
      try repo.commit(["m.txt": "main only\n"], message: "main moves on")
      try repo.git("switch", "-q", "feature")
      try repo.write("wip.txt", "uncommitted\n")

      let files = try GitClient.changedFiles(repoPath: repo.root, scope: .branch(base: "main"))
      // main's later commit is not part of this branch's diff.
      #expect(files.map(\.path).sorted() == ["f.txt", "wip.txt"])
      #expect(GitClient.defaultBaseBranch(repoPath: repo.root) == "main")
    }

    @Test func commitAndStashScopes() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      try repo.commit(["a.txt": "1\n2\n", "b.txt": "b\n"], message: "second")
      let head = try repo.git("rev-parse", "HEAD")
      let commitFiles = try GitClient.changedFiles(repoPath: repo.root, scope: .commit(sha: head))
      #expect(commitFiles.map(\.path).sorted() == ["a.txt", "b.txt"])

      try repo.write("a.txt", "1\n2\n3\n")
      try repo.git("stash", "-q")
      let stashFiles = try GitClient.changedFiles(repoPath: repo.root, scope: .stash(index: 0))
      #expect(stashFiles.map(\.path) == ["a.txt"])
      #expect(GitOperations.stashList(root: repo.root).count == 1)
    }

    @Test func stashReviewShowsTheUntrackedFilesItCarries() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      try repo.write("a.txt", "1\n2\n")
      try repo.write("notes/fresh.txt", "new\n")
      try repo.git("stash", "-q", "--include-untracked")

      let files = try GitClient.changedFiles(repoPath: repo.root, scope: .stash(index: 0))
      #expect(files.map(\.path).sorted() == ["a.txt", "notes/fresh.txt"])
      #expect(files.first { $0.path == "notes/fresh.txt" }?.status == .added)
      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "notes/fresh.txt", scope: .stash(index: 0))
      #expect(diff.hunks.flatMap(\.lines).map(\.content) == ["new"])
    }

    // MARK: Patch building + staging

    private func twoHunkRepo() throws -> TempRepo {
      let repo = try TempRepo.create()
      let lines = (1...30).map { "line \($0)" }
      try repo.commit(["f.txt": lines.joined(separator: "\n") + "\n"])
      var edited = lines
      edited[1] = "line 2 changed"
      edited[24] = "line 25 changed"
      edited.insert("inserted after 25", at: 25)
      try repo.write("f.txt", edited.joined(separator: "\n") + "\n")
      return repo
    }

    @Test func stagingOneHunkStagesOnlyThatHunk() throws {
      let repo = try twoHunkRepo()
      defer { repo.destroy() }
      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "f.txt", scope: .unstaged)
      #expect(diff.hunks.count == 2)

      let result = GitOperations.apply(
        .stage, selection: .wholeHunks([0]), path: "f.txt",
        expectedHunkIds: [0: diff.hunkIds[0]], root: repo.root)
      #expect(throws: Never.self) { try result.get() }
      let cached = try repo.git("diff", "--cached")
      #expect(cached.contains("+line 2 changed"))
      #expect(!cached.contains("line 25 changed"))
      let remaining = try repo.git("diff")
      #expect(remaining.contains("+line 25 changed"))
      #expect(!remaining.contains("line 2 changed"))
    }

    @Test func stagingSelectedLinesInsideAHunk() throws {
      let repo = try twoHunkRepo()
      defer { repo.destroy() }
      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "f.txt", scope: .unstaged)
      let hunk = diff.hunks[1]
      // Pick only the inserted line, not the "line 25" modification.
      let inserted = try #require(
        hunk.lines.firstIndex { $0.kind == .added && $0.content == "inserted after 25" })

      let result = GitOperations.apply(
        .stage, selection: .lines([inserted], inHunk: 1), path: "f.txt", root: repo.root)
      #expect(throws: Never.self) { try result.get() }
      let cached = try repo.git("diff", "--cached")
      #expect(cached.contains("+inserted after 25"))
      #expect(!cached.contains("line 25 changed"))
      #expect(!cached.contains("line 2 changed"))
    }

    @Test func unstagingAndDiscardingHunks() throws {
      let repo = try twoHunkRepo()
      defer { repo.destroy() }
      try repo.git("add", "f.txt")
      let staged = try GitClient.fileDiff(repoPath: repo.root, path: "f.txt", scope: .staged)
      let unstage = GitOperations.apply(
        .unstage, selection: .wholeHunks([1]), path: "f.txt",
        expectedHunkIds: [1: staged.hunkIds[1]], root: repo.root)
      #expect(throws: Never.self) { try unstage.get() }
      #expect(try repo.git("diff", "--cached").contains("line 2 changed"))
      #expect(!(try repo.git("diff", "--cached")).contains("line 25 changed"))
      #expect(try repo.git("diff").contains("line 25 changed"))

      // Discard the remaining unstaged hunk from the working tree.
      let discard = GitOperations.apply(
        .discard, selection: .wholeHunks([0]), path: "f.txt", root: repo.root)
      #expect(throws: Never.self) { try discard.get() }
      #expect(try repo.git("diff") == "")
      #expect(!(try repo.read("f.txt")).contains("line 25 changed"))
      #expect(try repo.read("f.txt").contains("line 2 changed"))
    }

    @Test func revertingALaterHunkLandsInItsOwnBlock() throws {
      // Hunk 0 adds lines at the top; hunks 1 and 2 delete the same line in
      // two identical blocks. Reverting only hunk 2 (or unstaging it) must
      // restore block 2, not the identical block 1.
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let block = "begin\nalpha\nbeta\nTARGET\ngamma\ndelta\nend\n"
      let filler = (1...12).map { "filler \($0)" }.joined(separator: "\n") + "\n"
      try repo.commit(["f.txt": filler + block + filler + block])
      let added = (1...30).map { "new \($0)" }.joined(separator: "\n") + "\n"
      let edited = block.replacingOccurrences(of: "TARGET\n", with: "")
      try repo.write("f.txt", added + filler + edited + filler + edited)

      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "f.txt", scope: .unstaged)
      try #require(diff.hunks.count == 3)
      let discard = GitOperations.apply(.discard, selection: .wholeHunks([2]), path: "f.txt", root: repo.root)
      #expect(throws: Never.self) { try discard.get() }
      #expect(try repo.read("f.txt") == added + filler + edited + filler + block)

      // The same through the index: stage everything, unstage hunk 2.
      try repo.write("f.txt", added + filler + edited + filler + edited)
      try repo.git("add", "f.txt")
      let unstage = GitOperations.apply(.unstage, selection: .wholeHunks([2]), path: "f.txt", root: repo.root)
      #expect(throws: Never.self) { try unstage.get() }
      #expect(try repo.git("show", ":f.txt") + "\n" == added + filler + edited + filler + block)
    }

    @Test func hunksOfNonUTF8FilesAreRefusedNotCorrupted() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let url = URL(fileURLWithPath: repo.root).appendingPathComponent("latin1.txt")
      try Data("caf".utf8 + [0xE9] + Array("\nline\n".utf8)).write(to: url)
      try repo.git("add", "latin1.txt")
      try repo.git("commit", "-q", "-m", "latin1")
      try Data("caf".utf8 + [0xE9] + Array(" au lait\nline\n".utf8)).write(to: url)

      let result = GitOperations.apply(.stage, selection: .wholeHunks([0]), path: "latin1.txt", root: repo.root)
      guard case .failure(.invalid) = result else {
        Issue.record("expected the partial stage to be refused, got \(result)")
        return
      }
      #expect(try repo.git("diff", "--cached", "--name-only") == "", "nothing reached the index")
    }

    @Test func partOfARenameIsRefusedNotUndone() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let lines = (1...40).map { "line \($0)" }
      try repo.commit(["old.txt": lines.joined(separator: "\n") + "\n"])
      try repo.git("mv", "old.txt", "new.txt")
      var edited = lines
      edited[1] = "line 2 changed"
      edited[35] = "line 36 changed"
      try repo.write("new.txt", edited.joined(separator: "\n") + "\n")
      try repo.git("add", "-A")
      let staged = try GitClient.fileDiff(repoPath: repo.root, path: "new.txt", oldPath: "old.txt", scope: .staged)
      try #require(staged.hunks.count == 2)

      let result = GitOperations.apply(
        .unstage, selection: .wholeHunks([1]), path: "new.txt", oldPath: "old.txt", root: repo.root)
      guard case .failure(.invalid) = result else {
        Issue.record("expected a partial unstage of a rename to be refused, got \(result)")
        return
      }
      #expect(try repo.git("status", "--porcelain") == "R  old.txt -> new.txt", "the rename is intact")
    }

    @Test func staleHunkIdIsRejected() throws {
      let repo = try twoHunkRepo()
      defer { repo.destroy() }
      let result = GitOperations.apply(
        .stage, selection: .wholeHunks([0]), path: "f.txt",
        expectedHunkIds: [0: "deadbeefdeadbeef"], root: repo.root)
      guard case .failure(.stale) = result else {
        Issue.record("expected a stale failure, got \(result)")
        return
      }
    }

    @Test func stagingAtEndOfFileWithoutNewline() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["n.txt": "a\nb"])
      try repo.write("n.txt", "a\nb\nc")
      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "n.txt", scope: .unstaged)
      #expect(diff.oldMissingNewlineAtEnd)
      #expect(diff.newMissingNewlineAtEnd)
      let result = GitOperations.apply(
        .stage, selection: .wholeHunks([0]), path: "n.txt", root: repo.root)
      #expect(throws: Never.self) { try result.get() }
      #expect(try repo.git("diff") == "")
    }

    @Test func zeroContextHunksAndLinesStageUnstageAndRevertInPlace() throws {
      // "Changed Lines Only": hunks carry no context, so pure insertions and
      // deletions have nothing to match and must land at their own line.
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let base = (1...30).map { "line \($0)" }
      try repo.commit(["f.txt": base.joined(separator: "\n") + "\n"])
      func inserting(_ line: String, after anchor: String, in lines: [String]) -> [String] {
        var result = lines
        result.insert(line, at: result.firstIndex(of: anchor)! + 1)
        return result
      }
      var working = base
      working[1] = "line 2 changed"
      working = inserting("inserted after 10", after: "line 10", in: working)
      working.removeAll { $0 == "line 20" }
      working[working.firstIndex(of: "line 25")!] = "line 25 changed"
      working = inserting("inserted after 25", after: "line 25 changed", in: working)
      try repo.write("f.txt", working.joined(separator: "\n") + "\n")

      let zero = DiffOptions(contextLines: 0)
      func hunk(_ scope: DiffScope, containing text: String) throws -> (index: Int, diff: FileDiff) {
        let diff = try GitClient.fileDiff(repoPath: repo.root, path: "f.txt", scope: scope, options: zero)
        let index = try #require(diff.hunks.firstIndex { $0.lines.contains { $0.content == text } })
        #expect(diff.hunks[index].lines.allSatisfy { $0.kind != .context })
        return (index, diff)
      }
      func run(_ target: PatchTarget, _ selection: PatchSelection) {
        let result = GitOperations.apply(target, selection: selection, path: "f.txt", options: zero, root: repo.root)
        #expect(throws: Never.self) { try result.get() }
      }
      func index() throws -> String { try repo.git("show", ":f.txt") }

      // Stage a pure insertion, one line of a mixed hunk, and a pure deletion.
      var expected = inserting("inserted after 10", after: "line 10", in: base)
      run(.stage, .wholeHunks([try hunk(.unstaged, containing: "inserted after 10").index]))
      #expect(try index() == expected.joined(separator: "\n"))

      let mixed = try hunk(.unstaged, containing: "inserted after 25")
      let line = try #require(mixed.diff.hunks[mixed.index].lines.firstIndex { $0.content == "inserted after 25" })
      run(.stage, .lines([line], inHunk: mixed.index))
      expected = inserting("inserted after 25", after: "line 25", in: expected)
      #expect(try index() == expected.joined(separator: "\n"))

      run(.stage, .wholeHunks([try hunk(.unstaged, containing: "line 20").index]))
      expected.removeAll { $0 == "line 20" }
      #expect(try index() == expected.joined(separator: "\n"))

      // Unstage the deletion and the insertion again.
      run(.unstage, .wholeHunks([try hunk(.staged, containing: "line 20").index]))
      expected = inserting("line 20", after: "line 19", in: expected)
      #expect(try index() == expected.joined(separator: "\n"))
      run(.unstage, .wholeHunks([try hunk(.staged, containing: "inserted after 10").index]))
      expected.removeAll { $0 == "inserted after 10" }
      #expect(try index() == expected.joined(separator: "\n"))

      // Revert the working tree's deletion and modification.
      run(.discard, .wholeHunks([try hunk(.unstaged, containing: "line 20").index]))
      working = inserting("line 20", after: "line 19", in: working)
      #expect(try repo.read("f.txt") == working.joined(separator: "\n") + "\n")
      run(.discard, .wholeHunks([try hunk(.unstaged, containing: "line 2 changed").index]))
      working[1] = "line 2"
      #expect(try repo.read("f.txt") == working.joined(separator: "\n") + "\n")
      #expect(try index() == expected.joined(separator: "\n"), "reverting leaves the index alone")
    }

    @Test func hunksOfACutShortDiffAreRefused() throws {
      // With the whole file as context the one hunk ends where the diff was
      // cut, so staging it would take changes past what was shown.
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let base = (1...(maxDiffTotalLines + 100)).map { "line \($0)" }
      try repo.commit(["big.txt": base.joined(separator: "\n") + "\n"])
      var edited = base
      edited[0] = "first changed"
      edited[edited.count - 1] = "last changed"
      try repo.write("big.txt", edited.joined(separator: "\n") + "\n")
      let whole = DiffOptions(contextLines: 100_000)  // the review's "Whole File"
      let diff = try GitClient.fileDiff(repoPath: repo.root, path: "big.txt", scope: .unstaged, options: whole)
      try #require(diff.truncated && diff.hunks.count == 1)

      for target in [PatchTarget.stage, .discard] {
        let result = GitOperations.apply(
          target, selection: .wholeHunks([0]), path: "big.txt", expectedHunkIds: [0: diff.hunkIds[0]],
          options: whole, root: repo.root)
        guard case .failure(.invalid(let message)) = result else {
          Issue.record("expected \(target) to be refused, got \(result)")
          continue
        }
        #expect(message == GitOperations.truncatedDiffMessage)
      }
      #expect(try repo.git("diff", "--cached", "--name-only") == "", "nothing staged")
      #expect(try repo.read("big.txt") == edited.joined(separator: "\n") + "\n", "nothing reverted")
      // The whole file still stages.
      _ = try GitOperations.stage(paths: ["big.txt"], root: repo.root).get()
      #expect(try repo.git("diff", "--name-only") == "")
    }

    @Test func partialSelectionOfNewFileIsRefused() throws {
      let patch = """
        diff --git a/n.txt b/n.txt
        new file mode 100644
        index 0000000..1111111
        --- /dev/null
        +++ b/n.txt
        @@ -0,0 +1,2 @@
        +one
        +two

        """
      #expect(throws: PatchBuilder.BuildError.partialAddOrDelete) {
        try PatchBuilder.build(patch: patch, selection: .lines([0], inHunk: 0), reverse: false)
      }
      let whole = try PatchBuilder.build(patch: patch, selection: .wholeHunks([0]), reverse: false)
      #expect(whole.contains("+two"))
    }

    // MARK: Commit + branches + worktrees

    @Test func commitRunsThroughGitAndCanAmendAndUncommit() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      try repo.write("a.txt", "2\n")
      _ = try GitOperations.stage(paths: ["a.txt"], root: repo.root).get()
      let sha = try GitOperations.commit(message: "Change a\n\nWith a body.", root: repo.root).get()
      #expect(sha == (try repo.git("rev-parse", "HEAD")))
      #expect(try repo.git("log", "-1", "--format=%B") == "Change a\n\nWith a body.")

      let amended = try GitOperations.commit(
        message: "Change a (amended)", options: .init(amend: true), root: repo.root
      ).get()
      #expect(amended != sha)
      #expect(try repo.git("rev-list", "--count", "HEAD") == "2")

      _ = try GitOperations.uncommit(root: repo.root).get()
      #expect(try repo.git("rev-list", "--count", "HEAD") == "1")
      #expect(try repo.git("diff", "--cached", "--name-only") == "a.txt")
    }

    @Test func commitMessagesKeepHashLines() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.write("a.txt", "1\n")
      try repo.git("add", "a.txt")
      let message = "Fix crash\n\n#482 was caused by a race.\n## Notes\nKeep"
      _ = try GitOperations.commit(message: message, root: repo.root).get()
      #expect(try repo.git("log", "-1", "--format=%B") == message)
    }

    @Test func pathsAreLiteralNotGlobs() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["app/[id].tsx": "a\n", "app/d.tsx": "b\n"])
      try repo.write("app/[id].tsx", "a2\n")
      try repo.write("app/d.tsx", "b2\n")
      // As a glob, "[id]" would also match "d.tsx".
      _ = try GitOperations.discardWorkingTree(paths: ["app/[id].tsx"], root: repo.root).get()
      #expect(try repo.read("app/[id].tsx") == "a\n")
      #expect(try repo.read("app/d.tsx") == "b2\n")
      _ = try GitOperations.stage(paths: ["app/[id].tsx"], root: repo.root).get()
      #expect(try repo.git("diff", "--cached", "--name-only") == "")
    }

    @Test func refsThatLookLikeOptionsStayRefs() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      try repo.git("update-ref", "refs/heads/--orphan=x", "HEAD")
      _ = try GitOperations.switchBranch("--orphan=x", root: repo.root).get()
      #expect(try repo.git("rev-parse", "--abbrev-ref", "HEAD") == "--orphan=x")
      #expect(repo.exists("a.txt"), "no orphan branch emptied the tree")
    }

    @Test func emptyMessageIsRejected() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let result = GitOperations.commit(message: "  \n", root: repo.root)
      guard case .failure(.invalid) = result else {
        Issue.record("expected invalid")
        return
      }
    }

    @Test func branchAndWorktreeOperations() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      _ = try GitOperations.createBranch("topic", checkout: false, root: repo.root).get()
      #expect(GitOperations.branches(root: repo.root).local.sorted() == ["main", "topic"])
      _ = try GitOperations.renameBranch("topic", to: "topic-2", root: repo.root).get()
      _ = try GitOperations.deleteBranch("topic-2", root: repo.root).get()
      #expect(GitOperations.branches(root: repo.root).local == ["main"])

      let path = repo.root + "-wt-feature"
      defer { try? FileManager.default.removeItem(atPath: path) }
      _ = try GitOperations.addWorktree(
        path: path, branch: "feature", newBranch: true, root: repo.root
      ).get()
      let trees = GitOperations.worktrees(root: repo.root)
      #expect(trees.count == 2)
      #expect(trees.contains { $0.branch == "feature" })
      _ = try GitOperations.removeWorktree(path: path, root: repo.root).get()
      #expect(GitOperations.worktrees(root: repo.root).count == 1)
    }
  }
#endif

#if canImport(Testing)
  @Suite(.serialized)
  struct SafetySnapshotTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func restoringAnUntrackedFileSucceeds() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.write("new.txt", "draft\n")
      let snapshot = try SafetySnapshots.create(reason: "discard", root: repo.root).get()
      try FileManager.default.removeItem(atPath: repo.root + "/new.txt")
      // The file is in neither index; the index step must not fail the restore.
      _ = try SafetySnapshots.restore(snapshot, paths: ["new.txt"], root: repo.root).get()
      #expect(try repo.read("new.txt") == "draft\n")
      #expect(try repo.git("status", "--porcelain") == "?? new.txt")
    }

    @Test func snapshotsLeaveTheRealIndexUnlocked() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.write("a.txt", "two\n")
      try repo.git("add", "a.txt")
      // Someone else's git command holds the index lock.
      let lock = repo.root + "/.git/index.lock"
      FileManager.default.createFile(atPath: lock, contents: nil)
      let snapshot = try SafetySnapshots.create(reason: "turn start", root: repo.root).get()
      try FileManager.default.removeItem(atPath: lock)
      let staged = try repo.git("write-tree")
      #expect(snapshot.indexTree == staged, "the index was still recorded")
    }

    @Test func nestedCheckpointsListAndSurvivePruning() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      let prefix = SafetySnapshots.checkpointPrefix + "TERMINAL-UUID-1234/"
      let start = try SafetySnapshots.create(reason: "turn start", root: repo.root, prefix: prefix).get()
      try repo.write("a.txt", "two\n")
      let end = try SafetySnapshots.create(reason: "turn end", root: repo.root, prefix: prefix).get()

      let listed = SafetySnapshots.list(root: repo.root, prefix: SafetySnapshots.checkpointPrefix)
      #expect(listed.map(\.ref) == [end.ref, start.ref])
      #expect(abs(listed[1].date.timeIntervalSince(start.date)) < 1)
      // Fresh checkpoints aren't mistaken for ancient ones.
      #expect(SafetySnapshots.prune(root: repo.root, prefix: SafetySnapshots.checkpointPrefix).isEmpty)
      #expect(SafetySnapshots.list(root: repo.root, prefix: SafetySnapshots.checkpointPrefix).count == 2)
    }

    @Test func pruningDeletesOldAndExcessSnapshotsInOneBatch() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      let head = try repo.git("rev-parse", "HEAD")
      let now = Int(Date().timeIntervalSince1970 * 1000)
      let day = 24 * 3600 * 1000
      // 30 ancient snapshots, 12 recent ones, and one outside the prefix.
      let old = (0..<30).map { SafetySnapshots.oplogPrefix + "\(now - 20 * day - $0)-discard" }
      let recent = (0..<12).map { SafetySnapshots.oplogPrefix + "\(now - $0 * 1000)-discard" }
      let other = SafetySnapshots.checkpointPrefix + "\(now - 20 * day)-turn-start"
      let commands = (old + recent + [other]).map { "create \($0) \(head)\n" }.joined()
      try repo.git(["update-ref", "--stdin"], stdin: commands)
      try repo.git("pack-refs", "--all")

      let deleted = SafetySnapshots.prune(root: repo.root, keep: 10)
      #expect(Set(deleted) == Set(old + recent.suffix(2)), "the old ones, and the recent ones past the newest 10")
      let left = try repo.git("for-each-ref", "--format=%(refname)", "refs/impulse/").split(separator: "\n")
      #expect(Set(left.map(String.init)) == Set(recent.prefix(10) + [other]))
      #expect(SafetySnapshots.prune(root: repo.root, keep: 10).isEmpty)
    }

    @Test func snapshotRestoresDiscardedAndUntrackedWork() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "committed\n"])
      try repo.write("a.txt", "staged edit\n")
      try repo.git("add", "a.txt")
      try repo.write("a.txt", "worktree edit\n")
      try repo.write("notes.md", "untracked notes\n")
      let indexBefore = try repo.git("diff", "--cached")

      let snapshot = try SafetySnapshots.create(reason: "Discard everything", root: repo.root).get()
      // The real index is untouched by taking a snapshot.
      #expect(try repo.git("diff", "--cached") == indexBefore)
      #expect(try repo.git("status", "--porcelain").contains("?? notes.md"))

      // Destroy the work.
      try repo.git("reset", "-q", "--hard")
      try FileManager.default.removeItem(atPath: repo.root + "/notes.md")
      #expect(try repo.read("a.txt") == "committed\n")

      _ = try SafetySnapshots.restore(snapshot, root: repo.root).get()
      #expect(try repo.read("a.txt") == "worktree edit\n")
      #expect(try repo.read("notes.md") == "untracked notes\n")
      #expect(try repo.git("diff", "--cached") == indexBefore)

      let listed = SafetySnapshots.list(root: repo.root)
      #expect(listed.first?.ref == snapshot.ref)
      #expect(listed.first?.reason == "Discard everything")
      #expect(listed.first?.indexTree == snapshot.indexTree)
      // Snapshot refs aren't branches.
      #expect(GitOperations.branches(root: repo.root).local == ["main"])

      SafetySnapshots.prune(root: repo.root, keep: 0)
      #expect(SafetySnapshots.list(root: repo.root).isEmpty)
    }

    @Test func snapshotWorksWithoutAnyCommits() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.write("first.txt", "hello\n")
      let snapshot = try SafetySnapshots.create(reason: "before", root: repo.root).get()
      try FileManager.default.removeItem(atPath: repo.root + "/first.txt")
      _ = try SafetySnapshots.restore(snapshot, root: repo.root).get()
      #expect(try repo.read("first.txt") == "hello\n")
    }
  }
#endif

#if canImport(Testing)
  struct RepoWatcherTests {
    @Test func classifiesGitDirectoryPaths() {
      #expect(RepoWatcher.classifyGitPath("index") == .index)
      #expect(RepoWatcher.classifyGitPath("HEAD") == .refs)
      #expect(RepoWatcher.classifyGitPath("refs/heads/main") == .refs)
      #expect(RepoWatcher.classifyGitPath("refs/stash") == .refs)
      #expect(RepoWatcher.classifyGitPath("MERGE_HEAD") == .operation)
      #expect(RepoWatcher.classifyGitPath("rebase-merge/msgnum") == .operation)
      #expect(RepoWatcher.classifyGitPath("objects/ab/cdef") == [])
      #expect(RepoWatcher.classifyGitPath("index.lock") == [])
      #expect(RepoWatcher.classifyGitPath("worktrees/feature/index") == .index)
      #expect(RepoWatcher.classifyGitPath("worktrees/feature/HEAD") == .refs)
    }

    @Test func reportsWorkingTreeAndIndexChanges() async throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      let snapshot = try #require(GitClient.snapshot(forPath: repo.root))

      let received = LockedChanges()
      let watcher = RepoWatcher(
        root: snapshot.root, gitDir: snapshot.gitDir, commonDir: snapshot.commonDir,
        debounce: 0.05
      ) { change in received.add(change) }
      watcher.start()
      defer { watcher.stop() }
      try await Task.sleep(for: .milliseconds(300))

      try repo.write("a.txt", "2\n")
      try repo.git("add", "a.txt")
      for _ in 0..<40 where !received.value.contains([.workingTree, .index]) {
        try await Task.sleep(for: .milliseconds(100))
      }
      #expect(received.value.contains(.workingTree))
      #expect(received.value.contains(.index))
    }
  }

  final class LockedChanges: @unchecked Sendable {
    private let lock = NSLock()
    private var changes: RepoWatcher.Change = []
    func add(_ change: RepoWatcher.Change) {
      lock.lock()
      changes.formUnion(change)
      lock.unlock()
    }
    var value: RepoWatcher.Change {
      lock.lock()
      defer { lock.unlock() }
      return changes
    }
  }
#endif

#if canImport(Testing)
  @Suite(.serialized)
  struct BlameAndBaseTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func blameAttributesLinesAndMarksUncommitted() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\ntwo\n"], message: "first lines")
      try repo.write("a.txt", "one\ntwo\nthree\n")
      let blame = GitOperations.blame(path: "a.txt", root: repo.root)
      #expect(blame.count == 3)
      #expect(blame[1]?.summary == "first lines")
      #expect(blame[1]?.author == "Impulse Test")
      #expect(blame[2]?.isUncommitted == false)
      #expect(blame[3]?.isUncommitted == true)
    }

    @Test func baseContentIsTheIndexVersion() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit([".gitignore": "*.log\n", "a.txt": "committed\n"])
      try repo.write("a.txt", "staged\n")
      try repo.git("add", "a.txt")
      try repo.write("a.txt", "working\n")
      try repo.write("new.txt", "fresh\n")
      try repo.write("debug.log", "ignored\n")
      #expect(GitClient.baseContent(forFile: repo.root + "/a.txt") == "staged\n")
      #expect(GitClient.baseContent(forFile: repo.root + "/new.txt") == "")
      #expect(GitClient.baseContent(forFile: repo.root + "/debug.log") == nil)
      #expect(GitClient.baseContent(forFile: "/tmp/not-in-a-repo-\(UUID().uuidString).txt") == nil)
    }
  }

  struct GitLogTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func readsHistoryWithParentsAndRefs() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.git("checkout", "-q", "-b", "topic")
      try repo.commit(["b.txt": "topic\n"])
      try repo.git("checkout", "-q", "main")
      try repo.commit(["a.txt": "two\n"])
      try repo.git("merge", "-q", "--no-ff", "-m", "Merge topic", "topic")
      try repo.git("tag", "v1")

      let entries = try GitLog.entries(root: repo.root).get()
      #expect(entries.count == 4)
      let merge = entries[0]
      #expect(merge.subject == "Merge topic")
      #expect(merge.parents.count == 2)
      #expect(merge.refs.contains("HEAD -> main"))
      #expect(merge.refs.contains("tag: v1"))
      #expect(merge.author == "Impulse Test")
      #expect(entries.last?.parents.isEmpty == true)

      // Paging.
      let page = try GitLog.entries(root: repo.root, skip: 1, limit: 2).get()
      #expect(page.map(\.sha) == Array(entries[1...2]).map(\.sha))

      // Path filter: only commits that touched b.txt.
      let file = try GitLog.entries(root: repo.root, path: "b.txt").get()
      #expect(file.count == 1)
      #expect(file[0].refs.contains("topic"))
    }

    @Test func detailsCarryTheWholeMessage() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "First")
      try repo.commit(["a.txt": "two\n"], message: "Second change\n\nWhy it matters.\nMore detail.")
      let head = try repo.git("rev-parse", "HEAD")
      let details = try #require(GitLog.details(root: repo.root, sha: "HEAD"))
      #expect(details.sha == head)
      #expect(details.parents == [try repo.git("rev-parse", "HEAD~1")])
      #expect(details.subject == "Second change")
      #expect(details.body == "Why it matters.\nMore detail.")
      #expect(details.author == "Impulse Test")
      #expect(GitLog.details(root: repo.root, sha: "nope") == nil)
    }

    @Test func divergenceSplitsIncomingAndOutgoing() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      #expect(GitLog.divergence(root: repo.root).outgoing.isEmpty, "no upstream yet")
      try repo.git("branch", "up")
      try repo.git("branch", "--set-upstream-to=up")
      try repo.commit(["a.txt": "mine\n"])
      let mine = try repo.git("rev-parse", "HEAD")
      try repo.git("checkout", "-q", "up")
      try repo.commit(["b.txt": "theirs\n"])
      let theirs = try repo.git("rev-parse", "HEAD")
      try repo.git("checkout", "-q", "main")

      let divergence = GitLog.divergence(root: repo.root)
      #expect(divergence.outgoing == [mine])
      #expect(divergence.incoming == [theirs])
    }

    @Test func queryLimitsAuthorDatesAndPath() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "First")
      try repo.write("b.txt", "two\n")
      try repo.git("add", "b.txt")
      try repo.git("commit", "-q", "--author=Jane Doe <jane@example.com>", "-m", "Jane's change")

      let jane = try GitLog.entries(root: repo.root, query: .parse("author:JANE")).get()
      #expect(jane.map(\.subject) == ["Jane's change"], "author matches case-insensitively")
      let byPath = try GitLog.entries(root: repo.root, query: .parse("path:a.txt")).get()
      #expect(byPath.map(\.subject) == ["First"])
      // Commits are dated 2026-01-01 (UTC); days well away from it keep the
      // test independent of the local time zone.
      #expect(try GitLog.entries(root: repo.root, query: .parse("since:2025-12-25")).get().count == 2)
      #expect(try GitLog.entries(root: repo.root, query: .parse("since:2026-01-05")).get().isEmpty)
      #expect(try GitLog.entries(root: repo.root, query: .parse("until:2025-12-25")).get().isEmpty)
      #expect(try GitLog.entries(root: repo.root, query: .parse("until:2026-01-05")).get().count == 2)
    }

    @Test func forkPointFromTheDefaultBranch() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "base")
      let base = try repo.git("rev-parse", "HEAD")
      #expect(GitLog.forkPoint(root: repo.root) == nil, "on the default branch itself")
      try repo.git("checkout", "-q", "-b", "topic")
      try repo.commit(["b.txt": "work\n"], message: "topic work")
      let work = try repo.git("rev-parse", "HEAD")
      try repo.git("checkout", "-q", "main")
      try repo.commit(["c.txt": "later\n"], message: "main moves on")
      try repo.git("checkout", "-q", "topic")

      let fork = try #require(GitLog.forkPoint(root: repo.root))
      #expect(fork.base == "main")
      #expect(fork.sha == base)
      #expect(fork.branchOnly == [work])
      #expect(GitLog.forkPoint(root: repo.root, limit: 0) == nil, "too long to list")
    }

    @Test func allBranchesSkipsPrivateRefs() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.write("a.txt", "dirty\n")
      _ = try SafetySnapshots.create(reason: "test", root: repo.root).get()
      try repo.git("checkout", "-q", "-b", "side")
      try repo.git("checkout", "-q", "-")
      let all = try GitLog.entries(root: repo.root, scope: .all).get()
      #expect(all.count == 1, "the snapshot commit isn't history")
    }

    @Test func folderHistoryMatchesGitLog() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["src/a.txt": "one\n"], message: "add a")
      try repo.commit(["README.md": "readme\n"], message: "outside src")
      try repo.commit(["src/b.txt": "two\n"], message: "add b")
      try repo.git("mv", "src/a.txt", "src/c.txt")
      try repo.git("commit", "-q", "-m", "rename a")
      try repo.commit(["src/c.txt": "three\n"], message: "edit c")

      // A folder (as the file tree passes it, without a trailing slash) gets
      // every commit that touched anything in it.
      let expected = try repo.git("log", "--format=%H", "--", "src").split(separator: "\n").map(String.init)
      #expect(expected.count == 4)
      #expect(try GitLog.entries(root: repo.root, path: "src").get().map(\.sha) == expected)
      #expect(try GitLog.entries(root: repo.root, path: "src/").get().map(\.sha) == expected)

      // A deleted folder is still a folder.
      try repo.git("rm", "-q", "-r", "src")
      try repo.git("commit", "-q", "-m", "drop src")
      let deleted = try repo.git("log", "--format=%H", "--", "src").split(separator: "\n").map(String.init)
      #expect(try GitLog.entries(root: repo.root, path: "src").get().map(\.sha) == deleted)
    }

    @Test func fileHistoryFollowsRenames() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["old.txt": "a\nb\nc\nd\n"], message: "add old")
      try repo.git("mv", "old.txt", "new.txt")
      try repo.git("commit", "-q", "-m", "rename")
      try repo.commit(["new.txt": "a\nb\nc\nd\ne\n"], message: "edit new")
      let expected = try repo.git("log", "--follow", "--format=%H", "--", "new.txt")
        .split(separator: "\n").map(String.init)
      #expect(expected.count == 3)
      #expect(try GitLog.entries(root: repo.root, path: "new.txt").get().map(\.sha) == expected)
    }

    @Test func branchScopeListsThatBranch() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "base")
      try repo.git("checkout", "-q", "-b", "topic")
      try repo.commit(["b.txt": "topic\n"], message: "topic work")
      try repo.git("checkout", "-q", "main")
      try repo.commit(["a.txt": "two\n"], message: "main work")
      // A file named like the branch doesn't make the revision ambiguous.
      try repo.write("topic", "not a branch\n")

      let topic = try GitLog.entries(root: repo.root, scope: .branch("topic")).get()
      #expect(topic.map(\.subject) == ["topic work", "base"])
      let expected = try repo.git("log", "--format=%H", "topic", "--").split(separator: "\n").map(String.init)
      #expect(topic.map(\.sha) == expected)
      #expect(try GitLog.entries(root: repo.root).get().map(\.subject) == ["main work", "base"])
    }
  }

  struct BranchDetailsTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func reportsMergedAndCurrentBranches() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "first")
      try repo.git("branch", "merged-topic")
      try repo.git("checkout", "-q", "-b", "open-topic")
      try repo.commit(["b.txt": "work\n"], message: "topic work")
      try repo.git("checkout", "-q", "main")

      let branches = GitOperations.branchDetails(root: repo.root, base: "main")
      let byName = Dictionary(uniqueKeysWithValues: branches.map { ($0.name, $0) })
      #expect(Set(byName.keys) == ["main", "merged-topic", "open-topic"])
      #expect(byName["main"]?.isCurrent == true)
      #expect(byName["main"]?.isMerged == false, "the base itself isn't 'merged'")
      #expect(byName["merged-topic"]?.isMerged == true)
      #expect(byName["open-topic"]?.isMerged == false)
      #expect(byName["open-topic"]?.subject == "topic work")
      #expect(byName["open-topic"]?.upstream == nil)
    }
  }

  struct StashUndoTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    @Test func droppedStashCanBeStoredBack() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "first")
      try repo.write("a.txt", "two\n")
      try repo.write("fresh.txt", "new\n")
      _ = try GitOperations.stash(message: "wip", root: repo.root).get()

      let sha = try #require(GitOperations.stashCommit(0, root: repo.root))
      #expect(GitOperations.stashUntrackedPaths(sha, root: repo.root) == ["fresh.txt"])
      #expect(GitOperations.stashPaths(sha, root: repo.root) == ["a.txt", "fresh.txt"])
      _ = try GitOperations.stashDrop(0, root: repo.root).get()
      #expect(GitOperations.stashList(root: repo.root).isEmpty)

      _ = try GitOperations.stashStore(sha, message: "On main: wip", root: repo.root).get()
      let restored = GitOperations.stashList(root: repo.root)
      #expect(restored.map(\.message) == ["On main: wip"])
      #expect(GitOperations.stashCommit(0, root: repo.root) == sha)
      #expect(GitOperations.stashCommit(3, root: repo.root) == nil)
    }

    @Test func undoingAPopRestoresOnlyTheStashsFiles() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n", "b.txt": "b\n"], message: "first")
      try repo.write("a.txt", "stashed\n")
      try repo.write("fresh.txt", "new\n")
      _ = try GitOperations.stash(message: "wip", root: repo.root).get()
      try repo.write("a.txt", "before pop\n")
      _ = try GitOperations.stash(message: "keep", root: repo.root).get()
      // stash@{0} = keep, stash@{1} = wip; pop wip onto a clean tree.
      let sha = try #require(GitOperations.stashCommit(1, root: repo.root))
      let snapshot = try SafetySnapshots.create(reason: "pop stash", root: repo.root).get()
      _ = try GitOperations.stashApply(1, pop: true, root: repo.root).get()
      #expect(try repo.git("show", ":a.txt") == "one", "pop --index keeps the index as it was")
      #expect(FileManager.default.fileExists(atPath: repo.root + "/fresh.txt"))
      try repo.write("b.txt", "edited after the pop\n")

      _ = try GitOperations.undoStashPop(sha, message: "On main: wip", snapshot: snapshot, root: repo.root).get()
      #expect(try String(contentsOfFile: repo.root + "/a.txt", encoding: .utf8) == "one\n")
      #expect(!FileManager.default.fileExists(atPath: repo.root + "/fresh.txt"))
      #expect(
        try String(contentsOfFile: repo.root + "/b.txt", encoding: .utf8) == "edited after the pop\n",
        "files outside the stash are left alone")
      #expect(GitOperations.stashList(root: repo.root).map(\.message) == ["On main: wip", "On main: keep"])
    }
  }
#endif
