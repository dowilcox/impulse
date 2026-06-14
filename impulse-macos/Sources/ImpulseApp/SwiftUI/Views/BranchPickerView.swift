import SwiftUI

/// Warp-style branch switcher shown from the input bar's git chip: a search
/// field over a filtered list of the repo's local branches. Selecting one
/// invokes `onSelect` (which runs `git checkout`).
struct BranchPickerView: View {
  let currentBranch: String
  let cwd: String
  /// Theme accent so the picker matches the app's theme rather than the macOS
  /// system accent (which is unrelated to the chosen color scheme).
  var accent: Color = .accentColor
  let onSelect: (String) -> Void

  @State private var query: String = ""
  @State private var branches: [String] = []
  @State private var highlighted: Int = 0
  @FocusState private var searchFocused: Bool

  private var filtered: [String] {
    let trimmed = query.trimmingCharacters(in: .whitespaces)
    let base = trimmed.isEmpty
      ? branches
      : branches.filter { $0.localizedCaseInsensitiveContains(trimmed) }
    // Current branch first, the rest in their existing (alphabetical) order.
    return base.filter { $0 == currentBranch } + base.filter { $0 != currentBranch }
  }

  var body: some View {
    VStack(spacing: 0) {
      // Search field.
      HStack(spacing: 6) {
        Image(systemName: "magnifyingglass")
          .font(.system(size: 11))
          .foregroundStyle(.secondary)
        TextField("Search branches…", text: $query)
          .textFieldStyle(.plain)
          .font(.system(size: 12))
          .focused($searchFocused)
          .onSubmit { selectHighlighted() }
          .onChange(of: query) { _, _ in highlighted = 0 }
          .onKeyPress(.downArrow) {
            highlighted = min(highlighted + 1, max(0, filtered.count - 1))
            return .handled
          }
          .onKeyPress(.upArrow) {
            highlighted = max(highlighted - 1, 0)
            return .handled
          }
      }
      .padding(.horizontal, 10)
      .padding(.vertical, 8)

      Divider()

      // Branch list.
      if filtered.isEmpty {
        Text(branches.isEmpty ? "No branches" : "No matches")
          .font(.system(size: 12))
          .foregroundStyle(.secondary)
          .frame(maxWidth: .infinity)
          .padding(.vertical, 16)
      } else {
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 1) {
            ForEach(Array(filtered.enumerated()), id: \.element) { index, branch in
              branchRow(branch, isHighlighted: index == highlighted)
            }
          }
          .padding(6)
        }
        .frame(height: min(CGFloat(filtered.count) * 29 + 12, 420))
      }
    }
    .frame(width: 320)
    .onAppear {
      branches = ImpulseCore.gitBranches(path: cwd)
      searchFocused = true
    }
  }

  private func branchRow(_ branch: String, isHighlighted: Bool) -> some View {
    Button {
      onSelect(branch)
    } label: {
      HStack(spacing: 8) {
        Image(systemName: "arrow.triangle.branch")
          .font(.system(size: 11))
          .foregroundStyle(branch == currentBranch ? accent : .secondary)
        Text(branch)
          .font(.system(size: 12.5))
          .foregroundStyle(.primary)
          .lineLimit(1)
          .truncationMode(.middle)
        Spacer(minLength: 4)
        if branch == currentBranch {
          Image(systemName: "checkmark")
            .font(.system(size: 10, weight: .semibold))
            .foregroundStyle(accent)
        }
      }
      .padding(.horizontal, 8)
      .frame(height: 28)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(
        RoundedRectangle(cornerRadius: 5, style: .continuous)
          .fill(isHighlighted ? accent.opacity(0.18) : Color.clear)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }

  private func selectHighlighted() {
    guard filtered.indices.contains(highlighted) else { return }
    onSelect(filtered[highlighted])
  }
}
