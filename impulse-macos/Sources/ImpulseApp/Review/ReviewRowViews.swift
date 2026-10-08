import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// SwiftUI content for the review's non-code rows.
enum ReviewRowContent {
  static func view(for row: ReviewRow, context: ReviewDiffContext) -> AnyView {
    let chrome = context.colors.palette
    let content: AnyView
    switch row {
    case .fileHeader(let path):
      guard let file = context.files[path] else { return AnyView(EmptyView()) }
      content = AnyView(ReviewFileHeaderRow(info: .init(file), capabilities: context.capabilities, handler: context.handler))
    case .hunkHeader(let path, let hunk):
      guard let file = context.files[path], let diff = file.diff, diff.hunks.indices.contains(hunk) else {
        return AnyView(EmptyView())
      }
      let selected = file.selection.map { $0.hunk == hunk ? $0.lines.count : 0 } ?? 0
      content = AnyView(
        ReviewHunkHeaderRow(
          path: path, hunk: hunk, header: diff.hunks[hunk].header, selectedLines: selected,
          capabilities: context.capabilities, truncated: diff.truncated,
          focused: context.isFocused(path, hunk: hunk), busy: file.busy, handler: context.handler))
    case .notice(_, let notice):
      content = AnyView(ReviewNoticeRow(notice: notice))
    case .outdatedTitle:
      content = AnyView(
        Text("Outdated comments — their lines changed or aren't in this diff")
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
          .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
          .padding(.leading, ReviewMetrics.cardInset + 12)
          .padding(.bottom, 4))
    case .comment(let path, let id):
      guard let file = context.files[path], let comment = file.comments.first(where: { $0.id == id }) else {
        return AnyView(EmptyView())
      }
      content = AnyView(
        ReviewCommentRow(
          comment: comment, outdated: file.outdated.contains(id), editing: file.editingComment == id,
          editingDraft: file.editingComment == id ? file.editingDraft : nil,
          indent: file.outdated.contains(id) ? 12 : ReviewMetrics.commentIndent(context.layout),
          handler: context.handler))
    case .composer(let path):
      guard let composer = context.files[path]?.composer else { return AnyView(EmptyView()) }
      content = AnyView(
        ReviewComposerRow(
          path: path, composer: composer, indent: ReviewMetrics.commentIndent(context.layout),
          handler: context.handler))
    default:
      return AnyView(EmptyView())
    }
    return AnyView(content.environment(\.chrome, chrome))
  }
}

// MARK: - File header

struct ReviewFileHeaderRow: View {
  struct Info {
    let path: String
    let oldPath: String?
    let status: ChangeStatus
    let added: Int?
    let removed: Int?
    let binary: Bool
    let viewed: Bool
    let changedSinceViewed: Bool
    let expanded: Bool
    let busy: Bool
    let firstChangedLine: Int?

    init(_ file: ReviewFile) {
      path = file.path
      oldPath = file.change.oldPath
      status = file.change.status
      added = file.diff?.added ?? file.change.added
      removed = file.diff?.removed ?? file.change.removed
      binary = file.diff?.isBinary ?? file.change.isBinary
      viewed = file.viewed
      changedSinceViewed = file.changedSinceViewed
      expanded = file.expanded
      busy = file.busy
      firstChangedLine = file.diff?.hunks.first.flatMap { hunk in
        let line = hunk.lines.first { $0.kind != .context } ?? hunk.lines.first
        return line.map { Int($0.newLineno ?? $0.oldLineno ?? 1) }
      }
    }
  }

  @Environment(\.chrome) private var chrome
  let info: Info
  let capabilities: ReviewCapabilities
  weak var handler: ReviewDiffHandler?

