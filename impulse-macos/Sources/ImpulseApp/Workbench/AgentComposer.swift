import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// ⌘I over a terminal program (an agent's TUI): write a prompt with a real
/// editor — several lines, @file mentions from the workspace — and hand it
/// to the program as a paste. ⌘↩ sends and presses Return, ⌥⌘↩ only
/// pastes, Esc puts focus back in the program.
struct AgentComposerView: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  @State private var text = ""
  @State private var focusRequest = 0
  @State private var mentions: [String] = []
  @State private var selectedMention = 0
  @State private var fileIndex: (root: String, files: [String])?

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      if !mentions.isEmpty {
        mentionList
      }
      HStack(spacing: 6) {
        Icon(.bot, size: 12).foregroundStyle(chrome.accent)
        Text(model.composerTarget.isEmpty ? "Message" : "Message \(model.composerTarget)")
          .font(ChromeFont.ui(11, weight: .medium))
          .foregroundStyle(chrome.textSecondary)
        Spacer()
        KeyHint("⌘↩").help("Send")
        Text("send").font(ChromeFont.ui(10.5)).foregroundStyle(chrome.textTertiary)
        KeyHint("⌥⌘↩").help("Paste without sending")
        Text("paste").font(ChromeFont.ui(10.5)).foregroundStyle(chrome.textTertiary)
        KeyHint("esc")
      }
      CommandEditor(
        text: $text,
        placeholder: "Describe the change… (@ mentions a file)",
        suggestion: nil,
        colors: CommandEditorColors(theme: model.theme),
        font: NSFont.systemFont(ofSize: 13),
        shellSyntax: false,
        returnSubmits: false,
        focusToken: focusRequest + model.composerFocusToken,
        onSubmit: { send(submit: true) },
        onKey: handleKey
      )
      .padding(8)
      .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(model.theme.colorBg))
      .overlay(
        RoundedRectangle(cornerRadius: 8, style: .continuous)
          .strokeBorder(chrome.accent.opacity(0.6), lineWidth: 1))
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .background(model.theme.colorBgDark)
    .overlay(alignment: .top) { Rectangle().fill(model.theme.colorBorder).frame(height: 1) }
    .onAppear {
      text = model.composerDraft
      focusRequest += 1
    }
    .onChange(of: text) { _, newValue in
      model.composerDraft = newValue
      updateMentions(for: newValue)
    }
  }

  private var mentionList: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(mentions.enumerated()), id: \.element) { index, path in
        HStack(spacing: 6) {
          Icon(.file, size: 11).foregroundStyle(chrome.textTertiary)
          Text(path).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.text).lineLimit(1)
            .truncationMode(.middle)
          Spacer()
        }
        .padding(.horizontal, 8)
        .frame(height: 22)
        .background(
          RoundedRectangle(cornerRadius: 4).fill(index == selectedMention ? chrome.selection : .clear))
        .contentShape(Rectangle())
        .onTapGesture { acceptMention(path) }
      }
    }
    .padding(4)
    .background(RoundedRectangle(cornerRadius: 6).fill(chrome.raised))
  }

  private func handleKey(_ key: CommandEditorKey) -> Bool {
    switch key {
    case .alternateSubmit:
      send(submit: false)
      return true
    case .escape:
      if !mentions.isEmpty {
        mentions = []
      } else {
        model.onCloseComposer?()
      }
      return true
    case .up where !mentions.isEmpty:
      selectedMention = (selectedMention - 1 + mentions.count) % mentions.count
      return true
    case .down where !mentions.isEmpty:
      selectedMention = (selectedMention + 1) % mentions.count
      return true
    case .tab:
      if mentions.indices.contains(selectedMention) { acceptMention(mentions[selectedMention]) }
      return true
    case .controlC:
      model.onSendInterrupt?()
      return true
    default:
      return false
    }
  }

  private func send(submit: Bool) {
    let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !message.isEmpty else { return }
    model.onComposerSend?(message, submit)
    text = ""
    model.composerDraft = ""
  }

  // MARK: @ mentions

  /// The word being typed, when it starts with "@".
  private func mentionQuery(in text: String) -> String? {
    guard let last = text.split(separator: " ", omittingEmptySubsequences: false).last,
      last.hasPrefix("@"), !last.contains("\n")
    else { return nil }
    return String(last.dropFirst())
  }

  private func updateMentions(for text: String) {
    guard let query = mentionQuery(in: text) else {
      mentions = []
      return
    }
    let root = model.fileTreeRootPath
    guard !root.isEmpty else { return }
    if fileIndex?.root != root {
      fileIndex = (root, [])
      DispatchQueue.global(qos: .userInitiated).async {
        let files = FileIndex.files(root: root, limit: 50_000)
        DispatchQueue.main.async {
          fileIndex = (root, files)
          updateMentions(for: self.text)
        }
      }
      return
    }
    let files = fileIndex?.files ?? []
    mentions = FuzzyMatcher.rank(files, query: query, limit: 6, isPath: true) { $0 }.map(\.item)
    selectedMention = 0
  }

  private func acceptMention(_ path: String) {
    guard let query = mentionQuery(in: text) else { return }
    text = String(text.dropLast(query.count + 1)) + "@" + path + " "
    mentions = []
  }
}
