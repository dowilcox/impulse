#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct PullRequestTests {
    @Test func parsesGhOutput() throws {
      let json = #"""
        {"number": 42, "title": "Add history", "state": "OPEN", "isDraft": true,
         "reviewDecision": "REVIEW_REQUIRED", "url": "https://github.com/o/r/pull/42",
         "statusCheckRollup": [
           {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS"},
           {"__typename": "StatusContext", "state": "SUCCESS"}]}
        """#
      let pr = try #require(PullRequestInfo.parse(Data(json.utf8)))
      #expect(pr.number == 42)
      #expect(pr.state == .open)
      #expect(pr.isDraft)
      #expect(pr.reviewDecision == "REVIEW_REQUIRED")
      #expect(pr.checks == .passed)
      #expect(pr.url.hasSuffix("/42"))
      #expect(PullRequestInfo.parse(Data("no pull requests found".utf8)) == nil)
    }

    @Test func checksRollUp() {
      #expect(PullRequestInfo.rollup([]) == .none)
      #expect(PullRequestInfo.rollup([["status": "IN_PROGRESS", "conclusion": ""]]) == .pending)
      #expect(PullRequestInfo.rollup([["state": "PENDING"], ["status": "COMPLETED", "conclusion": "SUCCESS"]]) == .pending)
      #expect(
        PullRequestInfo.rollup([["status": "QUEUED"], ["status": "COMPLETED", "conclusion": "FAILURE"]]) == .failed)
      #expect(PullRequestInfo.rollup([["state": "ERROR"]]) == .failed)
      #expect(PullRequestInfo.rollup([["status": "COMPLETED", "conclusion": "SKIPPED"], ["status": "COMPLETED", "conclusion": "NEUTRAL"]]) == .passed)
    }
  }
#endif
