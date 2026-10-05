import AppKit
import SwiftUI

// MARK: - Status bar

/// Always-visible window status bar: repository state on the left, context for
/// the focused tab (editor position/language, or the terminal's shell) on the
/// right.
struct WorkbenchStatusBar: View {
  var model: WindowModel
  @State private var showBranchPicker = false

  var body: some View {
    HStack(spacing: 2) {
      leftItems
      Spacer(minLength: 8)
      rightItems
    }
    .padding(.horizontal, 6)
    .frame(height: Metrics.statusBarHeight)
    .background(model.palette.chrome)
    .overlay(alignment: .top) { Hairline() }
    .environment(\.chrome, model.palette)
  }

  @ViewBuilder
  private var leftItems: some View {
    if let branch = model.gitBranch, !branch.isEmpty {
      StatusItem(icon: .gitBranch, help: "Switch branch") {
        showBranchPicker.toggle()
      } label: {
        Text(branch)
      }
      .popover(isPresented: $showBranchPicker, arrowEdge: .top) {
        BranchPickerView(
          currentBranch: branch, cwd: model.currentCwd, accent: model.palette.accent
        ) { selected in
          showBranchPicker = false
          guard selected != branch else { return }
          model.onSwitchBranch?(selected)
        }
      }
    }
    if model.reviewChangedFileCount > 0 {
      StatusItem(icon: .fileDiff, help: "Review changes (⌘⇧G)") {
        model.onOpenDiffReview?()
      } label: {
        HStack(spacing: 5) {
          Text("\(model.reviewChangedFileCount)")
          DiffStat(added: model.reviewAddedLines, removed: model.reviewRemovedLines)
        }
      }
    }
    if !model.currentCwd.isEmpty, model.cursorLine == nil {
      StatusItem(
        icon: .folder, help: model.currentCwd,
        label: { Text(TabManager.abbreviateHomePath(model.currentCwd)) })
    }
  }

  @ViewBuilder
  private var rightItems: some View {
    if let version = model.updateAvailableVersion, let url = model.updateURL {
      StatusItem(icon: .arrowDown, help: "Impulse \(version) is available", tint: model.palette.success)
      {
        NSWorkspace.shared.open(url)
      } label: {
        Text("Update \(version)")
      }
    }
    if let line = model.cursorLine, let col = model.cursorCol {
      StatusItem(help: "Go to Line (⌘G)") {
        NotificationCenter.default.post(name: .impulseGoToLine, object: nil)
      } label: {
        Text("Ln \(line + 1), Col \(col + 1)")
      }
      if let indent = model.currentIndent {
        StatusItem(help: "Indentation", label: { Text(indent) })
      }
      StatusItem(help: "Encoding", label: { Text(model.currentEncoding) })
    }
    if let lang = model.currentLanguage {
      StatusItem(icon: .code, help: "Language", label: { Text(lang) })
    }
    if model.isPreviewable {
      StatusItem(
        icon: model.isPreviewing ? .eyeOff : .eye,
        help: model.isPreviewing ? "Hide preview (⌘⇧M)" : "Show preview (⌘⇧M)",
        tint: model.isPreviewing ? model.palette.accent : nil
      ) {
        model.onPreviewToggle?()
      } label: {
        Text("Preview")
      }
    }
    if !model.shellName.isEmpty, model.cursorLine == nil {
      StatusItem(icon: .terminal, help: "Shell", label: { Text(model.shellName) })
    }
  }
}

/// One status-bar entry: optional icon + label; clickable when `action` is set.
struct StatusItem<Label: View>: View {
  @Environment(\.chrome) private var chrome
  var icon: LucideIcon? = nil
  var help: String
  var tint: Color? = nil
  var action: (() -> Void)? = nil
  @ViewBuilder var label: () -> Label

  @State private var hovering = false

  init(
    icon: LucideIcon? = nil, help: String, tint: Color? = nil,
    action: (() -> Void)? = nil, @ViewBuilder label: @escaping () -> Label
  ) {
    self.icon = icon
    self.help = help
    self.tint = tint
    self.action = action
    self.label = label
  }

  var body: some View {
    let content = HStack(spacing: 4) {
      if let icon { Icon(icon, size: 11.5) }
      label()
    }
    .font(ChromeFont.ui(11))
    .lineLimit(1)
    .foregroundStyle(tint ?? chrome.textSecondary)
    .padding(.horizontal, 6)
    .frame(height: 20)
    .background(
      RoundedRectangle(cornerRadius: Metrics.radiusSmall, style: .continuous)
        .fill(hovering && action != nil ? chrome.hover : .clear)
    )
    .contentShape(Rectangle())

    if let action {
      Button(action: action) { content }
        .buttonStyle(ChromePressStyle())
        .onHover { hovering = $0 }
        .help(help)
    } else {
      content.help(help)
    }
  }
}

// MARK: - Left dock

/// The left dock: a Files / Search switcher with the file actions, above the
/// file tree or project search.
struct LeftDockView: View {
  var model: WindowModel

