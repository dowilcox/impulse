import SwiftUI
import AppKit

/// Root SwiftUI view for the Impulse window. Uses NavigationSplitView for the
/// standard macOS sidebar + detail layout. Toolbar items are inline in
/// SidebarView rather than via .toolbar {} (which doesn't propagate to
/// NSToolbar when inside an NSHostingView).
struct MainContentView: View {
    var windowModel: WindowModel
    let tabManagerContentView: NSView

    var body: some View {
        NavigationSplitView {
            SidebarView(model: windowModel)
                .navigationSplitViewColumnWidth(min: 180, ideal: 250, max: 450)
        } detail: {
            VStack(spacing: 0) {
                TabBarView(windowModel: windowModel)
                ContentAreaRepresentable(contentView: tabManagerContentView)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                StatusBarView(model: windowModel)
            }
        }
        .navigationSplitViewStyle(.balanced)
    }
}
