import AppKit
import ImpulseGit
import ImpulseKit
import SwiftUI

// Finish Task: a tab that takes a task from done to landed and cleaned up.
// It commits nothing itself; it checks the task is committed, merges the
// base's new commits into it, runs the check script in a terminal, merges
// the branch into its base and pushes (or pushes it for review), updates the
// main checkout when that's safe, and archives the task. A step that stops
// says why and what to do; Continue picks up from there.

@Observable
final class TaskFinishModel {
  enum Step: Int, CaseIterable, Identifiable {
    case commit, sync, check, land, update, cleanUp
    var id: Int { rawValue }
  }

  enum Status: Equatable {
    case pending
    case running(String)
    case done(String)
    case skipped(String)
    case failed(String)
  }

  /// What a stopped step offers besides Continue.
  enum Remedy: Equatable { case showChanges, skipCheck, pushForReview }

  var palette: ChromePalette
  var workspaceID: UUID?
  /// The task's folder, its main checkout and branch.
  var root = ""
  var mainRoot = ""
  var branch = ""
  /// The base branch (`main`) and its remote (`origin`; nil: none).
  var base = ""
  var remote: String?
  var checkScript: String?
  /// The repository's saved answer; nil until the first Finish asks.
  var landing: ProjectConfig.Landing?
  var chosenLanding: ProjectConfig.Landing = .merge
  var remembersLanding = true
  var deletesBranch = true
  var deletesRemoteBranch = true
  /// The branch is on the remote (`origin/fix-elevation`).
  var isPublished = false
  /// Merged into its base already (on the server, after Push for Review).
  var alreadyMerged = false
  /// Why Finish can't run here (a worktree Impulse didn't make).
  var unavailable: String?
  var isLoading = true
  var isRunning = false
  var statuses: [Step: Status] = [:]
  /// Where Continue starts, and what else the stop offers.
  var resumeAt: Step?
  var remedy: Remedy?
  /// What the server said on the push.
  var serverMessage: GitServerMessage?
  /// The merge commit, for the summary.
  var landedCommit: String?
  /// Pushed for review in this tab.
  var pushedForReview = false

  @ObservationIgnored var onFinish: (() -> Void)?
  @ObservationIgnored var onRemedy: ((Remedy) -> Void)?
  @ObservationIgnored var onSetUpCheck: (() -> Void)?
  @ObservationIgnored var onClose: (() -> Void)?
  /// Observers of the check's terminal, removed when it ends.
  @ObservationIgnored var checkObservers: [NSObjectProtocol] = []

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var name: String { (root as NSString).lastPathComponent }
  var mainName: String { (mainRoot as NSString).lastPathComponent }
  var baseRef: String { remote.map { "\($0)/\(base)" } ?? base }
  var effectiveLanding: ProjectConfig.Landing { landing ?? chosenLanding }
  var steps: [Step] { Step.allCases }

  func title(of step: Step) -> String {
    switch step {
    case .commit: return "Commit"
    case .sync: return "Sync with \(baseRef)"
    case .check: return "Check"
    case .land:
      if effectiveLanding == .review { return "Push for review" }
      return remote == nil ? "Merge into \(base)" : "Merge into \(baseRef) and push"
    case .update: return "Update \(mainName)"
    case .cleanUp: return "Clean up"
    }
  }

  /// What a step will do, before it runs.
  func plan(of step: Step) -> String {
    let review = effectiveLanding == .review && !alreadyMerged
    switch step {
    case .commit: return "Everything in \(name) is committed, and no agent there is working."
    case .sync: return "\(baseRef)'s new commits are merged into \(branch), here in the task (never a rebase)."
    case .check:
      return checkScript.map { "Runs \($0) in a terminal tab; it has to pass." } ?? "No check script, so this is skipped."
    case .land:
      if review { return "Pushes \(branch) to \(remote ?? "the remote"). Your git host takes it from there." }
      return remote == nil
        ? "A merge commit in \(mainName), when it's on \(base) with nothing uncommitted."
        : "A merge commit, made in a throwaway folder and pushed as \(base). \(mainName) isn't touched."
    case .update:
      if review { return "After \(branch) is merged." }
      return remote == nil ? "The merge is already there." : "Fast-forwards \(mainName) when it's on \(base) with nothing uncommitted."
    case .cleanUp:
      if review { return "After \(branch) is merged, Impulse offers to clean up." }
      return "Archives the task (its archive script runs) and deletes its branch."
    }
  }