  var body: some View {
    let name = (info.path as NSString).lastPathComponent
    let dir = (info.path as NSString).deletingLastPathComponent
    HStack(spacing: 8) {
      Icon(.chevronDown, size: 12)
        .foregroundStyle(chrome.textTertiary)
        .rotationEffect(.degrees(info.expanded ? 0 : -90))
      ReviewStatusLetter(status: info.status)
      HStack(spacing: 4) {
        if let oldPath = info.oldPath {
          Text(oldPath).strikethrough().foregroundStyle(chrome.textTertiary)
          Icon(.chevronRight, size: 10).foregroundStyle(chrome.textTertiary)
        }
        if !dir.isEmpty {
          Text(dir + "/").foregroundStyle(chrome.textTertiary)
            .padding(.trailing, -4)
        }
        Text(name).foregroundStyle(chrome.text)
      }
      .font(ChromeFont.ui(12))
      .lineLimit(1)
      .truncationMode(.head)
      .help(info.path)
      if info.changedSinceViewed {
        Text("changed since viewed").font(ChromeFont.ui(10)).foregroundStyle(chrome.warning)
      }
      Spacer(minLength: 6)
      if info.binary {
        Text("binary").font(ChromeFont.mono(10.5)).foregroundStyle(chrome.textTertiary)
      } else {
        ReviewStat(added: info.added, removed: info.removed)
      }
      Button {
        handler?.reviewSetViewed(info.path, viewed: !info.viewed)
      } label: {
        HStack(spacing: 5) {
          ReviewCheckbox(on: info.viewed)
          Text("Viewed").font(ChromeFont.ui(11)).foregroundStyle(chrome.textSecondary)
        }
        .padding(.horizontal, 6)
        .frame(height: 22)
        .contentShape(Rectangle())
      }
      .buttonStyle(ChromePressStyle())
      .help("Mark as viewed (v)")
      if capabilities.stage {
        ReviewActionButton(title: "Stage", kind: .primary, help: "Stage file") {
          handler?.reviewFileAction(.stage, path: info.path)
        }
      }
      if capabilities.unstage {
        ReviewActionButton(title: "Unstage", kind: .primary, help: "Unstage file") {
          handler?.reviewFileAction(.unstage, path: info.path)
        }
      }
      if capabilities.revert {
        ReviewActionButton(title: "Revert", kind: .danger, help: "Discard changes to this file") {
          handler?.reviewFileAction(.revert, path: info.path)
        }
      }
      if capabilities.stage, info.status != .deleted {
        ReviewActionButton(title: "Edit Diff", help: "Edit the file beside its staged version") {
          handler?.reviewOpenFile(path: info.path, line: nil, diff: true)
        }
      }
      ReviewActionButton(title: "Open", help: "Open file (o)") {
        handler?.reviewOpenFile(path: info.path, line: info.firstChangedLine, diff: false)
      }
    }
    .padding(.leading, ReviewMetrics.cardInset + 10)
    .padding(.trailing, ReviewMetrics.cardInset + 8)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .contentShape(Rectangle())
    .onTapGesture { handler?.reviewToggleExpanded(info.path) }
    .opacity(info.busy ? 0.6 : 1)
    .contextMenu {
      Button("Copy Path") { handler?.reviewCopyPath(info.path) }
      Button("Open File") { handler?.reviewOpenFile(path: info.path, line: info.firstChangedLine, diff: false) }
      if capabilities.stage, info.status != .deleted {
        Button("Open in Diff Editor") { handler?.reviewOpenFile(path: info.path, line: nil, diff: true) }
      }
      Divider()
      Button(info.expanded ? "Collapse" : "Expand") { handler?.reviewToggleExpanded(info.path) }
    }
    .accessibilityElement(children: .contain)
    .accessibilityLabel("\(info.path), \(info.viewed ? "viewed" : "not viewed")")
  }
}

// MARK: - Hunk header

struct ReviewHunkHeaderRow: View {
  @Environment(\.chrome) private var chrome
  let path: String
  let hunk: Int
  let header: String
  let selectedLines: Int
  let capabilities: ReviewCapabilities
  /// The file's diff was cut short: only whole-file actions apply.
  let truncated: Bool
  let focused: Bool
  let busy: Bool
  weak var handler: ReviewDiffHandler?

