#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ReviewCommentTests {
    let comment = ReviewComment(
      path: "src/a.swift", side: .new, line: 10, endLine: 11,
      snippet: "let x = 1\nlet y = 2", text: "Rename these.")

    @Test func matchingLinesAreNotOutdated() {
      #expect(!ReviewCommentAnchoring.isOutdated(comment, lines: [10: "let x = 1", 11: "let y = 2"]))
    }

    @Test func changedOrMissingLinesAreOutdated() {
      #expect(ReviewCommentAnchoring.isOutdated(comment, lines: [10: "let x = 1", 11: "let y = 3"]))
      #expect(ReviewCommentAnchoring.isOutdated(comment, lines: [10: "let x = 1"]))
      #expect(ReviewCommentAnchoring.isOutdated(comment, lines: [:]))
    }

    @Test func promptGroupsByFileAndLine() {
      let later = ReviewComment(
        path: "src/a.swift", side: .new, line: 40, endLine: 40, snippet: "return nil",
        text: "Throw instead.")
      let other = ReviewComment(
        path: "README.md", side: .old, line: 3, endLine: 3, snippet: "old text",
        text: "Keep this line.")
      let prompt = ReviewCommentAnchoring.prompt(for: [later, comment, other])
      #expect(prompt.hasPrefix("Please address these review comments"))
      let readme = prompt.range(of: "1. README.md:3 (removed lines)")
      let first = prompt.range(of: "2. src/a.swift:10-11")
      let second = prompt.range(of: "3. src/a.swift:40")
      #expect(readme != nil && first != nil && second != nil)
      #expect(prompt.contains("```\nlet x = 1\nlet y = 2\n```\nRename these."))
      #expect(ReviewCommentAnchoring.prompt(for: []) == "")
    }
  }
#endif
