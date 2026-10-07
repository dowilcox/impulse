#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct DiffScopeTitleTests {
    @Test func checkpointAndReviewRefsGetFriendlyTitles() {
      let millis = "1700000000000"
      let time = DateFormatter.localizedString(
        from: Date(timeIntervalSince1970: 1_700_000_000), dateStyle: .none, timeStyle: .short)
      let start = "refs/impulse/checkpoints/ABC-123/\(millis)-turn-start"
      #expect(DiffScope.snapshot(from: start, to: nil).title == "Agent turn since \(time)")
      #expect(DiffScope.snapshot(from: start, to: start).title == "Agent turn at \(time)")
      #expect(
        DiffScope.snapshot(from: "refs/impulse/reviews/\(millis)-reviewed", to: nil).title
          == "Since your review at \(time)")
      // Other refs keep the short form.
      #expect(DiffScope.snapshot(from: "refs/impulse/oplog/123-discard", to: nil).title == "Since 123-discard")
    }

    @Test func fullCommitIDsAreShortened() {
      let a = "4e1b9c2d0a5f6e7d8c9b0a1f2e3d4c5b6a7f8e9d"
      let b = "9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b"
      #expect(DiffScope.snapshot(from: a, to: nil).title == "Since 4e1b9c2")
      #expect(DiffScope.snapshot(from: a, to: b).title == "4e1b9c2…9a8b7c6")
      #expect(DiffScope.range(from: a, to: b).title == "4e1b9c2…9a8b7c6")
      // Names (and short IDs) are left as they are.
      #expect(DiffScope.range(from: "main", to: "feature/cache").title == "main…feature/cache")
      #expect(DiffScope.snapshot(from: "abc1234", to: nil).title == "Since abc1234")
    }
  }
#endif
