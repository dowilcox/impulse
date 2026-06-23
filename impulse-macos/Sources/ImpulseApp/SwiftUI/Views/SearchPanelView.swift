import AppKit
import SwiftUI

// MARK: - Sidebar Search Bar

/// The project-search input, pinned at the top of the sidebar's lower region
/// while search is active (below the vertical tab list). Holds the text field,
/// the match-case toggle, and a close button. The search lifecycle lives on
/// `WindowModel` (see `WindowModel+Search`); this view only drives it.
///
/// Exit paths all converge on restoring the file tree:
///   - ✕ button or Escape on an empty field → `resetSearch()` (leaves search).
///   - Escape with text → clears the text; the bar stays and the tree shows
///     beneath it (a second Escape then leaves search).
struct SidebarSearchBar: View {
  @Bindable var model: WindowModel
  @FocusState private var fieldFocused: Bool

  var body: some View {
    HStack(spacing: 6) {
      Image(systemName: "magnifyingglass")
        .font(.system(size: 11))
        .foregroundStyle(.secondary)

      TextField("Search project…", text: $model.searchQuery)
        .textFieldStyle(.plain)
        .font(.system(size: 12))
        .focused($fieldFocused)
        .onSubmit { model.runSearchNow() }
        .onChange(of: model.searchQuery) { _, _ in model.scheduleSearch() }
        .onKeyPress(.escape) {
          if model.searchQuery.isEmpty {
            model.resetSearch()
          } else {
            model.searchQuery = ""
          }
          return .handled
        }

      // Match-case toggle.
      Button {
        model.searchCaseSensitive.toggle()
        model.runSearchNow()
      } label: {
        Text("Aa")
          .font(.system(size: 11, weight: .medium, design: .monospaced))
          .foregroundStyle(
            model.searchCaseSensitive ? model.theme.colorAccent : .secondary
          )
          .padding(.horizontal, 6)
          .padding(.vertical, 2)
          .background(
            RoundedRectangle(cornerRadius: 4)
              .fill(model.searchCaseSensitive
                ? model.theme.colorAccent.opacity(0.15)
                : .clear)
          )
      }
      .buttonStyle(.plain)
      .help("Match Case")

      // Close search and return to the file tree.
      Button {
        model.resetSearch()
      } label: {
        Image(systemName: "xmark.circle.fill")
          .font(.system(size: 12))
          .foregroundStyle(.tertiary)
      }
      .buttonStyle(.plain)
      .help("Close Search")
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 6)
    // Defer focus to the next runloop tick: setting @FocusState synchronously
    // in onAppear (or in onChange while the view is still being committed)
    // races the field joining the responder chain inside NavigationSplitView /
    // NSHostingView, and the focus is silently dropped.
    .onAppear { focusField() }
    .onChange(of: model.searchFocusToken) { _, _ in focusField() }
  }

  private func focusField() {
    DispatchQueue.main.async { fieldFocused = true }
  }
}

// MARK: - Search Results List

/// The scrollable list of search results shown below the search bar when the
/// query is non-empty. Reads its state (`isSearching`, `searchResults`) from
/// `WindowModel`.
struct SearchResultsList: View {
  var model: WindowModel

  var body: some View {
    VStack(spacing: 0) {
      if model.isSearching {
        ProgressView()
          .controlSize(.small)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else if model.searchResults.isEmpty {
        Text("No results")
          .font(.system(size: 12))
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        HStack {
          Text("\(model.searchResults.count) results")
            .font(.system(size: 11))
            .foregroundStyle(.tertiary)
          Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)

        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            ForEach(model.searchResults, id: \.stableId) { result in
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
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .onReceive(NotificationCenter.default.publisher(for: .impulseFileTreeChanged)) { _ in
      // Re-run the current search when the project tree changes so results
      // don't go stale after file creates/deletes/renames.
      if !model.searchQuery.isEmpty {
        model.scheduleSearch()
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