  var finishTitle: String {
    if alreadyMerged { return "Clean Up" }
    if resumeAt != nil { return "Continue" }
    if pushedForReview { return "Push Again" }
    return effectiveLanding == .review ? "Push for Review" : "Finish"
  }
}

final class TaskFinishSurface: NSView, ToolSurface {
  let model: TaskFinishModel
  let taskRoot: String

  static func kind(_ root: String) -> String { "task-finish:\(root)" }

  var toolKind: String { Self.kind(taskRoot) }
  var toolTitle: String { "Finish \((taskRoot as NSString).lastPathComponent)" }
  var toolSymbol: String { "flag.checkered" }

  init(root: String, palette: ChromePalette) {
    taskRoot = root
    model = TaskFinishModel(palette: palette)
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
    let hosting = WorkbenchHosting.make(TaskFinishView(model: model))
    hosting.translatesAutoresizingMaskIntoConstraints = false
    addSubview(hosting)
    NSLayoutConstraint.activate([
      hosting.topAnchor.constraint(equalTo: topAnchor),
      hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
      hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  func applyToolTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
  }

  func cleanupTool() {
    model.checkObservers.forEach(NotificationCenter.default.removeObserver)
    model.checkObservers = []
  }
}

// MARK: - View

struct TaskFinishView: View {
  var model: TaskFinishModel

  var body: some View {
    let chrome = model.palette
    VStack(spacing: 0) {
      header
      Rectangle().fill(chrome.hairline).frame(height: 1)
      if model.isLoading {
        ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        ScrollView {
          content
            .padding(.horizontal, 28)
            .padding(.vertical, 20)
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
      }
    }
    .background(chrome.content)
    .environment(\.chrome, chrome)
  }

  private var header: some View {
    let chrome = model.palette
    return HStack(spacing: 10) {
      VStack(alignment: .leading, spacing: 2) {
        Text("Finish \(model.name)").font(ChromeFont.ui(15, weight: .semibold)).foregroundStyle(chrome.text)
        Text(model.branch.isEmpty ? " " : "\(model.branch) → \(model.baseRef)")
          .font(ChromeFont.mono(11.5)).foregroundStyle(chrome.textSecondary)
      }
      Spacer()
      ChromeButton(title: "Close", kind: .secondary) { model.onClose?() }
      if model.unavailable == nil {
        ChromeButton(title: model.finishTitle, icon: .gitMerge, kind: .primary) { model.onFinish?() }
          .disabled(model.isRunning || model.isLoading)
      }
    }
    .padding(.horizontal, 20)
    .frame(height: 64)
  }

  @ViewBuilder
  private var content: some View {
    let chrome = model.palette
    VStack(alignment: .leading, spacing: 18) {
      if let unavailable = model.unavailable {
        Text(unavailable).font(ChromeFont.ui(12)).foregroundStyle(chrome.textSecondary)
          .fixedSize(horizontal: false, vertical: true)
      } else {
        if model.landing == nil, !model.alreadyMerged { landingChoice }
        if !model.alreadyMerged, model.effectiveLanding == .merge { branchOptions }
        VStack(alignment: .leading, spacing: 0) {
          ForEach(model.steps) { step in
            stepRow(step)
            if step != model.steps.last { Rectangle().fill(chrome.hairline).frame(height: 1) }
          }
        }
        .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.panel))
        .overlay(RoundedRectangle(cornerRadius: Metrics.radius).strokeBorder(chrome.hairline))
        if let message = model.serverMessage { serverReply(message) }
      }
    }
  }

  private var landingChoice: some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 8) {
      Text("How does work land in \(model.mainName)?").font(ChromeFont.ui(13, weight: .semibold)).foregroundStyle(chrome.text)
      ForEach(ProjectConfig.Landing.allCases, id: \.self) { landing in
        Button {
          model.chosenLanding = landing
        } label: {
          HStack(alignment: .top, spacing: 8) {
            Icon(model.chosenLanding == landing ? .circleDot : .circle, size: 13)
              .foregroundStyle(model.chosenLanding == landing ? chrome.accent : chrome.textTertiary)
            VStack(alignment: .leading, spacing: 2) {
              Text(landing == .merge ? "Merge into \(model.baseRef) and push" : "Push for review")
                .font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
              Text(
                landing == .merge
                  ? "Impulse merges the branch with a merge commit and pushes \(model.base). Nobody reviews it on the server first."
                  : "Impulse pushes the branch; you open a merge request on your git host and it's merged there."
              )
              .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
          }
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(model.isRunning)
      }
      Toggle(isOn: Binding(get: { model.remembersLanding }, set: { model.remembersLanding = $0 })) {
        Text("Remember for \(model.mainName) (on this Mac; Project Setup can change it)")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      }
      .toggleStyle(.checkbox)
      .disabled(model.isRunning)
    }
  }

  private var branchOptions: some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 4) {
      Toggle(isOn: Binding(get: { model.deletesBranch }, set: { model.deletesBranch = $0 })) {
        Text("Delete \(model.branch) once it's merged").font(ChromeFont.ui(12)).foregroundStyle(chrome.text)
      }
      .toggleStyle(.checkbox)
      if model.isPublished, let remote = model.remote {
        Toggle(isOn: Binding(get: { model.deletesRemoteBranch }, set: { model.deletesRemoteBranch = $0 })) {
          Text("…and on \(remote)").font(ChromeFont.ui(12)).foregroundStyle(chrome.text)
        }
        .toggleStyle(.checkbox)
        .disabled(!model.deletesBranch)
        .padding(.leading, 18)
      }
    }
    .disabled(model.isRunning)
  }

  private func stepRow(_ step: TaskFinishModel.Step) -> some View {
    let chrome = model.palette
    let status = model.statuses[step] ?? .pending
    let (text, color): (String, Color) = {
      switch status {
      case .pending: return (model.plan(of: step), chrome.textTertiary)
      case .running(let text): return (text, chrome.textSecondary)
      case .done(let text): return (text, chrome.textSecondary)
      case .skipped(let text): return (text, chrome.textTertiary)
      case .failed(let text): return (text, chrome.danger)
      }
    }()
    return HStack(alignment: .top, spacing: 10) {
      statusIcon(status).frame(width: 16, height: 16)
      VStack(alignment: .leading, spacing: 3) {
        Text(model.title(of: step)).font(ChromeFont.ui(12.5, weight: .medium)).foregroundStyle(chrome.text)
        Text(text).font(ChromeFont.ui(11.5)).foregroundStyle(color).fixedSize(horizontal: false, vertical: true)
          .textSelection(.enabled)
        stepButtons(step, status: status)
      }
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 14)
    .padding(.vertical, 10)
  }

  @ViewBuilder
  private func stepButtons(_ step: TaskFinishModel.Step, status: TaskFinishModel.Status) -> some View {
    let failed: Bool = {
      if case .failed = status { return true }
      return false
    }()
    let stopped = (failed && !model.isRunning) || (step == .check && model.isRunning)
    HStack(spacing: 8) {
      if stopped, let remedy = model.remedy {
        switch remedy {
        case .showChanges: ChromeButton(title: "Show Changes", icon: .gitCommitHorizontal) { model.onRemedy?(.showChanges) }
        case .skipCheck: ChromeButton(title: "Skip Check", kind: .ghost) { model.onRemedy?(.skipCheck) }
        case .pushForReview: ChromeButton(title: "Push for Review Instead", icon: .upload) { model.onRemedy?(.pushForReview) }
        }
      }
      if step == .check, model.checkScript == nil {
        ChromeButton(title: "Set Up a Check…", kind: .ghost) { model.onSetUpCheck?() }
      }
    }
    .padding(.top, 2)
  }

  @ViewBuilder
  private func statusIcon(_ status: TaskFinishModel.Status) -> some View {
    let chrome = model.palette
    switch status {
    case .pending: Icon(.circle, size: 13).foregroundStyle(chrome.textTertiary)
    case .running: ProgressView().controlSize(.mini)
    case .done: Icon(.circleCheck, size: 13).foregroundStyle(chrome.success)
    case .skipped: Icon(.minus, size: 13).foregroundStyle(chrome.textTertiary)
    case .failed: Icon(.circleX, size: 13).foregroundStyle(chrome.danger)
    }
  }

  private func serverReply(_ message: GitServerMessage) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 6) {
      Text("The server said").font(ChromeFont.ui(12, weight: .semibold)).foregroundStyle(chrome.text)
      Text(message.lines.joined(separator: "\n")).font(ChromeFont.mono(11)).foregroundStyle(chrome.textSecondary)
        .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
      if let link = message.link {
        ChromeButton(title: message.linkCaption.map { "Open: \($0)" } ?? "Open Link", icon: .externalLink, kind: .primary) {
          NSWorkspace.shared.open(link)
        }
      }
    }
  }
}

