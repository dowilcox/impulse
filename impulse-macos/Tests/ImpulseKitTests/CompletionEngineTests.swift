#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct CompletionEngineTests {
    private func context(_ cwd: String? = nil, home: String = "/nonexistent") -> CompletionContext {
      CompletionContext(
        cwd: cwd, home: home, gitBranches: { ["main", "feature/search", "fix-login"] },
        gitRemotes: { ["origin", "upstream"] }, gitTags: { ["v1.0", "v1.1"] })
    }

    private func values(_ input: String, _ context: CompletionContext) -> [String] {
      CompletionEngine.candidates(input: input, context: context).candidates.map(\.value)
    }

    @Test func subcommandsWithDescriptions() {
      let result = CompletionEngine.candidates(input: "git ch", context: context())
      #expect(result.candidates.map(\.value) == ["checkout", "cherry-pick"])
      #expect(result.candidates[0].kind == "subcommand")
      #expect(result.candidates[0].detail == "Switch branches or restore files")
      #expect(result.span == TextSpan(start: 4, end: 6))
    }

    @Test func argumentsComeFromGenerators() {
      #expect(values("git checkout f", context()) == ["feature/search", "fix-login"])
      #expect(values("git push ", context()) == ["origin", "upstream"])
      #expect(values("git push origin m", context()) == ["main"], "second argument is a branch")
      #expect(values("git branch -D fe", context()) == ["feature/search"], "option arguments")
      #expect(values("git stash p", context()) == ["push", "pop"])
      #expect(values("git commit -m ", context()).isEmpty, "free-form option values")
      #expect(values("git switch -c new-thing ", context()).contains("main"), "then a start point")
    }

    @Test func optionsMatchTheSubcommand() {
      let result = CompletionEngine.candidates(input: "git commit --a", context: context())
      #expect(result.candidates.map(\.value) == ["--all", "--amend", "--allow-empty"])
      #expect(result.candidates.allSatisfy { $0.kind == "option" })
    }

    @Test func pipelinesStartAFreshCommand() {
      #expect(values("echo hi | git sta", context()) == ["status", "stash"])
      #expect(values("make && git rem", context()) == ["remote"])
    }

    @Test func commandWordsPreferKnownTools() {
      let names = values("gi", context())
      #expect(names.first == "git")
      #expect(CompletionEngine.candidates(input: "", context: context()).candidates.isEmpty)
    }

    @Test func projectFilesFeedScriptsTargetsAndRecipes() throws {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("completion-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: dir) }
      try #"{"scripts": {"dev": "vite", "build": "vite build", "test": "vitest"}}"#
        .write(to: dir.appendingPathComponent("package.json"), atomically: true, encoding: .utf8)
      try "CC := clang\n.PHONY: all\nall: build\nbuild test:\n\techo\n%.o: %.c\n\t$(CC)\n"
        .write(to: dir.appendingPathComponent("Makefile"), atomically: true, encoding: .utf8)
      try "set shell := [\"bash\", \"-c\"]\n# Run the app\nrun port=\"8080\":\n  echo\n_hidden:\n  echo\n@lint:\n  echo\n"
        .write(to: dir.appendingPathComponent("justfile"), atomically: true, encoding: .utf8)
      let sub = dir.appendingPathComponent("src")
      try FileManager.default.createDirectory(at: sub, withIntermediateDirectories: true)

      let ctx = context(sub.path)
      #expect(values("npm run ", ctx) == ["build", "dev", "test"], "nearest package.json above cwd")
      let yarn = CompletionEngine.candidates(input: "yarn d", context: ctx).candidates
      #expect(yarn.map(\.value) == ["dev"])
      #expect(yarn.first?.detail == "vite")
      #expect(values("make ", context(dir.path)) == ["all", "build", "test"])
      let recipes = CompletionEngine.candidates(input: "just ", context: ctx).candidates
      #expect(recipes.map(\.value) == ["run", "lint"])
      #expect(recipes.first?.detail == "Run the app")
    }

    @Test func sshHostsSkipWildcards() throws {
      let home = FileManager.default.temporaryDirectory.appendingPathComponent("home-\(UUID().uuidString)")
      try FileManager.default.createDirectory(
        at: home.appendingPathComponent(".ssh"), withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: home) }
      try "Host *\n  User me\nHost web db-1 *.internal\n  HostName x\n"
        .write(to: home.appendingPathComponent(".ssh/config"), atomically: true, encoding: .utf8)
      #expect(values("ssh ", context(home: home.path)) == ["web", "db-1"])
      #expect(values("ssh -i ", context(home: home.path)).allSatisfy { !$0.isEmpty }, "-i takes a path")
    }

    @Test func unknownCommandsCompletePaths() throws {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("paths-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir.appendingPathComponent("docs"), withIntermediateDirectories: true)
      try "x".write(to: dir.appendingPathComponent("notes.md"), atomically: true, encoding: .utf8)
      defer { try? FileManager.default.removeItem(at: dir) }
      let result = CompletionEngine.candidates(input: "mytool ", context: context(dir.path))
      #expect(result.candidates.map(\.value) == ["docs/", "notes.md"])
      #expect(result.candidates.allSatisfy { $0.kind == "path" })
      #expect(values("cd ", context(dir.path)) == ["docs/"], "cd only offers folders")
    }
  }
#endif
