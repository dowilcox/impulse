import SwiftUI

/// Warp-style input area pinned below the terminal: a context-chip row
/// (shell, cwd, git branch, last command status) above a monospaced command
/// input with history ghost suggestions. Enter runs the command in the
/// active terminal; ↑/↓ cycle history; Tab accepts the suggestion; Esc moves
/// focus into the terminal grid. While a command runs the input swaps to a
/// running indicator with a Stop button. Native materials and SF Symbols
/// keep it reading as macOS chrome.
struct TerminalContextBarView: View {
  var model: WindowModel

  @State private var text: String = ""
  @State private var suggestion: String? = nil
  /// Index into the recent-history list while cycling with ↑/↓; nil = live draft.
  @State private var historyIndex: Int? = nil
  @State private var savedDraft: String = ""
  @FocusState private var inputFocused: Bool

  private var monoFont: Font { .system(size: 13, design: .monospaced) }

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      chipRow
      inputRow
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .background(.bar)
    .overlay(alignment: .top) { Divider() }
    .onChange(of: model.commandRunning) { _, running in
      if !running {
        // Shell returned to the prompt — reclaim focus for the next command.
        inputFocused = true
      }
    }
    .onChange(of: model.inputBarFocusToken) {
      inputFocused = true
    }
    .onAppear { inputFocused = true }
  }

  // MARK: - Context chips

  private var chipRow: some View {
    HStack(spacing: 6) {
      if !model.shellName.isEmpty {
        ContextChip(symbol: "terminal", text: model.shellName)
      }
      if !model.currentCwd.isEmpty {
        ContextChip(symbol: "folder", text: TabManager.abbreviateHomePath(model.currentCwd))
      }
      if let branch = model.gitBranch, !branch.isEmpty {
        BranchChip(model: model, branch: branch)
      }
      statusChip
      Spacer(minLength: 8)
      actionButton(symbol: "clock.arrow.circlepath", help: "Command History (⌘R)") {
        model.onShowCommandHistory?()
      }
    }
  }

  @ViewBuilder
  private var statusChip: some View {
    if !model.commandRunning, let exitCode = model.lastCommandExitCode {
      let failed = exitCode != 0
      HStack(spacing: 4) {
        Image(systemName: failed ? "xmark.circle.fill" : "checkmark.circle.fill")
          .font(.system(size: 10))
          .foregroundStyle(failed ? Color(nsColor: .systemRed) : Color(nsColor: .systemGreen))
        Text(statusText(exitCode: exitCode))
          .font(.system(size: 11, design: .monospaced))
          .foregroundStyle(.secondary)
      }
      .padding(.horizontal, 8)
      .padding(.vertical, 3)
      .background(Capsule().fill(Color.primary.opacity(0.05)))
      .help(failed ? "Last command failed with exit code \(exitCode)" : "Last command succeeded")
    }
  }

  private func statusText(exitCode: Int32) -> String {
    var parts: [String] = []
    if exitCode != 0 { parts.append("exit \(exitCode)") }
    if let ms = model.lastCommandDurationMs {
      parts.append(TerminalRenderer.formatBlockDuration(ms))
    }
    return parts.isEmpty ? "ok" : parts.joined(separator: " · ")
  }

  private func actionButton(
    symbol: String, help: String, action: @escaping () -> Void
  ) -> some View {
    Button(action: action) {
      Image(systemName: symbol)
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(.secondary)
        .frame(width: 22, height: 22)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .help(help)
    .accessibilityLabel(help)
  }

  // MARK: - Input row

  /// The input field is always present — even while a command runs, so
  /// line-based prompts (npm questions, `read`, REPLs) can receive stdin.
  /// The leading glyph and trailing accessory reflect the running state.
  private var inputRow: some View {
    HStack(spacing: 8) {
      if model.commandRunning {
        ProgressView()
          .controlSize(.small)
          .scaleEffect(0.8)
          .frame(width: 14)
      } else {
        Image(systemName: "chevron.right")
          .font(.system(size: 12, weight: .semibold))
          .foregroundStyle(model.theme.colorAccent)
          .frame(width: 14)
          .accessibilityHidden(true)
      }

      ZStack(alignment: .leading) {
        // Ghost suggestion: typed prefix invisible, completion dimmed.
        if !model.commandRunning, let suggestion, suggestion.hasPrefix(text),
          suggestion != text, !text.isEmpty {
          (Text(text).foregroundColor(.clear)
            + Text(suggestion.dropFirst(text.count))
            .foregroundColor(.secondary.opacity(0.55)))
            .font(monoFont)
            .lineLimit(1)
            .allowsHitTesting(false)
        }

        TextField(inputPlaceholder, text: $text)
          .textFieldStyle(.plain)
          .font(monoFont)
          .focused($inputFocused)
          .onSubmit(runCurrentCommand)
          .onChange(of: text) { _, newValue in
            if historyIndex == nil || newValue != currentHistoryEntry() {
              historyIndex = nil
            }
            suggestion =
              (newValue.isEmpty || model.commandRunning)
              ? nil : model.onInputSuggestion?(newValue)
          }
          .onKeyPress(.upArrow) { cycleHistory(direction: 1) }
          .onKeyPress(.downArrow) { cycleHistory(direction: -1) }
          .onKeyPress(.tab) { acceptSuggestion() }
          .onKeyPress(.rightArrow) { acceptSuggestionWord() }
          .onKeyPress(.escape) {
            model.onFocusTerminal?()
            return .handled
          }
          .onKeyPress(phases: .down) { press in
            guard press.modifiers.contains(.control),
              press.key == KeyEquivalent("c")
            else { return .ignored }
            model.onSendInterrupt?()
            return .handled
          }
          .accessibilityLabel("Command input")
      }

      if model.commandRunning {
        Button(action: { model.onSendInterrupt?() }) {
          Label("Stop", systemImage: "stop.fill")
            .labelStyle(.iconOnly)
            .font(.system(size: 11, weight: .medium))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Color(nsColor: .systemRed))
        .help("Stop (Ctrl+C)")
      } else if !text.isEmpty {
        Text("⏎ run")
          .font(.system(size: 10))
          .foregroundStyle(.tertiary)
      }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 7)
    .background(
      RoundedRectangle(cornerRadius: 8, style: .continuous)
        .fill(Color.primary.opacity(0.04))
    )
    .overlay(
      RoundedRectangle(cornerRadius: 8, style: .continuous)
        .strokeBorder(
          inputFocused ? model.theme.colorAccent.opacity(0.5) : Color.primary.opacity(0.1),
          lineWidth: 1
        )
    )
  }

  private var inputPlaceholder: String {
    model.commandRunning ? "Send input to the running command…" : "Run a command…"
  }

  // MARK: - Actions

  private func runCurrentCommand() {
    let command = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !command.isEmpty else { return }
    model.onRunCommand?(command)
    text = ""
    suggestion = nil
    historyIndex = nil
    savedDraft = ""
  }

  private func acceptSuggestion() -> KeyPress.Result {
    guard let suggestion, suggestion.hasPrefix(text), suggestion != text, !text.isEmpty else {
      return .ignored
    }
    text = suggestion
    return .handled
  }

  /// → accepts the next word of the suggestion (up to and including the next
  /// space or `/`). When no suggestion is showing, → moves the cursor normally.
  private func acceptSuggestionWord() -> KeyPress.Result {
    guard let suggestion, suggestion.hasPrefix(text), suggestion != text, !text.isEmpty else {
      return .ignored
    }
    let remainder = Array(suggestion.dropFirst(text.count))
    guard !remainder.isEmpty else { return .ignored }

    let isBoundary: (Character) -> Bool = { $0 == " " || $0 == "/" }
    var end = 0
    if isBoundary(remainder[0]) {
      // Leading boundary: take just it.
      end = 1
    } else {
      while end < remainder.count, !isBoundary(remainder[end]) { end += 1 }
      // Include the trailing boundary so the next press starts a fresh word.
      if end < remainder.count, isBoundary(remainder[end]) { end += 1 }
    }
    text += String(remainder[0..<end])
    return .handled
  }

  /// direction: +1 = older (↑), -1 = newer (↓).
  private func cycleHistory(direction: Int) -> KeyPress.Result {
    let recents = model.onRecentCommands?(50) ?? []
    guard !recents.isEmpty else { return .ignored }

    let next: Int?
    switch (historyIndex, direction) {
    case (nil, 1):
      savedDraft = text
      next = 0
    case (let .some(index), 1):
      next = min(index + 1, recents.count - 1)
    case (let .some(index), -1):
      next = index > 0 ? index - 1 : nil
    default:
      return .ignored
    }

    historyIndex = next
    if let next {
      text = recents[next]
    } else {
      text = savedDraft
    }
    suggestion = nil
    return .handled
  }

  private func currentHistoryEntry() -> String? {
    guard let historyIndex else { return nil }
    let recents = model.onRecentCommands?(50) ?? []
    guard historyIndex < recents.count else { return nil }
    return recents[historyIndex]
  }
}
