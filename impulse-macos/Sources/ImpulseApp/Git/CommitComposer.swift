import AppKit
import ImpulseGit
import SwiftUI

/// Commit message editor + options + commit button, pinned to the bottom of
/// the Changes panel. ⌘↩ commits (and pushes, with the "Commit and push"
/// setting; ⇧⌘↩ does the other one), ↑ in an empty field recalls the
/// previous message, Amend prefills the last commit's message.
struct CommitComposer: View {
  @Environment(\.chrome) private var chrome
  var repository: GitRepositoryState
  let actions: GitActions

  @State private var message = ""
  @State private var amend = false
  @State private var signOff = false
  @State private var skipHooks = false
  @State private var historyIndex = -1
  /// The message ↑ last put in the field.
  @State private var recalledMessage = ""
  @State private var isCommitting = false
  @State private var commitKeys = CommitKeyMonitor()
  @FocusState private var focused: Bool

  private var subject: String {
    message.split(separator: "\n", omittingEmptySubsequences: false).first.map(String.init) ?? ""
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      ZStack(alignment: .topLeading) {
        if message.isEmpty {
          Text(amend ? "Amend message (empty keeps the current one)" : "Commit message")
            .font(ChromeFont.ui(12))
            .foregroundStyle(chrome.textTertiary)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .allowsHitTesting(false)
        }
        TextEditor(text: $message)
          .font(ChromeFont.ui(12))
          .foregroundStyle(chrome.text)
          .scrollContentBackground(.hidden)
          .focused($focused)
          .onChange(of: message) { _, new in
            // Editing a recalled message makes it the user's own: ↑/↓
            // move the caret again instead of swapping it out.
            if historyIndex >= 0, new != recalledMessage { historyIndex = -1 }
          }
          .frame(minHeight: 40, maxHeight: 120)
          .fixedSize(horizontal: false, vertical: true)
          .onKeyPress(.return, phases: .down) { press in
            guard press.modifiers.contains(.command) else { return .ignored }
            commit(thenPush: pushesByDefault != press.modifiers.contains(.shift))
            return .handled
          }
          // ⇧⌘↩ is also View ▸ Zoom Pane, and the menu would take it before
          // the field sees it: while the field has focus, ⌘↩ and ⇧⌘↩ are
          // caught first.
          .onChange(of: focused) { _, isFocused in
            if isFocused {
              commitKeys.start { shift in commit(thenPush: pushesByDefault != shift) }
            } else {
              commitKeys.stop()
            }
          }
          // The monitor's action holds this view's repository: when a
          // Scratch workspace's repository changes under the focused field,
          // restart it so the keys commit in the new one.
          .onChange(of: repository.root) { _, _ in
            guard focused else { return }
            commitKeys.start { shift in commit(thenPush: pushesByDefault != shift) }
          }
          .onDisappear { commitKeys.stop() }
          .onKeyPress(.upArrow) {
            guard message.isEmpty || historyIndex >= 0 else { return .ignored }
            recallHistory(step: 1)
            return .handled
          }
          .onKeyPress(.downArrow) {
            guard historyIndex >= 0 else { return .ignored }
            recallHistory(step: -1)
            return .handled
          }
          .accessibilityLabel("Commit message")
      }
      .padding(6)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous).fill(chrome.content)
      )
      .overlay(
        RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous)
          .strokeBorder(focused ? chrome.focusRing : chrome.hairlineStrong, lineWidth: 1)
      )
      .overlay(alignment: .topTrailing) {
        if subject.count > 0 {
          Text("\(subject.count)")
            .font(ChromeFont.mono(10))
            .foregroundStyle(subject.count > 72 ? chrome.danger : subject.count > 50 ? chrome.warning : chrome.textTertiary)
            .padding(5)
            .help("Subject length (aim for ≤ 50, wrap at 72)")
        }
      }

      HStack(spacing: 10) {
        toggle("Amend", isOn: $amend)
          .onChange(of: amend) { _, isOn in
            guard isOn, message.isEmpty else { return }
            let root = repository.root
            DispatchQueue.global(qos: .userInitiated).async {
              let previous = GitOperations.headMessage(root: root) ?? ""
              DispatchQueue.main.async { if message.isEmpty { message = previous } }
            }
          }
        toggle("Sign-off", isOn: $signOff)
        toggle("Skip hooks", isOn: $skipHooks)
        Spacer(minLength: 0)
      }

      HStack(spacing: 6) {
        commitButton
        ChromeMenuButton(help: "More commit options") {
          [
            ChromeMenuItem("Commit") { commit(thenPush: false) },
            ChromeMenuItem("Commit & Push") { commit(thenPush: true) },
            .separator,
            ChromeMenuItem("Amend Last Commit") {
              amend = true
              commit(thenPush: false)
            },
          ]
        } label: {
          Icon(.chevronDown, size: 12).foregroundStyle(chrome.textSecondary)
        }
        Spacer(minLength: 0)
        KeyHint("⌘↩")
      }
    }
    .padding(10)
  }

  /// The button and ⌘↩ push too (the "Commit and push" setting). Amending
  /// never pushes by default: a pushed commit would need a force push.
  private var pushesByDefault: Bool {
    SettingsStore.shared.settings.gitCommitAndPush && !amend
  }

  private var commitButton: some View {
    let staged = repository.snapshot?.staged.count ?? 0
    let push = pushesByDefault
    let title: String = {
      if isCommitting { return push ? "Committing & Pushing…" : "Committing…" }
      if amend { return "Amend" }
      let files = staged > 0 ? " \(staged) file\(staged == 1 ? "" : "s")" : ""
      return push ? "Commit\(files) & Push" : "Commit\(files)"
    }()
    let help = push ? "Commit and push (⌘↩) · ⇧⌘↩ only commits" : "Commit (⌘↩) · ⇧⌘↩ commits and pushes"
    return ChromeButton(title: title, icon: push ? .arrowUp : .check, kind: .primary, help: help) {
      commit(thenPush: push)
    }
    .disabled(isCommitting)
  }

  private func toggle(_ title: String, isOn: Binding<Bool>) -> some View {
    Button {
      isOn.wrappedValue.toggle()
    } label: {
      HStack(spacing: 4) {
        Icon(isOn.wrappedValue ? .circleCheck : .circle, size: 12)
          .foregroundStyle(isOn.wrappedValue ? chrome.accent : chrome.textTertiary)
        Text(title).font(ChromeFont.ui(11)).foregroundStyle(chrome.textSecondary)
      }
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .accessibilityAddTraits(isOn.wrappedValue ? [.isSelected] : [])
  }

  private func commit(thenPush: Bool) {
    guard !isCommitting else { return }
    isCommitting = true
    actions.commit(
      message: message, amend: amend, signOff: signOff, noVerify: skipHooks, thenPush: thenPush
    ) { success in
      isCommitting = false
      guard success else { return }
      message = ""
      amend = false
      historyIndex = -1
    }
  }

  private func recallHistory(step: Int) {
    let history = CommitMessageHistory.messages(root: repository.root)
    guard !history.isEmpty else { return }
    let next = historyIndex + step
    if next < 0 {
      historyIndex = -1
      message = ""
    } else if next < history.count {
      historyIndex = next
      recalledMessage = history[next]
      message = history[next]
    }
  }
}

