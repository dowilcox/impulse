import AppKit
import ImpulseGit
import ImpulseKit

/// The Project Setup tab: opening it on a repository, filling it from the
/// repository's settings and from what's in it, then saving into
/// `.impulse/project.toml` (which trusts what it wrote, unless the file then
/// holds settings the screen didn't show), trying the
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
    surface.onModifiedChange = { [weak self] in self?.tabManager.refreshSegmentLabels() }
    model.onSave = { [weak self, weak model] then in
      guard let self, let model else { return }
      self.saveProjectSetup(model, then: then)
    }
    model.onTry = { [weak self, weak model] in
      guard let self, let model else { return }
      self.saveProjectSetup(model) { [weak self] saved in
        guard saved else { return }
        let workspace = self?.tabManager.workspaces.first { $0.kind == .folder && $0.root == model.root }
        self?.presentNewTaskSheet(from: workspace?.id, title: "Try project setup")
      }
    }
    model.onReapply = { [weak self, weak model] in
      guard let model else { return }
      self?.saveProjectSetup(model) { [weak self] saved in
        if saved { self?.reapplyTaskValues(root: model.root) }
      }
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
    model.onReload = { [weak self, weak model] in
      guard let model else { return }
      self?.loadProjectSetup(model, root: model.root, section: nil)
    }
    model.onChoosePath = { [weak self, weak model] folders, done in
      guard let model else { return }
      self?.chooseProjectPath(in: model.root, folders: folders, then: done)
    }
    // Changes not saved yet stay: opening the tab again shows them, and
    // another repository's setup only replaces them once the user says so.
    guard model.isModified else { return loadProjectSetup(model, root: root, section: section) }
    if model.root == root {
      if let section { model.reveal(section) }
      return
    }
    gitConfirm(
      title: "Discard your changes to \(model.name)'s setup?",
      message: "Project Setup shows one repository at a time, and the changes made to \(model.name)'s aren't saved.",
      confirmTitle: "Discard Changes", destructive: true
    ) { [weak self] discard in
      if discard { self?.loadProjectSetup(model, root: root, section: section) }
    }
  }

  private func loadProjectSetup(_ model: ProjectSetupModel, root: String, section: String?) {
    // Reading the same repository again keeps the screen, and its place,
    // until the settings are in.
    if model.root != root || model.loadError != nil { model.isLoading = true }
    model.root = root
    DispatchQueue.global(qos: .userInitiated).async {
      let common = GitClient.commonGitDirectory(forPath: root)
      let localText = common.flatMap { try? String(contentsOfFile: ProjectConfig.localPath(commonGitDirectory: $0), encoding: .utf8) }
      let committedText = try? String(
        contentsOfFile: (root as NSString).appendingPathComponent(ProjectConfig.relativePath), encoding: .utf8)
      let unreadable = committedText.flatMap { text -> String? in
        if case .failure(.invalid(let message)) = ProjectConfig.parse(text) { return message }
        return nil
      }
      let saved = Self.savedSettings(committed: committedText, local: localText)
      let found = ProjectDetector.suggest(root: root, ignored: GitOperations.ignoredEntries(root: root))
      let services = ComposeFile.find(in: root).map { ComposeFile.parse($0.text).services } ?? []
      let database = ProjectDetector.databaseService(in: services)
      let dump = database.flatMap { service in service.image.flatMap { ProjectDetector.dumpAndLoad(service: service.name, image: $0) } }
      let earlierDump = database.flatMap { service in
        service.image.flatMap { ProjectDetector.earlierDumpAndLoad(service: service.name, image: $0) }
      }
      let empty = ProjectDetector.freshDatabase(root: root)
      let taskCount = TaskRegistryStore.registry(root: root)?.tasks.filter { $0.slot != nil }.count ?? 0
      DispatchQueue.main.async { [weak self] in
        model.loadedText = committedText
        model.loadError = unreadable
        guard unreadable == nil else {
          model.isLoading = false
          return
        }
        Self.fill(model, existing: saved.config, found: found, dump: dump, empty: empty)
        model.savedConfig = model.config()
        // 0.33's dump and load ran only in bash and zsh and could wait forever.
        if let earlierDump, let dump, model.setup.contains(earlierDump) {
          model.setup = model.setup.replacingOccurrences(of: earlierDump, with: dump)
          model.database = .dump
        }
        model.localFile = saved.local
        model.dropsComments = committedText.map(ProjectSettingsFile.managedSectionsHaveComments) ?? false
        model.unknownKeys = committedText.map(ProjectSettingsFile.unknownManagedKeys) ?? []
        model.taskCount = taskCount
        model.isLoading = false
        if let section { model.reveal(section) }
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
    if case .failure(.invalid(let message)) = ProjectConfig.parse(local) { return (project, .unreadable(message)) }
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
    // Suggestions fill in a project with no settings yet. Once it has some,
    // the screen shows what's saved, so a field emptied and saved stays
    // empty; the view offers the suggestion beside it instead.
    let hasSettings = existing != nil
    model.found = found
    model.copies = rows(found.copies, chosen: settings.worktreeCopy, hasSettings: hasSettings)
    model.clones = rows(found.clones, chosen: settings.worktreeClone, hasSettings: hasSettings)
    // Ports and an offset the file sets but that can't be used are shown
    // as written, marked, so they're fixed rather than dropped unseen.
    let ports = hasSettings ? settings.ports.merging(settings.unusablePorts) { _, unusable in unusable } : found.ports
    model.ports = ports.keys.sorted().map { .init(name: $0, value: String(ports[$0]!)) }
    let values = hasSettings ? settings.worktreeEnv : found.values
    model.values = values.keys.sorted().map { .init(name: $0, value: values[$0]!) }
    model.portOffset = String(settings.unusablePortOffset ?? settings.portOffset)
    model.envFile = settings.envFile
    model.setup = (hasSettings ? settings.setupScript : found.setup) ?? ""
    model.check = (hasSettings ? settings.checkScript : found.check) ?? ""
    model.archive = (hasSettings ? settings.archiveScript : found.archive) ?? ""
    model.landing = settings.landing
    let names = Set(settings.actions.map(\.name))
    model.actions =
      settings.actions.map { .init(isOn: true, action: $0) }
      + found.actions.filter { !names.contains($0.name) }.map { .init(isOn: false, action: $0) }
    let rules = hasSettings ? settings.onChange : found.onChange
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

  /// Save what the screen has into `.impulse/project.toml` and trust what
  /// was written when the file reads back as just that. Impulse's own local
  /// file, folded into the screen, is removed: its settings are in the
  /// project's now. A file
  /// changed since the screen read it is only written over once the user
  /// says so (`overwriting`). `then` says whether the settings were saved.
  private func saveProjectSetup(
    _ model: ProjectSetupModel, overwriting: Bool = false, then: ((Bool) -> Void)? = nil
  ) {
    guard model.loadError == nil else {
      then?(false)
      return
    }
    guard model.problemCount == 0 else {
      toasts.show(Toast(kind: .warning, message: "Fix the entries marked in red first: they can't be saved as they are."))
      then?(false)
      return
    }
    let root = model.root
    let config = model.config()
    let loaded = model.loadedText
    var folded: String?
    if case .impulses(let text, _)? = model.localFile { folded = text }
    let write = { [weak self] in
      model.isSaving = true
      DispatchQueue.global(qos: .userInitiated).async {
        let outcome = Self.writeProjectSettings(
          config, root: root, removingLocal: folded, unlessChangedFrom: overwriting ? nil : .some(loaded))
        DispatchQueue.main.async {
          guard let self else { return }
          model.isSaving = false
          switch outcome {
          case .saved(let digest, let text, let local):
            if let digest {
              var trust = ProjectTrustStore.current
              trust.trust(root: Self.trustKey(for: root), digest: digest)
              ProjectTrustStore.current = trust
              self.toasts.show(Toast(kind: .success, message: "Saved \(ProjectConfig.relativePath)"))
            } else {
              self.toasts.show(
                Toast(
                  kind: .success, message: "Saved \(ProjectConfig.relativePath)",
                  detail: "It also holds settings this screen didn't write, so it asks before running its commands.",
                  lifetime: 10))
            }
            model.markSaved(config, text: text, local: local)
            then?(true)
          case .changed:
            self.askAboutChangedSettings(model, then: then)
          case .failed(let error):
            self.toasts.show(Toast(kind: .warning, message: "Couldn't save the project settings: \(error.localizedDescription)"))
            then?(false)
          }
        }
      }
    }
    if let dropped = model.dropped, !overwriting {
      gitConfirm(
        title: "Rewrite \(ProjectConfig.relativePath)'s settings?",
        message: "What this screen doesn't show in the sections it manages is dropped: \(dropped). The rest of the file is kept.",
        confirmTitle: "Save", destructive: false
      ) { if $0 { write() } else { then?(false) } }
    } else {
      write()
    }
  }

  /// `.impulse/project.toml` changed since the screen read it (edited in
  /// the editor, brought in by a pull): show it as it is now, losing the
  /// screen's changes, or write over it.
  private func askAboutChangedSettings(_ model: ProjectSetupModel, then: ((Bool) -> Void)?) {
    guard let window else {
      then?(false)
      return
    }
    let alert = NSAlert()
    alert.messageText = "\(ProjectConfig.relativePath) changed since Project Setup read it"
    alert.informativeText =
      "Reload shows the file as it is now, discarding your changes here. Save Anyway replaces the sections this screen manages with what it shows, discarding the changes made to them in the file."
    alert.alertStyle = .warning
    alert.addButton(withTitle: "Reload")
    alert.addButton(withTitle: "Save Anyway")
    alert.addButton(withTitle: "Cancel")
    alert.beginSheetModal(for: window) { [weak self, weak model] response in
      guard let self, let model else {
        then?(false)
        return
      }
      switch response {
      case .alertFirstButtonReturn:
        self.loadProjectSetup(model, root: model.root, section: nil)
        then?(false)
      case .alertSecondButtonReturn:
        self.saveProjectSetup(model, overwriting: true, then: then)
      default:
        then?(false)
      }
    }
  }

  private enum SaveOutcome {
    /// What was written, the local file's state after, and the digest to
    /// trust: nil when the file reads as more than the screen saved.
    case saved(digest: String?, text: String, local: ProjectSetupModel.LocalFile?)
    /// The file isn't what the screen read.
    case changed
    case failed(Error)
  }

  /// Write `config` into the main checkout's `.impulse/project.toml`, and
  /// remove the local file when it still holds `removingLocal`. Given
  /// `unlessChangedFrom`, only while the file still holds that text (`nil`
  /// inside: there's still no file).
  private static func writeProjectSettings(
    _ config: ProjectConfig, root: String, removingLocal local: String?, unlessChangedFrom loaded: String??
  ) -> SaveOutcome {
    let path = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    let existing = try? String(contentsOfFile: path, encoding: .utf8)
    if let loaded, existing != loaded { return .changed }
    let text = ProjectSettingsFile.merging(config, into: existing ?? "")
    let data = Data(text.utf8)
    do {
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      try data.write(to: URL(fileURLWithPath: path), options: .atomic)
    } catch {
      return .failed(error)
    }
    let localPath = GitClient.commonGitDirectory(forPath: root).map(ProjectConfig.localPath(commonGitDirectory:))
    if let local, let localPath, (try? String(contentsOfFile: localPath, encoding: .utf8)) == local {
      try? FileManager.default.removeItem(atPath: localPath)
    }
    let localText = localPath.flatMap { try? String(contentsOfFile: $0, encoding: .utf8) }
    return .saved(
      digest: ProjectSettingsFile.holdsOnly(config, in: text) ? ProjectConfig.digest(data) : nil, text: text,
      local: savedSettings(committed: text, local: localText).local)
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
