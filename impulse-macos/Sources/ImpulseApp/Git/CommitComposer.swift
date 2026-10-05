import AppKit
import ImpulseGit
import SwiftUI

/// Commit message editor + options + commit button, pinned to the bottom of
/// the Changes panel. ⌘↩ commits, ↑ in an empty field recalls the previous
/// message, Amend prefills the last commit's message.
struct CommitComposer: View {
  @Environment(\.chrome) private var chrome
  var repository: GitRepositoryState
  let actions: GitActions

  @State private var message = ""
  @State private var amend = false
  @State private var signOff = false
  @State private var skipHooks = false
  @State private var historyIndex = -1
  @State private var isCommitting = false
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
          .frame(minHeight: 40, maxHeight: 120)
          .fixedSize(horizontal: false, vertical: true)
          .onKeyPress(.return, phases: .down) { press in
            guard press.modifiers.contains(.command) else { return .ignored }
            commit(thenPush: press.modifiers.contains(.shift))
            return .handled
          }
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
        Menu {
          Button("Commit") { commit(thenPush: false) }
          Button("Commit & Push") { commit(thenPush: true) }
          Divider()
          Button("Amend Last Commit") {
            amend = true
            commit(thenPush: false)
          }
        } label: {
          Icon(.chevronDown, size: 12)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("More commit options")
        Spacer(minLength: 0)
        KeyHint("⌘↩")
      }
    }
    .padding(10)
  }

  private var commitButton: some View {
    let staged = repository.snapshot?.staged.count ?? 0
    let title: String = {
      if isCommitting { return "Committing…" }
      if amend { return "Amend" }
      return staged > 0 ? "Commit \(staged) file\(staged == 1 ? "" : "s")" : "Commit"
    }()
    return ChromeButton(title: title, icon: .check, kind: .primary, help: "Commit (⌘↩) · ⇧⌘↩ commits and pushes") {
      commit(thenPush: false)
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
      message = history[next]
    }
  }
}
