#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  struct GitCLITests {
    @Test func classifiesCommonFailures() {
      let cases: [(String, [String], GitCLIError.Kind)] = [
        ("fatal: not a git repository (or any of the parent directories): .git", ["status"], .notARepository),
        ("*** Please tell me who you are.\n\nRun\n  git config --global user.email", ["commit"], .identityNotConfigured),
        ("fatal: Unable to create '/r/.git/index.lock': File exists.", ["add"], .indexLocked),
        ("error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt", ["switch", "x"], .localChangesWouldBeOverwritten),
        (" ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs", ["push"], .nonFastForward),
        ("fatal: The current branch topic has no upstream branch.", ["push"], .noUpstream),
        ("fatal: could not read Username for 'https://github.com': terminal prompts disabled", ["fetch"], .authenticationFailed),
        ("git@github.com: Permission denied (publickey).", ["fetch"], .authenticationFailed),
        ("CONFLICT (content): Merge conflict in a.txt", ["merge", "x"], .mergeConflict),
        ("nothing to commit, working tree clean", ["commit"], .nothingToCommit),
        ("fatal: a branch named 'x' already exists", ["branch", "x"], .branchAlreadyExists),
        ("fatal: invalid reference: nope", ["switch", "nope"], .unknownRevision),
        ("error: pathspec 'nope' did not match any file(s) known to git", ["checkout", "nope"], .unknownRevision),
        ("husky - pre-commit hook exited with code 1 (error)", ["commit", "-F", "-"], .hookFailed),
        ("something unexpected", ["status"], .other),
      ]
      for (output, arguments, expected) in cases {
        #expect(GitCLI.classify(output: output, arguments: arguments) == expected, "\(output)")
      }
    }

    @Test func runsGitAndReturnsOutput() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"], message: "first")

      let result = GitCLI.run(
        ["log", "--format=%s"], in: repo.root, environment: TempRepo.gitOverrides)
      let output = try result.get()
      #expect(output.status == 0)
      #expect(output.stdout.trimmingCharacters(in: .whitespacesAndNewlines) == "first")
    }

    @Test func switchingToUnknownBranchIsClassified() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])

      let result = GitCLI.run(
        ["switch", "does-not-exist"], in: repo.root, environment: TempRepo.gitOverrides)
      guard case .failure(let error) = result else {
        Issue.record("expected failure")
        return
      }
      #expect(error.kind == .unknownRevision)
      #expect(!error.message.isEmpty)
    }

    @Test func passesStdinToGit() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.write("a.txt", "two\n")
      try repo.git("add", "a.txt")

      let message = "Subject line\n\nBody paragraph.\n"
      _ = try GitCLI.run(
        ["commit", "-q", "-F", "-"], in: repo.root, stdin: Data(message.utf8),
        environment: TempRepo.gitOverrides
      ).get()
      #expect(try repo.git("log", "-1", "--format=%B") == "Subject line\n\nBody paragraph.")
    }

    @Test func dirtyTreeSwitchIsClassified() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      try repo.git("branch", "other")
      try repo.git("switch", "-q", "other")
      try repo.commit(["a.txt": "other\n"])
      try repo.git("switch", "-q", "main")
      try repo.write("a.txt", "local edit\n")

      let result = GitCLI.run(["switch", "other"], in: repo.root, environment: TempRepo.gitOverrides)
      guard case .failure(let error) = result else {
        Issue.record("expected failure")
        return
      }
      #expect(error.kind == .localChangesWouldBeOverwritten)
    }

    @Test func streamsProgressLines() throws {
      let repo = try TempRepo.create()
      defer { repo.destroy() }
      try repo.commit(["a.txt": "one\n"])
      var lines: [String] = []
      let lock = NSLock()
      _ = GitCLI.run(
        ["rev-parse", "--verify", "nope"], in: repo.root, environment: TempRepo.gitOverrides
      ) { line in
        lock.lock()
        lines.append(line)
        lock.unlock()
      }
      #expect(lines.contains { $0.contains("fatal") || $0.contains("Needed a single revision") })
    }
  }
#endif
