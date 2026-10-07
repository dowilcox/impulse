#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct SaveCommandTests {
    @Test func filePlaceholderIsReplacedEverywhere() {
      let args = ["--write", "{file}", "--stdin-filepath={file}", "--file", "."]
      #expect(
        SaveCommand.expandArguments(args, file: "/tmp/a b.ts")
          == ["--write", "/tmp/a b.ts", "--stdin-filepath=/tmp/a b.ts", "--file", "."])
      #expect(SaveCommand.expandArguments([], file: "/x").isEmpty)
    }

    @Test func failureSummaryPrefersTheFirstErrorLine() {
      let summary = SaveCommand.failureSummary(
        status: 2, stdout: Data("progress\n".utf8),
        stderr: Data("\n  \n  error: unexpected token  \nat line 3\n".utf8), timedOut: false)
      #expect(summary == "error: unexpected token")
    }

    @Test func failureSummaryFallsBackToOutputThenStatus() {
      #expect(
        SaveCommand.failureSummary(
          status: 1, stdout: Data("Found 2 problems\n".utf8), stderr: Data(), timedOut: false)
          == "Found 2 problems")
      #expect(
        SaveCommand.failureSummary(status: 127, stdout: Data(), stderr: Data(" \n".utf8), timedOut: false)
          == "It exited with status 127.")
      #expect(
        SaveCommand.failureSummary(status: 143, stdout: Data(), stderr: Data("x".utf8), timedOut: true)
          == "It didn't finish in time and was stopped.")
    }

    @Test func longLinesAreCut() {
      let line = String(repeating: "e", count: 500)
      let summary = SaveCommand.failureSummary(
        status: 1, stdout: Data(), stderr: Data(line.utf8), timedOut: false)
      #expect(summary.count == SaveCommand.maxSummaryLength)
      #expect(summary.hasSuffix("…"))
    }
  }
#endif
