#if canImport(Testing)
  import Foundation
  import ImpulseKit
  import Testing

  @testable import ImpulseGit

  /// Tags, remotes, pull modes, merge and rebase, against a bare repository
  /// standing in for the remote. The git CLI is the oracle.
  @Suite(.serialized)
  struct GitRemoteTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// A clone of a bare "origin" with one pushed commit on main.
    private func cloneWithOrigin() throws -> (repo: TempRepo, origin: TempRepo, other: TempRepo) {
      let origin = try TempRepo.create()
      try origin.git("config", "core.bare", "true")
      let seed = try TempRepo.create()
      try seed.commit(["a.txt": "1\n"], message: "first")
      try seed.git("remote", "add", "origin", origin.root + "/.git")
      try seed.git("push", "-q", "-u", "origin", "main")
      let repo = try TempRepo.create()
      try repo.git("remote", "add", "origin", origin.root + "/.git")
      try repo.git("fetch", "-q", "origin")
      try repo.git("checkout", "-q", "-B", "main", "--track", "origin/main")
      return (repo, origin, seed)
    }

    @Test func lightweightAndAnnotatedTags() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"], message: "first")
      let first = try repo.git("rev-parse", "HEAD")
      try repo.commit(["a.txt": "2\n"], message: "second")

      _ = try GitOperations.createTag("v1.0", at: first, root: repo.root).get()
      _ = try GitOperations.createTag("v2.0", message: "Second release\n\nNotes.", root: repo.root).get()
      #expect(try repo.git("cat-file", "-t", "v1.0") == "commit")
      #expect(try repo.git("cat-file", "-t", "v2.0") == "tag")
      #expect(try repo.git("tag", "-l", "--format=%(contents)", "v2.0") == "Second release\n\nNotes.")

      let tags = GitOperations.tagDetails(root: repo.root)
      #expect(Set(tags.map(\.name)) == ["v1.0", "v2.0"])
      let v1 = try #require(tags.first { $0.name == "v1.0" })
      #expect(v1.commit == first && !v1.isAnnotated && v1.subject == "first")
      let v2 = try #require(tags.first { $0.name == "v2.0" })
      #expect(v2.commit == (try repo.git("rev-parse", "HEAD")) && v2.isAnnotated && v2.subject == "Second release")

      // Existing names fail unless forced; bad names never reach git.
      guard case .failure = GitOperations.createTag("v1.0", root: repo.root) else {
        Issue.record("expected an existing tag to fail")
        return
      }
      _ = try GitOperations.createTag("v1.0", force: true, root: repo.root).get()
      #expect(try repo.git("rev-parse", "v1.0^{commit}") == (try repo.git("rev-parse", "HEAD")))
      guard case .failure(.invalid) = GitOperations.createTag("bad name", root: repo.root) else {
        Issue.record("expected an invalid name")
        return
      }

      _ = try GitOperations.deleteTag("v1.0", root: repo.root).get()
      #expect(try repo.git("tag") == "v2.0")
    }

    @Test func pushAndDeleteTagsOnRemote() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      defer { [repo, origin, seed].forEach { $0.destroy() } }
      #expect(GitOperations.defaultRemote(root: repo.root) == "origin")
      #expect(GitOperations.remoteURL("origin", root: repo.root) == origin.root + "/.git")

      _ = try GitOperations.createTag("v1", root: repo.root).get()
      _ = try GitOperations.createTag("v2", message: "two", root: repo.root).get()
      _ = try GitOperations.pushTag("v1", remote: "origin", root: repo.root).get()
      #expect(try origin.git("tag") == "v1")
      _ = try GitOperations.pushAllTags(remote: "origin", root: repo.root).get()
      #expect(try origin.git("tag") == "v1\nv2")
      _ = try GitOperations.deleteRemoteTag("v1", remote: "origin", root: repo.root).get()
      #expect(try origin.git("tag") == "v2")
      #expect(try repo.git("tag") == "v1\nv2", "local tags stay")
    }

    @Test func pushFollowsAnnotatedTags() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      defer { [repo, origin, seed].forEach { $0.destroy() } }
      try repo.commit(["b.txt": "b\n"], message: "second")
      _ = try GitOperations.createTag("light", root: repo.root).get()
      _ = try GitOperations.createTag("release", message: "release", root: repo.root).get()
      _ = try GitOperations.push(followTags: true, root: repo.root).get()
      #expect(try origin.git("rev-parse", "main") == (try repo.git("rev-parse", "HEAD")))
      #expect(try origin.git("tag") == "release", "only annotated tags follow")
    }

    @Test func pullModes() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      defer { [repo, origin, seed].forEach { $0.destroy() } }
      // Upstream moves on.
      try seed.commit(["a.txt": "1\nupstream\n"], message: "upstream")
      try seed.git("push", "-q", "origin", "main")

      _ = try GitOperations.fetch(allRemotes: true, root: repo.root).get()
      #expect(try repo.git("rev-list", "--count", "main..origin/main") == "1")
      _ = try GitOperations.pull(mode: .fastForwardOnly, root: repo.root).get()
      #expect(try repo.git("rev-parse", "HEAD") == (try seed.git("rev-parse", "HEAD")))

      // Diverged: fast-forward only refuses; rebase replays; merge merges.
      try seed.commit(["c.txt": "c\n"], message: "upstream 2")
      try seed.git("push", "-q", "origin", "main")
      try repo.commit(["d.txt": "d\n"], message: "local")
      guard case .failure = GitOperations.pull(mode: .fastForwardOnly, root: repo.root) else {
        Issue.record("expected ff-only to refuse a diverged branch")
        return
      }
      _ = try GitOperations.pull(mode: .rebase, root: repo.root).get()
      #expect(try repo.git("log", "--format=%s", "-3") == "local\nupstream 2\nupstream")
      #expect(try repo.git("rev-list", "--merges", "--count", "HEAD") == "0")

      try seed.commit(["e.txt": "e\n"], message: "upstream 3")
      try seed.git("push", "-q", "origin", "main")
      _ = try GitOperations.pull(mode: .merge, root: repo.root).get()
      #expect(try repo.git("rev-list", "--merges", "--count", "HEAD") == "1")
      #expect(repo.exists("e.txt") && repo.exists("d.txt"))
    }

    @Test func mergeAndRebaseBranches() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"], message: "base")
      try repo.git("switch", "-q", "-c", "topic")
      try repo.commit(["t.txt": "t\n"], message: "topic work")
      try repo.git("switch", "-q", "main")
      try repo.commit(["m.txt": "m\n"], message: "main work")

      // Rebase topic onto main: its commit now sits on main's.
      try repo.git("switch", "-q", "topic")
      _ = try GitOperations.rebase(onto: "main", root: repo.root).get()
      #expect(try repo.git("log", "--format=%s", "-3") == "topic work\nmain work\nbase")

      // Merge it back: a fast-forward, or a merge commit with --no-ff.
      try repo.git("switch", "-q", "main")
      try repo.git("branch", "-q", "topic-copy", "topic")
      _ = try GitOperations.merge("topic", root: repo.root).get()
      #expect(try repo.git("rev-parse", "HEAD") == (try repo.git("rev-parse", "topic")))
      try repo.git("reset", "-q", "--hard", "HEAD~1")
      _ = try GitOperations.merge("topic-copy", noFastForward: true, root: repo.root).get()
      #expect(try repo.git("rev-list", "--merges", "--count", "HEAD") == "1")

      // A conflicting merge stops with the merge open.
      try repo.git("switch", "-q", "-c", "clash", "HEAD~1")
      try repo.commit(["m.txt": "clash\n"], message: "clash")
      try repo.git("switch", "-q", "main")
      try repo.commit(["m.txt": "mine\n"], message: "mine")
      guard case .failure(.cli(let error)) = GitOperations.merge("clash", root: repo.root) else {
        Issue.record("expected a conflict")
        return
      }
      #expect(error.kind == .mergeConflict)
      #expect(GitClient.snapshot(forPath: repo.root)?.operation == .merge)
    }
  }
#endif
