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
        ContentAreaRepresentable(contentView: tabManagerContentView)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
        StatusBarView(model: windowModel)
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
