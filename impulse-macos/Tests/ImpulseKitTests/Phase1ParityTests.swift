// Parity tests for the Phase 1 ports (glob, close risk, command palette),
// asserted against golden fixtures generated from the Rust implementation.
#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct GlobParityTests {
    struct GlobCase: Decodable {
      let path: String
      let pattern: String
      let matches: Bool
    }

    @Test func matchesRustGlobBehavior() throws {
      let cases = try Fixtures.decode([GlobCase].self, from: "glob.json")
      #expect(cases.count >= 15)
      for c in cases {
        #expect(
          Glob.matchesFilePattern(path: c.path, pattern: c.pattern) == c.matches,
          "path=\(c.path) pattern=\(c.pattern) expected \(c.matches)")
      }
    }
  }

  struct CloseRiskParityTests {
    struct CloseRiskCase: Decodable {
      let input: CloseRiskInput
      let summary: CloseRiskSummary
    }

    @Test func matchesRustSummaries() throws {
      let cases = try Fixtures.decode([CloseRiskCase].self, from: "close_risk.json")
      #expect(cases.count >= 4)
      for c in cases {
        #expect(c.input.summarize() == c.summary)
      }
    }

    // Ported from the Rust unit tests in close_risk.rs.
    @Test func summarizesQuitWithRunningCommand() {
      let summary = CloseRisk.summarize(
        CloseRiskInput(
          action: .quit,
          unsavedEditorCount: 0,
          runningTerminalProcessCount: 1,
          runningCommands: [
            RunningCommandRisk(
              command: "cargo test -p impulse-core", cwd: "/tmp/project", startedAtMs: 1_000)
          ],
          nowMs: 66_000,
          longCommandThresholdSeconds: 30
        ))

      #expect(summary.hasRisk)
      #expect(summary.title == "Quit Impulse with running terminal work?")
      #expect(summary.runningCommandCount == 1)
      #expect(summary.longRunningCommandCount == 1)
      #expect(
        summary.detailLines == [
          "1 running terminal process",
          "cargo test -p impulse-core running for 1m 05s",
        ])
    }

    @Test func summarizesWindowCloseWithUnsavedAndProcesses() {
      let summary = CloseRisk.summarize(
        CloseRiskInput(
          action: .closeWindow,
          unsavedEditorCount: 2,
          runningTerminalProcessCount: 3
        ))

      #expect(summary.title == "Close window with unsaved changes and running terminal work?")
      #expect(
        summary.informativeText
          == "2 editors have unsaved changes that may be lost. 3 terminal processes will be terminated."
      )
      #expect(summary.detailLines == ["2 unsaved editors", "3 running terminal processes"])
    }

    @Test func formatsUnknownAndShortRunningCommand() {
      let summary = CloseRisk.summarize(
        CloseRiskInput(
          action: .closeWindow,
          runningCommands: [RunningCommandRisk(command: "  ", startedAtMs: 12_000)],
          nowMs: 20_000
        ))

      #expect(summary.commands[0].command == "Running command")
      #expect(summary.commands[0].durationSeconds == 8)
      #expect(!summary.commands[0].isLongRunning)
    }
  }

  struct CommandPaletteParityTests {
    @Test func builtinItemsMatchFixture() throws {
      let expected = try Fixtures.decode([CommandPaletteItem].self, from: "palette_items.json")
      #expect(CommandPalette.builtinItems() == expected)
    }

    @Test func customItemMatchesFixture() throws {
      let expected = try Fixtures.decode(
        CommandPaletteItem.self, from: "palette_custom_item.json")
      let item = CommandPalette.customCommandItem(
        name: "Deploy Staging",
        shortcut: "cmd+shift+d",
        command: "./scripts/deploy.sh",
        args: ["staging", "--verbose"]
      )
      #expect(item == expected)
    }

    // The palette ranks with FuzzyMatcher now; the old substring scorer
    // (filterItems) and its palette_filter.json fixture are retired.
    struct RecentsFixture: Decodable {
      let store: RecentCommandStore
    }

    @Test func recentsRecordingMatchesFixture() throws {
      let fixture = try Fixtures.decode(RecentsFixture.self, from: "palette_recents.json")
      let items = CommandPalette.builtinItems()
      let nowMs: UInt64 = 1_700_000_000_000
      var recents = RecentCommandStore()
      recents.record(items[4], nowMs: nowMs - 60_000, maxItems: 50)
      recents.record(items[1], nowMs: nowMs, maxItems: 50)
      #expect(recents == fixture.store)
    }

    // Ported from the Rust unit tests in command_palette.rs.
    @Test func recentsDedupeByStableIdAcrossRenames() {
      let args = ["test"]
      let first = CommandPalette.customCommandItem(
        name: "Test Runner", shortcut: "Ctrl+R", command: "cargo", args: args)
      let renamed = CommandPalette.customCommandItem(
        name: "Run Tests", shortcut: "Ctrl+R", command: "cargo", args: args)

      #expect(first.id == renamed.id)

      var recents = RecentCommandStore()
      recents.record(first, nowMs: 10, maxItems: 20)
      recents.record(renamed, nowMs: 20, maxItems: 20)

      #expect(recents.items.count == 1)
      #expect(recents.items[0].title == "Run Tests")
      #expect(recents.items[0].useCount == 2)
    }

    @Test func searchResultItemsIncludePayloads() {
      let results = [
        SearchResult(path: "/repo/src/main.rs", name: "main.rs", matchType: "file"),
        SearchResult(
          path: "/repo/src/lib.rs", name: "lib.rs", lineNumber: 42,
          lineContent: "pub fn search_items()", columnStart: 7, columnEnd: 19,
          matchType: "content"),
      ]

      let items = CommandPalette.searchResultItems(root: "/repo", results: results)

      #expect(items.count == 2)
      #expect(items[0].title == "src/main.rs")
      #expect(items[0].category == "Files")
      #expect(items[0].payload?["kind"] == "file")
      #expect(items[1].title == "src/lib.rs:42")
      #expect(items[1].category == "Project Search")
      #expect(items[1].payload?["line"] == "42")
      #expect(items[1].keywords.contains { $0.contains("search_items") })
    }
  }

  struct UpdateCheckerTests {
    // Ported from the Rust unit tests in update.rs.
    @Test func parseVersion() {
      #expect(UpdateChecker.parseVersion("v1.2.3")! == (1, 2, 3))
      #expect(UpdateChecker.parseVersion("0.13.2")! == (0, 13, 2))
      #expect(UpdateChecker.parseVersion("v0.14.0")! == (0, 14, 0))
      #expect(UpdateChecker.parseVersion("invalid") == nil)
    }

    @Test func isNewer() {
      #expect(UpdateChecker.isNewer(latest: "v0.14.0", current: "0.13.2"))
      #expect(UpdateChecker.isNewer(latest: "v1.0.0", current: "0.99.99"))
      #expect(!UpdateChecker.isNewer(latest: "v0.13.2", current: "0.13.2"))
      #expect(!UpdateChecker.isNewer(latest: "v0.13.1", current: "0.13.2"))
    }
  }

  struct ShellTests {
    // Ported from the Rust unit tests in shell.rs.
    @Test func detectShellType() {
      #expect(LoginShell.detectShellType("/bin/bash") == .bash)
      #expect(LoginShell.detectShellType("/usr/local/bin/zsh") == .zsh)
      #expect(LoginShell.detectShellType("/usr/bin/fish") == .fish)
      #expect(LoginShell.detectShellType("/bin/sh") == .bash)
      #expect(LoginShell.detectShellType("/usr/bin/dash") == .bash)
    }

    @Test func integrationScriptsCarryOscMarkers() {
      for shellType in [ShellType.bash, .zsh, .fish] {
        let script = ShellIntegration.script(for: shellType)
        #expect(!script.isEmpty, "\(shellType) script missing")
        #expect(script.contains("133"), "\(shellType) script lacks OSC 133 marks")
        #expect(script.contains("6973"), "\(shellType) script lacks OSC 6973 command marker")
      }
    }

    @Test func defaultShellLooksReasonable() {
      let path = LoginShell.defaultShellPath()
      #expect(path.contains("/"))
      #expect(!LoginShell.defaultShellName().isEmpty)
    }
  }
#endif