// MARK: - Running it

extension MainWindowController {
  /// "Finish Task…" for the task workspace `workspaceID` (else the active
  /// one): opens (or shows) its Finish tab.
  func openFinishTask(from workspaceID: UUID? = nil) {
    let id = workspaceID ?? tabManager.activeWorkspaceID
    guard let workspace = tabManager.workspace(id), workspace.kind == .folder, workspace.isTask else {
      toasts.show(Toast(kind: .info, message: "Finish Task works in a task's workspace."))
      return
    }
    let root = workspace.root
    tabManager.activateWorkspace(id)
    let palette = windowModel.palette
    guard
      let surface = tabManager.openTool(
        kind: TaskFinishSurface.kind(root), make: { TaskFinishSurface(root: root, palette: palette) })
        as? TaskFinishSurface
    else { return }
    let model = surface.model
    guard !model.isRunning else { return }
    model.workspaceID = id
    model.onFinish = { [weak self, weak model] in
      guard let self, let model else { return }
      self.startFinish(model)
    }
    model.onRemedy = { [weak self, weak model] remedy in
      guard let self, let model else { return }
      self.applyFinishRemedy(remedy, model: model)
    }
    model.onSetUpCheck = { [weak self, weak model] in
      self?.openProjectSetup(from: model?.workspaceID, section: "scripts")
    }
    model.onClose = { [weak self, weak surface] in
      guard let self, let surface,
        let location = self.tabManager.locate(where: { entry in
          if case .tool(let view) = entry { return view === surface }
          return false
        })
      else { return }
      self.requestCloseTab(index: location.tabIndex)
    }
    loadFinish(model, root: root)
  }

