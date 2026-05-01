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
