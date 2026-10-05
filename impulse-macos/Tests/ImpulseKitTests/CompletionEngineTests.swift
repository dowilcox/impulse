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

  struct ShellCompletionsTests {
    @Test func parsesFishOutput() {
      let out = ShellCompletions.parseFish(
        "checkout\tCheckout and switch to a branch\ncherry-pick\tReapply a commit\nsrc/\nsrc/lib.rs\t\ncheckout\tdup\n")
      #expect(out.map(\.value) == ["checkout", "cherry-pick", "src/", "src/lib.rs"])
      #expect(out[0].detail == "Checkout and switch to a branch")
      #expect(out[0].kind == "shell")
      #expect(out[2].isDir && out[2].kind == "path" && out[2].display == "src/")
      #expect(out[3].display == "lib.rs" && out[3].detail == nil)
    }

    @Test func mergesAfterTheBuiltInOnes() {
      let context = CompletionContext(cwd: nil) { [] } gitRemotes: { [] } gitTags: { [] } shellCompletions: { _ in
        ShellCompletions.parseFish("commit\tRecord changes\ncherry\tFind commits\n")
      }
      let result = CompletionEngine.candidates(input: "git c", context: context)
      let values = result.candidates.map(\.value)
      #expect(values.contains("commit") && values.contains("cherry"))
      #expect(values.filter { $0 == "commit" }.count == 1, "no duplicates")
      #expect(values.firstIndex(of: "cherry")! > values.firstIndex(of: "commit")!, "built-in first")
    }

    @Test func fishCompletesForReal() {
      // Only where fish is installed.
      guard
        let fish = ["/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish"]
          .first(where: { FileManager.default.isExecutableFile(atPath: $0) })
      else { return }
      let out = ShellCompletions.fish(line: "set --erase\"; echo; ", cwd: "/", fishPath: fish)
      #expect(out.allSatisfy { !$0.value.contains("\n") }, "the line is data, not code")
      #expect(ShellCompletions.fish(line: "cd /us", cwd: "/", fishPath: fish).contains { $0.value == "/usr/" })
    }
  }
#endif
