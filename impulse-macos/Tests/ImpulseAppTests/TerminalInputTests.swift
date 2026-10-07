#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import SwiftUI
  import Testing

  /// Where keys typed in a terminal go, and the input bar's suggestion keys.
  @MainActor
  struct TerminalInputTests {
    /// A grid at a shell prompt with the input bar on, recording what it
    /// hands to the bar.
    private func renderer(
      typed: @escaping (String) -> Void = { _ in }, focus: @escaping () -> Void = {}
    ) -> TerminalRenderer {
      let renderer = TerminalRenderer(
        frame: NSRect(x: 0, y: 0, width: 400, height: 200), fontFamily: "Menlo", fontSize: 12)
      renderer.tracksPrompts = true
      renderer.onTypeIntoInputBar = typed
      renderer.onRequestInputFocus = focus
      return renderer
    }

    private func key(_ characters: String, keyCode: UInt16, flags: NSEvent.ModifierFlags = []) -> NSEvent {
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: 0, context: nil,
        characters: characters, charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode)!
    }

    private let noReplacement = NSRange(location: NSNotFound, length: 0)

    @Test func textTypedInTheGridAtAPromptGoesToTheInputBar() {
      var typed: [String] = []
      let grid = renderer(typed: { typed.append($0) })
      #expect(grid.typingGoesToInputBar)
      grid.insertText("ls -la", replacementRange: noReplacement)
      // An unbound ⌃ key's control character has nowhere to go.
      grid.insertText("\u{3}", replacementRange: noReplacement)
      #expect(typed == ["ls -la"])
    }

    @Test func typingStaysInTheGridWhenTheBarDoesNotOwnIt() {
      var typed: [String] = []
      let running = renderer(typed: { typed.append($0) })
      running.commandRunning = true  // keys reach the running program
      let classic = renderer(typed: { typed.append($0) })
      classic.suppressLivePrompt = false  // input bar off: the shell's prompt is live
      let unknown = renderer(typed: { typed.append($0) })
      unknown.tracksPrompts = false  // no shell integration: can't tell a prompt
      for grid in [running, classic, unknown] {
        #expect(!grid.typingGoesToInputBar)
        grid.insertText("x", replacementRange: noReplacement)
      }
      #expect(typed.isEmpty)
    }

    @Test func returnAndEscapeInTheGridAtAPromptGoBackToTheBar() {
      var focused = 0
      let grid = renderer(focus: { focused += 1 })
      grid.keyDown(with: key("\r", keyCode: 36))
      grid.keyDown(with: key("\u{1b}", keyCode: 53))
      grid.keyDown(with: key("\u{F700}", keyCode: 126))  // ↑: nothing
      #expect(focused == 2)
    }

    @Test func theKeyThatEndsABlockSelectionGoesToTheBarEvenWhileACommandRuns() {
      var typed: [String] = []
      var exited = false
      let grid = renderer(typed: { typed.append($0) })
      grid.commandRunning = true
      grid.selectedBlockIds = [1]
      grid.onBlockSelectionKey = { if case .exit = $0 { exited = true } }
      grid.keyDown(with: key("y", keyCode: 16))
      #expect(exited)
      #expect(typed == ["y"])
    }

    @Test func controlAndNonTextKeysThatEndASelectionReachTheRunningProgram() {
      var typed: [String] = []
      var exits = 0
      let grid = renderer(typed: { typed.append($0) })
      grid.commandRunning = true
      grid.onBlockSelectionKey = { if case .exit = $0 { exits += 1 } }
      let controlC = key("c", keyCode: 8, flags: .control)
      #expect(!grid.keyGoesToInputBar(controlC, endingSelection: true))
      #expect(!grid.keyGoesToInputBar(key("\u{F702}", keyCode: 123), endingSelection: true))  // ←
      #expect(!grid.keyGoesToInputBar(key("\t", keyCode: 48), endingSelection: true))
      #expect(grid.keyGoesToInputBar(key("y", keyCode: 16), endingSelection: true))
      grid.selectedBlockIds = [1]
      grid.keyDown(with: controlC)
      #expect(exits == 1)
      #expect(typed.isEmpty)
      // At a prompt nothing reaches the shell's line: it all goes to the bar.
      grid.commandRunning = false
      #expect(grid.keyGoesToInputBar(controlC, endingSelection: true))
      #expect(grid.keyGoesToInputBar(key("\u{F702}", keyCode: 123), endingSelection: false))
    }

    @Test func commandUpInTheGridSelectsBlocks() {
      var selected = false
      let grid = renderer()
      grid.onBlockSelectionKey = { if case .up(extend: false) = $0 { selected = true } }
      grid.keyDown(with: key("\u{F700}", keyCode: 126, flags: [.command, .numericPad, .function]))
      #expect(selected)
    }

    // MARK: Input bar suggestion keys

    @Test func rightArrowTakesTheSuggestionAndOptionRightAWord() {
      var keys: [CommandEditorKey] = []
      let editor = CommandEditor(
        text: .constant("git"), placeholder: "", suggestion: "git status --short",
        colors: CommandEditorColors(theme: ThemeManager.theme(forName: "nord")), focusToken: 0,
        onSubmit: {}, onKey: { keys.append($0); return true })
      let coordinator = editor.makeCoordinator()
      let textView = CommandTextView()
      textView.string = "git"
      textView.setSelectedRange(NSRange(location: 3, length: 0))
      #expect(coordinator.textView(textView, doCommandBy: #selector(NSResponder.moveRight(_:))))
      #expect(coordinator.textView(textView, doCommandBy: #selector(NSResponder.moveWordRight(_:))))
      // Not at the end: the arrows move the cursor as usual.
      textView.setSelectedRange(NSRange(location: 1, length: 0))
      #expect(!coordinator.textView(textView, doCommandBy: #selector(NSResponder.moveRight(_:))))
      #expect(!coordinator.textView(textView, doCommandBy: #selector(NSResponder.moveWordRight(_:))))
      #expect(keys == [.right, .wordRight])
    }

    // MARK: Program status (OSC 21337)

    @Test func statusColorColorsTheTabDotWithoutAnIndicator() {
      var status = TerminalSessionStatus()
      status.apply(["status": "Building", "status-color": "rgb:ff/a5/00"])
      #expect(!status.isEmpty)
      #expect(status.dotColor == "#ffa500")
      status.apply(["indicator": "#00ff00"])
      #expect(status.dotColor == "#00ff00")
      var colorOnly = TerminalSessionStatus()
      colorOnly.apply(["status-color": "#123456"])
      #expect(!colorOnly.isEmpty)
      colorOnly.apply(["status-color": ""])
      #expect(colorOnly.isEmpty && colorOnly.dotColor == nil)
    }
  }
#endif
