import AppKit
import SwiftUI

/// Warp-style input area pinned below the terminal: a context-chip row
/// (shell, cwd, git branch, last command status) above a monospaced command
/// input with history ghost suggestions. Enter runs the command in the
/// active terminal; ↑/↓ cycle history; Tab opens/accepts path completions or
/// accepts the inline suggestion; Esc moves focus into the terminal grid.
/// While a command runs the input swaps to a running indicator with a Stop
/// button. Native materials and SF Symbols keep it reading as macOS chrome.
struct TerminalContextBarView: View {
  var model: WindowModel

  @State private var text: String = ""
  @State private var suggestion: String? = nil
  /// Index into the recent-history list while cycling with ↑/↓; nil = live draft.
  @State private var historyIndex: Int? = nil
  @State private var savedDraft: String = ""
  @FocusState private var inputFocused: Bool

  // MARK: Completion dropdown state
  /// Candidates for the active argument token. Non-empty == dropdown open.
  @State private var completions: [CompletionCandidate] = []
  /// The input byte range the accepted candidate replaces.
  @State private var completionSpan: TextSpan? = nil
  /// Highlighted candidate (drives ↑/↓ and Enter/Tab accept). nil == none.
  @State private var selectedIndex: Int? = nil
  /// Bumped on each request so stale off-main results can be discarded.
  @State private var completionGeneration: Int = 0
  /// The typed basename prefix for the active token (matched-prefix emphasis).
  @State private var completionPrefix: String = ""

  // MARK: Completion panel (floating dropdown)
  /// The borderless child-window dropdown. Created once, shown/hidden on demand.
  @State private var completionPanel = CompletionPanel()
  /// Latest screen rect + window of the input field, tracked by the anchor view
  /// so the panel can be positioned above it.
  @State private var anchorScreenRect: NSRect = .zero
  @State private var anchorWindow: NSWindow? = nil

  /// The dropdown is open exactly when there are candidates to show.
  private var isDropdownOpen: Bool { !completions.isEmpty }

