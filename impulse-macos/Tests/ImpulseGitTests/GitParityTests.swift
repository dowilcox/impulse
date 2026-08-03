// Parity tests for the ImpulseGit port, asserted against golden fixtures
// generated from the Rust implementation (impulse-core/src/git.rs +
// filesystem.rs). The scenario repo is rebuilt with the git CLI under a pinned
// author/committer/date environment so results are fully deterministic.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  struct GitParityTests {
    private func expectMatchesFixture<T: Encodable>(
      _ value: T, fixture: String, root: String,
      sourceLocation: SourceLocation = #_sourceLocation
    ) throws {
      let actual = JSONCompare.normalize(try JSONCompare.jsonObject(value), root: root)
      let expected = try Fixtures.json(fixture)
      let difference = JSONCompare.firstDifference(actual: actual, expected: expected)
      #expect(
        difference == nil, "\(fixture): \(difference ?? "")", sourceLocation: sourceLocation)
    }

    @Test func changesetMatchesFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let changeSet = try #require(GitClient.changedFiles(repoPath: repo.root))
      try expectMatchesFixture(changeSet, fixture: "changeset.json", root: repo.root)
    }

    @Test func hunksMatchFixtures() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let cases: [(file: String, fixture: String)] = [
        ("src/sample.rs", "hunks_src__sample.rs.json"),
        ("notes.txt", "hunks_notes.txt.json"),
        ("extra.txt", "hunks_extra.txt.json"),
      ]
      for testCase in cases {
        let hunks = try #require(
          GitClient.fileHunks(repoPath: repo.root, filePath: testCase.file),
          "fileHunks(\(testCase.file)) returned nil")
        try expectMatchesFixture(hunks, fixture: testCase.fixture, root: repo.root)
      }
    }

    @Test func diffMarkersMatchFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let markers = try #require(
        GitClient.diffMarkers(filePath: repo.root + "/src/sample.rs"))
      try expectMatchesFixture(
        markers, fixture: "markers_src__sample.rs.json", root: repo.root)
    }

    @Test func blameMatchesFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let blame = try #require(GitClient.lineBlame(filePath: repo.root + "/keep.md", line: 1))
      try expectMatchesFixture(blame, fixture: "blame_keep.md.json", root: repo.root)
    }

    @Test func branchesMatchFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let actual: [String: Any] = [
        "branch": GitClient.branch(forPath: repo.root) as Any,
        "branches": GitClient.branches(forPath: repo.root),
      ]
      let normalized = JSONCompare.normalize(actual, root: repo.root)
      let expected = try Fixtures.json("branches.json")
      let difference = JSONCompare.firstDifference(actual: normalized, expected: expected)
      #expect(difference == nil, "branches.json: \(difference ?? "")")
    }

    @Test func statusForRootDirectoryMatchesFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let status = try #require(GitClient.statusForDirectory(repo.root))
      try expectMatchesFixture(status, fixture: "status_root_dir.json", root: repo.root)
    }

    @Test func allStatusesMatchFixture() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let statuses = try #require(GitClient.allStatuses(root: repo.root))
      try expectMatchesFixture(statuses, fixture: "status_all.json", root: repo.root)
    }
  }

  struct GitClientUnitTests {
    @Test func repoRootDiscoveryAndCache() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }

      // Discovery from a nested path resolves to the repo root, twice (the
      // second call is served from the LRU cache).
      let first = GitClient.repoRoot(forPath: repo.root + "/src")
      let second = GitClient.repoRoot(forPath: repo.root + "/src")
      #expect(first == repo.root)
      #expect(second == repo.root)

      // File paths are looked up via their parent directory.
      #expect(GitClient.repoRoot(forPath: repo.root + "/src/sample.rs") == repo.root)

      // A non-repo path yields nil.
      #expect(GitClient.repoRoot(forPath: "/") == nil)
    }

    @Test func repoCacheEvictsLeastRecentlyUsed() {
      let cache = RepoCache(capacity: 2)
      cache.store(root: "/r1", forDirectory: "/d1")
      cache.store(root: "/r2", forDirectory: "/d2")
      // Touch d1 so d2 becomes least-recently used.
      #expect(cache.root(forDirectory: "/d1") == "/r1")
      cache.store(root: "/r3", forDirectory: "/d3")
      #expect(cache.count == 2)
      #expect(cache.root(forDirectory: "/d2") == nil)
      #expect(cache.root(forDirectory: "/d1") == "/r1")
      #expect(cache.root(forDirectory: "/d3") == "/r3")
    }

    @Test func discardPathDeletesUntrackedFile() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      let untracked = repo.root + "/extra.txt"
      #expect(FileManager.default.fileExists(atPath: untracked))

      try GitClient.discardPath(repoPath: repo.root, filePath: "extra.txt")

      #expect(!FileManager.default.fileExists(atPath: untracked))
    }

    @Test func discardPathRestoresTrackedFiles() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }

      // Modified file is restored to its committed content.
      try GitClient.discardPath(repoPath: repo.root, filePath: "src/sample.rs")
      let restored = try String(
        contentsOfFile: repo.root + "/src/sample.rs", encoding: .utf8)
      #expect(restored.contains("let message = \"hello world\";"))
      #expect(!restored.contains("swift"))

      // Deleted file reappears.
      try GitClient.discardPath(repoPath: repo.root, filePath: "notes.txt")
      let notes = try String(contentsOfFile: repo.root + "/notes.txt", encoding: .utf8)
      #expect(notes == "alpha\nbeta\ngamma\n")
    }

    @Test func commitAllProducesCommitAndCleanTree() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }

      let result = GitClient.commitAll(repoPath: repo.root, message: "port to swift")
      switch result {
      case .success(let oid):
        #expect(oid.count == 40)
      case .failure(let error):
        Issue.record("commitAll failed: \(error.message)")
      }

      // The change set and directory status are clean afterwards.
      let changeSet = try #require(GitClient.changedFiles(repoPath: repo.root))
      #expect(changeSet.files.isEmpty)
      #expect(GitClient.statusForDirectory(repo.root) == [:])

      // A second commit with nothing changed is refused.
      let empty = GitClient.commitAll(repoPath: repo.root, message: "again")
      guard case .failure(let error) = empty else {
        Issue.record("expected 'nothing to commit' failure")
        return
      }
      #expect(error.message == "nothing to commit")
    }

    @Test func commitAllRejectsEmptyMessage() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }
      guard case .failure(let error) = GitClient.commitAll(repoPath: repo.root, message: "   ")
      else {
        Issue.record("expected empty-message failure")
        return
      }
      #expect(error.message == "Commit message is empty")
    }

    @Test func pathValidationRejectsEscapes() throws {
      let repo = try ScenarioRepo.create()
      defer { repo.destroy() }

      // Lexical validation rejects any `..` component and absolute paths.
      #expect(throws: GitError.self) {
        try GitClient.validateRelPathLexically(root: repo.root, rel: "../escape.txt")
      }
      #expect(throws: GitError.self) {
        try GitClient.validateRelPathLexically(root: repo.root, rel: "a/../../escape.txt")
      }
      #expect(throws: GitError.self) {
        try GitClient.validateRelPathLexically(root: repo.root, rel: "/etc/passwd")
      }
      let joined = try GitClient.validateRelPathLexically(root: repo.root, rel: "src/./sample.rs")
      #expect(joined == repo.root + "/src/sample.rs")

      // Disk-based validation rejects paths that resolve outside the root.
      #expect(throws: GitError.self) {
        try GitClient.validatePathWithinRoot(repo.root + "/../", root: repo.root)
      }
      #expect(throws: GitError.self) {
        try GitClient.validatePathWithinRoot("/etc/passwd", root: repo.root)
      }
      let inside = try GitClient.validatePathWithinRoot(
        repo.root + "/keep.md", root: repo.root)
      #expect(inside == repo.root + "/keep.md")

      // discardPath refuses traversal attempts.
      #expect(throws: GitError.self) {
        try GitClient.discardPath(repoPath: repo.root, filePath: "../escape.txt")
      }
    }
  }
#endif