  private func loadFinish(_ model: TaskFinishModel, root: String) {
    model.root = root
    model.isLoading = true
    DispatchQueue.global(qos: .userInitiated).async {
      let main = Self.mainCheckoutRoot(of: root)
      let record = TaskRegistryStore.record(forPath: root)
      let branch = GitOperations.currentBranch(root: root) ?? record?.branch ?? ""
      let remotes = GitOperations.remotes(root: main)
      // Adopted tasks don't know their base: the remote's default branch.
      var base = record?.base
      var remote = record?.remote
      if base == nil, let fallback = GitClient.defaultBaseBranch(repoPath: main) {
        if let match = remotes.first(where: { fallback.hasPrefix("\($0)/") }) {
          remote = match
          base = String(fallback.dropFirst(match.count + 1))
        } else {
          base = fallback
        }
      }
      let settings = (try? Self.loadProjectConfig(root: root)?.config.get()) ?? nil
      let baseRef = remote.map { "\($0)/\(base ?? "")" } ?? base ?? ""
      let tip = GitClient.resolveCommit(repoPath: main, revision: "refs/heads/\(branch)")
      let merged =
        !branch.isEmpty && base != nil && tip != nil && tip != record?.start
        && GitOperations.isMerged("refs/heads/\(branch)", into: baseRef, root: main)
      let published = remote.map { GitOperations.resolveRef("refs/remotes/\($0)/\(branch)", root: main) != nil } ?? false
      DispatchQueue.main.async {
        model.mainRoot = main
        model.branch = branch
        model.base = base ?? ""
        model.remote = remote
        model.checkScript = settings?.checkScript
        model.landing = settings?.landing
        model.isPublished = published
        model.alreadyMerged = merged
        model.statuses = [:]
        model.resumeAt = nil
        model.remedy = nil
        model.serverMessage = nil
        if record == nil {
          model.unavailable =
            "Impulse didn't start this worktree, so it doesn't finish it. Merge its branch from Git ▸ Manage Branches…, then remove it with git worktree remove."
        } else if branch.isEmpty || base == nil {
          model.unavailable = "Finish needs the task on a branch, and a branch to land it in. Neither could be worked out here."
        } else if merged {
          for step in [TaskFinishModel.Step.commit, .sync, .check, .land] {
            model.statuses[step] = .done("Already merged into \(baseRef).")
          }
        }
        model.isLoading = false
      }
    }
  }

  private func startFinish(_ model: TaskFinishModel) {
    if model.alreadyMerged {
      model.chosenLanding = .merge
      model.landing = model.landing ?? .merge
      return runFinish(model, from: .update)
    }
    if model.landing == nil {
      model.landing = model.chosenLanding
      if model.remembersLanding { saveLanding(model.chosenLanding, root: model.mainRoot) }
    }
    runFinish(model, from: model.resumeAt ?? .commit)
  }

