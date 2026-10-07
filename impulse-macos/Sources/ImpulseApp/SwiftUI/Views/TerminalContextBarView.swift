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
  /// The password field's focus (the command editor reports its own).
  @FocusState private var secureFocused: Bool
  @State private var editorFocused = false
  /// Bumped to put keyboard focus in the input (editor or password field).
  @State private var focusRequest = 0
  private var inputFocused: Bool { editorFocused || secureFocused }

  /// Move keyboard focus into whichever field is showing.
  private func focusInput() {
    if model.passwordInputActive {
      secureFocused = true
    } else {
      focusRequest += 1
    }
  }

  // MARK: Completion dropdown state
  /// Candidates for the active argument token. Non-empty == dropdown open.
  @State private var completions: [CompletionCandidate] = []
  /// The input byte range the accepted candidate replaces.
  @State private var completionSpan: TextSpan? = nil
  /// Highlighted candidate (drives ↑/↓ and Enter/Tab accept). nil == none.
  @State private var selectedIndex: Int? = nil
  /// Bumped on each request so stale off-main results can be discarded.
  @State private var completionGeneration: Int = 0
  /// Drops ghost suggestions computed for text that has since changed.
  @State private var suggestionGeneration: Int = 0
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
        focusInput()
      }
    }
    .onChange(of: model.inputBarFocusToken) {
      focusInput()
    }
    .onAppear {
      if !model.passwordInputActive { text = model.inputDraft }
    }
    .onChange(of: model.inputDraftRestoreToken) {
      // The bar moved to another terminal: show that terminal's draft.
      text = model.passwordInputActive ? "" : model.inputDraft
      suggestion = nil
      historyIndex = nil
      savedDraft = ""
      closeDropdown()
    }
    .onChange(of: model.passwordInputActive) {
      // Entering password mode: a half-typed command draft must not be sent
      // as (part of) the password. Leaving it: a half-typed password must not
      // become visible in the plain field. Drop the draft on both flips.
      text = ""
      suggestion = nil
      historyIndex = nil
      savedDraft = ""
      closeDropdown()
      // The flip swaps the plain field for the secure one (or back), tearing
      // down the focused NSTextField — AppKit drops first responder after
      // this update, so a same-transaction focus write loses the race.
      // Re-grab focus once the responder churn has settled.
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) {
        focusInput()
      }
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
    .onAppear { focusInput() }
    .onDisappear { completionPanel.hide() }
  }

  // MARK: - Context chips

  private var chipRow: some View {
    HStack(spacing: 6) {
      // In a narrow split pane, chips drop off the trailing end (whole, not
      // cut in half) instead of forcing the bar wider than the pane.
      ViewThatFits(in: .horizontal) {
        chips(limit: 5)
        chips(limit: 4)
        chips(limit: 3)
        chips(limit: 2)
        chips(limit: 1)
      }
      .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
      .clipped()
      actionButton(symbol: "clock.arrow.circlepath", help: "Command History (⌃R)") {
        model.onShowCommandHistory?()
      }
    }
  }

  private enum Chip {
    case shell, cwd, branch(String), review, status
  }

  private var availableChips: [Chip] {
    var chips: [Chip] = []
    if !model.shellName.isEmpty { chips.append(.shell) }
    if !model.currentCwd.isEmpty { chips.append(.cwd) }
    if let branch = model.gitBranch, !branch.isEmpty { chips.append(.branch(branch)) }
    if model.reviewChangedFileCount > 0 { chips.append(.review) }
    if !model.commandRunning, model.lastCommandExitCode != nil { chips.append(.status) }
    return chips
  }

  /// The first `limit` chips, at their natural width.
  private func chips(limit: Int) -> some View {
    let chips = Array(availableChips.prefix(limit))
    return HStack(spacing: 6) {
      ForEach(chips.indices, id: \.self) { index in
        chip(chips[index])
      }
    }
    .fixedSize()
  }

  @ViewBuilder
  private func chip(_ chip: Chip) -> some View {
    switch chip {
    case .shell:
      ContextChip(symbol: "terminal", text: model.shellName, theme: model.theme)
    case .cwd:
      ContextChip(
        symbol: "folder", text: TabManager.abbreviateHomePath(model.currentCwd),
        theme: model.theme)
    case .branch(let branch):
      BranchChip(model: model, branch: branch)
    case .review:
      ReviewChip(
        model: model,
        fileCount: model.reviewChangedFileCount,
        added: model.reviewAddedLines,
        removed: model.reviewRemovedLines)
    case .status:
      statusChip
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
      if model.passwordInputActive {
        Image(systemName: "lock.fill")
          .font(.system(size: 12, weight: .semibold))
          .foregroundStyle(model.theme.colorYellow)
          .frame(width: 14)
          .accessibilityHidden(true)
      } else if model.commandRunning {
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
        if model.passwordInputActive {
          // The running program disabled terminal echo (sudo, ssh, `read -s`):
          // mask keystrokes so the password is never rendered. No suggestions,
          // history, or completions apply — the text is a secret, not a command.
          SecureField(inputPlaceholder, text: $text)
            .textFieldStyle(.plain)
            .font(monoFont)
            .foregroundStyle(model.theme.colorFg)
            .tint(model.theme.colorAccent)
            .focused($secureFocused)
            .onSubmit(handleSubmit)
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
            .accessibilityLabel("Password input")
        } else {
          CommandEditor(
            text: $text,
            placeholder: inputPlaceholder,
            suggestion: model.commandRunning ? nil : suggestion,
            colors: CommandEditorColors(theme: model.theme),
            font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
            isKnownCommand: model.onIsKnownCommand,
            focusToken: focusRequest,
            onSubmit: handleSubmit,
            onKey: handleEditorKey,
            onFocusChange: { editorFocused = $0 },
            onMouseDown: { model.onInputBarClicked?() }
          )
          .onChange(of: model.completionRequestToken) { _, _ in requestCompletionsFromTab() }
          .onChange(of: text) { _, newValue in
            model.inputDraft = newValue
            if historyIndex == nil || newValue != currentHistoryEntry() {
              historyIndex = nil
            }
            updateSuggestion(for: newValue)
            // Tab-only dropdown: typing never opens it. While it's already
            // open, re-fetch so the list narrows/widens to the new prefix.
            if isDropdownOpen {
              scheduleCompletions(for: newValue)
            }
          }
        }
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
    if model.passwordInputActive { return "Password (input hidden)…" }
    return model.commandRunning ? "Send input to the running command…" : "Run a command…"
  }

  // MARK: - Actions

  /// Enter handler: accept the highlighted candidate when the dropdown is open,
  /// otherwise run the typed command.
  private func handleSubmit() {
    if model.passwordInputActive {
      // Password replies go through verbatim: no trimming (passwords may
      // carry whitespace) and an empty line is a valid empty password.
      model.onSendSecureInput?(text)
      text = ""
      return
    }
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

  /// Tab with the dropdown closed. Candidates are fetched off the main thread
  /// (branch values run git; fish completions run fish), then: two or more
  /// open the dropdown, a lone one is inserted like a shell's Tab, and with
  /// none (or a visible ghost suggestion and one candidate) the suggestion is
  /// accepted.
  private func requestCompletionsFromTab() {
    guard !text.isEmpty, !model.commandRunning, let resolve = model.onCompletionResolver?() else {
      _ = acceptSuggestion()
      return
    }
    completionGeneration &+= 1
    let generation = completionGeneration
    let input = text
    DispatchQueue.global(qos: .userInitiated).async {
      let result = resolve(input)
      DispatchQueue.main.async {
        // Typing (or another Tab) since makes this answer stale.
        guard generation == completionGeneration, text == input else { return }
        applyTabCompletions(result)
      }
    }
  }

  private func applyTabCompletions(_ result: CompletionResult?) {
    guard let result, !result.candidates.isEmpty else {
      _ = acceptSuggestion()
      return
    }
    if result.candidates.count == 1 {
      // A visible history suggestion keeps Tab.
      if let suggestion, suggestion.hasPrefix(text), suggestion != text {
        _ = acceptSuggestion()
        return
      }
      completions = result.candidates
      completionSpan = result.span
      selectedIndex = 0
      acceptCompletion()
      return
    }
    completions = result.candidates
    completionSpan = result.span
    completionPrefix = matchedPrefix(in: text, span: result.span)
    selectedIndex = 0
    refreshPanel()
  }

  /// The ghost suggestion: a history continuation right away; otherwise
  /// word and path completion, which read the filesystem, off the main
  /// thread. Meanwhile a suggestion the typing still follows stays up.
  private func updateSuggestion(for input: String) {
    suggestionGeneration &+= 1
    guard !input.isEmpty, !model.commandRunning else {
      suggestion = nil
      return
    }
    if let fromHistory = model.onInputSuggestion?(input) {
      suggestion = fromHistory
      return
    }
    if let current = suggestion, !(current.hasPrefix(input) && current != input) {
      suggestion = nil
    }
    guard let resolve = model.onSuggestionResolver?() else { return }
    let generation = suggestionGeneration
    DispatchQueue.global(qos: .userInitiated).async {
      let result = resolve(input)
      DispatchQueue.main.async {
        guard generation == suggestionGeneration, text == input else { return }
        suggestion = result
      }
    }
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

    // Read the terminal's state now, on the main thread; only the resolving
    // runs in the background.
    guard let resolve = model.onCompletionResolver?() else { return }

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

  /// Keys the command editor offers before handling them itself.
  private func handleEditorKey(_ key: CommandEditorKey) -> Bool {
    switch key {
    case .up:
      // When the dropdown is open, ↑ moves the highlight (wrap); else it
      // cycles command history.
      if isDropdownOpen {
        moveSelection(by: -1)
        return true
      }
      return cycleHistory(direction: 1) == .handled
    case .down:
      if isDropdownOpen {
        moveSelection(by: 1)
        return true
      }
      return cycleHistory(direction: -1) == .handled
    case .tab:
      // Tab is the dropdown trigger. When it's already open, Tab accepts the
      // highlighted candidate. When closed, try to open it (two or more
      // candidates); otherwise accept the inline ghost suggestion.
      if isDropdownOpen {
        acceptCompletion()
      } else {
        requestCompletionsFromTab()
      }
      return true
    case .right:
      return acceptSuggestionWord() == .handled
    case .escape:
      // First Esc closes the dropdown; a second moves focus into the grid.
      if isDropdownOpen {
        closeDropdown()
      } else {
        model.onFocusTerminal?()
      }
      return true
    case .controlC:
      model.onSendInterrupt?()
      return true
    case .controlR:
      // Reverse history search, like the shell's own Ctrl-R.
      model.onShowCommandHistory?()
      return true
    case .alternateSubmit:
      return false
    case .selectBlock:
      return model.onSelectBlocks?() ?? false
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
