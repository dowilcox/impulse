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
            label(model.shellName, color: Color(NSColor(hex: model.theme.cyan)))
        }

        // Git branch
        if let branch = model.gitBranch {
            separator
            HStack(spacing: 3) {
                Image(systemName: "arrow.triangle.branch")
                    .font(.system(size: 9))
                label(branch, color: Color(NSColor(hex: model.theme.magenta)))
            }
        }

        // CWD
        if !model.currentCwd.isEmpty {
            separator
            label(shortenHome(model.currentCwd), color: Color(NSColor(hex: model.theme.fg)))
        }

        // Blame
        if let blame = model.blameInfo {
            separator
            label(blame, color: Color(NSColor(hex: model.theme.fgMuted)))
        }
    }

    // MARK: - Right Group

    @ViewBuilder
    private var rightGroup: some View {
        // Encoding
        if model.cursorLine != nil {
            label(model.currentEncoding, color: Color(NSColor(hex: model.theme.fgMuted)))
            separator
        }

        // Indent info
        if let indent = model.currentIndent {
            label(indent, color: Color(NSColor(hex: model.theme.fgMuted)))
            separator
        }

        // Language
        if let lang = model.currentLanguage {
            label(lang, color: Color(NSColor(hex: model.theme.blue)))
            separator
        }

        // Cursor position
        if let line = model.cursorLine, let col = model.cursorCol {
            label("Ln \(line + 1), Col \(col + 1)", color: Color(NSColor(hex: model.theme.fgMuted)))
        }

        // Preview toggle
        if model.isPreviewable {
            separator
            Button { model.onPreviewToggle?() } label: {
                Text("Preview")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(model.isPreviewing ? Color(NSColor(hex: model.theme.bgSurface)) : Color(NSColor(hex: model.theme.green)))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(
                        RoundedRectangle(cornerRadius: 3)
                            .fill(model.isPreviewing ? Color(NSColor(hex: model.theme.green)) : .clear)
                            .strokeBorder(Color(NSColor(hex: model.theme.green)), lineWidth: 1)
                    )
            }
            .buttonStyle(.plain)
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
            .fill(Color(NSColor(hex: model.theme.border)).opacity(0.3))
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
}
