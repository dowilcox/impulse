import AppKit
import ImpulseGit
import ImpulseKit

/// The Project Setup tab: opening it on a repository, filling it from the
/// repository's settings and from what's in it, then saving (which trusts
/// exactly what it wrote), trying the settings in a new task, and
/// re-applying values to tasks that already exist.
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
      self?.openProjectSettingsFile(root: model.root, location: model.location)
    }
    loadProjectSetup(model, root: root, section: section)
  }

  private func loadProjectSetup(_ model: ProjectSetupModel, root: String, section: String?) {
    model.root = root
    model.isLoading = true
    DispatchQueue.global(qos: .userInitiated).async {
      let common = GitClient.commonGitDirectory(forPath: root)
      let loaded = Self.loadProjectConfig(root: root)
      let existing = loaded.flatMap { try? $0.config.get() }
      let localText = common.flatMap { try? String(contentsOfFile: ProjectConfig.localPath(commonGitDirectory: $0), encoding: .utf8) }
      let committedText = try? String(
        contentsOfFile: (root as NSString).appendingPathComponent(ProjectConfig.relativePath), encoding: .utf8)
      // The screen owns the local file: one it didn't write was edited by hand.
      let handEdited = localText.map { text in
        (try? ProjectConfig.parse(text).get()).map { ProjectSettingsFile.text($0) != text } ?? true
      } ?? false
      let found = ProjectDetector.suggest(root: root, ignored: GitOperations.ignoredEntries(root: root))
      let services = ComposeFile.find(in: root).map { ComposeFile.parse($0.text).services } ?? []
      let database = ProjectDetector.databaseService(in: services)
      let dump = database.flatMap { service in service.image.flatMap { ProjectDetector.dumpAndLoad(service: service.name, image: $0) } }
      let empty = database == nil ? nil : ProjectDetector.freshDatabase(root: root)
      let taskCount = TaskRegistryStore.registry(root: root)?.tasks.filter { $0.slot != nil }.count ?? 0
      DispatchQueue.main.async { [weak self] in
        Self.fill(
          model, existing: existing, found: found, dump: dump, empty: empty, hasLocal: localText != nil,
          hasCommitted: committedText != nil, handEdited: handEdited,
          committedComments: committedText.map(ProjectSettingsFile.managedSectionsHaveComments) ?? false)
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

  private static func fill(
    _ model: ProjectSetupModel, existing: ProjectConfig?, found: ProjectSuggestions, dump: String?, empty: String?,
    hasLocal: Bool, hasCommitted: Bool, handEdited: Bool, committedComments: Bool
  ) {
    let settings = existing ?? ProjectConfig()
    let hasSettings = existing != nil
    model.location = hasLocal || !hasCommitted ? .local : .project
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
    let names = Set(settings.actions.map(\.name))
    model.actions =
      settings.actions.map { .init(isOn: true, action: $0) }
      + found.actions.filter { !names.contains($0.name) }.map { .init(isOn: false, action: $0) }
    let rules = settings.onChange.isEmpty ? found.onChange : settings.onChange
    model.rules = rules.keys.sorted().map { .init(name: $0, value: rules[$0]!) }
    model.composeFileName = found.composeFileName
    model.composeWarnings = found.composeWarnings
    model.composeOverride = settings.composeOverride
    model.databaseFolder = settings.databaseFolder ?? found.databaseFolder
    model.databaseService = settings.databaseFolder != nil ? settings.databaseService : found.databaseService
    model.dumpCommand = dump
    model.emptyCommand = empty
    var options: [ProjectSetupModel.Database] = []
    if model.databaseFolder != nil { options.append(.clone) }
    if dump != nil { options.append(.dump) }
    if empty != nil { options.append(.empty) }
    model.databaseOptions = options.isEmpty ? [] : [.none] + options
    model.database =
      settings.databaseFolder != nil
      ? .clone
      : dump.map { model.setup.contains($0) } == true ? .dump : empty.map { model.setup.contains($0) } == true ? .empty : .none
    var notes: [String] = []
    if hasLocal, hasCommitted {
      notes.append("This project has settings in both .impulse/project.toml and on this Mac; they're shown together, and this Mac's win.")
    }
    if handEdited { notes.append("The settings on this Mac were edited by hand: saving to This Mac rewrites that file.") }
    if committedComments {
      notes.append("Saving to the project rewrites the sections this screen manages in .impulse/project.toml; comments in them are dropped.")
    }
    model.notes = notes
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
      rows.append(.init(path: path, isOn: true))
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

  /// Save what the screen has, trust exactly what was written, and reload.
  private func saveProjectSetup(_ model: ProjectSetupModel, then: (() -> Void)? = nil) {
    let root = model.root
    let config = model.config()
    let location = model.location
    let replacing = location == .local
      ? model.notes.contains { $0.hasPrefix("The settings on this Mac were edited by hand") }
      : model.notes.contains { $0.hasPrefix("Saving to the project rewrites") }
    let write = { [weak self] in
      model.isSaving = true
      DispatchQueue.global(qos: .userInitiated).async {
        let result = Self.writeProjectSettings(config, root: root, location: location)
        DispatchQueue.main.async {
          guard let self else { return }
          model.isSaving = false
          switch result {
          case .success(let saved):
            var trust = ProjectTrustStore.current
            trust.trust(root: saved.trustKey, digest: saved.digest)
            ProjectTrustStore.current = trust
            self.toasts.show(
              Toast(kind: .success, message: location == .local ? "Saved the project settings on this Mac" : "Saved \(ProjectConfig.relativePath); commit it to share"))
            self.loadProjectSetup(model, root: root, section: nil)
            then?()
          case .failure(let error):
            self.toasts.show(Toast(kind: .warning, message: "Couldn't save the project settings: \(error.localizedDescription)"))
          }
        }
      }
    }
    if replacing {
      gitConfirm(
        title: location == .local ? "Rewrite the settings on this Mac?" : "Rewrite .impulse/project.toml's settings?",
        message: location == .local
          ? "The file was edited by hand. Saving writes it again from this screen."
          : "Comments inside the sections this screen manages are dropped; the rest of the file is kept.",
        confirmTitle: "Save", destructive: false
      ) { if $0 { write() } }
    } else {
      write()
    }
  }

  /// Write `config` where `location` says; the trust key and digest of what
  /// was written.
  private static func writeProjectSettings(
    _ config: ProjectConfig, root: String, location: ProjectSetupModel.Location
  ) -> Result<(trustKey: String, digest: String), Error> {
    let committedPath = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    let committedText = try? String(contentsOfFile: committedPath, encoding: .utf8)
    let path: String
    let text: String
    let key: String
    switch location {
    case .local:
      guard let common = GitClient.commonGitDirectory(forPath: root) else {
        return .failure(CocoaError(.fileNoSuchFile))
      }
      path = ProjectConfig.localPath(commonGitDirectory: common)
      let committed = committedText.flatMap { try? ProjectConfig.parse($0).get() }
      text = ProjectSettingsFile.text(config, clearing: committed)
      key = path
    case .project:
      path = committedPath
      text = ProjectSettingsFile.merging(config, into: committedText ?? "")
      key = trustKey(for: root)
    }
    do {
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      let data = Data(text.utf8)
      try data.write(to: URL(fileURLWithPath: path), options: .atomic)
      return .success((key, ProjectConfig.digest(data)))
    } catch {
      return .failure(error)
    }
  }

  /// Write the current ports and values into every task's env file (the
  /// tasks Impulse made, from their own settings once trusted).
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

  /// Open the settings file the screen saves to, creating it if needed.
  private func openProjectSettingsFile(root: String, location: ProjectSetupModel.Location) {
    let path: String
    switch location {
    case .local:
      guard let common = GitClient.commonGitDirectory(forPath: root) else { return }
      path = ProjectConfig.localPath(commonGitDirectory: common)
    case .project:
      path = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    }
    if !FileManager.default.fileExists(atPath: path) {
      try? FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      FileManager.default.createFile(atPath: path, contents: Data())
    }
    openFile(path: path)
  }
}
