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
  }
#endif
