#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  @MainActor
  struct TerminalAttentionTests {
    private final class Counter: @unchecked Sendable {
      var value = 0
    }

    /// A terminal closed while it wants attention (the agent of a task that
    /// was just finished) stops wanting it, and says so, so the Dock badge
    /// counts it no longer.
    @Test func closingATerminalClearsItsAttention() {
      _ = NSApplication.shared
      let terminal = TerminalTab(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
      terminal.setAttentionFromAgent()
      #expect(terminal.needsAttention)
      let changes = Counter()
      let observer = NotificationCenter.default.addObserver(
        forName: .terminalAttentionChanged, object: terminal, queue: nil
      ) { _ in changes.value += 1 }
      defer { NotificationCenter.default.removeObserver(observer) }
      terminal.terminateProcess()
      #expect(!terminal.needsAttention)
      #expect(changes.value == 1)
    }
  }
#endif
