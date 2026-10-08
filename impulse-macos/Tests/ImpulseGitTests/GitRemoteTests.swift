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

    @Test func aPushCarriesTheServersMessage() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      defer { [repo, origin, seed].forEach { $0.destroy() } }
      // A server hook prints what hosts print: git relays it as "remote:" lines.
      let hook = origin.root + "/.git/hooks/post-receive"
      try FileManager.default.createDirectory(
        atPath: (hook as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try """
        #!/bin/sh
        echo "To create a merge request for topic, visit:"
        echo "  https://git.example.edu/web/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=topic"
        """.write(toFile: hook, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: hook)
      try repo.git("switch", "-q", "-c", "topic")
      try repo.commit(["b.txt": "2\n"], message: "topic")

      var lines: [String] = []
      _ = try GitOperations.push(setUpstream: true, remote: "origin", branch: "topic", root: repo.root) {
        lines.append($0)
      }.get()
      let message = try #require(GitServerMessage.parse(lines))
      #expect(message.linkCaption == "To create a merge request for topic, visit:")
      #expect(message.link?.host == "git.example.edu")
    }

    @Test func newWorktreeBranchDoesNotTrackARemoteBase() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      let path = repo.root + "-wt-task"
      defer {
        [repo, origin, seed].forEach { $0.destroy() }
        try? FileManager.default.removeItem(atPath: path)
      }
      _ = try GitOperations.addWorktree(
        path: path, branch: "task", newBranch: true, base: "origin/main", root: repo.root
      ).get()
      #expect(try repo.git("rev-parse", "task") == repo.git("rev-parse", "origin/main"))
      #expect(throws: (any Error).self) { try repo.git("rev-parse", "--abbrev-ref", "task@{upstream}") }
    }

    @Test func aRemoteBranchOpenedAsATaskTracksIt() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      let path = repo.root + "-wt-shared"
      defer {
        [repo, origin, seed].forEach { $0.destroy() }
        try? FileManager.default.removeItem(atPath: path)
      }
      try seed.git("switch", "-q", "-c", "shared")
      try seed.commit(["s.txt": "1\n"], message: "shared")
      try seed.git("push", "-q", "origin", "shared")
      try repo.git("fetch", "-q", "origin")
      _ = try GitOperations.addWorktree(
        path: path, branch: "shared", newBranch: true, base: "origin/shared", track: true, root: repo.root
      ).get()
      #expect(try repo.git("rev-parse", "--abbrev-ref", "shared@{upstream}") == "origin/shared")
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

    @Test func takingASideCanBeReopened() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["m.txt": "base\n"], message: "base")
      try repo.git("switch", "-q", "-c", "clash")
      try repo.commit(["m.txt": "theirs\n"], message: "theirs")
      try repo.git("switch", "-q", "main")
      try repo.commit(["m.txt": "ours\n"], message: "ours")
      guard case .failure = GitOperations.merge("clash", root: repo.root) else {
        Issue.record("expected a conflict")
        return
      }
      let stages = try repo.git("ls-files", "-u", "--", "m.txt")
      // Half-resolved by hand before taking a side.
      let edited = try repo.read("m.txt").replacingOccurrences(of: "ours\n", with: "ours, edited\n")
      try repo.write("m.txt", edited)
      let snapshot = try SafetySnapshots.create(reason: "take incoming side", root: repo.root).get()
      #expect(snapshot.indexTree == nil, "no index tree while files are in conflict")

      let point = GitOperations.conflictPoint(root: repo.root)
      #expect(point.operation == .merge)
      _ = try GitOperations.resolveConflicts(["m.txt"], takeOurs: false, root: repo.root).get()
      #expect(try repo.read("m.txt") == "theirs\n")
      #expect(try repo.git("ls-files", "-u").isEmpty, "resolved and staged")

      // Undo: the conflict is back in the index, markers in the file.
      _ = try GitOperations.reopenConflicts(["m.txt"], at: point, root: repo.root).get()
      #expect(try repo.git("ls-files", "-u", "--", "m.txt") == stages)
      #expect(try repo.read("m.txt").contains("<<<<<<<"))
      #expect(GitClient.snapshot(forPath: repo.root)?.conflicted.map(\.path) == ["m.txt"])
      // …and the snapshot brings back the hand edits.
      _ = try SafetySnapshots.restore(snapshot, paths: ["m.txt"], root: repo.root).get()
      #expect(try repo.read("m.txt") == edited)
      #expect(try repo.git("ls-files", "-u", "--", "m.txt") == stages)
    }

    @Test func reopeningIsRefusedOnceTheOperationMovedOn() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["m.txt": "base\n", "n.txt": "base\n"], message: "base")
      try repo.git("switch", "-q", "-c", "clash")
      try repo.commit(["m.txt": "theirs\n"], message: "theirs m")
      try repo.commit(["n.txt": "theirs\n"], message: "theirs n")
      try repo.git("switch", "-q", "main")
      try repo.commit(["m.txt": "ours\n", "n.txt": "ours\n"], message: "ours")
      func resolve(_ paths: [String], after start: () -> GitResult) throws -> GitOperations.ConflictPoint {
        guard case .failure = start() else { throw GitOperationError.invalid("expected a conflict") }
        let point = GitOperations.conflictPoint(root: repo.root)
        _ = try GitOperations.resolveConflicts(paths, takeOurs: false, root: repo.root).get()
        return point
      }
      func expectRefused(_ paths: [String], at point: GitOperations.ConflictPoint, _ phrase: String) throws {
        let result = GitOperations.reopenConflicts(paths, at: point, root: repo.root)
        guard case .failure(.stale(let message)) = result else {
          Issue.record("expected a refusal, got \(result)")
          return
        }
        #expect(message.contains(phrase), "\(message)")
        #expect(try repo.git(["ls-files", "-u", "--"] + paths).isEmpty, "the conflict stayed resolved")
      }

      // The merge was committed.
      var point = try resolve(["m.txt", "n.txt"]) { GitOperations.merge("clash", root: repo.root) }
      #expect(point.operation == .merge)
      try repo.git("commit", "-q", "--no-edit")
      try expectRefused(["m.txt", "n.txt"], at: point, "merge is no longer in progress")

      // The merge was aborted (HEAD didn't move).
      try repo.git("reset", "-q", "--hard", "HEAD~1")
      point = try resolve(["m.txt", "n.txt"]) { GitOperations.merge("clash", root: repo.root) }
      try repo.git("merge", "--abort")
      try expectRefused(["m.txt", "n.txt"], at: point, "merge is no longer in progress")

      // The rebase went on to its next step, which stopped on a conflict too.
      try repo.git("switch", "-q", "clash")
      point = try resolve(["m.txt"]) { GitOperations.rebase(onto: "main", root: repo.root) }
      _ = try? repo.git("-c", "core.editor=true", "rebase", "--continue")
      #expect(GitClient.currentOperation(repoPath: repo.root) == .rebase(step: 2, total: 2))
      try expectRefused(["m.txt"], at: point, "rebase has moved on")
    }

    @Test func defaultRemoteFollowsTheBranch() throws {
      let (repo, origin, seed) = try cloneWithOrigin()
      defer { [repo, origin, seed].forEach { $0.destroy() } }
      try repo.git("remote", "add", "fork", origin.root + "/.git")
      try repo.git("branch", "-q", "topic")
      try repo.git("config", "branch.topic.remote", "fork")
      try repo.git("branch", "-q", "loose")

      #expect(GitOperations.defaultRemote(root: repo.root) == "origin", "main tracks origin")
      #expect(GitOperations.defaultRemote(root: repo.root, branch: "topic") == "fork")
      #expect(GitOperations.defaultRemote(root: repo.root, branch: "loose") == "origin", "no remote of its own")
      try repo.git("switch", "-q", "topic")
      #expect(GitOperations.defaultRemote(root: repo.root) == "fork", "the checked-out branch's")

      // All at once, the same answers.
      try repo.git("branch", "-q", "release.1.2")
      try repo.git("config", "branch.release.1.2.remote", "fork")
      try repo.git("branch", "-q", "orphaned")
      try repo.git("config", "branch.orphaned.remote", "gone")
      let names = ["main", "topic", "loose", "release.1.2", "orphaned"]
      let all = GitOperations.defaultRemotes(root: repo.root, branches: names)
      #expect(all == ["main": "origin", "topic": "fork", "loose": "origin", "release.1.2": "fork", "orphaned": "origin"])
      for name in names {
        #expect(all[name] == GitOperations.defaultRemote(root: repo.root, branch: name), "\(name)")
      }
    }
  }
#endif
