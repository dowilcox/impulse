import AppKit
import SwiftUI

// MARK: - Status bar

/// Always-visible window status bar: repository state on the left, context for
/// the focused tab (editor position/language, or the terminal's shell) on the
/// right.
struct WorkbenchStatusBar: View {
  var model: WindowModel

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
    if model.workspaceRestricted {
      StatusItem(
        icon: .shieldAlert,
        help: "Restricted: language servers, formatters on save and background fetch are off in this folder. Click to trust it.",
        tint: model.palette.warning
      ) {
        model.onTrustWorkspace?()
      } label: {
        Text("Restricted")
      }
    }
    if let branch = model.gitBranch, !branch.isEmpty {
      StatusItem(icon: .gitBranch, help: "Switch branch (⌃⌘B)") {
        model.onShowBranchSwitcher?()
      } label: {
        Text(branch)
      }
    }
    if model.reviewChangedFileCount > 0 {
      StatusItem(icon: .fileDiff, help: "Review changes (⇧⌘G)") {
        model.onOpenDiffReview?()
      } label: {
        HStack(spacing: 5) {
          Text("\(model.reviewChangedFileCount)")
          DiffStat(added: model.reviewAddedLines, removed: model.reviewRemovedLines)
        }
      }
    }
    if model.problemCounts.total > 0 {
      StatusItem(
        icon: model.problemCounts.errors > 0 ? .circleX : .triangleAlert,
        help: "Problems: \(model.problemCounts.errors) errors, \(model.problemCounts.warnings) warnings",
        tint: model.problemCounts.errors > 0 ? model.palette.danger : nil
      ) {
        model.onShowProblems?()
      } label: {
        HStack(spacing: 6) {
          Text(verbatim: "\(model.problemCounts.errors)")
          HStack(spacing: 3) {
            Icon(.triangleAlert, size: 10)
            Text(verbatim: "\(model.problemCounts.warnings)")
          }
          .foregroundStyle(model.problemCounts.warnings > 0 ? model.palette.warning : model.palette.textTertiary)
        }
        .monospacedDigit()
      }
    }
    ForEach(model.ports.prefix(3), id: \.port) { port in
      StatusItem(icon: .globe, help: "\(port.process) is listening on \(port.port) — open in browser") {
        if let url = URL(string: "http://localhost:\(port.port)") { NSWorkspace.shared.open(url) }
      } label: {
        Text(verbatim: ":\(port.port)").monospacedDigit()
      }
    }
    if model.ports.count > 3 {
      StatusItem(help: model.ports.dropFirst(3).map { ":\($0.port) \($0.process)" }.joined(separator: "\n")) {
        Text(verbatim: "+\(model.ports.count - 3)")
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
        Text(verbatim: "Ln \(line), Col \(col)")
      }
      if let indent = model.currentIndent {
        StatusItem(help: "Indentation", label: { Text(indent) })
      }
      StatusItem(help: "Encoding", label: { Text(model.currentEncoding) })
    }
    if let progress = model.lspProgress {
      StatusItem(help: "\(progress.server): \([progress.title, progress.message].compactMap { $0 }.joined(separator: " — "))") {
        HStack(spacing: 5) {
          ProgressRing(progress: progress.fraction, color: model.palette.textSecondary, size: 10, lineWidth: 1.4)
          Text(progress.title).lineLimit(1)
          if let fraction = progress.fraction {
            Text(verbatim: "\(Int(fraction * 100))%").monospacedDigit()
          }
        }
      }
    }
    if let lang = model.currentLanguage {
      StatusItem(icon: .code, help: "Language", label: { Text(lang) })
    }
    if model.isPreviewable {
      StatusItem(
        icon: model.isPreviewing ? .eyeOff : .eye,
        help: model.isPreviewing ? "Hide preview (⇧⌘M)" : "Show preview (⇧⌘M)",
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
  @State private var dockHeight: CGFloat = 0

  /// What a dragged workspaces section leaves for the files panel.
  private static let filesPanelMinHeight: CGFloat = 120

  var body: some View {
    VStack(spacing: 0) {
      WorkspacesSection(
        model: model,
        heightLimit: dockHeight > 0 ? max(0, dockHeight - Self.filesPanelMinHeight) : nil
      )
      // Above the header, so the resize handle's grab band overlapping its
      // top edge gets the mouse.
      .zIndex(1)
      header
      switch model.sidebarPanel {
      case .search:
        SidebarSearchBar(model: model)
        Hairline()
        if model.searchQuery.isEmpty {
          FileTreeListView(model: model)
        } else {
          SearchResultsList(model: model)
        }
      case .changes:
        Hairline()
        ChangesPanelView(model: model)
      case .files:
        FileTreeListView(model: model)
      }
    }
    .background(model.palette.chrome)
    .environment(\.chrome, model.palette)
    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { dockHeight = $0 }
  }

  private var header: some View {
    HStack(spacing: 2) {
      DockSegmentedTabs(
        selection: Binding(
          get: {
            switch model.sidebarPanel {
            case .files: return 0
            case .changes: return 1
            case .search: return 2
            }
          },
          set: { index in
            switch index {
            case 1:
              model.resetSearch()
              model.sidebarPanel = .changes
            case 2:
              model.beginSearch()
            default:
              model.resetSearch()
              model.sidebarPanel = .files
            }
          }),
        items: [
          ("Files", LucideIcon.folderTree),
          (changesLabel, LucideIcon.gitBranch),
          ("Search", LucideIcon.search),
        ]
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

extension LeftDockView {
  /// "Changes" plus the changed-file count when there are changes.
  fileprivate var changesLabel: String {
    let count = model.reviewChangedFileCount
    return count > 0 ? "Changes \(count)" : "Changes"
  }
}

/// Text segments with an underline on the selected one (dock headers).
/// When the dock is too narrow for every label, only the selected segment
/// keeps its label, then none do (icons, with the label as help).
struct DockSegmentedTabs: View {
  @Environment(\.chrome) private var chrome
  @Binding var selection: Int
  let items: [(String, LucideIcon)]

  private enum Labels { case all, selected, none }

  var body: some View {
    ViewThatFits(in: .horizontal) {
      row(.all)
      row(.selected)
      row(.none)
    }
  }

  private func row(_ labels: Labels) -> some View {
    HStack(spacing: 2) {
      ForEach(Array(items.enumerated()), id: \.offset) { index, item in
        let selected = index == selection
        let showsLabel = labels == .all || (labels == .selected && selected)
        Button {
          selection = index
        } label: {
          HStack(spacing: 5) {
            Icon(item.1, size: 12)
            if showsLabel {
              Text(item.0).font(ChromeFont.ui(11.5, weight: selected ? .semibold : .medium)).lineLimit(1)
            }
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
        .help(showsLabel ? "" : item.0)
        .accessibilityLabel(item.0)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
      }
    }
    .fixedSize()
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
    if showsComposer {
      AgentComposerView(model: model)
        .environment(\.chrome, model.palette)
    } else if showsInput, let agent = toolbeltAgent {
      VStack(spacing: 0) {
        AgentToolbelt(model: model, agent: agent)
        TerminalContextBarView(model: model)
      }
      .environment(\.chrome, model.palette)
    } else if let agent = toolbeltAgent {
      AgentToolbelt(model: model, agent: agent)
        .environment(\.chrome, model.palette)
    } else if showsInput {
      TerminalContextBarView(model: model)
        .environment(\.chrome, model.palette)
    }
  }

  /// ⌘I over a program that owns the terminal (where the input bar hides).
  private var showsComposer: Bool {
    model.composerVisible && model.terminalDirectInteraction
      && (model.selectedTabInfo?.isTerminal ?? false)
  }

  /// An agent runs in the focused terminal: show its toolbelt.
  private var toolbeltAgent: AgentSummary? {
    guard model.selectedTabInfo?.isTerminal ?? false, let agent = model.focusedAgent,
      agent.state != .exited
    else { return nil }
    return agent
  }

  private var showsInput: Bool {
    guard model.contextBarEnabled, !model.terminalDirectInteraction else { return false }
    return model.selectedTabInfo?.isTerminal ?? false
  }
}
