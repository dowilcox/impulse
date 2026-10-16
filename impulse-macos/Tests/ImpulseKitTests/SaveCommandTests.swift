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

    @Test func onlyASuccessfulNonEmptyRewriteIsAdopted() {
      #expect(SaveCommand.shouldAdopt(rewritten: "let a = 1\n", saved: "let a=1", succeeded: true))
      // Failed or stopped: whatever it left behind isn't shown.
      #expect(!SaveCommand.shouldAdopt(rewritten: "let a", saved: "let a=1", succeeded: false))
      // Emptied a file that had text.
      #expect(!SaveCommand.shouldAdopt(rewritten: "", saved: "let a=1", succeeded: true))
      #expect(!SaveCommand.shouldAdopt(rewritten: " \n\n", saved: "let a=1", succeeded: true))
      // A blank file may stay blank.
      #expect(SaveCommand.shouldAdopt(rewritten: "", saved: "\n", succeeded: true))
    }

    @Test func aGoodResultIsShownUnlessTypingMovedOn() {
      #expect(SaveCommand.rewriteOutcome(saved: "a=1", disk: "a = 1", buffer: "a=1", succeeded: true) == .adopt)
      #expect(
        SaveCommand.rewriteOutcome(saved: "a=1", disk: "a = 1", buffer: "a=12", succeeded: true)
          == .record(dirty: true))
    }

    @Test func untouchedFileIsRecordedAndOnlyNewerTypingIsDirty() {
      #expect(SaveCommand.rewriteOutcome(saved: "x", disk: "x", buffer: "x", succeeded: true) == .record(dirty: false))
      #expect(SaveCommand.rewriteOutcome(saved: "x", disk: "x", buffer: "x", succeeded: false) == .record(dirty: false))
      #expect(SaveCommand.rewriteOutcome(saved: "x", disk: "x", buffer: "xy", succeeded: true) == .record(dirty: true))
    }

    @Test func formatterThenReloadingCommandLeavesACleanBufferClean() {
      // The formatter's result F is shown; the command's "saved" may still
      // be the text from before formatting, and it changed nothing.
      #expect(SaveCommand.rewriteOutcome(saved: "S", disk: "F", buffer: "F", succeeded: true) == .record(dirty: false))
    }

    @Test func aFailedRunsLeftoversKeepTheSavedDiskState() {
      // Whether the formatter half-wrote the file or another program wrote
      // to it meanwhile, it isn't taken as known: the next save asks.
      #expect(
        SaveCommand.rewriteOutcome(saved: "a=1", disk: "written by an agent", buffer: "a=1", succeeded: false)
          == .keepSavedState)
      #expect(SaveCommand.rewriteOutcome(saved: "a=1", disk: "", buffer: "a=1", succeeded: true) == .keepSavedState)
    }
  }
#endif
