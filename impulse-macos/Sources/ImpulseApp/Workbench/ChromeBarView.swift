import AppKit
import SwiftUI

/// The titlebar band: traffic-light inset, dock toggle, workspace breadcrumb,
/// the tab strip, and window-level actions (palette, diff, docks). Empty
/// space drags the window; double-click honors the system zoom/minimize
/// preference.
struct ChromeBarView: View {
  var model: WindowModel

  var body: some View {
    HStack(spacing: 0) {
      leadingSection
      TitlebarTabStrip(model: model)
        .frame(maxWidth: .infinity, alignment: .leading)
      trailingSection
    }
    .frame(height: Metrics.titlebarHeight)
    .background {
      ZStack(alignment: .bottom) {
        model.palette.chrome
        TitlebarDragArea()
        Hairline()
      }
    }
    .environment(\.chrome, model.palette)
  }

  // MARK: Leading

  /// Sits above the left dock so tabs start where the content column starts.
  private var leadingSection: some View {
    HStack(spacing: 6) {
      Color.clear.frame(width: model.isFullScreen ? 6 : Metrics.trafficLightInset)
        .allowsHitTesting(false)
      ChromeIconButton(
        icon: .panelLeft, help: "Toggle Sidebar (⌘B)", isActive: model.sidebarVisible
      ) {
        model.sidebarVisible.toggle()
      }
      WorkspaceBreadcrumb(model: model)
      Spacer(minLength: 0)
    }
    .padding(.trailing, 8)
    .frame(width: leadingWidth, alignment: .leading)
  }

  private var leadingWidth: CGFloat? {
    model.sidebarVisible ? max(model.sidebarWidth, 220) : nil
  }

  // MARK: Trailing

  private var trailingSection: some View {
    HStack(spacing: 4) {
      PaletteButton { model.onShowCommandPalette?() }
      if model.reviewChangedFileCount > 0 {
        DiffPill(
          files: model.reviewChangedFileCount,
          added: model.reviewAddedLines,
          removed: model.reviewRemovedLines
        ) { model.onOpenDiffReview?() }
      }
      ChromeIconButton(
        icon: .panelRight, help: "Toggle Right Panel (⌥⌘B)", isActive: model.rightDockVisible
      ) {
        model.onToggleRightDock?()
      }
    }
    .padding(.horizontal, 8)
  }
}

// MARK: - Breadcrumb

/// `project › branch` for the active tab's repository/folder.
private struct WorkspaceBreadcrumb: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel

  var body: some View {
    let project = projectName
    HStack(spacing: 5) {
      if !project.isEmpty {
        Text(project)
          .font(ChromeFont.ui(12, weight: .semibold))
          .foregroundStyle(chrome.text)
          .lineLimit(1)
          .truncationMode(.middle)
      }
      if let branch = model.gitBranch, !branch.isEmpty {
        Button {
          model.onShowBranchSwitcher?()
        } label: {
          HStack(spacing: 4) {
            Icon(.gitBranch, size: 11)
              .foregroundStyle(chrome.textTertiary)
            Text(branch)
              .font(ChromeFont.ui(11.5))
              .foregroundStyle(chrome.textSecondary)
              .lineLimit(1)
              .truncationMode(.middle)
            if let repo = model.repository?.snapshot, repo.ahead > 0 || repo.behind > 0 {
              Text(syncText(repo.ahead, repo.behind))
                .font(ChromeFont.mono(10.5))
                .foregroundStyle(chrome.textTertiary)
            }
          }
          .contentShape(Rectangle())
        }
        .buttonStyle(ChromePressStyle())
        .help("Switch branch (⌃⌘B)")
      }
    }
    .help(model.fileTreeRootPath)
  }

  private func syncText(_ ahead: Int, _ behind: Int) -> String {
    [ahead > 0 ? "↑\(ahead)" : nil, behind > 0 ? "↓\(behind)" : nil].compactMap { $0 }
      .joined(separator: " ")
  }

  private var projectName: String {
    let root = model.fileTreeRootPath
    guard !root.isEmpty else { return "" }
    if root == NSHomeDirectory() { return "~" }
    return (root as NSString).lastPathComponent
  }
}

// MARK: - Trailing controls

private struct PaletteButton: View {
  @Environment(\.chrome) private var chrome
  let action: () -> Void
  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      HStack(spacing: 6) {
        Icon(.search, size: 12)
        Text("Search or run a command")
          .font(ChromeFont.ui(11.5))
          .lineLimit(1)
        KeyHint("⌘⇧P")
      }
      .foregroundStyle(chrome.textTertiary)
      .padding(.leading, 8)
      .padding(.trailing, 4)
      .frame(height: 24)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous)
          .fill(hovering ? chrome.pressed : chrome.raised)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .help("Command Palette (⌘⇧P)")
    .accessibilityLabel("Command Palette")
  }
}

private struct DiffPill: View {
  @Environment(\.chrome) private var chrome
  let files: Int
  let added: Int
  let removed: Int
  let action: () -> Void
  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      HStack(spacing: 6) {
        Icon(.fileDiff, size: 12)
          .foregroundStyle(chrome.textSecondary)
        Text("\(files)")
          .font(ChromeFont.mono(11))
          .foregroundStyle(chrome.textSecondary)
        DiffStat(added: added, removed: removed)
      }
      .padding(.horizontal, 8)
      .frame(height: 24)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous)
          .fill(hovering ? chrome.pressed : chrome.raised)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .help("Review \(files) changed file\(files == 1 ? "" : "s") (⌘⇧G)")
    .accessibilityLabel("Review changes: \(files) files, \(added) added, \(removed) removed lines")
  }
}

// MARK: - Window drag area

/// Transparent AppKit view behind the titlebar controls: dragging it moves the
/// window, double-clicking zooms or minimizes per the user's system setting.
struct TitlebarDragArea: NSViewRepresentable {
  func makeNSView(context: Context) -> NSView { DragView() }
  func updateNSView(_ nsView: NSView, context: Context) {}

  final class DragView: NSView {
    override var mouseDownCanMoveWindow: Bool { true }

    override func mouseDown(with event: NSEvent) {
      guard let window else { return }
      if event.clickCount == 2 {
        let action =
          UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") ?? "Maximize"
        switch action {
        case "Minimize": window.miniaturize(nil)
        case "Maximize", "Fill": window.zoom(nil)
        default: break
        }
        return
      }
      window.performDrag(with: event)
    }
  }
}
