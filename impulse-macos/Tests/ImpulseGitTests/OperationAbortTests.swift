#if canImport(Testing)
  import Foundation
  import ImpulseKit
  import Testing

  @testable import ImpulseGit

  /// Aborting after edits, against real merges. The git CLI is the oracle.
  @Suite(.serialized)
  struct OperationAbortTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// main and upgrade both change page.tsx (a conflict); upgrade also
    /// changes link.tsx (merges cleanly). notes.txt has uncommitted work
    /// from before the merge.
    private func conflictedMerge() throws -> TempRepo {
      let repo = try TempRepo.create()
      try repo.commit(["page.tsx": "a\n", "link.tsx": "Link\n", "other.txt": "x\n", "notes.txt": "n\n"], message: "base")
      try repo.git("switch", "-q", "-c", "upgrade")
      try repo.commit(["page.tsx": "upgrade\n", "link.tsx": "RouterLink\n"], message: "upgrade")
      try repo.git("switch", "-q", "main")
      try repo.commit(["page.tsx": "overhaul\n"], message: "overhaul")
      try repo.write("notes.txt", "local work\n")
      _ = try? repo.git("merge", "upgrade")
      #expect(OperationAbort.incomingHead(root: repo.root)?.name == "MERGE_HEAD")
      return repo
    }

    @Test func gitsOwnAbortIsUsedWhenItWorks() throws {
      let repo = try conflictedMerge()
      defer { repo.destroy() }
      try repo.write("page.tsx", "resolved\n")
      let result = try OperationAbort.abort(.merge, root: repo.root).get()
      #expect(result.outcome == .aborted)
      #expect(try repo.read("notes.txt") == "local work\n")
      #expect(try repo.read("page.tsx") == "overhaul\n")
      #expect(OperationAbort.incomingHead(root: repo.root) == nil)
    }

    @Test func aRefusedAbortResetsAndKeepsExactlyTheWorkFromBefore() throws {
      let repo = try conflictedMerge()
      defer { repo.destroy() }
      OperationAbort.recordStartIfNeeded(root: repo.root)
      // Resolving: a cleanly merged file edited again makes git refuse.
      try repo.write("page.tsx", "resolved\n")
      try repo.write("link.tsx", "fixed Link\n")
      try repo.write("other.txt", "edited while resolving\n")
      guard case .failure = GitOperations.perform(.abort, on: .merge, root: repo.root) else {
        Issue.record("git's own abort should refuse here")
        return
      }

      let result = try OperationAbort.abort(.merge, root: repo.root).get()
      #expect(result.outcome == .reset(kept: ["notes.txt"], exact: true))
      #expect(OperationAbort.incomingHead(root: repo.root) == nil, "the merge is over")
      #expect(try repo.read("notes.txt") == "local work\n", "work from before the merge is back")
      #expect(try repo.read("page.tsx") == "overhaul\n")
      #expect(try repo.read("link.tsx") == "Link\n")
      #expect(try repo.read("other.txt") == "x\n", "edits made while resolving aren't mistaken for earlier work")
      #expect(result.undo != nil)
    }

    @Test func withoutARecordEditsOutsideTheMergeAreKept() throws {
      let repo = try conflictedMerge()
      defer { repo.destroy() }
      try repo.write("link.tsx", "fixed Link\n")
      try repo.write("other.txt", "edited while resolving\n")
      let result = try OperationAbort.abort(.merge, root: repo.root).get()
      #expect(result.outcome == .reset(kept: ["notes.txt", "other.txt"], exact: false))
      #expect(try repo.read("other.txt") == "edited while resolving\n")
      #expect(try repo.read("link.tsx") == "Link\n")
    }

    @Test func undoReopensTheMergeWithTheEditsBack() throws {
      let repo = try conflictedMerge()
      defer { repo.destroy() }
      OperationAbort.recordStartIfNeeded(root: repo.root)
      try repo.write("page.tsx", "resolved\n")
      try repo.write("link.tsx", "fixed Link\n")
      let result = try OperationAbort.abort(.merge, root: repo.root).get()
      _ = try OperationAbort.undo(result, operation: .merge, root: repo.root).get()
      #expect(OperationAbort.incomingHead(root: repo.root)?.name == "MERGE_HEAD", "the merge is open again")
      #expect(try repo.read("page.tsx") == "resolved\n")
      #expect(try repo.read("link.tsx") == "fixed Link\n")
      #expect(try repo.read("notes.txt") == "local work\n")
    }

    @Test func aCherryPickIsAbortedTheSameWay() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n", "b.txt": "1\n"], message: "base")
      try repo.git("switch", "-q", "-c", "side")
      try repo.commit(["a.txt": "side\n", "b.txt": "side\n"], message: "side")
      let pick = try repo.git("rev-parse", "HEAD")
      try repo.git("switch", "-q", "main")
      try repo.commit(["a.txt": "main\n"], message: "main")
      _ = try? repo.git("cherry-pick", pick)
      #expect(OperationAbort.incomingHead(root: repo.root)?.name == "CHERRY_PICK_HEAD")
      try repo.write("b.txt", "edited\n")
      let result = try OperationAbort.abort(.cherryPick, root: repo.root).get()
      if case .aborted = result.outcome {
      } else {
        #expect(try repo.read("b.txt") == "1\n")
      }
      #expect(OperationAbort.incomingHead(root: repo.root) == nil)
      #expect(try repo.read("a.txt") == "main\n")
    }
  }
#endif