  private func runFinish(_ model: TaskFinishModel, from first: TaskFinishModel.Step) {
    model.isRunning = true
    model.resumeAt = nil
    model.remedy = nil
    for step in TaskFinishModel.Step.allCases where step.rawValue >= first.rawValue {
      model.statuses[step] = .pending
    }
    advanceFinish(model, to: first)
  }

  /// Run `step`, then the next, until one stops or they're done.
  private func advanceFinish(_ model: TaskFinishModel, to step: TaskFinishModel.Step?) {
    guard let step else {
      model.isRunning = false
      return
    }
    let next = TaskFinishModel.Step(rawValue: step.rawValue + 1)
    let proceed: (TaskFinishModel.Status) -> Void = { [weak self, weak model] status in
      guard let self, let model else { return }
      model.statuses[step] = status
      self.advanceFinish(model, to: next)
    }
    let stop: (String, TaskFinishModel.Step, TaskFinishModel.Remedy?) -> Void = { [weak model] text, resume, remedy in
      guard let model else { return }
      model.statuses[step] = .failed(text)
      model.resumeAt = resume
      model.remedy = remedy
      model.isRunning = false
    }
    switch step {
    case .commit: finishCommitStep(model, proceed: proceed, stop: stop)
    case .sync: finishSyncStep(model, proceed: proceed, stop: stop)
    case .check: finishCheckStep(model, proceed: proceed, stop: stop)
    case .land: finishLandStep(model, proceed: proceed, stop: stop)
    case .update: finishUpdateStep(model, proceed: proceed)
    case .cleanUp: finishCleanUpStep(model, stop: stop)
    }
  }

  private typealias Proceed = (TaskFinishModel.Status) -> Void
  private typealias Stop = (String, TaskFinishModel.Step, TaskFinishModel.Remedy?) -> Void

  private func finishCommitStep(_ model: TaskFinishModel, proceed: @escaping Proceed, stop: @escaping Stop) {
    model.statuses[.commit] = .running("Looking at \(model.name)…")
    if let agent = tabManager.agents(inFolder: model.root).first(where: { $0.state == .working }) {
      return stop("\(agent.name) is working in this task. Let it finish its turn, then Continue.", .commit, nil)
    }
    let root = model.root
    DispatchQueue.global(qos: .userInitiated).async {
      let snapshot = GitClient.snapshot(forPath: root)
      DispatchQueue.main.async {
        guard let snapshot else { return stop("The task's folder couldn't be read.", .commit, nil) }
        if snapshot.operation != nil || !snapshot.conflicted.isEmpty {
          return stop(
            "A merge (or rebase) is under way in the task. Finish it in the Changes panel, then Continue.", .commit,
            .showChanges)
        }
        let count = snapshot.changedFileCount
        if count > 0 {
          return stop(
            "\(count) uncommitted file\(count == 1 ? "" : "s"). Commit (or stash) \(count == 1 ? "it" : "them"), then Continue.",
            .commit, .showChanges)
        }
        proceed(.done("Everything is committed."))
      }
    }
  }

  private func finishSyncStep(_ model: TaskFinishModel, proceed: @escaping Proceed, stop: @escaping Stop) {
    let (root, remote, target, branch) = (model.root, model.remote, model.baseRef, model.branch)
    model.statuses[.sync] = .running(remote.map { "Fetching \($0)…" } ?? "Merging \(target)…")
    DispatchQueue.global(qos: .userInitiated).async {
      if let remote, case .failure(let error) = GitOperations.fetch(remote: remote, root: root) {
        return DispatchQueue.main.async { stop("Couldn't fetch \(remote): \(error.message)", .sync, nil) }
      }
      if TaskLanding.isAncestor(target, of: "HEAD", root: root) {
        return DispatchQueue.main.async { proceed(.done("Up to date with \(target).")) }
      }
      let count = GitOperations.commitCount(from: "HEAD", to: target, root: root) ?? 0
      let merged = GitOperations.merge(target, root: root)
      let conflicted = GitClient.snapshot(forPath: root)?.conflicted.count ?? 0
      DispatchQueue.main.async {
        switch merged {
        case .success:
          proceed(.done("Merged \(count) new commit\(count == 1 ? "" : "s") from \(target) into \(branch)."))
        case .failure where conflicted > 0:
          stop(
            "Merging \(target) into \(branch) conflicts in \(conflicted) file\(conflicted == 1 ? "" : "s"). Resolve them in the Changes panel and commit the merge (or ask the task's agent to), then Continue.",
            .commit, .showChanges)
        case .failure(let error):
          stop("Couldn't merge \(target): \(error.message)", .sync, nil)
        }
      }
    }
  }

