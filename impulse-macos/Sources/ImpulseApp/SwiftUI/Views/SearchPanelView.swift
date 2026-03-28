import SwiftUI
import AppKit

// MARK: - Search Panel View

/// Displays search results in the sidebar. The search input is the
/// NSSearchToolbarItem in the window toolbar. This view shows options
/// and results.
struct SearchPanelView: View {
    var model: WindowModel
    @State private var searchTask: Task<Void, Never>?
    @State private var activeSearchTask: Task<Void, Never>?
    @State private var isSearching = false
    @State private var searchGeneration: UInt = 0

    var body: some View {
        VStack(spacing: 0) {
            // Options row
            HStack {
                Button {
                    model.searchCaseSensitive.toggle()
                    triggerSearch()
                } label: {
                    Text("Aa")
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                        .foregroundStyle(
                            model.searchCaseSensitive ? Color.accentColor : .secondary
                        )
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(
                            RoundedRectangle(cornerRadius: 4)
                                .fill(model.searchCaseSensitive
                                    ? Color.accentColor.opacity(0.15)
                                    : .clear)
                        )
                }
                .buttonStyle(.plain)
                .help("Match Case")

                Spacer()

                if !model.searchResults.isEmpty {
                    Text("\(model.searchResults.count) results")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)

            Divider()

            // Results
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
            activeSearchTask?.cancel()
        }
    }

    // MARK: - Search Logic

    private func triggerSearch() {
        searchTask?.cancel()
        performSearch()
    }

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

        searchGeneration &+= 1
        let generation = searchGeneration
        activeSearchTask?.cancel()

        isSearching = true
        activeSearchTask = Task.detached {
            let fileResults = ImpulseCore.searchFiles(root: root, query: query)
            let contentResults = ImpulseCore.searchContent(
                root: root, query: query, caseSensitive: caseSensitive
            )

            let contentPaths = Set(contentResults.map(\.path))
            let dedupedFiles = fileResults.filter { !contentPaths.contains($0.path) }
            let combined = dedupedFiles + contentResults

            await MainActor.run {
                // Only apply results if this is still the latest search.
                guard generation == searchGeneration else { return }
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
