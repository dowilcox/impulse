#if canImport(Testing)
  import Foundation
  import ImpulseKit
  import Testing

  @testable import ImpulseGit

  /// Repository states that trip up git tooling (renames, binaries,
  /// conflicts mid-rebase, worktrees, submodules, detached and unborn HEAD,
  /// ignored files, CRLF, Unicode paths), each checked against
  /// `git status --porcelain -z` as the oracle. The golden-fixture scenario
  /// in ScenarioRepo.swift stays as it is.
  @Suite(.serialized)
  struct GitScenarioTests {
    init() {
      GitOperations.environment = TempRepo.gitOverrides
    }

    /// What `git status` says, by section.
    struct Oracle: Equatable {
      var staged: Set<String> = []
      var unstaged: Set<String> = []
      var untracked: Set<String> = []
      var conflicted: Set<String> = []
    }

    private func oracle(_ repo: TempRepo, at root: String? = nil) throws -> Oracle {
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
      process.arguments = ["git", "-C", root ?? repo.root, "status", "--porcelain=v1", "-z", "--untracked-files=all"]
      process.environment = TempRepo.environment
      let out = Pipe()
      process.standardOutput = out
      process.standardError = FileHandle.nullDevice
      try process.run()
      let data = out.fileHandleForReading.readDataToEndOfFile()
      process.waitUntilExit()
      let fields = data.split(separator: 0, omittingEmptySubsequences: false).map {
        String(decoding: $0, as: UTF8.self)
      }
      var result = Oracle()
      var index = 0
      let conflictCodes: Set<String> = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"]
      while index < fields.count {
        let field = fields[index]
        index += 1
        guard field.count > 3 else { continue }
        let code = String(field.prefix(2))
        let path = String(field.dropFirst(3))
        let x = code.first!
        let y = code.last!
        if code.first == "R" || code.first == "C" { index += 1 }  // the old path follows
        if code == "??" {
          result.untracked.insert(path)
        } else if conflictCodes.contains(code) {
          result.conflicted.insert(path)
        } else {
          if x != " " { result.staged.insert(path) }
          if y != " " { result.unstaged.insert(path) }
        }
      }
      return result
    }

    private func sections(_ snap: RepoSnapshot) -> Oracle {
      Oracle(
        staged: Set(snap.staged.map(\.path)), unstaged: Set(snap.unstaged.map(\.path)),
        untracked: Set(snap.untracked.map(\.path)), conflicted: Set(snap.conflicted.map(\.path)))
    }

    private func expectMatchesGit(_ repo: TempRepo, at root: String? = nil) throws -> RepoSnapshot {
      let snap = try #require(GitClient.snapshot(forPath: root ?? repo.root))
      #expect(sections(snap) == (try oracle(repo, at: root)))
      return snap
    }

    private func writeData(_ repo: TempRepo, _ path: String, _ data: Data) throws {
      let url = URL(fileURLWithPath: repo.root).appendingPathComponent(path)
      try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
      try data.write(to: url)
    }

    // MARK: Index and working tree

    @Test func stagedOnly() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.write("a.txt", "two\n")
      try repo.git("add", "a.txt")
      let snap = try expectMatchesGit(repo)
      #expect(snap.unstaged.isEmpty)
      #expect(snap.staged.first?.added == 1 && snap.staged.first?.removed == 1)
    }

    @Test func stagedAndUnstagedInOneFile() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n2\n3\n"])
      try repo.write("a.txt", "1\nTWO\n3\n")
      try repo.git("add", "a.txt")
      try repo.write("a.txt", "1\nTWO\n3\n4\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.staged.map(\.path) == ["a.txt"] && snap.unstaged.map(\.path) == ["a.txt"])
      #expect(snap.unstaged.first?.added == 1 && snap.unstaged.first?.removed == 0)
    }

    @Test func renamePlusEdit() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      let body = (1...20).map { "line \($0)" }.joined(separator: "\n") + "\n"
      try repo.commit(["old.txt": body])
      try repo.git("mv", "old.txt", "new.txt")
      try repo.write("new.txt", body + "one more\n")
      try repo.git("add", "new.txt")
      let snap = try expectMatchesGit(repo)
      let renamed = try #require(snap.staged.first)
      #expect(renamed.status == .renamed)
      #expect(renamed.path == "new.txt" && renamed.oldPath == "old.txt")
      #expect(renamed.added == 1 && renamed.removed == 0)
    }

    @Test func binaryAndTooLarge() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try writeData(repo, "image.bin", Data([0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02]))
      try repo.write("big.txt", String(repeating: "x\n", count: 10))
      try repo.git("add", "-A")
      try repo.git("commit", "-q", "-m", "files")
      try writeData(repo, "image.bin", Data([0x89, 0x50, 0x4E, 0x47, 0x00, 0x09]))
      // Over the 1 MB diff limit: listed, without line counts.
      try repo.write("big.txt", String(repeating: "y\n", count: 600_000))
      let snap = try expectMatchesGit(repo)
      let binary = try #require(snap.unstaged.first { $0.path == "image.bin" })
      #expect(binary.isBinary && binary.added == nil)
      let big = try #require(snap.unstaged.first { $0.path == "big.txt" })
      #expect(big.added == nil && big.removed == nil)
    }

    @Test func ignoredFilesStayOut() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit([".gitignore": "build/\n*.log\n"])
      try repo.write("build/out.o", "obj")
      try repo.write("debug.log", "log")
      try repo.write("src/keep.swift", "let x = 1\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.untracked.map(\.path) == ["src/keep.swift"])
    }

    @Test func crlfFiles() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.git("config", "core.autocrlf", "false")
      try repo.commit(["dos.txt": "one\r\ntwo\r\nthree\r\n"])
      try repo.write("dos.txt", "one\r\nTWO\r\nthree\r\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.unstaged.first?.added == 1 && snap.unstaged.first?.removed == 1)

      // With autocrlf on, a CRLF working copy of an LF blob differs only in
      // line endings.
      let converted = try TempRepo.create()
      defer { converted.destroy() }
      try converted.git("config", "core.autocrlf", "true")
      try converted.commit(["unix.txt": "a\nb\n"])
      try converted.write("unix.txt", "a\r\nb\r\n")
      // git lists it as modified (the file was rewritten) with no changed lines.
      let rewritten = try expectMatchesGit(converted)
      #expect(rewritten.unstaged.first?.added == 0 && rewritten.unstaged.first?.removed == 0)
    }

    @Test func unicodeAndSpacesInPaths() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["naïve résumé.txt": "v1\n", "日本/メモ.md": "a\n"])
      try repo.write("naïve résumé.txt", "v2\n")
      try repo.write("日本/メモ.md", "b\n")
      try repo.write("new folder/ünïcode.txt", "x\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.unstaged.count == 2)
      #expect(snap.untracked.map(\.path) == ["new folder/ünïcode.txt"])
    }

    // MARK: HEAD states

    @Test func detachedHead() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      try repo.commit(["a.txt": "2\n"])
      let first = try repo.git("rev-parse", "HEAD~1")
      try repo.git("checkout", "-q", "--detach", first)
      try repo.write("a.txt", "changed\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.isDetached && snap.branch == nil)
      #expect(snap.headOid == first)
    }

    @Test func unbornHeadWithStagedFiles() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.write("first.txt", "hello\n")
      try repo.git("add", "first.txt")
      try repo.write("second.txt", "later\n")
      let snap = try expectMatchesGit(repo)
      #expect(snap.isUnborn && snap.headOid == nil)
      #expect(snap.staged.first?.status == .added)
    }

    // MARK: Operations

    @Test func mergeConflict() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "base\n", "b.txt": "b\n"])
      try repo.git("switch", "-q", "-c", "other")
      try repo.commit(["a.txt": "other\n", "c.txt": "new on other\n"])
      try repo.git("switch", "-q", "main")
      try repo.commit(["a.txt": "main\n"])
      _ = try? repo.git("merge", "-q", "other")
      let snap = try expectMatchesGit(repo)
      #expect(snap.operation == .merge)
      #expect(snap.conflicted.map(\.path) == ["a.txt"])
      #expect(snap.staged.map(\.path) == ["c.txt"], "the clean part of the merge is staged")
    }

    @Test func rebaseStoppedAtTheSecondOfThree() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "base\n"])
      try repo.git("switch", "-q", "-c", "topic")
      try repo.commit(["one.txt": "1\n"], message: "one")
      try repo.commit(["a.txt": "topic\n"], message: "two")
      try repo.commit(["three.txt": "3\n"], message: "three")
      try repo.git("switch", "-q", "main")
      try repo.commit(["a.txt": "main\n"], message: "main")
      try repo.git("switch", "-q", "topic")
      _ = try? repo.git("rebase", "main")
      let snap = try expectMatchesGit(repo)
      #expect(snap.operation == .rebase(step: 2, total: 3))
      #expect(snap.conflicted.map(\.path) == ["a.txt"])
      #expect(snap.isDetached, "HEAD is detached during a rebase")
    }

    // MARK: Worktrees and submodules

    @Test func linkedWorktree() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "1\n"])
      let worktree = repo.root + "-wt"
      defer { try? FileManager.default.removeItem(atPath: worktree) }
      try repo.git("worktree", "add", "-q", "-b", "feature", worktree)
      try "changed\n".write(toFile: worktree + "/a.txt", atomically: true, encoding: .utf8)
      try "new\n".write(toFile: worktree + "/b.txt", atomically: true, encoding: .utf8)

      let snap = try expectMatchesGit(repo, at: worktree)
      #expect(snap.root == worktree)
      #expect(snap.branch == "feature")
      #expect(snap.commonDir == repo.root + "/.git")
      // The main checkout is untouched.
      #expect(!(try expectMatchesGit(repo)).hasChanges)
    }

    @Test func submoduleWithNewCommits() throws {
      let library = try TempRepo.create()
      defer { library.destroy() }
      try library.commit(["lib.txt": "v1\n"])
      let app = try TempRepo.create()
      defer { app.destroy() }
      try app.commit(["app.txt": "app\n"])
      try app.git("-c", "protocol.file.allow=always", "submodule", "add", "-q", library.root, "vendor/lib")
      try app.git("commit", "-q", "-m", "add submodule")

      // New commit inside the submodule's checkout.
      let submodule = app.root + "/vendor/lib"
      let process = Process()
      process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
      process.arguments = ["git", "-C", submodule, "commit", "-q", "--allow-empty", "-m", "bump"]
      process.environment = TempRepo.environment
      try process.run()
      process.waitUntilExit()

      let snap = try expectMatchesGit(app)
      #expect(snap.unstaged.map(\.path) == ["vendor/lib"])
    }
  }
#endif
