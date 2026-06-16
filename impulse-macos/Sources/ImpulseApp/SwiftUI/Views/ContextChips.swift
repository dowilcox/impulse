import SwiftUI

/// A context chip shared by the terminal input bar and the bottom status bar so
/// the two read as the same chrome: a soft capsule with an optional leading SF
/// Symbol and a monospaced label. Keeping it in one place guarantees the input
/// bar and status bar pills stay visually identical.
struct ContextChip: View {
  var symbol: String? = nil
  let text: String
  var showsChevron: Bool = false
  /// Theme so the chip is tinted to the active color scheme rather than the
  /// neutral system grays — keeps the bar reading as part of the themed
  /// terminal surface instead of a separate macOS material strip.
  let theme: Theme

  var body: some View {
    HStack(spacing: 4) {
      if let symbol {
        Image(systemName: symbol)
          .font(.system(size: 9.5, weight: .medium))
          .foregroundStyle(theme.colorFgComment)
      }
      Text(text)
        .font(.system(size: 11, design: .monospaced))
        .foregroundStyle(theme.colorFgMuted)
        .lineLimit(1)
        .truncationMode(.middle)
      if showsChevron {
        Image(systemName: "chevron.up.chevron.down")
          .font(.system(size: 7, weight: .semibold))
          .foregroundStyle(theme.colorFgComment)
      }
    }
    .padding(.horizontal, 8)
    .padding(.vertical, 3)
    .background(Capsule().fill(theme.colorFg.opacity(0.07)))
    .frame(maxWidth: 280, alignment: .leading)
    .fixedSize()
  }
}

/// The git-branch chip. Shared by the input bar and the status bar so both look
/// identical (capsule, branch glyph, chevron). When `interactive` is true it's a
/// branch-switcher button: tap to open the picker, choosing a branch runs `git
/// checkout` in the active terminal. When false it renders the exact same chip
/// but inert — used in the status bar while a TUI (Claude Code) owns the grid,
/// where typing a checkout command would land in the program instead of the
/// shell.
struct BranchChip: View {
  var model: WindowModel
  let branch: String
  var interactive: Bool = true

  @State private var showPicker = false

  private var chip: some View {
    ContextChip(
      symbol: "arrow.triangle.branch", text: branch, showsChevron: true, theme: model.theme)
  }

  var body: some View {
    if interactive {
      Button {
        showPicker.toggle()
      } label: {
        chip
      }
      .buttonStyle(.plain)
      .help("Switch branch")
      .popover(isPresented: $showPicker, arrowEdge: .top) {
        BranchPickerView(
          currentBranch: branch,
          cwd: model.currentCwd,
          accent: model.theme.colorAccent
        ) { selected in
          showPicker = false
          guard selected != branch else { return }
          model.onRunCommand?("git checkout \(Self.shellQuoted(selected))")
        }
      }
    } else {
      chip
    }
  }

  /// Minimal POSIX single-quote escaping for a branch name.
  static func shellQuoted(_ value: String) -> String {
    if value.allSatisfy({ $0.isLetter || $0.isNumber || "._-/".contains($0) }) {
      return value
    }
    return "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
  }
}

/// Warp-style "Review Changes" summary chip: a page glyph, the changed-file
/// count, and the aggregate +added / -removed line counts (green/red). Sits next
/// to the branch chip and opens the Review Changes tab when tapped. Rendered only
/// while the working tree has uncommitted changes.
struct ReviewChip: View {
  var model: WindowModel
  let fileCount: Int
  let added: Int
  let removed: Int

  private var theme: Theme { model.theme }

  var body: some View {
    Button(action: { model.onOpenDiffReview?() }) {
      HStack(spacing: 6) {
        Image(systemName: "doc")
          .font(.system(size: 9.5, weight: .medium))
          .foregroundStyle(theme.colorFgComment)
        Text("\(fileCount)")
          .foregroundStyle(theme.colorFgMuted)
        Text("•")
          .foregroundStyle(theme.colorFgComment)
        Text("+\(added)")
          .foregroundStyle(theme.colorGreen)
        Text("-\(removed)")
          .foregroundStyle(theme.colorRed)
      }
      .font(.system(size: 11, weight: .medium, design: .monospaced))
      .lineLimit(1)
      .padding(.horizontal, 8)
      .padding(.vertical, 3)
      .background(Capsule().fill(theme.colorFg.opacity(0.07)))
      .contentShape(Capsule())
    }
    .buttonStyle(.plain)
    .help("Review \(fileCount) changed file\(fileCount == 1 ? "" : "s") (⌘⇧G)")
    .accessibilityLabel("Review Changes")
  }
}
