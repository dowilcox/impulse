import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

/// "Manage Branches…": every local branch with its upstream, how far it is
/// ahead/behind, when it last changed and whether it's merged; switch,
/// merge, rebase, compare, rename, publish or delete from here.
@Observable
final class BranchManagerModel {
  let repository: GitRepositoryState
  private(set) var branches: [GitOperations.BranchInfo] = []
  private(set) var base: String?
  private(set) var isLoading = true
  var filter = ""

  init(repository: GitRepositoryState) {
    self.repository = repository
    reload()
  }

  func reload() {
    let root = repository.root
    isLoading = true
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let base = GitClient.defaultBaseBranch(repoPath: root).map { $0.hasPrefix("origin/") ? String($0.dropFirst(7)) : $0 }
      let list = GitOperations.branchDetails(root: root, base: base)
      DispatchQueue.main.async {
        self?.base = base
        self?.branches = list
        self?.isLoading = false
      }
    }
  }

  /// The checked-out branch.
  var current: String? { branches.first(where: \.isCurrent)?.name }

  var visible: [GitOperations.BranchInfo] {
    let query = filter.trimmingCharacters(in: .whitespaces).lowercased()
    guard !query.isEmpty else { return branches }
    return branches.filter { $0.name.lowercased().contains(query) }
  }

  /// Untouched for a month, and not the current branch.
  func isStale(_ branch: GitOperations.BranchInfo) -> Bool {
    !branch.isCurrent && Date().timeIntervalSince(branch.lastCommit) > 30 * 24 * 3600
  }
}

struct BranchManagerView: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: BranchManagerModel
  let onAction: (BranchAction, GitOperations.BranchInfo) -> Void
  let onClose: () -> Void

  private static let relative: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
  }()

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack {
        Text("Branches")
          .font(ChromeFont.ui(15, weight: .semibold))
          .foregroundStyle(chrome.text)
        if let base = model.base {
          Text("merged means merged into \(base)")
            .font(ChromeFont.ui(11))
            .foregroundStyle(chrome.textTertiary)
        }
        Spacer()
        TextField("Filter", text: $model.filter)
          .textFieldStyle(.roundedBorder)
          .frame(width: 180)
      }
      ScrollView {
        LazyVStack(spacing: 1) {
          ForEach(model.visible, id: \.name) { branch in
            row(branch)
          }
        }
      }
      .frame(height: 340)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.content))
      HStack {
        if model.isLoading {
          ProgressRing(progress: nil, color: chrome.textTertiary, size: 12, lineWidth: 1.5)
        }
        Spacer()
        ChromeButton(title: "Done", kind: .primary) { onClose() }
          .keyboardShortcut(.defaultAction)
      }
    }
    .padding(20)
    .frame(width: 640)
    .background(chrome.overlay)
  }

  private func row(_ branch: GitOperations.BranchInfo) -> some View {
    HStack(spacing: 8) {
      Icon(branch.isCurrent ? .circleDot : .gitBranch, size: 12)
        .foregroundStyle(branch.isCurrent ? chrome.accent : chrome.textTertiary)
      VStack(alignment: .leading, spacing: 1) {
        HStack(spacing: 6) {
          Text(branch.name)
            .font(ChromeFont.ui(12.5, weight: branch.isCurrent ? .semibold : .regular))
            .foregroundStyle(chrome.text)
            .lineLimit(1)
          if branch.isMerged { badge("merged", chrome.success) }
          if model.isStale(branch) { badge("stale", chrome.textTertiary) }
          if branch.upstreamGone { badge("upstream gone", chrome.warning) }
        }
        Text(
          [
            branch.upstream.map { "→ \($0)" } ?? "not published",
            branch.ahead > 0 ? "↑\(branch.ahead)" : nil, branch.behind > 0 ? "↓\(branch.behind)" : nil,
            Self.relative.localizedString(for: branch.lastCommit, relativeTo: Date()), branch.subject,
          ].compactMap { $0 }.joined(separator: " · ")
        )
        .font(ChromeFont.ui(10.5))
        .foregroundStyle(chrome.textTertiary)
        .lineLimit(1)
        .truncationMode(.tail)
      }
      Spacer(minLength: 6)
      if !branch.isCurrent {
        ChromeButton(title: "Switch", kind: .secondary) { onAction(.switchTo, branch) }
      }
      ChromeMenuButton(help: "More") {
        var items: [ChromeMenuItem] = []
        if !branch.isCurrent {
          let current = model.current ?? "Current Branch"
          items.append(ChromeMenuItem("Merge into \(current)") { onAction(.merge, branch) })
          items.append(ChromeMenuItem("Rebase \(current) onto This") { onAction(.rebase, branch) })
          items.append(.separator)
          items.append(ChromeMenuItem("Compare with Current Branch") { onAction(.compare, branch) })
        }
        items.append(ChromeMenuItem("Show History") { onAction(.history, branch) })
        items.append(ChromeMenuItem("Rename…") { onAction(.rename, branch) })
        if branch.upstream == nil {
          items.append(ChromeMenuItem("Publish to origin") { onAction(.publish, branch) })
        }
        items.append(.separator)
        items.append(ChromeMenuItem("Delete…", isEnabled: !branch.isCurrent) { onAction(.delete, branch) })
        return items
      } label: {
        Icon(.ellipsis, size: 13).foregroundStyle(chrome.textSecondary).frame(width: 22, height: 22)
      }
    }
    .padding(.horizontal, 10)
    .frame(height: 44)
  }

  private func badge(_ text: String, _ color: Color) -> some View {
    Text(text)
      .font(ChromeFont.ui(9.5, weight: .semibold))
      .foregroundStyle(color)
      .padding(.horizontal, 5)
      .frame(height: 15)
      .background(RoundedRectangle(cornerRadius: 3).fill(color.opacity(0.14)))
  }
}