  private func finishCheckStep(_ model: TaskFinishModel, proceed: @escaping Proceed, stop: @escaping Stop) {
    guard let script = model.checkScript else {
      return proceed(.skipped("No check script. Set one up so Finish can tell the merged code still works."))
    }
    model.statuses[.check] = .running("Waiting for the project settings to be trusted…")
    trustProjectConfig(root: model.root) { [weak self, weak model] config in
      guard let self, let model else { return }
      guard config != nil else {
        model.remedy = .skipCheck
        return stop("The project settings weren't trusted, so the check didn't run.", .check, .skipCheck)
      }
      guard let id = model.workspaceID, self.tabManager.workspace(id) != nil else {
        return stop("The task's workspace was closed.", .check, nil)
      }
      self.tabManager.activateWorkspace(id)
      let container = self.tabManager.addTerminalTab(directory: model.root, initialCommand: script)
      guard let terminal = container.activeTerminal else { return stop("The check's terminal didn't start.", .check, nil) }
      model.statuses[.check] = .running("Running \(script) in a terminal tab…")
      model.remedy = .skipCheck
      let started = Date()
      let end: (Int32?) -> Void = { [weak self, weak model] code in
        guard let self, let model, case .running = model.statuses[.check] ?? .pending else { return }
        model.checkObservers.forEach(NotificationCenter.default.removeObserver)
        model.checkObservers = []
        model.remedy = nil
        self.revealFinish(model)
        switch code {
        case 0?:
          let seconds = Int(Date().timeIntervalSince(started).rounded())
          proceed(.done("Passed in \(seconds < 60 ? "\(seconds)s" : "\(seconds / 60)m \(seconds % 60)s")."))
        case let code?:
          stop("Failed (exit status \(code)). See its terminal tab; fix it, commit, then Continue.", .commit, nil)
        case nil:
          stop("Its terminal closed before the check finished.", .check, .skipCheck)
        }
      }
      model.checkObservers = [
        NotificationCenter.default.addObserver(forName: .terminalCommandBlockChanged, object: terminal, queue: .main) { note in
          guard let block = note.userInfo?["block"] as? TerminalCommandBlock, block.endedAtMs != nil else { return }
          end(block.exitCode ?? 1)
        },
        NotificationCenter.default.addObserver(forName: .terminalProcessTerminated, object: terminal, queue: .main) { _ in
          end(nil)
        },
      ]
    }
  }

