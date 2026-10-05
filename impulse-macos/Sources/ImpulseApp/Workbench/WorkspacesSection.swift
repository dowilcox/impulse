import AppKit
import ImpulseGit
import SwiftUI

/// The top of the left dock: the window's workspaces, with worktrees of the
/// same repository grouped under it. Each row shows the branch, pending
/// changes and tabs that need attention; expanding a row lists its tabs.
struct WorkspacesSection: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  @AppStorage("workspacesSectionExpanded") private var isExpanded = true

  private static let rowHeight: CGFloat = 26
  private static let tabRowHeight: CGFloat = 24
  private static let maxHeight: CGFloat = 280

  var body: some View {
    VStack(spacing: 0) {
      SectionHeader(
        title: "Workspaces", count: model.workspaces.count > 1 ? model.workspaces.count : nil,
        isExpanded: $isExpanded
      ) {
        ChromeIconButton(
          icon: .plus, help: "Open Folder as Workspace…", size: 20, iconSize: 12
        ) {
          model.onOpenWorkspace?()
        }
      }
      if isExpanded {
        ScrollView(.vertical) {
          VStack(spacing: 1) {
            ForEach(groups, id: \.id) { group in
              if let name = group.name {
                RepoGroupHeader(name: name)
              }
              ForEach(group.workspaces) { workspace in
                WorkspaceRow(model: model, workspace: workspace, indented: group.name != nil)
                if workspace.isExpanded {
                  ForEach(workspace.tabs) { tab in
                    WorkspaceTabRow(model: model, tab: tab)
                  }
                }
              }
            }
          }
          .padding(.bottom, 6)
        }
        .scrollIndicators(.never)
        .frame(height: min(contentHeight, Self.maxHeight))
      }
      Hairline()
    }
  }

  // MARK: Grouping

  private struct Group {
    let id: String
    /// Repository name when two or more workspaces share it.
    let name: String?
    let workspaces: [WorkspaceInfo]
  }

  /// Workspaces in sidebar order, with those that share a repository (its
  /// worktrees) gathered under the first one's position.
  private var groups: [Group] {
    var order: [String] = []
    var members: [String: [WorkspaceInfo]] = [:]
    for workspace in model.workspaces {
      let key = workspace.repository?.snapshot?.commonDir ?? workspace.id.uuidString
      if members[key] == nil { order.append(key) }
      members[key, default: []].append(workspace)
    }
    return order.map { key in
      let list = members[key] ?? []
      let name: String? =
        list.count > 1
        ? ((key as NSString).deletingLastPathComponent as NSString).lastPathComponent : nil
      return Group(id: key, name: name, workspaces: list)
    }
  }

  private var contentHeight: CGFloat {
    let groupHeaders = groups.filter { $0.name != nil }.count
    let rows = model.workspaces.count
    let tabRows = model.workspaces.filter(\.isExpanded).reduce(0) { $0 + $1.tabs.count }
    return CGFloat(groupHeaders) * 22 + CGFloat(rows) * (Self.rowHeight + 1)
      + CGFloat(tabRows) * (Self.tabRowHeight + 1) + 6
  }
}

// MARK: - Rows

private struct RepoGroupHeader: View {
  @Environment(\.chrome) private var chrome
  let name: String

  var body: some View {
    HStack(spacing: 5) {
      Icon(.gitFork, size: 11).foregroundStyle(chrome.textTertiary)
      Text(name)
        .font(ChromeFont.ui(11, weight: .medium))
        .foregroundStyle(chrome.textSecondary)
        .lineLimit(1)
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 14)
    .frame(height: 22)
  }
}

