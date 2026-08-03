// Tests for FileSearch (port of impulse-core/src/search.rs): gitignore
// semantics via libgit2 validated against `git check-ignore`-style
// expectations, binary/hidden skipping, substring matching with
// character-based columns.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseGit

  struct FileSearchTests {
    /// Builds a git repo with nested .gitignore files, negations, hidden and
    /// binary files.
    private func makeTree() throws -> URL {
      let fm = FileManager.default
      let root = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("impulse-search-\(UUID().uuidString.prefix(8))")
      try fm.createDirectory(at: root, withIntermediateDirectories: true)

      func run(_ args: [String]) throws {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        p.arguments = ["git"] + args
        p.currentDirectoryURL = root
        var env = ProcessInfo.processInfo.environment
        env["GIT_CONFIG_GLOBAL"] = "/dev/null"
        env["GIT_CONFIG_SYSTEM"] = "/dev/null"
        p.environment = env
        p.standardOutput = Pipe()
        p.standardError = Pipe()
        try p.run()
        p.waitUntilExit()
      }
      func write(_ rel: String, _ content: String) throws {
        let url = root.appendingPathComponent(rel)
        try fm.createDirectory(
          at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try content.write(to: url, atomically: true, encoding: .utf8)
      }

      try run(["init", "-q", "-b", "main"])
      try write(".gitignore", "build/\n*.log\n!keep.log\n")
      try write("src/needle_main.rs", "fn main() { let needle = 1; }\n")
      try write("src/.gitignore", "generated.rs\n")
      try write("src/generated.rs", "// needle generated\n")
      try write("build/needle_out.txt", "needle output\n")
      try write("debug.log", "needle in log\n")
      try write("keep.log", "needle kept\n")
      try write(".hidden/needle_hidden.txt", "needle hidden\n")
      try write("notes.txt", "Needle here and nEEdle there\n")
      // Binary file containing the query bytes plus a NUL.
      let binary = Data("needle".utf8) + Data([0x00, 0x01, 0x02])
      try binary.write(to: root.appendingPathComponent("blob.bin"))
      return root.resolvingSymlinksInPath()
    }

    @Test func filenameSearchHonorsGitignoreAndNegations() throws {
      let root = try makeTree()
      defer { try? FileManager.default.removeItem(at: root) }

      let names = FileSearch.searchFilenames(root: root.path, query: "needle", limit: 100)
        .map(\.name).sorted()
      // build/needle_out.txt ignored (build/), needle_hidden hidden,
      // generated.rs excluded by nested .gitignore (name doesn't match query
      // anyway), keep.log un-ignored by negation but name doesn't match.
      #expect(names == ["needle_main.rs"])
    }

    @Test func contentSearchHonorsIgnoreHiddenAndBinary() throws {
      let root = try makeTree()
      defer { try? FileManager.default.removeItem(at: root) }

      let results = FileSearch.searchContents(
        root: root.path, query: "needle", limit: 100, caseSensitive: true)
      let files = Set(results.map(\.name))
      #expect(files.contains("needle_main.rs"))
      #expect(files.contains("keep.log"), "negated ignore pattern should be searchable")
      #expect(!files.contains("generated.rs"), "nested .gitignore must apply")
      #expect(!files.contains("needle_out.txt"), "build/ must be ignored")
      #expect(!files.contains("debug.log"), "*.log must be ignored")
      #expect(!files.contains("needle_hidden.txt"), "hidden dirs are skipped")
      #expect(!files.contains("blob.bin"), "binary files are skipped")
      #expect(!files.contains(".gitignore"), "hidden files are skipped")
    }

    @Test func caseInsensitiveColumnsAreCharacterBased() throws {
      let root = try makeTree()
      defer { try? FileManager.default.removeItem(at: root) }
      // "Needle here and nEEdle there" — two case-insensitive matches.
      let results = FileSearch.searchContents(
        root: root.path, query: "needle", limit: 100, caseSensitive: false
      ).filter { $0.name == "notes.txt" }
      #expect(results.count == 2)
      #expect(results[0].columnStart == 0)
      #expect(results[0].columnEnd == 6)
      #expect(results[1].columnStart == 16)
      #expect(results[1].columnEnd == 22)
      #expect(results[0].lineContent == "Needle here and nEEdle there")
    }

    @Test func unicodeColumnsCountCharactersNotBytes() throws {
      let fm = FileManager.default
      let root = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("impulse-search-uni-\(UUID().uuidString.prefix(8))")
      try fm.createDirectory(at: root, withIntermediateDirectories: true)
      defer { try? fm.removeItem(at: root) }
      try "héllo wörld target\n".write(
        to: root.appendingPathComponent("uni.txt"), atomically: true, encoding: .utf8)

      let results = FileSearch.searchContents(
        root: root.resolvingSymlinksInPath().path, query: "target", limit: 10,
        caseSensitive: true)
      #expect(results.count == 1)
      // "héllo wörld " is 12 characters (14 UTF-8 bytes).
      #expect(results[0].columnStart == 12)
      #expect(results[0].columnEnd == 18)
    }

    @Test func limitStopsEarly() throws {
      let root = try makeTree()
      defer { try? FileManager.default.removeItem(at: root) }
      let results = FileSearch.searchContents(
        root: root.path, query: "needle", limit: 1, caseSensitive: false)
      #expect(results.count == 1)
    }
  }
#endif
