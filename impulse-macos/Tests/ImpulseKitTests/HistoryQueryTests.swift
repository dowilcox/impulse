#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct HistoryQueryTests {
    @Test func tokensAndFreeText() {
      let query = HistoryQuery.parse("author:jane fix  crash path:src/ since:2w")
      #expect(query.author == "jane")
      #expect(query.path == "src/")
      #expect(query.since == "2.weeks.ago")
      #expect(query.until == nil)
      #expect(query.text == "fix crash")
      #expect(query.hasServerFilters)
    }

    @Test func aliasesQuotesAndUnknownKeys() {
      let query = HistoryQuery.parse("by:\"Jane Doe\" before:2026-01-01 after:yesterday fix:bug")
      #expect(query.author == "Jane Doe")
      #expect(query.until == "2026-01-01 23:59:59", "the whole day")
      #expect(query.since == "yesterday")
      #expect(query.text == "fix:bug", "unknown keys stay free text")
    }

    @Test func emptyValuesAndPlainText() {
      let query = HistoryQuery.parse("author: refactor")
      #expect(query.author == nil)
      #expect(query.text == "refactor")
      #expect(!query.hasServerFilters)
      #expect(HistoryQuery.parse("").server == HistoryQuery())
    }

    @Test func gitArguments() {
      let query = HistoryQuery.parse("author:jane since:3d until:1y")
      #expect(query.gitArguments == ["--author=jane", "--regexp-ignore-case", "--since=3.days.ago", "--until=1.years.ago"])
      #expect(HistoryQuery.parse("words only").gitArguments.isEmpty)
    }

    @Test func relativeDates() {
      #expect(HistoryQuery.gitDate("12h") == "12.hours.ago")
      #expect(HistoryQuery.gitDate("1M") == "1.months.ago")
      #expect(HistoryQuery.gitDate("2026-03-01") == "2026-03-01 00:00:00")
      #expect(HistoryQuery.gitDate("2026-03-01", endOfDay: true) == "2026-03-01 23:59:59")
      #expect(HistoryQuery.gitDate("last friday") == "last friday")
      #expect(HistoryQuery.gitDate("w") == "w")
    }

    @Test func settingReplacesOrRemovesATokenAndItsAliases() {
      #expect(HistoryQuery.setting("since", to: "1w", in: "fix after:2d") == "since:1w fix")
      #expect(HistoryQuery.setting("author", to: "Jane Doe", in: "") == "author:\"Jane Doe\"")
      #expect(HistoryQuery.setting("author", to: nil, in: "by:jane fix") == "fix")
      #expect(HistoryQuery.parse(HistoryQuery.setting("author", to: "Jane Doe", in: "x")).author == "Jane Doe")
    }
  }
#endif