private struct WorkspaceRow: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let workspace: WorkspaceInfo
  let indented: Bool
  @State private var hovering = false

  var body: some View {
    let snapshot = workspace.repository?.snapshot
    HStack(spacing: 6) {
      Button {
        model.onSetWorkspaceExpanded?(workspace.id, !workspace.isExpanded)
      } label: {
        Icon(workspace.isExpanded ? .chevronDown : .chevronRight, size: 10)
          .foregroundStyle(chrome.textTertiary)
          .frame(width: 12, height: 16)
          .contentShape(Rectangle())
      }
      .buttonStyle(ChromePressStyle())
      .opacity(hovering || workspace.isExpanded ? 1 : 0)
      .help(workspace.isExpanded ? "Hide tabs" : "Show tabs")

      leadingIcon
      Text(workspace.name)
        .font(ChromeFont.ui(12, weight: workspace.isActive ? .semibold : .regular))
        .foregroundStyle(workspace.isActive ? chrome.text : chrome.textSecondary)
        .lineLimit(1)
        .truncationMode(.middle)
      if let branch = snapshot?.branch ?? snapshot?.headOid.map({ String($0.prefix(7)) }) {
        Text(branch)
          .font(ChromeFont.mono(10.5))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .truncationMode(.middle)
          .layoutPriority(-1)
      } else if workspace.isScratch {
        Text("follows tab")
          .font(ChromeFont.ui(10.5))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .layoutPriority(-1)
      }
      Spacer(minLength: 4)
      trailing(snapshot: snapshot)
    }
    .padding(.leading, indented ? 16 : 8)
    .padding(.trailing, 12)
    .frame(height: 26)
    // When its tabs are listed, the selected tab carries the highlight.
    .rowBackground(selected: workspace.isActive && !workspace.isExpanded, hovered: hovering)
    .overlay(alignment: .leading) {
      if workspace.isActive {
        Capsule().fill(chrome.accent).frame(width: 2, height: 14).padding(.leading, 3)
      }
    }
    .contentShape(Rectangle())
    .onTapGesture { model.onSelectWorkspace?(workspace.id) }
    .onHover { hovering = $0 }
    .contextMenu { contextMenu }
    .help(help(snapshot: snapshot))
    .accessibilityElement(children: .combine)
    .accessibilityLabel(accessibilityLabel(snapshot: snapshot))
    .accessibilityAddTraits(workspace.isActive ? [.isSelected, .isButton] : [.isButton])
  }

  @ViewBuilder
  private var leadingIcon: some View {
    if let progress = workspace.progress {
      ProgressRing(progress: progress.fraction, color: chrome.working, size: 12, lineWidth: 1.6)
    } else {
      Icon(
        workspace.isScratch ? .squareTerminal : (workspace.repository != nil ? .folderGit2 : .folder),
        size: 13
      )
      .foregroundStyle(workspace.isActive ? chrome.accent : chrome.textTertiary)
    }
  }

  @ViewBuilder
  private func trailing(snapshot: RepoSnapshot?) -> some View {
    HStack(spacing: 6) {
      if workspace.agentsWorking > 0 {
        ProgressRing(progress: nil, color: chrome.working, size: 10, lineWidth: 1.4)
          .help("\(workspace.agentsWorking) agent(s) working")
      }
      if workspace.agentsWaiting > 0 {
        HStack(spacing: 2) {
          Icon(.bot, size: 10)
          Text("\(workspace.agentsWaiting)").font(ChromeFont.mono(10))
        }
        .foregroundStyle(chrome.attention)
        .help("\(workspace.agentsWaiting) agent(s) waiting for you")
      }
      if let snapshot, snapshot.changedFileCount > 0 {
        DiffStat(added: snapshot.totalAdded, removed: snapshot.totalRemoved)
      }
      if workspace.attentionCount > 0 {
        Badge(text: "\(workspace.attentionCount)", color: chrome.attention)
      } else if !workspace.isExpanded, workspace.tabs.count > 1 {
        Text("\(workspace.tabs.count)")
          .font(ChromeFont.mono(10))
          .foregroundStyle(chrome.textTertiary)
      }
    }
  }

  @ViewBuilder
  private var contextMenu: some View {
    Button("Rename…") { model.onRenameWorkspace?(workspace.id) }
    if !workspace.isScratch {
      Button("Reveal in Finder") {
        NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: workspace.root)
      }
      Button("Copy Path") {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(workspace.root, forType: .string)
      }
    }
    Divider()
    Button(workspace.isExpanded ? "Hide Tabs" : "Show Tabs") {
      model.onSetWorkspaceExpanded?(workspace.id, !workspace.isExpanded)
    }
    Divider()
    Button("Close Workspace") { model.onCloseWorkspace?(workspace.id) }
  }

  private func help(snapshot: RepoSnapshot?) -> String {
    var lines = [workspace.isScratch ? "Scratch — the file tree follows the active tab" : workspace.root]
    if let branch = snapshot?.branch { lines.append("Branch: \(branch)") }
    if let snapshot, snapshot.changedFileCount > 0 {
      lines.append("\(snapshot.changedFileCount) changed file(s)")
    }
    lines.append("\(workspace.tabs.count) tab(s)")
    return lines.joined(separator: "\n")
  }

  private func accessibilityLabel(snapshot: RepoSnapshot?) -> String {
    var label = "Workspace \(workspace.name)"
    if let branch = snapshot?.branch { label += ", branch \(branch)" }
    if workspace.attentionCount > 0 { label += ", \(workspace.attentionCount) need attention" }
    return label
  }
}

/// A tab listed under its expanded workspace.
private struct WorkspaceTabRow: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let tab: TabDisplayInfo
  @State private var hovering = false

  var body: some View {
    let selected = tab.index == model.selectedTabIndex
    HStack(spacing: 6) {
      if tab.isTerminal {
        Icon(tab.isDirectInteractionActive ? .squareTerminal : .terminal, size: 12)
          .foregroundStyle(chrome.textTertiary)
      } else if let icon = tab.icon {
        Image(nsImage: icon).resizable().interpolation(.high).frame(width: 13, height: 13)
      } else {
        Icon(.file, size: 12).foregroundStyle(chrome.textTertiary)
      }
      Text(tab.title)
        .font(ChromeFont.ui(11.5))
        .foregroundStyle(selected ? chrome.text : chrome.textSecondary)
        .lineLimit(1)
        .truncationMode(.middle)
      Spacer(minLength: 4)
      if tab.needsAttention {
        StatusDot(color: chrome.attention, size: 6)
      } else if tab.isDirty {
        StatusDot(color: chrome.textSecondary, size: 6)
      }
    }
    .padding(.leading, 40)
    .padding(.trailing, 12)
    .frame(height: 24)
    .rowBackground(selected: selected, hovered: hovering)
    .contentShape(Rectangle())
    .onTapGesture { model.onTabSelected?(tab.index) }
    .onHover { hovering = $0 }
    .help(tab.directory ?? tab.title)
  }
}
