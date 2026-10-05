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

  struct PullRequestThreadTests {
    private let json = #"""
      {"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": [
        {"id": "T1", "isResolved": false, "isOutdated": false, "path": "src/a.swift",
         "diffSide": "RIGHT", "line": 14, "startLine": 12, "originalLine": 14, "originalStartLine": 12,
         "comments": {"nodes": [
           {"author": {"login": "ana"}, "body": "Handle the nil case.\n", "url": "https://github.com/o/r/pull/7#discussion_r1",
            "createdAt": "2026-10-01T09:30:00Z"},
           {"author": {"login": "ben"}, "body": "Agreed", "url": "https://github.com/o/r/pull/7#discussion_r2",
            "createdAt": "2026-10-01T10:00:00Z"}]}},
        {"id": "T2", "isResolved": true, "isOutdated": false, "path": "b.txt", "diffSide": "RIGHT",
         "line": 3, "startLine": null, "originalLine": 3, "originalStartLine": null,
         "comments": {"nodes": [{"author": {"login": "ana"}, "body": "Done?", "url": "u2", "createdAt": "2026-10-01T09:30:00Z"}]}},
        {"id": "T3", "isResolved": false, "isOutdated": true, "path": "c.txt", "diffSide": "LEFT",
         "line": null, "startLine": null, "originalLine": 9, "originalStartLine": null,
         "comments": {"nodes": [{"author": null, "body": "Why remove this?", "url": "u3", "createdAt": "2026-10-01T09:30:00Z"}]}}
      ]}}}}}
      """#

    @Test func unresolvedThreadsBecomeAnchoredComments() throws {
      let comments = try #require(PullRequestThreads.parse(Data(json.utf8)))
      #expect(comments.map(\.id) == ["gh:T1", "gh:T3"], "resolved threads are skipped")

      let first = comments[0]
      #expect(first.path == "src/a.swift")
      #expect(first.side == .new)
      #expect(first.line == 12 && first.endLine == 14)
      #expect(first.text == "@ana: Handle the nil case.\n\n@ben: Agreed")
      #expect(first.remote == .init(author: "ana", url: "https://github.com/o/r/pull/7#discussion_r1", isOutdated: false))

      let moved = comments[1]
      #expect(moved.side == .old)
      #expect(moved.line == 9 && moved.endLine == 9, "falls back to the original line")
      #expect(moved.remote?.isOutdated == true)
      #expect(moved.remote?.author == "ghost")

      let all = try #require(PullRequestThreads.parse(Data(json.utf8), includeResolved: true))
      #expect(all.count == 3)
      #expect(PullRequestThreads.parse(Data("{}".utf8)) == nil)
    }

    @Test func remoteCommentsFloatWhenTheirLineIsntInTheDiff() throws {
      let comments = try #require(PullRequestThreads.parse(Data(json.utf8)))
      #expect(!ReviewCommentAnchoring.isOutdated(comments[0], lines: [12: "a", 13: "b", 14: "c"]))
      #expect(ReviewCommentAnchoring.isOutdated(comments[0], lines: [1: "a"]))
      #expect(ReviewCommentAnchoring.isOutdated(comments[1], lines: [9: "x"]), "the host says outdated")
    }

    @Test func coordinatesComeFromThePullRequestURL() throws {
      let coordinates = try #require(PullRequestThreads.coordinates(fromURL: "https://github.com/dowilcox/impulse/pull/31"))
      #expect(coordinates.owner == "dowilcox" && coordinates.name == "impulse" && coordinates.number == 31)
      #expect(PullRequestThreads.coordinates(fromURL: "https://github.com/dowilcox/impulse") == nil)
    }
  }
#endif