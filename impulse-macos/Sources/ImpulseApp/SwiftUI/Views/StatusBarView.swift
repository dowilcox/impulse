import AppKit
import SwiftUI

/// Status bar at the bottom of the window. Shown when the Warp-style input bar
/// is hidden (editor tabs, or a terminal running a full-screen/raw TUI). Its
/// context pills are the exact same `ContextChip`/`BranchChip` components the
/// input bar uses, so the two read as the same chrome — just without the
/// command field.
struct StatusBarView: View {
  var model: WindowModel

  var body: some View {
    HStack(spacing: 6) {
      leftGroup
      Spacer(minLength: 8)
      rightGroup
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 5)
    .background(.bar)
    .overlay(alignment: .top) { Divider() }
  }

  // MARK: - Left Group

  @ViewBuilder
  private var leftGroup: some View {
    if !model.shellName.isEmpty {
      ContextChip(symbol: "terminal", text: model.shellName)
    }
    if !model.currentCwd.isEmpty {
      ContextChip(symbol: "folder", text: TabManager.abbreviateHomePath(model.currentCwd))
    }
    if let branch = model.gitBranch, !branch.isEmpty {
      // Inert while a TUI owns the grid — a checkout would type into the program.
      BranchChip(model: model, branch: branch, interactive: !model.terminalDirectInteraction)
    }
  }

  // MARK: - Right Group

  @ViewBuilder
  private var rightGroup: some View {
    // Update available
    if let updateVersion = model.updateAvailableVersion,
      let updateURL = model.updateURL
    {
      actionChip(
        symbol: "arrow.down.circle",
        text: "Update \(updateVersion)",
        tint: model.theme.colorGreen,
        filled: true,
        help: updateHelpText(version: updateVersion)
      ) {
        NSWorkspace.shared.open(updateURL)
      }
    }

    // Language
    if let lang = model.currentLanguage {
      ContextChip(symbol: "chevron.left.forwardslash.chevron.right", text: lang)
    }

    // Encoding (editor tabs only)
    if model.cursorLine != nil {
      ContextChip(text: model.currentEncoding)
    }

    // Indent info
    if let indent = model.currentIndent {
      ContextChip(text: indent)
    }

    // Cursor position
    if let line = model.cursorLine, let col = model.cursorCol {
      ContextChip(text: "Ln \(line + 1), Col \(col + 1)")
    }

    // Preview toggle
    if model.isPreviewable {
      actionChip(
        symbol: model.isPreviewing ? "eye.fill" : "eye",
        text: "Preview",
        tint: model.theme.colorGreen,
        filled: model.isPreviewing,
        help: model.isPreviewing ? "Hide preview" : "Show preview"
      ) {
        model.onPreviewToggle?()
      }
    }
  }

  // MARK: - Action chip

  /// A tappable chip for actions (Update, Preview), styled like a `ContextChip`
  /// but tinted to read as interactive. `filled` paints the tint as the capsule
  /// fill; otherwise the tint colors the text on a soft capsule.
  private func actionChip(
    symbol: String,
    text: String,
    tint: Color,
    filled: Bool,
    help: String,
    action: @escaping () -> Void
  ) -> some View {
    Button(action: action) {
      HStack(spacing: 4) {
        Image(systemName: symbol)
          .font(.system(size: 9.5, weight: .semibold))
        Text(text)
          .font(.system(size: 11, weight: .medium))
          .lineLimit(1)
      }
      .foregroundStyle(filled ? model.theme.colorBgSurface : tint)
      .padding(.horizontal, 8)
      .padding(.vertical, 3)
      .background(
        Capsule().fill(filled ? AnyShapeStyle(tint) : AnyShapeStyle(tint.opacity(0.12)))
      )
      .contentShape(Capsule())
    }
    .buttonStyle(.plain)
    .help(help)
  }

  // MARK: - Helpers

  private func updateHelpText(version: String) -> String {
    if let current = model.updateCurrentVersion, !current.isEmpty {
      return "Impulse \(version) is available. Current version: \(current)."
    }
    return "Impulse \(version) is available."
  }
}
