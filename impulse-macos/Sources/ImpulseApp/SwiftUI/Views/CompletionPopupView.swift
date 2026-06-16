import AppKit
import SwiftUI

/// Path-completion dropdown for the terminal input bar. Shown as a `.popover`
/// anchored to the input field while the active argument token has two or more
/// candidates. Modeled on `BranchPickerView` styling: a themed `ScrollView` +
/// `LazyVStack` with rounded selected-row highlight.
///
/// The view is display-only: every row is a `.plain` `Button` that reports its
/// index via `onSelect`. It contains NO focusable controls, so the input
/// TextField keeps key focus and the popover never steals first-responder
/// status. Selection is driven externally (the input bar's ↑/↓ handling) via
/// `selectedIndex`; a `ScrollViewReader` keeps the selected row visible.
struct CompletionPopupView: View {
  let candidates: [CompletionCandidate]
  /// The basename prefix the user has typed for the active token, used to
  /// emphasize the matched leading characters of each candidate's display name.
  let matchedPrefix: String
  let selectedIndex: Int?
  let theme: Theme
  let iconCache: IconCache?
  let onSelect: (Int) -> Void

  /// Approximate per-row height (matches `BranchPickerView`'s 28pt rows + 1pt
  /// inter-row spacing) used to cap the visible height at ~10 rows.
  private static let rowHeight: CGFloat = 28
  private static let rowSpacing: CGFloat = 1
  private static let maxVisibleRows = 10

  /// Fixed list width — shared with `CompletionPanel` so the hosting panel and
  /// the SwiftUI view agree on size.
  static let listWidth: CGFloat = 340

  /// Total list height for `count` candidates, capped at `maxVisibleRows`.
  /// Exposed so `CompletionPanel` can size its window to match.
  static func listHeight(for count: Int) -> CGFloat {
    let rows = min(count, maxVisibleRows)
    return CGFloat(rows) * (rowHeight + rowSpacing) + 12
  }

  private var listHeight: CGFloat { Self.listHeight(for: candidates.count) }

  var body: some View {
    ScrollViewReader { proxy in
      ScrollView {
        LazyVStack(alignment: .leading, spacing: Self.rowSpacing) {
          ForEach(Array(candidates.enumerated()), id: \.offset) { index, candidate in
            row(candidate, index: index, isSelected: index == selectedIndex)
              .id(index)
          }
        }
        .padding(6)
      }
      .frame(width: Self.listWidth, height: listHeight)
      .onChange(of: selectedIndex) { _, newValue in
        guard let newValue else { return }
        withAnimation(.linear(duration: 0.08)) {
          proxy.scrollTo(newValue, anchor: .center)
        }
      }
      .onAppear {
        if let selectedIndex {
          proxy.scrollTo(selectedIndex, anchor: .center)
        }
      }
    }
    .background(theme.colorBgDark)
  }

  private func row(_ candidate: CompletionCandidate, index: Int, isSelected: Bool) -> some View {
    Button {
      onSelect(index)
    } label: {
      HStack(spacing: 8) {
        icon(for: candidate)
          .frame(width: 16, height: 16)
        emphasizedName(candidate.display)
          .font(.system(size: 12.5, design: .monospaced))
          .lineLimit(1)
          .truncationMode(.middle)
        Spacer(minLength: 6)
        Text(trailingLabel(for: candidate))
          .font(.system(size: 10.5, design: .monospaced))
          .foregroundStyle(trailingColor(for: candidate))
      }
      .padding(.horizontal, 8)
      .frame(height: Self.rowHeight)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(
        RoundedRectangle(cornerRadius: 5, style: .continuous)
          .fill(isSelected ? theme.colorAccent.opacity(0.18) : Color.clear)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }

  /// Folder icon for directories, file-type icon otherwise. Falls back to SF
  /// Symbols when the icon cache can't resolve an image (mirrors
  /// `FileTreeListView.fileIcon`).
  @ViewBuilder
  private func icon(for candidate: CompletionCandidate) -> some View {
    if let nsImage = iconCache?.icon(
      filename: candidate.display, isDirectory: candidate.isDir, expanded: false)
    {
      Image(nsImage: nsImage)
        .resizable()
        .interpolation(.high)
    } else if candidate.isDir {
      Image(systemName: "folder.fill")
        .font(.system(size: 13))
        .foregroundStyle(theme.colorAccent)
    } else {
      Image(systemName: "doc.fill")
        .font(.system(size: 13))
        .foregroundStyle(theme.colorFgMuted)
    }
  }

  /// Renders `name` with the matched leading prefix emphasized (accent +
  /// semibold) and the remainder in the normal foreground color. Matching is
  /// case-sensitive to mirror the Rust prefix match.
  private func emphasizedName(_ name: String) -> Text {
    guard !matchedPrefix.isEmpty, name.hasPrefix(matchedPrefix) else {
      return Text(name).foregroundColor(theme.colorFg)
    }
    let matched = String(name.prefix(matchedPrefix.count))
    let rest = String(name.dropFirst(matchedPrefix.count))
    return Text(matched).foregroundColor(theme.colorAccent).bold()
      + Text(rest).foregroundColor(theme.colorFg)
  }

  /// Trailing muted label: the git status code when present, else "dir"/"file".
  private func trailingLabel(for candidate: CompletionCandidate) -> String {
    if let status = candidate.gitStatus, !status.isEmpty {
      return status
    }
    return candidate.isDir ? "dir" : "file"
  }

  private func trailingColor(for candidate: CompletionCandidate) -> Color {
    if let status = candidate.gitStatus, !status.isEmpty {
      switch status {
      case "?", "A": return theme.colorGitAdded
      case "M": return theme.colorGitModified
      case "D": return theme.colorGitDeleted
      case "R": return theme.colorGitRenamed
      default: return theme.colorFgComment
      }
    }
    return theme.colorFgComment
  }
}
