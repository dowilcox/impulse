#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TaskListingTests {
    private let listing = TaskListing(
      repository: "pulseboard",
      workspaces: [
        .init(
          name: "pulseboard", path: "/code/pulseboard", branch: "main", base: nil, head: "7e7a97a1", isMainCheckout: true,
          isCaller: false, agent: "Claude Code", agentState: "working", uncommitted: 156,
          sharedWithCaller: ["Pages/Teams/Show.tsx", "Pages/Boards/Show.tsx"]),
        .init(
          name: "interia-upgrade", path: "/code/pulseboard.worktrees/interia-upgrade", branch: "interia-upgrade",
          base: "origin/main", head: "c5aa8141", isMainCheckout: false, isCaller: true, uncommitted: 2, ahead: 4,
          behind: 1),
      ])

    @Test func textListsEachWorkspace() {
      let text = listing.text()
      #expect(text.hasPrefix("pulseboard: 2 workspaces"))
      #expect(text.contains("pulseboard (main checkout)\n  branch main at 7e7a97a"))
      #expect(text.contains("  Claude Code: working\n  156 uncommitted files"))
      #expect(text.contains("also changes files you change: Pages/Teams/Show.tsx, Pages/Boards/Show.tsx"))
      #expect(text.contains("interia-upgrade (task) ← you\n  branch interia-upgrade from origin/main, 4 ahead, 1 behind"))
    }

    @Test func jsonRoundTrips() throws {
      let decoded = try JSONDecoder().decode(TaskListing.self, from: Data(listing.json().utf8))
      #expect(decoded == listing)
    }

    @Test func aSummaryForTheAgentStartingUp() throws {
      let summary = try #require(listing.sessionSummary())
      #expect(summary.hasPrefix("Impulse: you're in the task interia-upgrade of pulseboard: a git worktree on branch interia-upgrade, made from origin/main."))
      #expect(summary.contains("pulseboard (main checkout) — Claude Code working, 156 uncommitted, changes 2 of your files"))
      #expect(summary.hasSuffix("to see what they're changing."))
      #expect(TaskListing(repository: "x", workspaces: [listing.workspaces[1]]).sessionSummary() == nil, "alone: nothing to say")
    }

    @Test func hookRepliesAreClaudeCodeJSON() throws {
      let context = try JSONSerialization.jsonObject(with: Data(AgentHookReply.context("a \"note\"").utf8)) as? [String: Any]
      let output = context?["hookSpecificOutput"] as? [String: String]
      #expect(output == ["hookEventName": "PostToolUse", "additionalContext": "a \"note\""])
      let ask = try JSONSerialization.jsonObject(with: Data(AgentHookReply.ask("still moving").utf8)) as? [String: Any]
      #expect((ask?["hookSpecificOutput"] as? [String: String])?["permissionDecision"] == "ask")
    }
  }
#endif
