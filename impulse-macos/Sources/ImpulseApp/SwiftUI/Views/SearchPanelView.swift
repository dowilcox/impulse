import AppKit
import ImpulseKit
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
    VStack(spacing: 0) {
      searchRow
      if model.searchReplaceVisible { replaceRow }
    }
  }

  private var replaceRow: some View {
    HStack(spacing: 6) {
      Image(systemName: "arrow.2.squarepath")
        .font(.system(size: 10))
        .foregroundStyle(.secondary)
      TextField("Replace with…", text: $model.searchReplacement)
        .textFieldStyle(.plain)
        .font(.system(size: 12))
        .onSubmit { model.onReplaceAll?() }
      Button("Replace All") { model.onReplaceAll?() }
        .buttonStyle(.plain)
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(model.searchResults.contains { $0.matchType == "content" } ? model.theme.colorAccent : .secondary)
        .disabled(!model.searchResults.contains { $0.matchType == "content" })
        .help("Replace every match in the files listed (⌘↩ in the field)")
    }
    .padding(.horizontal, 10)
    .padding(.bottom, 6)
  }

  private var searchRow: some View {
    HStack(spacing: 6) {
      Button {
        model.searchReplaceVisible.toggle()
      } label: {
        Image(systemName: model.searchReplaceVisible ? "chevron.down" : "chevron.right")
          .font(.system(size: 9, weight: .semibold))
          .foregroundStyle(.secondary)
          .frame(width: 10)
      }
      .buttonStyle(.plain)
      .help(model.searchReplaceVisible ? "Hide Replace" : "Replace")
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
    .onChange(of: fieldFocused) { _, focused in model.noteSidebarFocus(.search, focused: focused) }
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
              SearchResultRow(
                result: result,
                replacement: model.searchReplaceVisible ? model.searchReplacement : nil,
                query: model.searchQuery, caseSensitive: model.searchCaseSensitive, theme: model.theme)
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
  /// When replacing: the line shown with matches struck and replaced.
  var replacement: String?
  var query: String = ""
  var caseSensitive = false
  var theme: Theme? = nil

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

      if let lineContent = result.lineContent, let replacement, let theme {
        previewText(lineContent.trimmingCharacters(in: .whitespaces), replacement: replacement, theme: theme)
          .font(.system(size: 11, design: .monospaced))
          .lineLimit(1)
          .truncationMode(.tail)
      } else if let lineContent = result.lineContent {
        Text(lineContent.trimmingCharacters(in: .whitespaces))
          .font(.system(size: 11, design: .monospaced))
          .foregroundStyle(.secondary)
          .lineLimit(1)
          .truncationMode(.tail)
      }
    }
  }

  /// The line with each match struck through and the replacement after it.
  private func previewText(_ line: String, replacement: String, theme: Theme) -> Text {
    ProjectReplace.preview(line: line, query: query, replacement: replacement, caseSensitive: caseSensitive)
      .reduce(Text("")) { text, segment in
        switch segment {
        case .same(let part):
          return text + Text(part).foregroundColor(.secondary)
        case .removed(let part):
          return text + Text(part).strikethrough().foregroundColor(theme.colorRed)
        case .added(let part):
          return text + Text(part).foregroundColor(theme.colorGreen)
        }
      }
  }
}
