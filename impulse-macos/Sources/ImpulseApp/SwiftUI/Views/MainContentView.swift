import AppKit
import SwiftUI

/// Root SwiftUI view for the Impulse window. Uses NavigationSplitView for the
/// standard macOS sidebar + detail layout. Toolbar items are inline in
/// SidebarView rather than via .toolbar {} (which doesn't propagate to
/// NSToolbar when inside an NSHostingView).
struct MainContentView: View {
  @Bindable var windowModel: WindowModel
  let tabManagerContentView: NSView
  @State private var columnVisibility: NavigationSplitViewVisibility

  init(windowModel: WindowModel, tabManagerContentView: NSView) {
    self.windowModel = windowModel
    self.tabManagerContentView = tabManagerContentView
    _columnVisibility = State(initialValue: windowModel.sidebarVisible ? .all : .detailOnly)
  }

  var body: some View {
    NavigationSplitView(columnVisibility: $columnVisibility) {
      SidebarView(model: windowModel)
        .navigationSplitViewColumnWidth(min: 180, ideal: windowModel.sidebarWidth, max: 450)
    } detail: {
      VStack(spacing: 0) {
        if let warning = windowModel.settingsLoadWarning {
          SettingsLoadWarningBanner(
            warning: warning,
            openAction: { windowModel.onOpenSettingsFile?() },
            dismissAction: { windowModel.onDismissSettingsWarning?() }
          )
        }
        TabBarView(windowModel: windowModel)
        contentArea
        if showContextBar {
          // Warp-style: the input bar replaces the status bar for terminals —
          // its chips already carry the shell, cwd, branch, and last status.
          TerminalContextBarView(model: windowModel)
        } else {
          StatusBarView(model: windowModel)
        }
      }
    }
    .navigationSplitViewStyle(.balanced)
    .onChange(of: columnVisibility) { _, visibility in
      let isVisible = visibility != .detailOnly
      if windowModel.sidebarVisible != isVisible {
        windowModel.sidebarVisible = isVisible
      }
    }
    .onChange(of: windowModel.sidebarVisible) { _, isVisible in
      let desired: NavigationSplitViewVisibility = isVisible ? .all : .detailOnly
      if columnVisibility != desired {
        columnVisibility = desired
      }
    }
  }

  /// Show the Warp-style input bar only for a single terminal pane, and never
  /// while the alternate screen (vim, htop, ...) owns the keyboard or the tab
  /// is split (split panes are classic typeable terminals).
  private var showContextBar: Bool {
    guard windowModel.contextBarEnabled,
      !windowModel.terminalAltScreen,
      !windowModel.activeTerminalSplit
    else { return false }
    let index = windowModel.selectedTabIndex
    guard index >= 0, index < windowModel.tabDisplayInfos.count else { return false }
    return windowModel.tabDisplayInfos[index].isTerminal
  }

  /// True when the selected tab is a terminal.
  private var selectedTabIsTerminal: Bool {
    let index = windowModel.selectedTabIndex
    guard index >= 0, index < windowModel.tabDisplayInfos.count else { return false }
    return windowModel.tabDisplayInfos[index].isTerminal
  }

  /// "card" surface themes (e.g. Harbor) float the editor area as a rounded
  /// card with a soft warm shadow. Terminals always render edge-to-edge
  /// (Warp-style) — the floating card reads as chrome the terminal doesn't want.
  @ViewBuilder
  private var contentArea: some View {
    if windowModel.theme.surfaceStyle == "card" && !selectedTabIsTerminal {
      ContentAreaRepresentable(contentView: tabManagerContentView, cornerRadius: 16)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(
          RoundedRectangle(cornerRadius: 16, style: .continuous)
            .fill(windowModel.theme.colorBg)
            .shadow(color: cardShadowColor.opacity(0.20), radius: 7, x: 0, y: 4)
            .shadow(color: cardShadowColor.opacity(0.08), radius: 1, x: 0, y: 1)
        )
        .padding(.top, 10)
        .padding(.horizontal, 16)
        .padding(.bottom, 14)
        .background(windowModel.theme.colorBgDark)
    } else {
      ContentAreaRepresentable(contentView: tabManagerContentView)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
  }

  /// Warm-hued shadow (#5c5142) per the Harbor spec — never pure black.
  private var cardShadowColor: Color {
    Color(red: 0.36, green: 0.32, blue: 0.26)
  }
}

private struct SettingsLoadWarningBanner: View {
  let warning: SettingsLoadWarning
  let openAction: () -> Void
  let dismissAction: () -> Void

  var body: some View {
    HStack(spacing: 10) {
      Image(systemName: "exclamationmark.triangle.fill")
        .foregroundStyle(.yellow)
        .accessibilityHidden(true)

      VStack(alignment: .leading, spacing: 2) {
        Text("Settings file could not be loaded")
          .font(.system(size: 12, weight: .semibold))
        Text(detailText)
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
          .lineLimit(2)
      }

      Spacer(minLength: 12)

      Button("Open Settings File", action: openAction)
        .controlSize(.small)

      Button(action: dismissAction) {
        Image(systemName: "xmark")
      }
      .buttonStyle(.borderless)
      .accessibilityLabel("Dismiss settings warning")
      .help("Dismiss")
    }
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .background(Color.yellow.opacity(0.13))
    .overlay(alignment: .bottom) {
      Rectangle()
        .fill(Color.yellow.opacity(0.35))
        .frame(height: 1)
    }
    .help(warning.message)
  }

  private var detailText: String {
    if let backupPath = warning.backupPath {
      return "Using defaults. The invalid file was backed up to \(backupPath.path)."
    }
    return "Using defaults. Automatic settings saves are paused until this is fixed."
  }
}