  var body: some View {
    let what = selectedLines > 0 ? "\(selectedLines) line\(selectedLines == 1 ? "" : "s")" : "hunk"
    let capabilities = truncated ? ReviewCapabilities(stage: false, unstage: false, revert: false) : self.capabilities
    HStack(spacing: 6) {
      Text(header)
        .font(ChromeFont.mono(11))
        .foregroundStyle(chrome.textTertiary)
        .lineLimit(1)
        .truncationMode(.tail)
        .frame(maxWidth: .infinity, alignment: .leading)
      if truncated, self.capabilities.stage || self.capabilities.unstage || self.capabilities.revert {
        Text("Whole file only")
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
          .help(GitOperations.truncatedDiffMessage)
      }
      if capabilities.stage {
        ReviewActionButton(title: "Stage \(what)", kind: .primary, help: "Stage (s / ⌘Y)") {
          handler?.reviewHunkAction(.stage, path: path, hunk: hunk)
        }
      }
      if capabilities.unstage {
        ReviewActionButton(title: "Unstage \(what)", kind: .primary, help: "Unstage (u / ⇧⌘Y)") {
          handler?.reviewHunkAction(.unstage, path: path, hunk: hunk)
        }
      }
      if capabilities.revert {
        ReviewActionButton(title: "Revert \(what)", kind: .danger, help: "Revert in the working tree (x / ⌥⌘Z)") {
          handler?.reviewHunkAction(.revert, path: path, hunk: hunk)
        }
      }
      ReviewActionButton(title: "Comment", help: "Comment (c)") {
        handler?.reviewOpenComposer(path: path, hunk: hunk, line: nil)
      }
    }
    .disabled(busy)
    .padding(.leading, ReviewMetrics.cardInset + 10)
    .padding(.trailing, ReviewMetrics.cardInset + 8)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(
      Color(nsColor: ChromePalette.mix(
        NSColor(chrome.content), toward: NSColor(chrome.accent), amount: 0.06))
        .padding(.horizontal, ReviewMetrics.cardInset + 1)
    )
    .overlay(alignment: .leading) {
      if focused {
        Rectangle().fill(chrome.accent).frame(width: 2).padding(.leading, ReviewMetrics.cardInset + 1)
      }
    }
    .contentShape(Rectangle())
    .onTapGesture { handler?.reviewFocusHunk(path: path, hunk: hunk) }
  }
}

// MARK: - Notices, comments, composer

struct ReviewNoticeRow: View {
  @Environment(\.chrome) private var chrome
  let notice: ReviewNotice

  var body: some View {
    let isError: Bool = { if case .error = notice { return true } else { return false } }()
    Text(notice.text)
      .font(ChromeFont.ui(12))
      .italic(!isError)
      .foregroundStyle(isError ? chrome.danger : chrome.textTertiary)
      .lineLimit(2)
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
      .padding(.horizontal, ReviewMetrics.cardInset + 16)
  }
}

struct ReviewCommentRow: View {
  @Environment(\.chrome) private var chrome
  let comment: ReviewComment
  let outdated: Bool
  let editing: Bool
  /// The inline edit's text so far, when the row was rebuilt mid-edit.
  var editingDraft: String? = nil
  let indent: CGFloat
  weak var handler: ReviewDiffHandler?
  @State private var draft = ""
  @FocusState private var focused: Bool

