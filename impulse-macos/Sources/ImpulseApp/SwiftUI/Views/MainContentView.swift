import SwiftUI
import AppKit

/// Root SwiftUI view for the Impulse window. Uses NavigationSplitView for the
/// standard macOS sidebar + detail layout, with toolbar items placed where
/// Apple puts them (sidebar toggle by traffic lights, search in top-right,
/// new tab button in toolbar).
struct MainContentView: View {
    var windowModel: WindowModel
    let tabManagerContentView: NSView
    @State private var searchText = ""

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
        .searchable(text: $searchText, placement: .toolbar, prompt: "Search")
        .onChange(of: searchText) { _, newValue in
            if !newValue.isEmpty {
                windowModel.sidebarPanel = .search
                windowModel.searchQuery = newValue
            }
        }
        .toolbar(id: "main") {
            ToolbarItem(id: "newTab", placement: .primaryAction) {
                Button(action: { windowModel.onNewTab?() }) {
                    Label("New Tab", systemImage: "plus")
                }
                .help("New Terminal Tab")
            }
        }
    }
}
