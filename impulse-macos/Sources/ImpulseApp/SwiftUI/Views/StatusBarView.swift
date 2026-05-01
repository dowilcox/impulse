import AppKit
import SwiftUI

/// Status bar at the bottom of the window showing context-sensitive info.
struct StatusBarView: View {
  var model: WindowModel

  var body: some View {
    HStack(spacing: 0) {
      // Left group
      leftGroup

      Spacer()

      // Right group
      rightGroup
    }
    .padding(.horizontal, 12)
    .frame(height: 28)
    .overlay(alignment: .top) { Divider() }
  }

  // MARK: - Left Group

  @ViewBuilder
  private var leftGroup: some View {
    // Shell name
    if !model.shellName.isEmpty {
      label(model.shellName, color: model.theme.colorCyan)
    }

    // Git branch
    if let branch = model.gitBranch {
      separator
      HStack(spacing: 3) {
        Image(systemName: "arrow.triangle.branch")
          .font(.system(size: 9))
        label(branch, color: model.theme.colorMagenta)
      }
    }

    // CWD
    if !model.currentCwd.isEmpty {
      separator
      label(shortenHome(model.currentCwd), color: model.theme.colorFg)
    }

    // Blame
    if let blame = model.blameInfo {
      separator
      label(blame, color: model.theme.colorFgMuted)
    }
  }

  // MARK: - Right Group

  @ViewBuilder
  private var rightGroup: some View {
    // Update available
    if let updateVersion = model.updateAvailableVersion,
      let updateURL = model.updateURL
    {
      Button {
        NSWorkspace.shared.open(updateURL)
      } label: {
        Text("Update \(updateVersion)")
          .font(.system(size: 10, weight: .medium))
          .foregroundStyle(model.theme.colorGreen)
          .padding(.horizontal, 8)
          .padding(.vertical, 2)
          .background(
            RoundedRectangle(cornerRadius: 3)
              .fill(model.theme.colorGreen.opacity(0.12))
              .strokeBorder(model.theme.colorGreen, lineWidth: 1)
          )
      }
      .buttonStyle(.plain)
      .help(updateHelpText(version: updateVersion))
      .padding(.trailing, 4)

      separator
    }

    // Encoding
    if model.cursorLine != nil {
      label(model.currentEncoding, color: model.theme.colorFgMuted)
      separator
    }

    // Indent info
    if let indent = model.currentIndent {
      label(indent, color: model.theme.colorFgMuted)
      separator
    }

    // Language
    if let lang = model.currentLanguage {
      label(lang, color: model.theme.colorBlue)
      separator
    }

    // Cursor position
    if let line = model.cursorLine, let col = model.cursorCol {
      label("Ln \(line + 1), Col \(col + 1)", color: model.theme.colorFgMuted)
    }

    // Preview toggle
    if model.isPreviewable {
      separator
      Button {
        model.onPreviewToggle?()
      } label: {
        Text("Preview")
          .font(.system(size: 10, weight: .medium))
          .foregroundStyle(model.isPreviewing ? model.theme.colorBgSurface : model.theme.colorGreen)
          .padding(.horizontal, 8)
          .padding(.vertical, 2)
          .background(
            RoundedRectangle(cornerRadius: 3)
              .fill(model.isPreviewing ? model.theme.colorGreen : .clear)
              .strokeBorder(model.theme.colorGreen, lineWidth: 1)
          )
      }
      .buttonStyle(.plain)
      .padding(.trailing, 4)
    }
  }

  // MARK: - Helpers

  private func label(_ text: String, color: Color) -> some View {
    Text(text)
      .font(.system(size: 11))
      .foregroundStyle(color)
      .lineLimit(1)
  }

  private var separator: some View {
    Rectangle()
      .fill(model.theme.colorBorder.opacity(0.3))
      .frame(width: 1, height: 14)
      .padding(.horizontal, 8)
  }

  private func shortenHome(_ path: String) -> String {
    let home = NSHomeDirectory()
    if path.hasPrefix(home) {
      return "~" + String(path.dropFirst(home.count))
    }
    return path
  }

  private func updateHelpText(version: String) -> String {
    if let current = model.updateCurrentVersion, !current.isEmpty {
      return "Impulse \(version) is available. Current version: \(current)."
    }
    return "Impulse \(version) is available."
  }
}