  var body: some View {
    let range =
      comment.endLine > comment.line ? "lines \(comment.line)–\(comment.endLine)" : "line \(comment.line)"
    VStack(alignment: .leading, spacing: 3) {
      HStack(spacing: 8) {
        Text((comment.side == .old ? "removed " : "") + range)
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
        Spacer()
        if editing {
          ReviewActionButton(title: "Cancel", help: "Cancel (Esc)") { handler?.reviewEditComment(nil, path: comment.path) }
          ReviewActionButton(title: "Save", kind: .primary, help: "Save (⌘↩)") { save() }
        } else {
          ReviewActionButton(title: "Edit", help: "Edit comment") { handler?.reviewEditComment(comment.id, path: comment.path) }
          ReviewActionButton(title: "Delete", kind: .danger, help: "Remove this comment") {
            handler?.reviewDeleteComment(id: comment.id)
          }
        }
      }
      if editing {
        TextEditor(text: $draft)
          .font(ChromeFont.ui(12))
          .scrollContentBackground(.hidden)
          .focused($focused)
          .frame(maxHeight: .infinity)
          .onAppear {
            draft = editingDraft ?? comment.text
            focused = true
          }
          .onChange(of: draft) { _, new in handler?.reviewDraftChanged(path: comment.path, text: new, editing: true) }
          .onKeyPress(phases: .down) { press in
            if press.key == .return, press.modifiers.contains(.command) {
              save()
              return .handled
            }
            if press.key == .escape {
              handler?.reviewEditComment(nil, path: comment.path)
              return .handled
            }
            return .ignored
          }
      } else {
        Text(comment.text)
          .font(ChromeFont.ui(12))
          .foregroundStyle(chrome.text)
          .textSelection(.enabled)
          .fixedSize(horizontal: false, vertical: true)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
    .padding(.vertical, 8)
    .padding(.horizontal, 10)
    .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(chrome.panel))
    .overlay(
      RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(chrome.hairlineStrong, lineWidth: 1))
    .overlay(alignment: .leading) {
      UnevenRoundedRectangle(topLeadingRadius: 6, bottomLeadingRadius: 6, style: .continuous)
        .fill(outdated ? chrome.textTertiary : chrome.accent)
        .frame(width: 3)
    }
    .opacity(outdated ? 0.85 : 1)
    .padding(.top, 4)
    .padding(.bottom, 8)
    .padding(.leading, ReviewMetrics.cardInset + indent)
    .padding(.trailing, ReviewMetrics.cardInset + ReviewMetrics.codeTrailing)
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
  }

  private func save() {
    let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty else { return }
    handler?.reviewSaveComment(id: comment.id, text: text)
  }
}

struct ReviewComposerRow: View {
  @Environment(\.chrome) private var chrome
  let path: String
  let composer: ReviewFile.Composer
  let indent: CGFloat
  weak var handler: ReviewDiffHandler?
  @State private var text = ""
  @FocusState private var focused: Bool

  var body: some View {
    let lines = composer.endLine > composer.line ? "lines" : "line"
    let range = composer.endLine > composer.line ? "\(composer.line)–\(composer.endLine)" : "\(composer.line)"
    VStack(alignment: .leading, spacing: 6) {
      ZStack(alignment: .topLeading) {
        if text.isEmpty {
          Text("Leave a comment for the agent or yourself…")
            .font(ChromeFont.ui(12))
            .foregroundStyle(chrome.textTertiary)
            .padding(.leading, 5)
            .allowsHitTesting(false)
        }
        TextEditor(text: $text)
          .font(ChromeFont.ui(12))
          .scrollContentBackground(.hidden)
          .focused($focused)
      }
      .frame(maxHeight: .infinity)
      HStack(spacing: 6) {
        Text((composer.side == .old ? "Removed \(lines) " : "\(lines.capitalized) ") + range + " · ⌘↩ to save")
          .font(ChromeFont.ui(11))
          .foregroundStyle(chrome.textTertiary)
        Spacer()
        ReviewActionButton(title: "Cancel", help: "Cancel (Esc)") { handler?.reviewCancelComposer(path: path) }
        ReviewActionButton(title: "Comment", kind: .primary, help: "Save (⌘↩)") { save() }
      }
    }
    .onKeyPress(phases: .down) { press in
      if press.key == .return, press.modifiers.contains(.command) {
        save()
        return .handled
      }
      if press.key == .escape {
        handler?.reviewCancelComposer(path: path)
        return .handled
      }
      return .ignored
    }
    .onAppear {
      // A rebuilt row picks up what was typed.
      if text.isEmpty { text = composer.draft }
      focused = true
    }
    .onChange(of: text) { _, new in handler?.reviewDraftChanged(path: path, text: new, editing: false) }
    .padding(.vertical, 8)
    .padding(.horizontal, 10)
    .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(chrome.panel))
    .overlay(
      RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(chrome.hairlineStrong, lineWidth: 1))
    .overlay(alignment: .leading) {
      UnevenRoundedRectangle(topLeadingRadius: 6, bottomLeadingRadius: 6, style: .continuous)
        .fill(chrome.accent)
        .frame(width: 3)
    }
    .padding(.top, 4)
    .padding(.bottom, 8)
    .padding(.leading, ReviewMetrics.cardInset + indent)
    .padding(.trailing, ReviewMetrics.cardInset + ReviewMetrics.codeTrailing)
  }