enum BranchAction { case switchTo, compare, history, rename, publish, delete, merge, rebase }

extension MainWindowController {
  func presentBranchManager() {
    guard let window, let repository = windowModel.repository else {
      toasts.show(Toast(kind: .info, message: "Not in a git repository."))
      return
    }
    let model = BranchManagerModel(repository: repository)
    let palette = windowModel.palette
    let sheet = NSWindow.themedSheet(palette: palette)
    let sheetHost = SheetGitHost(base: self, sheet: sheet, palette: palette)
    window.beginThemedSheet(
      sheet, palette: palette,
      content: BranchManagerView(
        model: model,
        onAction: { [weak self, weak window, weak sheet] action, branch in
          guard let self else { return }
          // Actions that move elsewhere close the sheet first.
          let leaves = [.switchTo, .compare, .history, .merge, .rebase].contains(action)
          if leaves, let sheet { window?.endSheet(sheet) }
          self.performBranchAction(
            action, branch: branch, model: model, sheet: sheet, host: leaves ? self : sheetHost)
        },
        onClose: { [weak window, weak sheet] in
          if let sheet { window?.endSheet(sheet) }
        }
      ))
  }

  private func performBranchAction(
    _ action: BranchAction, branch: GitOperations.BranchInfo, model: BranchManagerModel, sheet: NSWindow?,
    host: GitPanelHost
  ) {
    let repository = model.repository
    let actions = GitActions(repository: repository, host: host)
    switch action {
    case .switchTo:
      actions.switchBranch(branch.name)
    case .compare:
      gitOpenReview(scope: .branch(base: branch.name), focusPath: nil)
    case .history:
      tabManager.addHistoryTab(repository: repository, host: self)
    case .publish:
      repository.run("Publishing \(branch.name)…") {
        GitOperations.push(setUpstream: true, branch: branch.name, root: $0)
      } completion: { result, _ in
        if case .failure(let error) = result { host.gitPresentError(error, title: "Couldn't publish") }
        model.reload()
      }
    case .rename:
      ask(on: sheet, title: "Rename \(branch.name)", initial: branch.name, confirm: "Rename") { [weak self] name in
        guard name != branch.name else { return }
        guard PaletteModel.isValidBranchName(name) else {
          host.toasts.show(Toast(kind: .warning, message: "“\(name)” isn't a valid branch name."))
          return
        }
        repository.run { GitOperations.renameBranch(branch.name, to: name, root: $0) } completion: {
          result, _ in
          if case .failure(let error) = result { host.gitPresentError(error, title: "Couldn't rename") }
          model.reload()
        }
      }
    case .delete:
      actions.deleteBranch(branch.name, base: model.base) { model.reload() }
    case .merge:
      actions.merge(branch.name, label: branch.name)
    case .rebase:
      actions.rebase(onto: branch.name, label: branch.name)
    }
  }

  private func ask(
    on sheet: NSWindow?, title: String, initial: String, confirm: String, then: @escaping (String) -> Void
  ) {
    guard let target = sheet ?? window else { return }
    let alert = NSAlert()
    alert.messageText = title
    alert.addButton(withTitle: confirm)
    alert.addButton(withTitle: "Cancel")
    let field = NSTextField(string: initial)
    field.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
    alert.accessoryView = field
    alert.window.initialFirstResponder = field
    alert.beginSheetModal(for: target) { response in
      if response == .alertFirstButtonReturn {
        then(field.stringValue.trimmingCharacters(in: .whitespaces))
      }
    }
  }
}

/// Reports git actions started in a sheet on that sheet: toasts (and their
/// Undo) appear over it, where they can be clicked while it's open.
/// Everything else goes to the window.
final class SheetGitHost: GitPanelHost {
  private weak var base: MainWindowController?
  private weak var sheet: NSWindow?
  let toasts = ToastCenter()

  init(base: MainWindowController, sheet: NSWindow, palette: ChromePalette) {
    self.base = base
    self.sheet = sheet
    toasts.attach(to: sheet) { palette }
  }

  /// The sheet, while it's still up (alerts go on top of it).
  private var openSheet: NSWindow? {
    guard let sheet, sheet.sheetParent != nil, sheet.isVisible else { return nil }
    return sheet
  }

  var agentTargets: [AgentSummary] { base?.agentTargets ?? [] }
  func sendToAgent(_ text: String, terminalID: UUID) { base?.sendToAgent(text, terminalID: terminalID) }
  func gitOpenFile(_ absolutePath: String) { base?.gitOpenFile(absolutePath) }
  func gitOpenDiffEditor(_ absolutePath: String) { base?.gitOpenDiffEditor(absolutePath) }
  func gitOpenReview(scope: DiffScope, focusPath: String?) { base?.gitOpenReview(scope: scope, focusPath: focusPath) }
  func gitPresentError(_ error: GitOperationError, title: String) {
    base?.presentGitError(error, title: title, on: openSheet)
  }
  func gitConfirm(
    title: String, message: String, confirmTitle: String, destructive: Bool,
    completion: @escaping (Bool) -> Void
  ) {
    guard let base else { return completion(false) }
    base.gitConfirm(
      title: title, message: message, confirmTitle: confirmTitle, destructive: destructive, on: openSheet,
      completion: completion)
  }
}
