#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct FuzzyMatcherTests {
    @Test func requiresInOrderSubsequence() {
      #expect(FuzzyMatcher.match("tmn", in: "TabManager.swift") != nil)
      #expect(FuzzyMatcher.match("nmt", in: "TabManager.swift") == nil)
      #expect(FuzzyMatcher.match("xyz", in: "abc") == nil)
      #expect(FuzzyMatcher.match("", in: "anything")?.score == 0)
    }

    @Test func reportsMatchedPositions() {
      let match = FuzzyMatcher.match("tm", in: "TabManager")
      #expect(match?.positions == [0, 3])
    }

    @Test func smartCase() {
      #expect(FuzzyMatcher.match("tab", in: "TabManager") != nil)
      #expect(FuzzyMatcher.match("Tab", in: "tabmanager") == nil)
    }

    @Test func prefersWordBoundariesAndConsecutiveRuns() {
      let ranked = FuzzyMatcher.rank(
        ["text/bar.md", "the_big_ref.swift", "TabBar.swift"], query: "tb", isPath: true
      ) { $0 }
      #expect(ranked.first?.item == "TabBar.swift")

      let consecutive = FuzzyMatcher.match("main", in: "MainWindow.swift")!
      let scattered = FuzzyMatcher.match("main", in: "my_awful_init.swift")!
      #expect(consecutive.score > scattered.score)
    }

    @Test func pathMatchesFavorTheFileName() {
      let ranked = FuzzyMatcher.rank(
        [
          "impulse-macos/Sources/ImpulseApp/Workbench/ChromeBarView.swift",
          "impulse-macos/Sources/ImpulseApp/Workbench/WorkbenchView.swift",
        ],
        query: "workbenchview", isPath: true
      ) { $0 }
      #expect(ranked.first?.item.hasSuffix("WorkbenchView.swift") == true)
    }

    @Test func rankKeepsOrderForEmptyQueryAndRespectsLimit() {
      let items = ["b", "a", "c"]
      #expect(FuzzyMatcher.rank(items, query: "") { $0 }.map(\.item) == ["b", "a", "c"])
      #expect(FuzzyMatcher.rank(items, query: "", limit: 2) { $0 }.count == 2)
    }

    @Test func commandTitles() {
      let titles = ["Toggle Sidebar", "Toggle Right Panel", "Go to Line", "New Terminal Tab"]
      let ranked = FuzzyMatcher.rank(titles, query: "ntt") { $0 }
      #expect(ranked.first?.item == "New Terminal Tab")
      let sidebar = FuzzyMatcher.rank(titles, query: "tog side") { $0 }
      // Spaces are literal characters in the query; "tog side" still matches.
      #expect(sidebar.first?.item == "Toggle Sidebar")
    }

    private static let paths = [
      "impulse-macos/Sources/ImpulseApp/TabManager.swift",
      "impulse-macos/Sources/ImpulseApp/Workbench/TitlebarTabStrip.swift",
      "impulse-macos/Sources/ImpulseApp/Palette/PaletteModel.swift",
      "impulse-macos/Sources/ImpulseKit/FuzzyMatcher.swift",
      "impulse-macos/Tests/ImpulseKitTests/FuzzyMatcherTests.swift",
      "docs/command-palette.md",
      "README.md",
      "src/tab_bar.rs",
      "src/TabBar/index.ts",
      "Makefile",
    ]

    @Test func matchesThenBestIsRank() {
      for query in ["t", "tab", "tb", "Tab", "pm", "fzm", "md", " tab ", "", "zzz"] {
        let ranked = FuzzyMatcher.rank(Self.paths, query: query, limit: 4, isPath: true) { $0 }
        let matched = FuzzyMatcher.matches(Self.paths, query: query, isPath: true) { $0 } ?? []
        let best = FuzzyMatcher.best(matched, limit: 4)
        #expect(best.map(\.item) == ranked.map(\.item), "\(query)")
        #expect(best.map(\.match) == ranked.map(\.match), "\(query)")
      }
    }

    @Test func narrowingFromAShorterQueryRanksTheSame() {
      // Quick open searches the last query's matches when the query only
      // grew; that must give what searching everything gives.
      for (shorter, longer) in [("t", "tab"), ("ta", "taB"), ("s", "src/t"), ("", "pal"), ("m", "md")] {
        let narrowed = FuzzyMatcher.matches(Self.paths, query: shorter, isPath: true) { $0 } ?? []
        let fromNarrowed = FuzzyMatcher.best(
          FuzzyMatcher.matches(narrowed.map(\.item), query: longer, isPath: true) { $0 } ?? [])
        let fromAll = FuzzyMatcher.rank(Self.paths, query: longer, isPath: true) { $0 }
        #expect(fromNarrowed.map(\.item) == fromAll.map(\.item), "\(shorter) → \(longer)")
        #expect(fromNarrowed.map(\.match) == fromAll.map(\.match), "\(shorter) → \(longer)")
      }
    }

    @Test func matchingStopsWhenAsked() {
      let many = Array(repeating: "Sources/ImpulseApp/TabManager.swift", count: 5000)
      var asked = 0
      let stopped = FuzzyMatcher.matches(many, query: "tab", shouldStop: {
        asked += 1
        return asked > 2
      }) { $0 }
      #expect(stopped == nil)
      #expect(asked == 3)
      #expect(FuzzyMatcher.matches(many, query: "tab") { $0 }?.count == 5000)
    }
  }
#endif