  private func save() {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty {
      handler?.reviewCancelComposer(path: path)
    } else {
      handler?.reviewSaveComposer(path: path, text: trimmed)
    }
  }
}

// MARK: - Small pieces

/// The compact text buttons in file and hunk headers.
struct ReviewActionButton: View {
  enum Kind { case plain, primary, danger }

  @Environment(\.chrome) private var chrome
  @Environment(\.isEnabled) private var isEnabled
  let title: String
  var kind: Kind = .plain
  var help: String? = nil
  let action: () -> Void
  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      Text(title)
        .font(ChromeFont.ui(11))
        .foregroundStyle(
          kind == .danger && hovering ? chrome.danger : kind == .primary || hovering ? chrome.text : chrome.textSecondary)
        .lineLimit(1)
        .padding(.horizontal, 8)
        .frame(height: 22)
        .background(
          RoundedRectangle(cornerRadius: 5, style: .continuous).fill(hovering ? chrome.hover : .clear))
        .overlay(
          RoundedRectangle(cornerRadius: 5, style: .continuous)
            .strokeBorder(kind == .primary ? chrome.hairlineStrong : .clear, lineWidth: 1))
        .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .opacity(isEnabled ? 1 : 0.4)
    .help(help ?? title)
  }
}

struct ReviewStatusLetter: View {
  @Environment(\.chrome) private var chrome
  let status: ChangeStatus

  var body: some View {
    Text(status.letter)
      .font(ChromeFont.mono(10.5, weight: .bold))
      .foregroundStyle(color)
      .frame(width: 12)
  }

  private var color: Color {
    switch status {
    case .added, .untracked: return chrome.gitAdded
    case .deleted: return chrome.gitDeleted
    case .renamed: return chrome.gitRenamed
    case .conflicted: return chrome.gitConflict
    default: return chrome.gitModified
    }
  }
}

struct ReviewStat: View {
  @Environment(\.chrome) private var chrome
  let added: Int?
  let removed: Int?

  var body: some View {
    HStack(spacing: 4) {
      if let added { Text("+\(added)").foregroundStyle(chrome.gitAdded) }
      if let removed, removed > 0 { Text("−\(removed)").foregroundStyle(chrome.gitDeleted) }
    }
    .font(ChromeFont.mono(10.5))
  }
}

struct ReviewCheckbox: View {
  @Environment(\.chrome) private var chrome
  let on: Bool

  var body: some View {
    RoundedRectangle(cornerRadius: 3, style: .continuous)
      .fill(on ? chrome.success : .clear)
      .overlay(
        RoundedRectangle(cornerRadius: 3, style: .continuous)
          .strokeBorder(on ? chrome.success : chrome.textTertiary, lineWidth: 1))
      .overlay {
        if on { Icon(.check, size: 9, strokeWidth: 2.6).foregroundStyle(chrome.textOnAccent) }
      }
      .frame(width: 13, height: 13)
  }
}
