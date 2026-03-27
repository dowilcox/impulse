import SwiftUI
import AppKit

// MARK: - Search Panel View

/// Displays search results in the sidebar. The search input is the
/// NSSearchToolbarItem in the window toolbar — this view only shows results.
struct SearchPanelView: View {
    var model: WindowModel
    @State private var searchTask: Task<Void, Never>?
    @State private var isSearching = false

    var body: some View {
        Group {
            if isSearching {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if model.searchResults.isEmpty && !model.searchQuery.isEmpty {
                Text("No results")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(model.searchResults.enumerated()), id: \.offset) { _, result in
                            SearchResultRow(result: result)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .contentShape(Rectangle())
                                .onTapGesture {
                                    model.onOpenFile?(result.path, result.lineNumber.map { Int($0) })
                                }
                        }
                    }
                }
            }
        }
        .frame(maxHeight: .infinity)
        .onChange(of: model.searchQuery) { _, _ in
            debounceSearch()
        }
        .onDisappear {
            searchTask?.cancel()
        }
    }

    // MARK: - Search Logic

    private func debounceSearch() {
        searchTask?.cancel()
        searchTask = Task {
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            performSearch()
        }
    }

    private func performSearch() {
        guard !model.searchQuery.isEmpty, !model.fileTreeRootPath.isEmpty else {
            model.searchResults = []
            isSearching = false
            return
        }
        let query = model.searchQuery
        let root = model.fileTreeRootPath
        let caseSensitive = model.searchCaseSensitive

        isSearching = true
        Task.detached {
            let fileResults = ImpulseCore.searchFiles(root: root, query: query)
            let contentResults = ImpulseCore.searchContent(
                root: root, query: query, caseSensitive: caseSensitive
            )

            let contentPaths = Set(contentResults.map(\.path))
            let dedupedFiles = fileResults.filter { !contentPaths.contains($0.path) }
            let combined = dedupedFiles + contentResults

            await MainActor.run {
                model.searchResults = combined
                isSearching = false
            }
        }
    }
}

// MARK: - Search Result Row

private struct SearchResultRow: View {
    let result: SearchResult

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 0) {
                Text(result.name)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)

                if let lineNumber = result.lineNumber {
                    Text(":\(lineNumber)")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
            }

            if let lineContent = result.lineContent {
                Text(lineContent.trimmingCharacters(in: .whitespaces))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
    }
}
