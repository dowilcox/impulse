import AppKit
import ImpulseGit
import ImpulseKit

/// The Project Setup tab: opening it on a repository, filling it from the
/// repository's settings and from what's in it, then saving into
/// `.impulse/project.toml` (which trusts exactly what it wrote), trying the
/// settings in a new task, and re-applying values to tasks that already
/// exist.
extension MainWindowController {
  /// Open Project Setup for the repository of `workspaceID` (else the active
  /// workspace's), scrolled to `section` ("actions").
  func openProjectSetup(from workspaceID: UUID? = nil, section: String? = nil) {
    let repository =
      workspaceID.flatMap { tabManager.workspace($0)?.repository } ?? tabManager.activeWorkspace.repository
      ?? windowModel.repository
    guard let repository else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository first."))
      return
    }
    let root = Self.mainCheckoutRoot(of: repository.root)
    let palette = windowModel.palette
    guard
      let surface = tabManager.openTool(kind: "project-setup", make: { ProjectSetupSurface(palette: palette) })
        as? ProjectSetupSurface
    else { return }
    let model = surface.model
    model.onSave = { [weak self, weak model] in
      guard let model else { return }
      self?.saveProjectSetup(model)
    }
    model.onTry = { [weak self, weak model] in
      guard let self, let model else { return }
      self.saveProjectSetup(model) { [weak self] in
        let workspace = self?.tabManager.workspaces.first { $0.kind == .folder && $0.root == model.root }
        self?.presentNewTaskSheet(from: workspace?.id, title: "Try project setup")
      }
    }
    model.onReapply = { [weak self, weak model] in
      guard let model else { return }
      self?.saveProjectSetup(model) { [weak self] in self?.reapplyTaskValues(root: model.root) }
    }
    model.onShowCompose = { [weak self, weak model] in
      guard let model, let name = model.composeFileName else { return }
      self?.openFile(path: (model.root as NSString).appendingPathComponent(name))
    }
    model.onOpenFile = { [weak self, weak model] in
      guard let model else { return }
      self?.openProjectSettingsFile(root: model.root)
    }
    model.onOpenLocalFile = { [weak self, weak model] in
      guard let model, let common = GitClient.commonGitDirectory(forPath: model.root) else { return }
      self?.openFile(path: ProjectConfig.localPath(commonGitDirectory: common))
    }
    model.onChoosePath = { [weak self, weak model] folders, done in
      guard let model else { return }
      self?.chooseProjectPath(in: model.root, folders: folders, then: done)
    }
    loadProjectSetup(model, root: root, section: section)
  }

  private func loadProjectSetup(_ model: ProjectSetupModel, root: String, section: String?) {
    model.root = root
    model.isLoading = true
    DispatchQueue.global(qos: .userInitiated).async {
      let common = GitClient.commonGitDirectory(forPath: root)
      let localText = common.flatMap { try? String(contentsOfFile: ProjectConfig.localPath(commonGitDirectory: $0), encoding: .utf8) }
      let committedText = try? String(
        contentsOfFile: (root as NSString).appendingPathComponent(ProjectConfig.relativePath), encoding: .utf8)
      let saved = Self.savedSettings(committed: committedText, local: localText)
      let found = ProjectDetector.suggest(root: root, ignored: GitOperations.ignoredEntries(root: root))
      let services = ComposeFile.find(in: root).map { ComposeFile.parse($0.text).services } ?? []
      let database = ProjectDetector.databaseService(in: services)
      let dump = database.flatMap { service in service.image.flatMap { ProjectDetector.dumpAndLoad(service: service.name, image: $0) } }
      let empty = ProjectDetector.freshDatabase(root: root)
      let taskCount = TaskRegistryStore.registry(root: root)?.tasks.filter { $0.slot != nil }.count ?? 0
      DispatchQueue.main.async { [weak self] in
        Self.fill(model, existing: saved.config, found: found, dump: dump, empty: empty)
        model.localFile = saved.local
        model.dropsComments = committedText.map(ProjectSettingsFile.managedSectionsHaveComments) ?? false
        model.taskCount = taskCount
        model.isLoading = false
        if let section {
          model.revealSection = section
          DispatchQueue.main.async { model.revealToken += 1 }
        }
        self?.measureClones(model)
      }
    }
  }

  /// The saved settings the screen shows, and what it does about the local
  /// file. Impulse's own local file (an earlier save, Finish Task's answer)
  /// is folded in under the project's settings, which win, and goes on
  /// save; one edited by hand stays, and the screen shows the project's
  /// own settings beside a note of what it overrides.
  private static func savedSettings(committed: String?, local: String?)
    -> (config: ProjectConfig?, local: ProjectSetupModel.LocalFile?)
  {
    let project = committed.flatMap { try? ProjectConfig.parse($0).get() }
    guard let local else { return (project, nil) }
    let applied = try? ProjectConfig.parse(layers: [committed, local].compactMap { $0 }).get()
    if ProjectSettingsFile.isWrittenByImpulse(local, over: project),
      let folded = try? ProjectConfig.parse(layers: [local, committed].compactMap { $0 }).get()
    {
      return (folded, .impulses(text: local, dropped: applied?.differences(from: folded) ?? []))
    }
    return (project, .handEdited(overrides: applied?.differences(from: project ?? ProjectConfig()) ?? []))
  }

  private static func fill(
    _ model: ProjectSetupModel, existing: ProjectConfig?, found: ProjectSuggestions, dump: String?, empty: String?
  ) {
    let settings = existing ?? ProjectConfig()
    let hasSettings = existing != nil
    model.copies = rows(found.copies, chosen: settings.worktreeCopy, hasSettings: hasSettings)
    model.clones = rows(found.clones, chosen: settings.worktreeClone, hasSettings: hasSettings)
    let ports = settings.ports.isEmpty ? found.ports : settings.ports
    model.ports = ports.keys.sorted().map { .init(name: $0, value: String(ports[$0]!)) }
    let values = settings.worktreeEnv.isEmpty ? found.values : settings.worktreeEnv
    model.values = values.keys.sorted().map { .init(name: $0, value: values[$0]!) }
    model.portOffset = String(settings.portOffset)
    model.envFile = settings.envFile
    model.setup = settings.setupScript ?? found.setup ?? ""
    model.check = settings.checkScript ?? found.check ?? ""
    model.archive = settings.archiveScript ?? found.archive ?? ""
    model.landing = settings.landing
    let names = Set(settings.actions.map(\.name))
    model.actions =
      settings.actions.map { .init(isOn: true, action: $0) }
      + found.actions.filter { !names.contains($0.name) }.map { .init(isOn: false, action: $0) }
    let rules = settings.onChange.isEmpty ? found.onChange : settings.onChange
    model.rules = rules.keys.sorted().map { .init(name: $0, value: rules[$0]!) }
    model.overlapIgnore = settings.overlapIgnore.joined(separator: ", ")
    model.composeFileName = found.composeFileName
    model.fixedPorts = found.fixedPorts
    model.containerNames = found.containerNames
    model.composeOverride = settings.composeOverride
    model.databaseFolder = settings.databaseFolder ?? found.databaseFolder ?? ""
    model.databaseService = (settings.databaseFolder != nil ? settings.databaseService : found.databaseService) ?? ""
    model.dumpCommand = dump
    model.emptyCommand = empty
    model.databaseOptions = [.none, .clone] + (dump == nil ? [] : [.dump]) + (empty == nil ? [] : [.empty])
    model.database =
      settings.databaseFolder != nil
      ? .clone
      : dump.map { model.setup.contains($0) } == true ? .dump : empty.map { model.setup.contains($0) } == true ? .empty : .none
  }

  /// Rows for the detected entries plus any chosen ones not detected; ticked
  /// as chosen when the project has settings, else as suggested.
  private static func rows(
    _ found: [ProjectSuggestions.Entry], chosen: [String], hasSettings: Bool
  ) -> [ProjectSetupModel.Row] {
    var rows = found.map { entry in
      ProjectSetupModel.Row(
        path: entry.path, isOn: hasSettings ? chosen.contains(entry.path) : entry.suggested, note: entry.note)
    }
    for path in chosen where !found.contains(where: { $0.path == path }) {
      rows.append(.init(path: path, isOn: true, isCustom: true))
    }
    return rows
  }

  /// Each clone row's size ("212 MB"), measured off the main thread.
  private func measureClones(_ model: ProjectSetupModel) {
    let root = model.root
    let paths = model.clones.map(\.path)
    DispatchQueue.global(qos: .utility).async { [weak model] in
      for path in paths {
        let bytes = Self.folderSize((root as NSString).appendingPathComponent(path))
        let text = ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
        DispatchQueue.main.async {
          guard let model, let index = model.clones.firstIndex(where: { $0.path == path }) else { return }
          model.clones[index].size = text
        }
      }
    }
  }

  private static func folderSize(_ path: String) -> Int64 {
    guard
      let enumerator = FileManager.default.enumerator(
        at: URL(fileURLWithPath: path), includingPropertiesForKeys: [.totalFileAllocatedSizeKey])
    else { return 0 }
    var total: Int64 = 0
    for case let url as URL in enumerator {
      total += Int64((try? url.resourceValues(forKeys: [.totalFileAllocatedSizeKey]))?.totalFileAllocatedSize ?? 0)
    }
    return total
  }

  /// Pick a file or folder inside `root` for a path field.
  private func chooseProjectPath(in root: String, folders: Bool, then done: @escaping (String) -> Void) {
    guard let window else { return }
    let name = (root as NSString).lastPathComponent
    let panel = NSOpenPanel()
    panel.canChooseFiles = !folders
    panel.canChooseDirectories = folders
    panel.allowsMultipleSelection = false
    panel.showsHiddenFiles = true
    panel.directoryURL = URL(fileURLWithPath: root)
    panel.prompt = "Choose"
    panel.message = folders ? "Choose a folder in \(name)." : "Choose a file in \(name)."
    panel.beginSheetModal(for: window) { [weak self] response in
      guard response == .OK, let url = panel.url else { return }
      let base = URL(fileURLWithPath: root).resolvingSymlinksInPath().path
      let chosen = url.resolvingSymlinksInPath().path
      guard chosen.hasPrefix(base + "/") else {
        self?.toasts.show(Toast(kind: .warning, message: "Choose something inside \(name)."))
        return
      }
      done(String(chosen.dropFirst(base.count + 1)))
    }
  }

  /// Save what the screen has into `.impulse/project.toml`, trust exactly
  /// what was written, and reload. Impulse's own local file, folded into
  /// the screen, is removed: its settings are in the project's now.
  private func saveProjectSetup(_ model: ProjectSetupModel, then: (() -> Void)? = nil) {
    let root = model.root
    let config = model.config()
    var folded: String?
    if case .impulses(let text, _)? = model.localFile { folded = text }
    let write = { [weak self] in
      model.isSaving = true
      DispatchQueue.global(qos: .userInitiated).async {
        let result = Self.writeProjectSettings(config, root: root, removingLocal: folded)
        DispatchQueue.main.async {
          guard let self else { return }
          model.isSaving = false
          switch result {
          case .success(let digest):
            var trust = ProjectTrustStore.current
            trust.trust(root: Self.trustKey(for: root), digest: digest)
            ProjectTrustStore.current = trust
            self.toasts.show(Toast(kind: .success, message: "Saved \(ProjectConfig.relativePath)"))
            self.loadProjectSetup(model, root: root, section: nil)
            then?()
          case .failure(let error):
            self.toasts.show(Toast(kind: .warning, message: "Couldn't save the project settings: \(error.localizedDescription)"))
          }
        }
      }
    }
    if model.dropsComments {
      gitConfirm(
        title: "Rewrite \(ProjectConfig.relativePath)'s settings?",
        message: "Comments inside the sections this screen manages are dropped; the rest of the file is kept.",
        confirmTitle: "Save", destructive: false
      ) { if $0 { write() } }
    } else {
      write()
    }
  }

  /// Write `config` into the main checkout's `.impulse/project.toml`, and
  /// remove the local file when it still holds `removingLocal`; the digest
  /// of what was written.
  private static func writeProjectSettings(
    _ config: ProjectConfig, root: String, removingLocal local: String?
  ) -> Result<String, Error> {
    let path = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    let existing = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
    let data = Data(ProjectSettingsFile.merging(config, into: existing).utf8)
    do {
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try data.write(to: URL(fileURLWithPath: path), options: .atomic)
    } catch {
      return .failure(error)
    }
    if let local, let common = GitClient.commonGitDirectory(forPath: root) {
      let localPath = ProjectConfig.localPath(commonGitDirectory: common)
      if (try? String(contentsOfFile: localPath, encoding: .utf8)) == local {
        try? FileManager.default.removeItem(atPath: localPath)
      }
    }
    return .success(ProjectConfig.digest(data))
  }

  /// Write the current ports and values into the env file of every task
  /// Impulse made, once the settings are trusted.
  private func reapplyTaskValues(root: String) {
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      let tasks = TaskRegistryStore.registry(root: root)?.tasks.filter { $0.slot != nil } ?? []
      DispatchQueue.main.async {
        guard let self else { return }
        var count = 0
        for task in tasks {
          guard let slot = task.slot, let config = self.alreadyTrustedProjectConfig(root: task.path) else { continue }
          if Self.writeTaskValues(config, path: task.path, slot: slot) { count += 1 }
        }
        self.toasts.show(
          Toast(kind: .success, message: count == 0 ? "No task needed new values" : "Re-applied values to \(count) task\(count == 1 ? "" : "s")"))
      }
    }
  }

  /// Open `.impulse/project.toml`, creating it if needed.
  private func openProjectSettingsFile(root: String) {
    let path = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    if !FileManager.default.fileExists(atPath: path) {
      try? FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      FileManager.default.createFile(atPath: path, contents: Data())
    }
    openFile(path: path)
  }
}