  private var monoFont: Font { .system(size: 13, design: .monospaced) }

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      chipRow
      inputRow
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .background(model.theme.colorBgDark)
    .overlay(alignment: .top) {
      Rectangle().fill(model.theme.colorBorder).frame(height: 1)
    }
    .onChange(of: model.commandRunning) { _, running in
      if running {
        // A command started — the prompt is busy; close the path dropdown.
        closeDropdown()
      } else {
        // Shell returned to the prompt — reclaim focus for the next command.
        inputFocused = true
      }
    }
    .onChange(of: model.inputBarFocusToken) {
      inputFocused = true
    }
    .onChange(of: inputFocused) { _, focused in
      // Focus loss (clicking the grid, a sheet, another tab) dismisses the
      // dropdown so it never lingers detached from an editable field.
      if !focused { closeDropdown() }
    }
    .onChange(of: model.theme.id) {
      // Re-render the hosted list with the new theme colors while open.
      refreshPanel()
    }
    .onReceive(
      NotificationCenter.default.publisher(for: NSWindow.didResizeNotification)
    ) { note in
      // Only the input field's OWN window dismisses the dropdown. The completion
      // panel posts its own move/resize notifications when we position it; if we
      // reacted to those, the first Tab would open the panel and instantly
      // self-close (it survived only on the second Tab, when the frame was
      // unchanged and no notification fired).
      if isDropdownOpen, (note.object as? NSWindow) === anchorWindow { closeDropdown() }
    }
    .onReceive(
      NotificationCenter.default.publisher(for: NSWindow.didMoveNotification)
    ) { note in
      if isDropdownOpen, (note.object as? NSWindow) === anchorWindow { closeDropdown() }
    }
    .onAppear { inputFocused = true }
    .onDisappear { completionPanel.hide() }
  }

  // MARK: - Context chips

  private var chipRow: some View {
    HStack(spacing: 6) {
      if !model.shellName.isEmpty {
        ContextChip(symbol: "terminal", text: model.shellName, theme: model.theme)
      }
      if !model.currentCwd.isEmpty {
        ContextChip(
          symbol: "folder", text: TabManager.abbreviateHomePath(model.currentCwd),
          theme: model.theme)
      }
      if let branch = model.gitBranch, !branch.isEmpty {
        BranchChip(model: model, branch: branch)
      }
      if model.reviewChangedFileCount > 0 {
        ReviewChip(
          model: model,
          fileCount: model.reviewChangedFileCount,
          added: model.reviewAddedLines,
          removed: model.reviewRemovedLines)
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
          .foregroundStyle(failed ? model.theme.colorRed : model.theme.colorGreen)
        Text(statusText(exitCode: exitCode))
          .font(.system(size: 11, design: .monospaced))
          .foregroundStyle(model.theme.colorFgMuted)
      }
      .padding(.horizontal, 8)
      .padding(.vertical, 3)
      .background(Capsule().fill(model.theme.colorFg.opacity(0.07)))
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
        .foregroundStyle(model.theme.colorFgMuted)
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
            .foregroundColor(model.theme.colorFgComment))
            .font(monoFont)
            .lineLimit(1)
            .allowsHitTesting(false)
        }

        TextField(inputPlaceholder, text: $text)
          .textFieldStyle(.plain)
          .font(monoFont)
          .foregroundStyle(model.theme.colorFg)
          .tint(model.theme.colorAccent)
          .focused($inputFocused)
          .onSubmit(handleSubmit)
          .onChange(of: text) { _, newValue in
            if historyIndex == nil || newValue != currentHistoryEntry() {
              historyIndex = nil
            }
            suggestion =
              (newValue.isEmpty || model.commandRunning)
              ? nil : model.onInputSuggestion?(newValue)
            // Tab-only dropdown: typing never opens it. While it's already
            // open, re-fetch so the list narrows/widens to the new prefix.
            if isDropdownOpen {
              scheduleCompletions(for: newValue)
            }
          }
          .onKeyPress(.upArrow) {
            // When the dropdown is open, ↑ moves the highlight (wrap); else it
            // cycles command history.
            if isDropdownOpen {
              moveSelection(by: -1)
              return .handled
            }
            return cycleHistory(direction: 1)
          }
          .onKeyPress(.downArrow) {
            if isDropdownOpen {
              moveSelection(by: 1)
              return .handled
            }
            return cycleHistory(direction: -1)
          }
          .onKeyPress(.tab) {
            // Tab is the dropdown trigger. When it's already open, Tab accepts
            // the highlighted candidate. When closed, try to open it: fetch
            // candidates and, with two or more, show the panel (row 0 selected).
            // With zero/one candidate, fall back to accepting the inline ghost
            // suggestion. Always swallow Tab so it never triggers focus
            // traversal.
            if isDropdownOpen {
              acceptCompletion()
              return .handled
            }
            if openCompletionsFromTab() {
              return .handled
            }
            _ = acceptSuggestion()
            return .handled
          }
          .onKeyPress(.rightArrow) { acceptSuggestionWord() }
          .onKeyPress(.escape) {
            // First Esc closes the dropdown; a second Esc (dropdown already
            // closed) moves focus into the terminal grid.
            if isDropdownOpen {
              closeDropdown()
              return .handled
            }
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
      // Tracks the input field's screen rect so the floating completion panel
      // can anchor itself just above the field. Transparent / non-interactive.
      .overlay(
        CompletionAnchorView { rect, window in
          anchorScreenRect = rect
          anchorWindow = window
        }
        .allowsHitTesting(false)
      )

      if model.commandRunning {
        Button(action: { model.onSendInterrupt?() }) {
          Label("Stop", systemImage: "stop.fill")
            .labelStyle(.iconOnly)
            .font(.system(size: 11, weight: .medium))
        }
        .buttonStyle(.plain)
        .foregroundStyle(model.theme.colorRed)
        .help("Stop (Ctrl+C)")
      } else if !text.isEmpty {
        Text("⏎ run")
          .font(.system(size: 10))
          .foregroundStyle(model.theme.colorFgComment)
      }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 7)
    .background(
      RoundedRectangle(cornerRadius: 8, style: .continuous)
        .fill(model.theme.colorBg)
    )
    .overlay(
      RoundedRectangle(cornerRadius: 8, style: .continuous)
        .strokeBorder(
          inputFocused ? model.theme.colorAccent.opacity(0.6) : model.theme.colorBorder,
          lineWidth: 1
        )
    )
  }

  private var inputPlaceholder: String {
    model.commandRunning ? "Send input to the running command…" : "Run a command…"
  }

  // MARK: - Actions

  /// Enter handler: accept the highlighted candidate when the dropdown is open,
  /// otherwise run the typed command.
  private func handleSubmit() {
    if isDropdownOpen {
      // Enter commits the highlighted candidate and closes the dropdown (it
      // stays closed until Tab re-opens it) — unlike Tab, it does not drill into
      // a directory's contents.
      acceptCompletion(reopenForDirectory: false)
      return
    }
    runCurrentCommand()
  }

  private func runCurrentCommand() {
    let command = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !command.isEmpty else { return }
    model.onRunCommand?(command)
    text = ""
    suggestion = nil
    historyIndex = nil
    savedDraft = ""
    closeDropdown()
  }

  // MARK: - Path completion dropdown

  /// Clears all dropdown state and hides the floating panel.
  private func closeDropdown() {
    completions = []
    completionSpan = nil
    completionPrefix = ""
    selectedIndex = nil
    // Bump the generation so any in-flight fetch is discarded when it returns.
    completionGeneration &+= 1
    completionPanel.hide()
  }

  /// Moves the highlighted candidate by `delta` rows, wrapping at the ends.
  private func moveSelection(by delta: Int) {
    guard !completions.isEmpty else { return }
    let count = completions.count
    let current = selectedIndex ?? 0
    selectedIndex = ((current + delta) % count + count) % count
    refreshPanel()
  }

  /// Tab trigger: synchronously fetch candidates for the current input and open
  /// the dropdown when there are two or more. Returns true when the dropdown
  /// opened (so the caller swallows the keypress instead of accepting the
  /// inline suggestion). A single/zero-candidate result leaves the dropdown
  /// closed and returns false.
  private func openCompletionsFromTab() -> Bool {
    guard !text.isEmpty, !model.commandRunning,
      let resolve = model.onCompletionCandidates
    else { return false }

    // Invalidate any pending async fetch — this synchronous result wins.
    completionGeneration &+= 1
    guard let result = resolve(text), result.candidates.count >= 2 else {
      return false
    }
    completions = result.candidates
    completionSpan = result.span
    completionPrefix = matchedPrefix(in: text, span: result.span)
    selectedIndex = 0
    refreshPanel()
    return true
  }

  /// Shows or updates the floating panel to reflect the current dropdown state.
  /// Hides it when the dropdown is closed or there is no anchor/window yet.
  private func refreshPanel() {
    guard isDropdownOpen, let window = anchorWindow, anchorScreenRect != .zero else {
      completionPanel.hide()
      return
    }
    let content = CompletionPopupView(
      candidates: completions,
      matchedPrefix: completionPrefix,
      selectedIndex: selectedIndex,
      theme: model.theme,
      iconCache: model.iconCache,
      onSelect: { index in
        selectedIndex = index
        acceptCompletion()
      }
    )
    completionPanel.show(
      content: content,
      anchorScreenRect: anchorScreenRect,
      height: CompletionPopupView.listHeight(for: completions.count),
      in: window
    )
  }

  /// Debounced (~40ms), off-main path-candidate fetch. Stale results are
  /// dropped via the generation counter. The dropdown opens only when there are
  /// two or more candidates; otherwise it stays closed/clears.
  private func scheduleCompletions(for input: String) {
    completionGeneration &+= 1
    let generation = completionGeneration

    // An empty / running input can't complete a path — close immediately.
    guard !input.isEmpty, !model.commandRunning else {
      closeDropdown()
      return
    }

    guard let resolve = model.onCompletionCandidates else { return }

    DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.04) {
      // Skip if a newer keystroke superseded this request before the debounce
      // window elapsed.
      guard generation == completionGeneration else { return }
      let result = resolve(input)
      DispatchQueue.main.async {
        // Drop stale results: only the latest request may apply.
        guard generation == completionGeneration else { return }
        applyCompletions(result, for: input)
      }
    }
  }

  /// Applies a completion result on the main actor. Keeps the dropdown open only
  /// when there are >= 2 candidates; otherwise closes it (and the panel).
  private func applyCompletions(_ result: CompletionResult?, for input: String) {
    guard let result, result.candidates.count >= 2 else {
      closeDropdown()
      return
    }
    completions = result.candidates
    completionSpan = result.span
    completionPrefix = matchedPrefix(in: input, span: result.span)
    selectedIndex = 0
    refreshPanel()
  }

  /// Extracts the typed basename prefix for the active token (the text after
  /// the last path separator within the span) so the dropdown can emphasize the
  /// matched leading characters of each candidate.
  private func matchedPrefix(in input: String, span: TextSpan) -> String {
    let bytes = Array(input.utf8)
    guard span.start >= 0, span.end <= bytes.count, span.start <= span.end else { return "" }
    let tokenBytes = Array(bytes[span.start..<span.end])
    let token = String(decoding: tokenBytes, as: UTF8.self)
    if let slash = token.lastIndex(of: "/") {
      return String(token[token.index(after: slash)...])
    }
    return token
  }

  /// Accepts the highlighted candidate: splice its value over the active token
  /// span (UTF-8 byte range). Directory candidates already carry a trailing
  /// "/". When `reopenForDirectory` is true (Tab / click) the dropdown re-runs
  /// for the next path segment; when false (Enter) it closes instead. File
  /// candidates get a trailing space and the dropdown always closes.
  private func acceptCompletion(reopenForDirectory: Bool = true) {
    guard let span = completionSpan,
      let index = selectedIndex,
      completions.indices.contains(index)
    else { return }
    let candidate = completions[index]

    let bytes = Array(text.utf8)
    guard span.start >= 0, span.end <= bytes.count, span.start <= span.end else {
      closeDropdown()
      return
    }

    // The candidate's value already contains any trailing "/" for directories.
    // Shell-escape it so names with spaces are usable, preserving the trailing
    // slash (which `shellEscaped` keeps when present in the safe set / quotes).
    let replacement = candidate.value.shellEscaped

    let prefix = String(decoding: bytes[0..<span.start], as: UTF8.self)
    let suffix = String(decoding: bytes[span.end...], as: UTF8.self)

    if candidate.isDir {
      let newText = prefix + replacement + suffix
      text = newText
      suggestion = nil
      historyIndex = nil
      if reopenForDirectory {
        // Tab/click: keep drilling. The replacement ends in "/", so the active
        // token becomes the empty next segment.
        scheduleCompletions(for: newText)
      } else {
        // Enter: commit and stop.
        closeDropdown()
      }
    } else {
      // Files: append a trailing space and close.
      text = prefix + replacement + " " + suffix
      suggestion = nil
      historyIndex = nil
      closeDropdown()
    }
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
