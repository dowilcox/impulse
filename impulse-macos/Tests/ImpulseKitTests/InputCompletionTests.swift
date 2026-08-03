// Ported from the Rust unit tests in impulse-core/src/completion.rs.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct InputCompletionTests {
    @Test func historyContinuationWins() {
      let hist = ["git status", "cargo build"]
      #expect(InputCompletion.complete(input: "git st", cwd: nil, history: hist) == "git status")
      // Trailing space continues the most recent matching command.
      #expect(InputCompletion.complete(input: "cargo ", cwd: nil, history: hist) == "cargo build")
    }

    @Test func completesCommandWordFromCommonList() {
      #expect(InputCompletion.complete(input: "gi", cwd: nil, history: []) == "git")
      #expect(InputCompletion.complete(input: "car", cwd: nil, history: []) == "cargo")
    }

    @Test func completesSubcommandForKnownCommand() {
      #expect(InputCompletion.complete(input: "git stat", cwd: nil, history: []) == "git status")
      #expect(InputCompletion.complete(input: "cargo bui", cwd: nil, history: []) == "cargo build")
    }

    @Test func completesFlags() {
      #expect(InputCompletion.complete(input: "ls --al", cwd: nil, history: []) == "ls --all")
      #expect(InputCompletion.complete(input: "git --me", cwd: nil, history: []) == "git --message")
      // Falls back to the common flag set for unknown commands.
      #expect(
        InputCompletion.complete(input: "frobnicate --he", cwd: nil, history: [])
          == "frobnicate --help")
    }

    @Test func completesFilesystemPaths() throws {
      let fm = FileManager.default
      let dir = fm.temporaryDirectory.appendingPathComponent("impulse-completion-test")
      try? fm.removeItem(at: dir)
      try fm.createDirectory(
        at: dir.appendingPathComponent("alpha"), withIntermediateDirectories: true)
      try Data("x".utf8).write(
        to: dir.appendingPathComponent("alpha").appendingPathComponent("inner.txt"))
      try Data("x".utf8).write(to: dir.appendingPathComponent("alpha-file.txt"))
      defer { try? fm.removeItem(at: dir) }

      let cwd = dir.path
      // `alpha` (dir) sorts before `alpha-file.txt`; directories get a slash.
      #expect(
        InputCompletion.complete(input: "cat alph", cwd: cwd, history: []) == "cat alpha/")
      // Within a subdirectory, the directory part is preserved verbatim.
      #expect(
        InputCompletion.complete(input: "cat alpha/inn", cwd: cwd, history: [])
          == "cat alpha/inner.txt")
    }

    @Test func noCompletionForEmptyOrBlankInput() {
      #expect(InputCompletion.complete(input: "", cwd: nil, history: []) == nil)
      #expect(InputCompletion.complete(input: "   ", cwd: nil, history: []) == nil)
    }

    @Test func skipsIncompleteQuotedTokens() {
      #expect(InputCompletion.complete(input: "cat \"unterminated", cwd: nil, history: []) == nil)
    }
  }

  // ---------------------------------------------------------------------------
  // Multi-candidate dropdown
  // ---------------------------------------------------------------------------

  struct InputCompletionCandidateTests {
    /// Build a fresh temp directory laid out like a WordPress project root.
    private func wpFixture(_ tag: String) throws -> URL {
      let fm = FileManager.default
      let dir = fm.temporaryDirectory.appendingPathComponent("impulse-candidates-\(tag)")
      try? fm.removeItem(at: dir)
      try fm.createDirectory(
        at: dir.appendingPathComponent("wp-content"), withIntermediateDirectories: true)
      try fm.createDirectory(
        at: dir.appendingPathComponent("wp-admin"), withIntermediateDirectories: true)
      try fm.createDirectory(
        at: dir.appendingPathComponent("wp-includes"), withIntermediateDirectories: true)
      try Data("x".utf8).write(to: dir.appendingPathComponent("readme.md"))
      return dir
    }

    @Test func candidatesListsMatchingDirsFirstAlphabetical() throws {
      let dir = try wpFixture("wp-prefix")
      defer { try? FileManager.default.removeItem(at: dir) }

      let result = InputCompletion.completeCandidates(
        input: "cd wp-", cwd: dir.path, history: [], limit: 50)
      let values = result.candidates.map(\.value)
      // Three directories, dirs-first, alphabetical, each value ends with `/`.
      #expect(values == ["wp-admin/", "wp-content/", "wp-includes/"])
      #expect(result.candidates.allSatisfy { $0.isDir })
      #expect(result.candidates.allSatisfy { $0.kind == "path" })
      #expect(result.candidates[0].display == "wp-admin")
      // The span covers exactly the `wp-` token.
      #expect(result.span == TextSpan(start: 3, end: 6))
    }

    @Test func candidatesEmptyPrefixListsAllEntries() throws {
      let dir = try wpFixture("empty-prefix")
      defer { try? FileManager.default.removeItem(at: dir) }

      let result = InputCompletion.completeCandidates(
        input: "cd ", cwd: dir.path, history: [], limit: 50)
      let values = result.candidates.map(\.value)
      // Dirs first (alphabetical), then files.
      #expect(values == ["wp-admin/", "wp-content/", "wp-includes/", "readme.md"])
    }

    @Test func candidatesHiddenOnlyWhenPrefixStartsWithDot() throws {
      let dir = try wpFixture("hidden")
      try FileManager.default.createDirectory(
        at: dir.appendingPathComponent(".git"), withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: dir) }

      // No leading dot: `.git` is filtered out.
      let plain = InputCompletion.completeCandidates(
        input: "cd wp-", cwd: dir.path, history: [], limit: 50)
      #expect(plain.candidates.allSatisfy { $0.display != ".git" })

      // Leading dot: `.git` surfaces.
      let dotted = InputCompletion.completeCandidates(
        input: "cd .", cwd: dir.path, history: [], limit: 50)
      #expect(dotted.candidates.contains { $0.display == ".git" })
    }

    @Test func candidatesEmptyForCommandWord() throws {
      let dir = try wpFixture("command-word")
      defer { try? FileManager.default.removeItem(at: dir) }

      // `gi` is the command word, not a path argument.
      let result = InputCompletion.completeCandidates(
        input: "gi", cwd: dir.path, history: [], limit: 50)
      #expect(result.candidates.isEmpty)
    }

    @Test func candidatesRespectLimit() throws {
      let dir = try wpFixture("limit")
      defer { try? FileManager.default.removeItem(at: dir) }

      let result = InputCompletion.completeCandidates(
        input: "cd wp-", cwd: dir.path, history: [], limit: 2)
      #expect(result.candidates.count == 2)
      // The cap keeps the dirs-first alphabetical ordering.
      let values = result.candidates.map(\.value)
      #expect(values == ["wp-admin/", "wp-content/"])
    }

    @Test func candidatesSerializeToContractJSON() throws {
      let dir = try wpFixture("serialize")
      defer { try? FileManager.default.removeItem(at: dir) }

      let result = InputCompletion.completeCandidates(
        input: "cd wp-c", cwd: dir.path, history: [], limit: 50)
      let encoded = try JSONEncoder().encode(result)
      let json = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
      let span = try #require(json["span"] as? [String: Any])
      #expect(span["start"] as? Int == 3)
      #expect(span["end"] as? Int == 7)
      let candidates = try #require(json["candidates"] as? [[String: Any]])
      let first = try #require(candidates.first)
      #expect(first["value"] as? String == "wp-content/")
      #expect(first["display"] as? String == "wp-content")
      #expect(first["kind"] as? String == "path")
      #expect(first["is_dir"] as? Bool == true)
      // serde emits an explicit `null`; JSONEncoder omits the key. Either
      // way the decoded value must be nil-equivalent.
      #expect(first["git_status"] == nil || first["git_status"] is NSNull)
    }
  }
#endif
