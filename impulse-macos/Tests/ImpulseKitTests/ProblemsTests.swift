#if canImport(Testing)
  import Testing

  @testable import ImpulseKit

  struct ProblemsTests {
    private let problems = [
      Problem(path: "/r/b.swift", line: 9, column: 2, severity: .warning, message: "unused variable 'x'", source: "sourcekit"),
      Problem(path: "/r/a.swift", line: 3, column: 1, severity: .info, message: "consider let"),
      Problem(path: "/r/b.swift", line: 2, column: 5, severity: .error, message: "cannot find 'foo'", source: "sourcekit", code: "E1"),
      Problem(path: "/r/c.ts", line: 1, column: 1, severity: .hint, message: "line one\nline two"),
    ]

    @Test func countsBySeverity() {
      #expect(Problems.counts(problems) == ProblemCounts(errors: 1, warnings: 1, others: 2))
      #expect(Problems.counts([]).total == 0)
    }

    @Test func filtersBySeverityAndText() {
      #expect(Problems.filter(problems, severities: [.error], text: "").map(\.message) == ["cannot find 'foo'"])
      #expect(Problems.filter(problems, severities: Set(Problem.Severity.allCases), text: "SOURCEKIT").count == 2)
      #expect(Problems.filter(problems, severities: Set(Problem.Severity.allCases), text: "c.ts").count == 1)
      #expect(Problems.filter(problems, severities: [.warning], text: "foo").isEmpty)
    }

    @Test func groupsWorstFilesFirstSortedByPosition() {
      let groups = Problems.grouped(problems)
      #expect(groups.map(\.path) == ["/r/b.swift", "/r/a.swift", "/r/c.ts"])
      #expect(groups[0].problems.map(\.line) == [2, 9])
    }

    @Test func promptIsRelativeAndCapped() {
      let prompt = Problems.prompt(problems, root: "/r", limit: 2)
      #expect(prompt.contains("- b.swift:2:5 error [sourcekit E1]: cannot find 'foo'"))
      #expect(prompt.contains("- b.swift:9:2 warning [sourcekit]: unused variable 'x'"))
      #expect(!prompt.contains("a.swift"), "capped at two")
      #expect(prompt.contains("…and 2 more."))
      #expect(Problems.prompt([], root: nil).isEmpty)
      #expect(Problems.prompt([problems[3]], root: nil).contains("line one line two"))
    }
  }
#endif