  private func finishLandStep(_ model: TaskFinishModel, proceed: @escaping Proceed, stop: @escaping Stop) {
    let (root, main, branch, base, remote, target) = (
      model.root, model.mainRoot, model.branch, model.base, model.remote, model.baseRef
    )
    if model.effectiveLanding == .review {
      guard let remote = remote ?? GitOperations.defaultRemote(root: root, branch: branch) else {
        return stop("Push for Review needs a remote, and this repository has none.", .land, nil)
      }
      let published = model.isPublished
      model.statuses[.land] = .running("Pushing \(branch) to \(remote)…")
      DispatchQueue.global(qos: .userInitiated).async { [weak model] in
        let lines = LockedLines()
        let pushed =
          published
          ? GitOperations.push(root: root, onProgress: lines.append)
          : GitOperations.push(setUpstream: true, remote: remote, branch: branch, root: root, onProgress: lines.append)
        if case .success = pushed { TaskRegistryStore.markPushedForReview(path: root, root: main) }
        DispatchQueue.main.async { [weak model] in
          guard let model else { return }
          model.serverMessage = GitServerMessage.parse(lines.all)
          switch pushed {
          case .success:
            model.isPublished = true
            model.pushedForReview = true
            model.statuses[.land] = .done("Pushed \(branch) to \(remote). Open a merge request for it on your git host.")
            model.statuses[.update] = .skipped("After \(branch) is merged into \(target).")
            model.statuses[.cleanUp] = .skipped("Once \(branch) is merged, Impulse offers to clean up.")
            model.isRunning = false
          case .failure(let error):
            stop("Couldn't push: \(error.message)", .land, nil)
          }
        }
      }
      return
    }

    model.statuses[.land] = .running(remote == nil ? "Merging into \(base)…" : "Merging into \(target) and pushing…")
    DispatchQueue.global(qos: .userInitiated).async { [weak model] in
      let result =
        remote.map { TaskLanding.mergeAndPush(branch: branch, base: base, remote: $0, root: main) }
        ?? TaskLanding.mergeLocally(branch: branch, base: base, root: main)
      DispatchQueue.main.async { [weak model] in
        guard let model else { return }
        switch result {
        case .success(let landed):
          model.landedCommit = landed.commit
          model.serverMessage = GitServerMessage.parse(landed.output)
          let short = String(landed.commit.prefix(7))
          let moved = landed.retried ? " \(target) had moved meanwhile, so its new commits were merged in first." : ""
          proceed(.done(remote == nil ? "Merged into \(base) as \(short)." : "Merged as \(short) and pushed to \(target).\(moved)"))
        case .failure(.nothingToLand):
          proceed(.done("Nothing to merge: \(target) already has \(branch)'s commits."))
        case .failure(.conflicts(let files)):
          stop(
            "\(target) changed since the sync, and merging now conflicts in \(files.count == 1 ? files[0] : "\(files.count) files"). Nothing was pushed. Continue syncs again.",
            .sync, nil)
        case .failure(.refused(let output)):
          model.serverMessage = GitServerMessage.parse(output.components(separatedBy: "\n"))
          stop(
            "The server turned the push to \(base) down (a protected branch, perhaps). Nothing was pushed.", .land,
            .pushForReview)
        case .failure(.untracked(let files)):
          stop(
            "\(model.mainName) has untracked \(TaskLanding.naming(files)), which \(branch) adds. Without a remote, Finish merges in \(model.mainName): move \(files.count == 1 ? "it" : "them") aside there, then Continue.",
            .land, nil)
        case .failure(let failure):
          let hint =
            remote == nil
            ? " Without a remote, Finish merges in \(model.mainName): commit or stash there, switch it to \(base), then Continue."
            : ""
          stop(failure.message + hint, .land, nil)
        }
      }
    }
  }

  private func finishUpdateStep(_ model: TaskFinishModel, proceed: @escaping Proceed) {
    guard let remote = model.remote else { return proceed(.done("The merge is in \(model.mainName).")) }
    let (main, base, target, name) = (model.mainRoot, model.base, "\(remote)/\(model.base)", model.mainName)
    model.statuses[.update] = .running("Updating \(name)…")
    DispatchQueue.global(qos: .userInitiated).async {
      let current = GitOperations.currentBranch(root: main)
      // Untracked files don't hold it back: git refuses to overwrite one.
      let dirty = GitClient.snapshot(forPath: main)?.trackedChangeCount ?? 0
      var status: TaskFinishModel.Status
      if current != base {
        status = .skipped("\(name) is on \(current ?? "a detached commit"), so it was left alone.")
      } else if dirty > 0 {
        status = .skipped(
          "\(name) has \(dirty) uncommitted file\(dirty == 1 ? "" : "s"), so it wasn't updated. Its row shows how far behind it is; click that to pull when you're ready.")
      } else if case .failure(let error) = GitOperations.fastForward(to: target, root: main) {
        let untracked = TaskLanding.untrackedInTheWay(error.output ?? "")
        status = .skipped(
          untracked.isEmpty
            ? "\(name) has commits of its own on \(base), so it wasn't updated. Pull when you're ready."
            : "\(name) has untracked \(TaskLanding.naming(untracked)), which \(target) now tracks, so it wasn't updated. Move \(untracked.count == 1 ? "it" : "them") aside, then pull.")
      } else {
        status = .done("Fast-forwarded \(name) to \(target).")
      }
      DispatchQueue.main.async { proceed(status) }
    }
  }

