import SwiftUI

/// File-tree action bar shown between the vertical tab list and the file tree.
/// Holds the new-file / new-folder / refresh / collapse / hidden-files actions
/// that previously lived in the window toolbar.
struct SidebarActionBarView: View {
  var model: WindowModel

  var body: some View {
    HStack(spacing: 2) {
      actionButton(symbol: "doc.badge.plus", help: "New File") {
        model.onCreateFile?()
      }
      actionButton(symbol: "folder.badge.plus", help: "New Folder") {
        model.onCreateFolder?()
      }
      actionButton(symbol: "arrow.clockwise", help: "Refresh File Tree") {
        model.onRefreshTree?()
      }
      actionButton(
        symbol: "arrow.up.left.and.arrow.down.right", help: "Collapse All Folders"
      ) {
        model.onCollapseAll?()
      }

      Spacer(minLength: 0)

      actionButton(
        symbol: model.showHiddenFiles ? "eye" : "eye.slash",
        help: model.showHiddenFiles ? "Hide Hidden Files" : "Show Hidden Files",
        isActive: model.showHiddenFiles
      ) {
        model.onToggleHidden?()
      }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 5)
  }

  private func actionButton(
    symbol: String, help: String, isActive: Bool = false, action: @escaping () -> Void
  ) -> some View {
    Button(action: action) {
      Image(systemName: symbol)
        .font(.system(size: 12, weight: .medium))
        .foregroundStyle(isActive ? model.theme.colorAccent : Color.secondary)
        .frame(width: 24, height: 22)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .help(help)
    .accessibilityLabel(help)
  }
}