  var body: some View {
    VStack(spacing: 0) {
      header
      if model.sidebarPanel == .search {
        SidebarSearchBar(model: model)
        Hairline()
        if model.searchQuery.isEmpty {
          FileTreeListView(model: model)
        } else {
          SearchResultsList(model: model)
        }
      } else {
        FileTreeListView(model: model)
      }
    }
    .background(model.palette.chrome)
    .environment(\.chrome, model.palette)
  }

  private var header: some View {
    HStack(spacing: 2) {
      DockSegmentedTabs(
        selection: Binding(
          get: { model.sidebarPanel == .search ? 1 : 0 },
          set: { index in
            if index == 1 {
              model.beginSearch()
            } else {
              model.resetSearch()
            }
          }),
        items: [("Files", LucideIcon.folderTree), ("Search", LucideIcon.search)]
      )
      Spacer(minLength: 4)
      if model.sidebarPanel == .files {
        ChromeIconButton(icon: .filePlus, help: "New File", size: 22, iconSize: 13) {
          model.onCreateFile?()
        }
        ChromeIconButton(icon: .folderPlus, help: "New Folder", size: 22, iconSize: 13) {
          model.onCreateFolder?()
        }
        ChromeIconButton(icon: .refreshCw, help: "Refresh File Tree", size: 22, iconSize: 13) {
          model.onRefreshTree?()
        }
        ChromeIconButton(
          icon: .chevronsDownUp, help: "Collapse All Folders", size: 22, iconSize: 13
        ) {
          model.onCollapseAll?()
        }
        ChromeIconButton(
          icon: model.showHiddenFiles ? .eye : .eyeOff,
          help: model.showHiddenFiles ? "Hide Hidden Files" : "Show Hidden Files",
          size: 22, iconSize: 13, isActive: model.showHiddenFiles
        ) {
          model.onToggleHidden?()
        }
      }
    }
    .padding(.horizontal, 8)
    .frame(height: 34)
  }
}

/// Text segments with an underline on the selected one (dock headers).
struct DockSegmentedTabs: View {
  @Environment(\.chrome) private var chrome
  @Binding var selection: Int
  let items: [(String, LucideIcon)]

  var body: some View {
    HStack(spacing: 2) {
      ForEach(Array(items.enumerated()), id: \.offset) { index, item in
        let selected = index == selection
        Button {
          selection = index
        } label: {
          HStack(spacing: 5) {
            Icon(item.1, size: 12)
            Text(item.0).font(ChromeFont.ui(11.5, weight: selected ? .semibold : .medium))
          }
          .foregroundStyle(selected ? chrome.text : chrome.textTertiary)
          .padding(.horizontal, 7)
          .frame(height: 24)
          .background(
            RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
              .fill(selected ? chrome.raised : .clear)
          )
          .contentShape(Rectangle())
        }
        .buttonStyle(ChromePressStyle())
        .accessibilityAddTraits(selected ? [.isSelected] : [])
      }
    }
  }
}

// MARK: - Banner

/// Shown under the titlebar when settings.json failed to load.
struct WorkbenchBanner: View {
  var model: WindowModel

  var body: some View {
    if let warning = model.settingsLoadWarning {
      HStack(spacing: 10) {
        Icon(.triangleAlert, size: 14).foregroundStyle(model.palette.warning)
        VStack(alignment: .leading, spacing: 2) {
          Text("Settings file could not be loaded")
            .font(ChromeFont.ui(12, weight: .semibold))
            .foregroundStyle(model.palette.text)
          Text(detailText(warning))
            .font(ChromeFont.ui(11))
            .foregroundStyle(model.palette.textSecondary)
            .lineLimit(2)
        }
        Spacer(minLength: 12)
        ChromeButton(title: "Open Settings File", icon: .externalLink) {
          model.onOpenSettingsFile?()
        }
        ChromeIconButton(icon: .x, help: "Dismiss") {
          model.onDismissSettingsWarning?()
        }
      }
      .padding(.horizontal, 12)
      .padding(.vertical, 8)
      .background(model.palette.warning.opacity(0.12))
      .overlay(alignment: .bottom) { Hairline() }
      .environment(\.chrome, model.palette)
      .help(warning.message)
    }
  }

  private func detailText(_ warning: SettingsLoadWarning) -> String {
    if let backupPath = warning.backupPath {
      return "Using defaults. The invalid file was backed up to \(backupPath.path)."
    }
    return "Using defaults. Automatic settings saves are paused until this is fixed."
  }
}

// MARK: - Terminal input host

/// The Warp-style input bar under the active terminal; empty (zero height) for
/// other tabs and while a TUI owns the terminal.
struct TerminalInputHost: View {
  var model: WindowModel

  var body: some View {
    if showsInput {
      TerminalContextBarView(model: model)
        .environment(\.chrome, model.palette)
    }
  }

  private var showsInput: Bool {
    guard model.contextBarEnabled, !model.terminalDirectInteraction else { return false }
    let index = model.selectedTabIndex
    guard index >= 0, index < model.tabDisplayInfos.count else { return false }
    return model.tabDisplayInfos[index].isTerminal
  }
}
