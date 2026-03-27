import SwiftUI
import AppKit

// MARK: - Search Panel View

/// Project-wide search panel with a query text field, case-sensitivity toggle,
/// and a scrollable list of search results. Tapping a result opens the file at
/// the matched line.
struct SearchPanelView: View {
    var model: WindowModel
    @State private var searchTask: Task<Void, Never>?

    var body: some View {
        VStack(spacing: 0) {
            // Search field + options
            searchHeader

            Divider()
                .overlay(Color(model.theme.border))

            // Results list
            if model.searchResults.isEmpty && !model.searchQuery.isEmpty {
                Text("No results")
                    .font(.system(size: 12))
                    .foregroundColor(Color(model.theme.fgDark))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                resultsList
            }
        }
        .frame(maxHeight: .infinity)
    }

    // MARK: - Search Header

    private var searchHeader: some View {
        HStack(spacing: 6) {
            HStack(spacing: 4) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 12))
                    .foregroundColor(Color(model.theme.fgDark))

                TextField("Search...", text: Binding(
                    get: { model.searchQuery },
                    set: { newValue in
                        model.searchQuery = newValue
                        debounceSearch()
                    }
                ))
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .foregroundColor(Color(model.theme.fg))
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(Color(model.theme.bg))
            .cornerRadius(6)

            // Case sensitivity toggle
            Button {
                model.searchCaseSensitive.toggle()
                performSearch()
            } label: {
                Text("Aa")
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundColor(
                        model.searchCaseSensitive
                            ? Color(model.theme.accent)
                            : Color(model.theme.fgDark)
                    )
                    .frame(width: 26, height: 26)
                    .background(
                        model.searchCaseSensitive
                            ? Color(model.theme.accent).opacity(0.15)
                            : Color.clear
                    )
                    .cornerRadius(4)
            }
            .buttonStyle(.plain)
            .help("Match Case")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
    }

    // MARK: - Results List

    private var resultsList: some View {
        List {
            ForEach(Array(model.searchResults.enumerated()), id: \.offset) { _, result in
                SearchResultRow(result: result, theme: model.theme)
                    .contentShape(Rectangle())
                    .onTapGesture {
                        model.onOpenFile?(result.path, result.lineNumber.map { Int($0) })
                    }
            }
        }
        .listStyle(.plain)
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
            return
        }
        let query = model.searchQuery
        let root = model.fileTreeRootPath
        let caseSensitive = model.searchCaseSensitive

        Task.detached {
            let fileResults = ImpulseCore.searchFiles(root: root, query: query)
            let contentResults = ImpulseCore.searchContent(
                root: root, query: query, caseSensitive: caseSensitive
            )

            // Deduplicate: prefer content results (have line info)
            let contentPaths = Set(contentResults.map(\.path))
            let dedupedFiles = fileResults.filter { !contentPaths.contains($0.path) }
            let combined = dedupedFiles + contentResults

            await MainActor.run {
                model.searchResults = combined
            }
        }
    }
}

// MARK: - Search Result Row

/// A single search result row showing the file name and, for content matches,
/// the matched line content and line number.
private struct SearchResultRow: View {
    let result: SearchResult
    let theme: Theme

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            // File name
            HStack(spacing: 0) {
                Text(result.name)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(Color(theme.fg))
                    .lineLimit(1)
                    .truncationMode(.middle)

                if let lineNumber = result.lineNumber {
                    Text(":\(lineNumber)")
                        .font(.system(size: 11))
                        .foregroundColor(Color(theme.fgDark))
                }
            }

            // Line content (for content matches)
            if let lineContent = result.lineContent {
                Text(lineContent.trimmingCharacters(in: .whitespaces))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(Color(theme.fgDark))
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .padding(.vertical, 2)
    }
}
