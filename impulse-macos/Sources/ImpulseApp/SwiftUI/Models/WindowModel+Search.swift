import AppKit
import Foundation

/// Search lifecycle for the sidebar search bar.
///
/// Execution state (the generation counter, debounce work, and the in-flight
/// task) lives on the shared `WindowModel` rather than the search view so it
/// survives the view being destroyed/recreated when the sidebar swaps between
/// the file tree and the search results. Keeping the generation counter on the
/// stable model is what prevents a stale, superseded search from writing its
/// results back after a newer search (or a reset) has started.
///
/// All methods here run on the main thread (called from SwiftUI / AppKit); the
/// actual FFI search is dispatched off-thread and hops back via `MainActor`.
extension WindowModel {
  /// Enter search mode: reveal the sidebar, switch to the search panel, and
  /// request keyboard focus for the search field. Preserves any existing query
  /// (re-running it) so reopening search shows the last results.
  func beginSearch() {
    if !sidebarVisible { sidebarVisible = true }
    sidebarPanel = .search
    requestSearchFocus()
    if !searchQuery.isEmpty { scheduleSearch() }
  }

  /// Ask the sidebar search field to grab keyboard focus.
  func requestSearchFocus() {
    searchFocusToken += 1
  }

  /// Exit search mode and restore the file tree. The single convergence point
  /// for every exit path (Escape on an empty field, the ✕ button, switching
  /// project root).
  func resetSearch() {
    searchDebounceWork?.cancel()
    searchDebounceWork = nil
    searchTask?.cancel()
    searchTask = nil
    searchGeneration &+= 1
    searchQuery = ""
    searchResults = []
    isSearching = false
    sidebarPanel = .files
  }

  /// Debounce a search 300ms out, coalescing rapid keystrokes.
  func scheduleSearch() {
    searchDebounceWork?.cancel()
    let work = DispatchWorkItem { [weak self] in self?.performSearch() }
    searchDebounceWork = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
  }

  /// Run the current query immediately, bypassing the debounce (used by the
  /// match-case toggle and Enter).
  func runSearchNow() {
    searchDebounceWork?.cancel()
    searchDebounceWork = nil
    performSearch()
  }

  /// Execute the search off the main thread, applying results only if this is
  /// still the latest generation.
  private func performSearch() {
    guard !searchQuery.isEmpty, !fileTreeRootPath.isEmpty else {
      searchResults = []
      isSearching = false
      return
    }
    let query = searchQuery
    let root = fileTreeRootPath
    let caseSensitive = searchCaseSensitive

    searchGeneration &+= 1
    let generation = searchGeneration
    searchTask?.cancel()
    isSearching = true

    searchTask = Task.detached {
      let combined = ImpulseCore.searchAll(
        root: root, query: query, caseSensitive: caseSensitive)

      await MainActor.run { [weak self] in
        // Only apply if this is still the latest search.
        guard let self, generation == self.searchGeneration else { return }
        self.searchResults = combined
        self.isSearching = false
      }
    }
  }
}