/// Catches ⌘↩ / ⇧⌘↩ for the commit message field before menu key
/// equivalents (⇧⌘↩ is Zoom Pane), while that field is the first responder.
final class CommitKeyMonitor {
  private var monitor: Any?
  private weak var field: NSResponder?
  /// Bumped by every start and stop, so a start that's overtaken does
  /// nothing.
  private var generation = 0

  /// `commit(shift)` runs for ⌘↩ (false) and ⇧⌘↩ (true).
  func start(commit: @escaping (Bool) -> Void) {
    stop()
    let started = generation
    // The text view becomes first responder before SwiftUI reports focus;
    // read it once that's settled.
    DispatchQueue.main.async { [weak self] in
      guard let self, self.generation == started,
        let field = NSApp.keyWindow?.firstResponder as? NSTextView
      else { return }
      self.field = field
      self.monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
        let flags = event.modifierFlags.intersection([.command, .shift, .option, .control])
        guard let field = self?.field, event.window?.firstResponder === field,
          event.keyCode == 36 || event.keyCode == 76,  // Return, keypad Enter
          flags == .command || flags == [.command, .shift]
        else { return event }
        commit(flags.contains(.shift))
        return nil
      }
    }
  }

  func stop() {
    generation += 1
    if let monitor { NSEvent.removeMonitor(monitor) }
    monitor = nil
    field = nil
  }

  deinit { stop() }
}