  private func finishCleanUpStep(_ model: TaskFinishModel, stop: @escaping Stop) {
    guard let id = model.workspaceID, tabManager.workspace(id) != nil else {
      return stop("The task's workspace was closed.", .cleanUp, nil)
    }
    let (root, main, branch, remote, target) = (model.root, model.mainRoot, model.branch, model.remote, model.baseRef)
    let deleteBranch = model.deletesBranch
    let deleteRemote = deleteBranch && model.deletesRemoteBranch && model.isPublished
    let landed = model.landedCommit.map { " as \($0.prefix(7))" } ?? ""
    let script = alreadyTrustedProjectConfig(root: root)?.archiveScript
    model.statuses[.cleanUp] = .running("Closing the task's workspace…")
    tabManager.ensureScratchWorkspace()
    // Closing asks about unsaved files and running processes; the Finish
    // tab goes with the workspace, so the outcome is a toast.
    requestCloseWorkspace(
      id, recordForUndo: false,
      then: { [weak self] in
        DispatchQueue.global(qos: .userInitiated).async {
          let removed = Self.removeTask(root: root, branch: branch, dirty: false, archiveScript: script)
          var result = removed.result
          var deleted = false
          if case .success(var task) = result, deleteBranch,
            let commit = GitClient.resolveCommit(repoPath: main, revision: "refs/heads/\(branch)"),
            case .success = GitOperations.deleteBranch(branch, force: true, root: main)
          {
            task.deletedBranchCommit = commit
            deleted = true
            if deleteRemote, let remote { _ = GitOperations.deleteRemoteBranch(branch, remote: remote, root: main) }
            result = .success(task)
          }
          DispatchQueue.main.async {
            guard let self else { return }
            if removed.scriptFailed {
              self.toasts.show(Toast(kind: .warning, message: "The archive script failed; archived anyway."))
            }
            switch result {
            case .failure(let error):
              self.presentGitError(error, title: "Couldn't archive \(branch)")
            case .success(let task):
              self.toasts.dismiss(tag: "clean-up:\(TaskRegistry.canonical(root))")
              let what = deleted ? "The task is archived and its branch deleted." : "The task is archived; its branch is kept."
              self.toasts.show(
                Toast(
                  kind: .success, message: "Finished \(branch): merged into \(target)\(landed).", detail: what,
                  actionTitle: "Restore Task", action: { [weak self] in self?.restoreTasks([task]) }, lifetime: 15))
            }
          }
        }
      },
      cancelled: { [weak model] in
        guard let model else { return }
        model.statuses[.cleanUp] = .failed("The workspace stayed open, so the task wasn't archived. Continue when you're ready.")
        model.resumeAt = .cleanUp
        model.isRunning = false
      })
  }

  private func applyFinishRemedy(_ remedy: TaskFinishModel.Remedy, model: TaskFinishModel) {
    switch remedy {
    case .showChanges:
      if let id = model.workspaceID { tabManager.activateWorkspace(id) }
      showChangesPanel()
    case .skipCheck:
      model.checkObservers.forEach(NotificationCenter.default.removeObserver)
      model.checkObservers = []
      model.remedy = nil
      model.statuses[.check] = .skipped("Skipped.")
      model.isRunning = true
      model.resumeAt = nil
      advanceFinish(model, to: .land)
    case .pushForReview:
      model.landing = .review
      model.chosenLanding = .review
      model.serverMessage = nil
      runFinish(model, from: .land)
    }
  }

  /// Bring a task's Finish tab to the front.
  private func revealFinish(_ model: TaskFinishModel) {
    guard
      let location = tabManager.locate(where: { entry in
        if case .tool(let view) = entry { return view.toolKind == TaskFinishSurface.kind(model.root) }
        return false
      })
    else { return }
    tabManager.reveal(location)
  }

  /// Save Finish's answer in the repository's settings on this Mac. The file
  /// stays trusted if it was: a landing choice runs nothing.
  private func saveLanding(_ landing: ProjectConfig.Landing, root: String) {
    DispatchQueue.global(qos: .utility).async {
      guard let common = GitClient.commonGitDirectory(forPath: root) else { return }
      let path = ProjectConfig.localPath(commonGitDirectory: common)
      let before = Self.loadProjectConfig(root: root)
      let wasTrusted = before.flatMap { loaded in
        loaded.local.map { ProjectTrustStore.current.isTrusted(root: Self.trustKey(for: $0, in: loaded, root: root), digest: $0.digest) }
      } ?? true
      let existing = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
      try? FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      guard (try? ProjectSettingsFile.setting(landing, in: existing).write(toFile: path, atomically: true, encoding: .utf8)) != nil,
        wasTrusted, let after = Self.loadProjectConfig(root: root), let local = after.local
      else { return }
      DispatchQueue.main.async {
        var trust = ProjectTrustStore.current
        trust.trust(root: Self.trustKey(for: local, in: after, root: root), digest: local.digest)
        ProjectTrustStore.current = trust
      }
    }
  }
}

/// Output lines collected from a background git command.
private final class LockedLines: @unchecked Sendable {
  private var lines: [String] = []
  private let lock = NSLock()

  func append(_ line: String) {
    lock.lock()
    lines.append(line)
    lock.unlock()
  }

  var all: [String] {
    lock.lock()
    defer { lock.unlock() }
    return lines
  }
}
