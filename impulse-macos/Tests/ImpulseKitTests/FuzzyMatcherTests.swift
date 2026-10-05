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
  }
#endif
