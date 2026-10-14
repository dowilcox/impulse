import AppKit
import ImpulseKit
import SwiftUI

/// Project Setup as a tab: what a repository's tasks need (files to copy,
/// folders and a database to clone, ports and values of their own, scripts,
/// actions), proposed from what's in the repository and saved in its
/// `.impulse/project.toml`, which every task reads from the main checkout.
@Observable
final class ProjectSetupModel {
  struct Row: Identifiable, Equatable {
    let id = UUID()
    var path: String
    var isOn: Bool
    var note: String?
    var size: String?
    /// Added by the user, or saved but not found: its path is edited, and
    /// it can be removed.
    var isCustom = false
  }

  struct Pair: Identifiable, Equatable {
    let id = UUID()
    var name: String
    var value: String
  }

  struct ActionRow: Identifiable, Equatable {
    let id = UUID()
    var isOn: Bool
    var action: ProjectConfig.Action
  }

  enum Database: Hashable { case none, clone, dump, empty }

  /// The local `.git/impulse/project.toml`, when there is one. Impulse's own
  /// (an earlier save, Finish Task's answer) is shown folded in and removed
  /// on save; `dropped` names its settings the project's replace. One
  /// edited by hand is left alone, and wins in `overrides`.
  enum LocalFile: Equatable {
    case impulses(text: String, dropped: [String])
    case handEdited(overrides: [String])
    /// It doesn't parse: the message says where.
    case unreadable(String)
  }

  var palette: ChromePalette
  /// The repository's main checkout.
  var root = ""
  var isLoading = true
  /// `.impulse/project.toml` as the screen read it (nil: there was none),
  /// so saving can tell it was changed meanwhile.
  @ObservationIgnored var loadedText: String?
  /// Why `.impulse/project.toml` can't be read, when it can't: the screen
  /// says so instead of showing (and saving over) its settings.
  var loadError: String?
  var isSaving = false
  /// What the repository suggests. Fields the saved settings leave empty
  /// offer it rather than being filled with it.
  var found: ProjectSuggestions?
  var composeFileName: String?
  var fixedPorts: [ProjectSuggestions.FixedPort] = []
  var containerNames: [ProjectSuggestions.ContainerName] = []
  var composeOverride = false
  var copies: [Row] = []
  var clones: [Row] = []
  var ports: [Pair] = []
  var portOffset = "100"
  var values: [Pair] = []
  var envFile = ".env"
  var setup = ""
  var check = ""
  var archive = ""
  var actions: [ActionRow] = []
  /// `[on_change]`: file → command.
  var rules: [Pair] = []
  /// `overlap_ignore`, comma-separated.
  var overlapIgnore = ""
  /// How Finish Task lands work (nil: it asks the first time).
  var landing: ProjectConfig.Landing?
  /// What the Database row offers, and what's chosen.
  var databaseOptions: [Database] = []
  var database: Database = .none
  /// The data folder cloned into tasks, and the Compose service stopped in
  /// the main checkout meanwhile, as typed.
  var databaseFolder = ""
  var databaseService = ""
  var dumpCommand: String?
  var emptyCommand: String?
  /// Tasks Impulse made that have a slot, for Re-apply.
  var taskCount = 0
  var localFile: LocalFile?
  /// Saving drops comments in the sections of `.impulse/project.toml` the
  /// screen manages, and keys there it doesn't know (`scripts.lint`).
  var dropsComments = false
  var unknownKeys: [String] = []
  /// Bumped to scroll to a section ("actions").
  var revealSection: String?
  var revealToken = 0

  @ObservationIgnored var onSave: (() -> Void)?
  @ObservationIgnored var onTry: (() -> Void)?
  @ObservationIgnored var onReapply: (() -> Void)?
  @ObservationIgnored var onShowCompose: (() -> Void)?
  @ObservationIgnored var onOpenFile: (() -> Void)?
  @ObservationIgnored var onOpenLocalFile: (() -> Void)?
  @ObservationIgnored var onReload: (() -> Void)?
  /// Pick a file (or with `true`, a folder) in the repository; `done` gets
  /// its path relative to the root.
  @ObservationIgnored var onChoosePath: ((Bool, @escaping (String) -> Void) -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var name: String { (root as NSString).lastPathComponent }

  /// What saving drops from `.impulse/project.toml` because the screen
  /// doesn't show it ("comments and scripts.lint"), if anything.
  var dropped: String? {
    let parts = (dropsComments ? ["comments"] : []) + unknownKeys
    return parts.isEmpty ? nil : ListFormatter.localizedString(byJoining: parts)
  }

  /// What task n adds to each port, n times.
  var offset: Int { Int(portOffset.trimmingCharacters(in: .whitespaces)).flatMap { $0 > 0 ? $0 : nil } ?? 100 }

  /// Saved: the file holds what the screen shows, with `local` what's now
  /// in the local file. Nothing is read again, so the screen keeps its
  /// place and focus.
  func markSaved(text: String, local: LocalFile?) {
    loadedText = text
    localFile = local
    dropsComments = ProjectSettingsFile.managedSectionsHaveComments(text)
    unknownKeys = ProjectSettingsFile.unknownManagedKeys(text)
  }

  /// Choose what the Database row does. Dump-and-load and an empty
  /// database are commands in the setup script, so they're shown (and can
  /// be edited) there.
  func choose(_ choice: Database) {
    for command in [dumpCommand, emptyCommand].compactMap({ $0 }) {
      setup = removing(command, from: setup)
    }
    database = choice
    let added = choice == .dump ? dumpCommand : choice == .empty ? emptyCommand : nil
    if let added { setup = setup.isEmpty ? added : "\(setup) && \(added)" }
  }

  /// `script` without `command` and the `&&` that joined it to the rest,
  /// wherever it stands.
  private func removing(_ command: String, from script: String) -> String {
    let joined = [" && \(command)", "\(command) && "].first(where: script.contains) ?? command
    return script.replacingOccurrences(of: joined, with: "").trimmingCharacters(in: .whitespaces)
  }

  // MARK: Problems

  /// Why a port row can't be saved as shown; nil when it can, or when it's
  /// left empty (skipped).
  func problem(port pair: Pair) -> String? {
    if let problem = problem(name: pair, in: ports, variable: true) { return problem }
    if isBlank(pair) { return nil }
    if names(values).contains(trimmed(pair.name)) { return "Also a value for each task" }
    guard let port = Int(trimmed(pair.value)), (1...65535).contains(port) else { return "Not a port number (1–65535)" }
    return nil
  }

  func problem(value pair: Pair) -> String? {
    if let problem = problem(name: pair, in: values, variable: true) { return problem }
    if isBlank(pair) { return nil }
    if names(ports).contains(trimmed(pair.name)) { return "Also a port" }
    let known = Set(["task", "task_", "slot"]).union(names(ports))
    if let unknown = Self.placeholders(in: pair.value).first(where: { !known.contains($0) }) {
      return "No {\(unknown)}: use {task}, {task_}, {slot} or a port's name"
    }
    return nil
  }

  func problem(rule pair: Pair) -> String? {
    if let problem = problem(name: pair, in: rules, variable: false, missing: "Needs a file name") { return problem }
    if !isBlank(pair), trimmed(pair.value).isEmpty { return "Needs a command" }
    return nil
  }

  func problem(action row: ActionRow) -> String? {
    let name = trimmed(row.action.name)
    let command = trimmed(row.action.command)
    guard row.isOn, !(name.isEmpty && command.isEmpty) else { return nil }
    if name.isEmpty { return "Needs a name" }
    if command.isEmpty { return "Needs a command" }
    if actions.filter({ $0.isOn && trimmed($0.action.name) == name }).count > 1 { return "\(name) is listed twice" }
    return nil
  }

  var offsetProblem: String? {
    let value = trimmed(portOffset)
    return value.isEmpty || Int(value).map({ $0 > 0 }) == true ? nil : "A whole number above 0"
  }

  var databaseProblem: String? {
    database == .clone && trimmed(databaseFolder).isEmpty ? "Choose the folder that holds the data" : nil
  }

  /// How many entries can't be saved as shown. Save waits for none, so
  /// nothing on the screen is dropped without a word.
  var problemCount: Int {
    ports.filter { problem(port: $0) != nil }.count + values.filter { problem(value: $0) != nil }.count
      + rules.filter { problem(rule: $0) != nil }.count + actions.filter { problem(action: $0) != nil }.count
      + (offsetProblem == nil ? 0 : 1) + (databaseProblem == nil ? 0 : 1)
  }

  /// A row's name: given, a variable name where it goes into an env file,
  /// and not given twice.
  private func problem(name pair: Pair, in list: [Pair], variable: Bool, missing: String = "Needs a name") -> String? {
    guard !isBlank(pair) else { return nil }
    let name = trimmed(pair.name)
    if name.isEmpty { return missing }
    if variable, !Self.isVariableName(name) { return "Not a variable name (letters, digits and _)" }
    if list.filter({ trimmed($0.name) == name }).count > 1 { return "\(name) is listed twice" }
    return nil
  }

  private func isBlank(_ pair: Pair) -> Bool { trimmed(pair.name).isEmpty && trimmed(pair.value).isEmpty }

  private func names(_ list: [Pair]) -> Set<String> { Set(list.map { trimmed($0.name) }.filter { !$0.isEmpty }) }

  private func trimmed(_ text: String) -> String { text.trimmingCharacters(in: .whitespaces) }

  static func isVariableName(_ name: String) -> Bool {
    guard let first = name.unicodeScalars.first, first == "_" || (first.isASCII && first.properties.isAlphabetic) else {
      return false
    }
    return name.unicodeScalars.allSatisfy { $0 == "_" || ($0.isASCII && ($0.properties.isAlphabetic || ("0"..."9").contains($0))) }
  }

  /// The `{NAME}` placeholders in a value (not other braces, as in JSON).
  static func placeholders(in value: String) -> [String] {
    var names: [String] = []
    var rest = Substring(value)
    while let open = rest.firstIndex(of: "{") {
      rest = rest[rest.index(after: open)...]
      guard let close = rest.firstIndex(of: "}") else { break }
      let name = String(rest[..<close])
      if isVariableName(name) { names.append(name) }
      rest = rest[rest.index(after: close)...]
    }
    return names
  }

  /// The settings as the screen has them.
  func config() -> ProjectConfig {
    func text(_ value: String) -> String? {
      let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
      return trimmed.isEmpty ? nil : trimmed
    }
    var config = ProjectConfig(
      actions: actions.filter(\.isOn).map(\.action)
        .map { .init(name: trimmed($0.name), command: trimmed($0.command), cwd: $0.cwd, open: $0.open) }
        .filter { !$0.name.isEmpty && !$0.command.isEmpty },
      setupScript: text(setup), archiveScript: text(archive),
      worktreeCopy: copies.filter(\.isOn).compactMap { text($0.path) })
    config.checkScript = text(check)
    config.worktreeClone = clones.filter(\.isOn).compactMap { text($0.path) }
    config.envFile = text(envFile) ?? ProjectConfig().envFile
    config.portOffset = offset
    for pair in ports {
      let name = pair.name.trimmingCharacters(in: .whitespaces)
      if !name.isEmpty, let port = Int(pair.value.trimmingCharacters(in: .whitespaces)) { config.ports[name] = port }
    }
    for pair in values {
      let name = pair.name.trimmingCharacters(in: .whitespaces)
      if !name.isEmpty { config.worktreeEnv[name] = pair.value }
    }
    if database == .clone, let folder = text(databaseFolder) {
      config.databaseFolder = folder
      config.databaseService = text(databaseService)
    }
    config.composeOverride = composeOverride
    config.landing = landing
    config.overlapIgnore = overlapIgnore.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
      .filter { !$0.isEmpty }
    for pair in rules {
      let name = pair.name.trimmingCharacters(in: .whitespaces)
      let command = pair.value.trimmingCharacters(in: .whitespaces)
      if !name.isEmpty, !command.isEmpty { config.onChange[name] = command }
    }
    return config
  }
}

final class ProjectSetupSurface: NSView, ToolSurface {
  let model: ProjectSetupModel

  var toolKind: String { "project-setup" }
  var toolTitle: String { "Project Setup" }
  var toolSymbol: String { "hammer" }

  init(palette: ChromePalette) {
    model = ProjectSetupModel(palette: palette)
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
    let hosting = WorkbenchHosting.make(ProjectSetupView(model: model))
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
}

// MARK: - Views

struct ProjectSetupView: View {
  var model: ProjectSetupModel

  var body: some View {
    let chrome = model.palette
    VStack(spacing: 0) {
      header
      Rectangle().fill(chrome.hairline).frame(height: 1)
      if model.isLoading {
        VStack {
          ProgressView().controlSize(.small)
          Text("Looking through \(model.name)…").font(ChromeFont.ui(12)).foregroundStyle(chrome.textTertiary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else if let error = model.loadError {
        unreadable(error)
      } else {
        ScrollViewReader { proxy in
          ScrollView {
            sections
              .padding(.horizontal, 28)
              .padding(.vertical, 20)
              .frame(maxWidth: 820, alignment: .leading)
              .frame(maxWidth: .infinity, alignment: .leading)
          }
          .onChange(of: model.revealToken) { _, _ in
            if let section = model.revealSection { withAnimation { proxy.scrollTo(section, anchor: .top) } }
          }
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
        Text("Project Setup — \(model.name)").font(ChromeFont.ui(15, weight: .semibold)).foregroundStyle(chrome.text)
        Text("What new tasks get: files, folders, ports and values of their own, and scripts. Saved in \(ProjectConfig.relativePath).")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      }
      Spacer()
      ChromeButton(title: "Open File", kind: .ghost, help: "Open \(ProjectConfig.relativePath) in the editor") {
        model.onOpenFile?()
      }
      if model.problemCount > 0 {
        Text(model.problemCount == 1 ? "1 entry to fix" : "\(model.problemCount) entries to fix")
          .font(ChromeFont.ui(11.5)).foregroundStyle(model.palette.danger)
      }
      ChromeButton(title: model.isSaving ? "Saving…" : "Save", icon: .check, kind: .primary) { model.onSave?() }
        .disabled(model.isLoading || model.isSaving || model.loadError != nil || model.problemCount > 0)
    }
    .padding(.horizontal, 20)
    .frame(height: 58)
    .background(model.palette.panel)
  }

  @ViewBuilder private var sections: some View {
    VStack(alignment: .leading, spacing: 22) {
      notes

      section("Copy into tasks", detail: "Files a fresh checkout lacks (ignored or untracked), copied from the main checkout. Patterns work: config/*.local.json.") {
        if model.copies.isEmpty { caption("No ignored files found.") }
        paths(Binding(get: { model.copies }, set: { model.copies = $0 }), placeholder: "config/local.json", folders: false)
        ChromeButton(title: "Add File", icon: .plus, kind: .ghost) {
          model.copies.append(.init(path: "", isOn: true, isCustom: true))
        }
      }

      section("Clone into tasks", detail: "Folders cloned in at once, with no extra disk until they change.") {
        if model.clones.isEmpty { caption("No ignored dependency or build folders found.") }
        paths(Binding(get: { model.clones }, set: { model.clones = $0 }), placeholder: "storage/app", folders: true)
        ChromeButton(title: "Add Folder", icon: .plus, kind: .ghost) {
          model.clones.append(.init(path: "", isOn: true, isCustom: true))
        }
      }

      database

      ports
        .id("ports")

      section("Values for each task", detail: "Written into each new task's \(model.envFile). Use {task}, {task_}, {slot} and port names.") {
        pairs(
          Binding(get: { model.values }, set: { model.values = $0 }), namePlaceholder: "DB_DATABASE",
          valuePlaceholder: "myapp_{task_}", problem: model.problem(value:))
        suggested(model.found?.values ?? [:], in: Binding(get: { model.values }, set: { model.values = $0 }))
        ChromeButton(title: "Add Value", icon: .plus, kind: .ghost) { model.values.append(.init(name: "", value: "")) }
      }

      section("Scripts", detail: "Commands run in tasks: once trusted, and asked again whenever they change.") {
        script(
          "Setup", "Runs in a new task's first terminal, before the agent", Binding(get: { model.setup }, set: { model.setup = $0 }),
          suggested: model.found?.setup)
        script(
          "Check", "Checks a task is ready to land (types, tests)", Binding(get: { model.check }, set: { model.check = $0 }),
          suggested: model.found?.check)
        script(
          "Archive", "Runs before a task's folder is removed", Binding(get: { model.archive }, set: { model.archive = $0 }),
          suggested: model.found?.archive)
      }
      .id("scripts")

      section("Finishing tasks", detail: "How Finish Task lands a task's branch.") {
        ChromeSegmented(
          options: [(nil, "Ask the first time"), (ProjectConfig.Landing.merge, "Merge and push"), (.review, "Push for review")],
          selection: Binding(get: { model.landing }, set: { model.landing = $0 }))
        switch model.landing {
        case nil: caption("The first Finish in this repository asks, and remembers the answer.")
        case .review?:
          caption("Finish pushes the branch; you open a merge request on your git host. Once it's merged there, Impulse offers to clean up.")
        case .merge?:
          caption("Finish merges the branch into its base with a merge commit, made in a throwaway folder, and pushes the base. The main checkout isn't touched.")
        }
      }

      section("When files change", detail: "After a pull, merge or checkout brings in a change to one of these files, Impulse offers to run its command.") {
        pairs(
          Binding(get: { model.rules }, set: { model.rules = $0 }), namePlaceholder: "composer.lock",
          valuePlaceholder: "composer install", problem: model.problem(rule:))
        suggested(model.found?.onChange ?? [:], in: Binding(get: { model.rules }, set: { model.rules = $0 }))
        ChromeButton(title: "Add Rule", icon: .plus, kind: .ghost) { model.rules.append(.init(name: "", value: "")) }
      }

      section("Ignore when tasks overlap", detail: "Files that don't count when two workspaces change the same files: names, paths or patterns, separated by commas.") {
        TextField("CHANGELOG.md, *.snap", text: Binding(get: { model.overlapIgnore }, set: { model.overlapIgnore = $0 }))
          .textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
      }

      section("Actions", detail: "Commands in the palette's a: list.") {
        if model.actions.isEmpty { caption("No actions yet.") }
        ForEach(Binding(get: { model.actions }, set: { model.actions = $0 })) { $row in
          VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 8) {
              Toggle("", isOn: $row.isOn).toggleStyle(.checkbox).labelsHidden()
              TextField("Name", text: $row.action.name).textFieldStyle(.roundedBorder).font(ChromeFont.ui(12)).frame(width: 160)
              TextField("Command", text: $row.action.command).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
            }
            problem(model.problem(action: row)).padding(.leading, 22)
          }
        }
        ChromeButton(title: "Add Action", icon: .plus, kind: .ghost) {
          model.actions.append(.init(isOn: true, action: .init(name: "", command: "")))
        }
      }
      .id("actions")

      section("Try it", detail: "Save, then see the settings work.") {
        HStack(spacing: 10) {
          ChromeButton(title: "Try in a New Task", icon: .gitBranchPlus) { model.onTry?() }
          if model.taskCount > 0 {
            ChromeButton(title: "Re-apply Values to \(model.taskCount) Task\(model.taskCount == 1 ? "" : "s")") { model.onReapply?() }
          }
        }
        caption("Try saves the settings and opens New Task. Re-apply writes the current ports and values into the env files of tasks that already exist.")
      }
    }
  }

  // MARK: Sections

  /// In place of the settings when the file doesn't parse.
  private func unreadable(_ message: String) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 6) {
        Icon(.triangleAlert, size: 14).foregroundStyle(chrome.warning)
        Text("\(ProjectConfig.relativePath) can't be read").font(ChromeFont.ui(13, weight: .semibold))
          .foregroundStyle(chrome.text)
      }
      caption("New tasks get none of its settings until it's fixed, and this screen can't show or save them.")
      Text(message).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.textSecondary).textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
      HStack(spacing: 10) {
        ChromeButton(title: "Open File", kind: .primary) { model.onOpenFile?() }
        ChromeButton(title: "Reload") { model.onReload?() }
      }
    }
    .padding(.horizontal, 28)
    .padding(.vertical, 20)
    .frame(maxWidth: 820, alignment: .leading)
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
  }

  @ViewBuilder private var notes: some View {
    let local = ".git/impulse/project.toml"
    switch model.localFile {
    case .impulses(_, let dropped)?:
      note(
        "Settings kept on this Mac only, in \(local), are included here. Saving moves them into \(ProjectConfig.relativePath) and removes that file"
          + (dropped.isEmpty ? "." : "; where the two differed (\(list(dropped))), the project's are shown."))
    case .handEdited(let overrides)?:
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        note(
          "On this Mac, \(local) "
            + (overrides.isEmpty ? "has settings of its own" : "overrides the project's \(list(overrides))")
            + ". It was edited by hand, so saving leaves it alone.")
        ChromeButton(title: "Open", kind: .ghost) { model.onOpenLocalFile?() }
      }
    case .unreadable(let message)?:
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        note("On this Mac, \(local) can't be read, so new tasks get none of the project's settings until it's fixed: \(message)")
        ChromeButton(title: "Open", kind: .ghost) { model.onOpenLocalFile?() }
      }
    case nil:
      EmptyView()
    }
    if let dropped = model.dropped {
      note(
        "Saving rewrites the sections this screen manages in \(ProjectConfig.relativePath), dropping what it doesn't show there: \(dropped).")
    }
  }

  private var database: some View {
    let chrome = model.palette
    let summary =
      model.dumpCommand != nil
      ? "Each task's own stack starts with an empty database unless it gets one."
      : "What a task's database starts with."
    return section("Database", detail: summary) {
      ForEach(model.databaseOptions, id: \.self) { option in
        Button {
          model.choose(option)
        } label: {
          HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: model.database == option ? "largecircle.fill.circle" : "circle")
              .foregroundStyle(model.database == option ? chrome.accent : chrome.textTertiary)
            VStack(alignment: .leading, spacing: 2) {
              Text(title(of: option)).font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
              Text(detail(of: option)).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            }
          }
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        if option == .clone, model.database == .clone {
          databaseFolderFields.padding(.leading, 22).padding(.bottom, 4)
        }
      }
    }
  }

  private var databaseFolderFields: some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 6) {
      HStack(spacing: 8) {
        Text("Folder").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).frame(width: 52, alignment: .leading)
        TextField("docker/data/mysql", text: Binding(get: { model.databaseFolder }, set: { model.databaseFolder = $0 }))
          .textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
        ChromeIconButton(icon: .folderOpen, help: "Choose a folder", size: 22, iconSize: 12) {
          model.onChoosePath?(true) { model.databaseFolder = $0 }
        }
      }
      problem(model.databaseProblem).padding(.leading, 60)
      HStack(spacing: 8) {
        Text("Service").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).frame(width: 52, alignment: .leading)
        TextField("mysql", text: Binding(get: { model.databaseService }, set: { model.databaseService = $0 }))
          .textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 160)
        Text("Optional: the Compose service that writes it, stopped in the main checkout while it's cloned.")
          .font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).fixedSize(horizontal: false, vertical: true)
      }
    }
  }

  private var ports: some View {
    let chrome = model.palette
    let hasCompose = !model.fixedPorts.isEmpty || !model.containerNames.isEmpty
    return section(
      "Ports",
      detail: "Two checkouts can't listen on the same port, so task n moves each one up by n × \(model.offset): task 1 gets \(8000 + model.offset) for 8000."
    ) {
      if hasCompose { composePorts }
      if hasCompose {
        subheading("In \(model.envFile)", detail: "Variables each task gets its own value of, in its \(model.envFile). Compose ports written as ${NAME:-8000} come from here.")
      } else {
        caption("Each task gets its own value of these in its \(model.envFile).")
      }
      if model.ports.isEmpty, model.found?.ports.isEmpty != false {
        caption("None found. Add the variables your app or Compose file reads its ports from.")
      }
      ForEach(Binding(get: { model.ports }, set: { model.ports = $0 })) { $pair in
        VStack(alignment: .leading, spacing: 2) {
          HStack(spacing: 8) {
            TextField("APP_PORT", text: $pair.name).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 200)
            TextField("8000", text: $pair.value).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 90)
            Text(Int(pair.value.trimmingCharacters(in: .whitespaces)).map { "task 1: \($0 + model.offset)" } ?? "")
              .font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
            Spacer(minLength: 0)
            ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
              // Read through the binding before the list is changed: reading it
              // during removeAll overlaps the write to `ports` (a crash).
              let id = pair.id
              model.ports.removeAll { $0.id == id }
            }
          }
          problem(model.problem(port: pair))
        }
      }
      suggested((model.found?.ports ?? [:]).mapValues(String.init), in: Binding(get: { model.ports }, set: { model.ports = $0 }))
      HStack(spacing: 8) {
        ChromeButton(title: "Add Port", icon: .plus, kind: .ghost) { model.ports.append(.init(name: "", value: "")) }
        Spacer()
        Text("Offset per task").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
        TextField("100", text: Binding(get: { model.portOffset }, set: { model.portOffset = $0 }))
          .textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 70)
      }
      if let offsetProblem = model.offsetProblem {
        HStack {
          Spacer()
          problem(offsetProblem)
        }
      }
    }
  }

  /// The Compose file's fixed ports and container names, which a task's
  /// override moves and renames.
  @ViewBuilder private var composePorts: some View {
    let chrome = model.palette
    let file = model.composeFileName ?? "the Compose file"
    subheading("In \(file)", detail: "Written as plain numbers, so only one checkout's stack can run at a time.")
    ForEach(Array(model.fixedPorts.enumerated()), id: \.offset) { _, fixed in
      composeRow(fixed.service, fixed.port.raw, task: ComposeFile.moved(fixed.port, by: model.offset))
    }
    ForEach(Array(model.containerNames.enumerated()), id: \.offset) { _, container in
      composeRow(container.service, "container_name \(container.name)", task: "\(container.name)-<task>")
    }
    HStack(spacing: 10) {
      Toggle(isOn: Binding(get: { model.composeOverride }, set: { model.composeOverride = $0 })) {
        Text("Move them in each task").font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
      }
      .toggleStyle(.checkbox)
      ChromeButton(title: "Show File", kind: .ghost) { model.onShowCompose?() }
    }
    if model.composeOverride {
      caption(
        "\(file) itself isn't changed. Each task gets an override file (in .git/impulse, never committed) that moves these ports and renames the containers, and its \(model.envFile) sets COMPOSE_FILE so docker compose uses it. Needs Docker Compose 2.24 or later."
      )
    } else {
      HStack(spacing: 6) {
        Icon(.triangleAlert, size: 12).foregroundStyle(chrome.warning)
        caption("Off: a task's stack can't start while another checkout's is running.")
      }
    }
  }

  private func composeRow(_ service: String, _ value: String, task: String) -> some View {
    let chrome = model.palette
    return HStack(spacing: 8) {
      Text(service).font(ChromeFont.ui(12)).foregroundStyle(chrome.textSecondary).frame(width: 110, alignment: .leading)
      Text(value).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.text)
      if model.composeOverride {
        Text("task 1: \(task)").font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
      }
    }
  }

  private func title(of option: ProjectSetupModel.Database) -> String {
    switch option {
    case .clone: return "Clone a data folder"
    case .dump: return "Dump and load"
    case .empty: return "Empty, with migrations and seed data"
    case .none: return "Nothing"
    }
  }

  private func detail(of option: ProjectSetupModel.Database) -> String {
    switch option {
    case .clone: return "A folder holding the database's data is cloned from the main checkout into each task."
    case .dump: return "The setup script loads a dump of the main checkout's database into the task's."
    case .empty: return "The setup script runs \(model.emptyCommand ?? "the migrations")."
    case .none: return "Each task starts with an empty database."
    }
  }

  // MARK: Pieces

  private func section<Content: View>(_ title: String, detail: String, @ViewBuilder content: () -> Content) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 8) {
      Text(title).font(ChromeFont.ui(13, weight: .semibold)).foregroundStyle(chrome.text)
      Text(detail).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).fixedSize(horizontal: false, vertical: true)
      VStack(alignment: .leading, spacing: 6) { content() }.padding(.top, 2)
    }
  }

  private func subheading(_ title: String, detail: String) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 2) {
      Text(title).font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
      caption(detail)
    }
    .padding(.top, 4)
  }

  private func caption(_ text: String) -> some View {
    Text(text).font(ChromeFont.ui(11)).foregroundStyle(model.palette.textTertiary).fixedSize(horizontal: false, vertical: true)
  }

  private func note(_ text: String) -> some View {
    Label(text, systemImage: "info.circle").font(ChromeFont.ui(11.5)).foregroundStyle(model.palette.textSecondary)
      .fixedSize(horizontal: false, vertical: true)
  }

  private func list(_ names: [String]) -> String {
    ListFormatter.localizedString(byJoining: names)
  }

  /// Detected paths as checkboxes; the user's own as fields to edit, pick
  /// with a panel or remove.
  private func paths(_ rows: Binding<[ProjectSetupModel.Row]>, placeholder: String, folders: Bool) -> some View {
    let chrome = model.palette
    return ForEach(rows) { $row in
      HStack(spacing: 8) {
        if row.isCustom {
          Toggle("", isOn: $row.isOn).toggleStyle(.checkbox).labelsHidden()
          TextField(placeholder, text: $row.path).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
          ChromeIconButton(icon: .folderOpen, help: folders ? "Choose a folder" : "Choose a file", size: 22, iconSize: 12) {
            model.onChoosePath?(folders) { $row.path.wrappedValue = $0 }
          }
          ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
            let id = row.id
            rows.wrappedValue.removeAll { $0.id == id }
          }
        } else {
          Toggle(isOn: $row.isOn) {
            Text(row.path).font(ChromeFont.mono(12)).foregroundStyle(chrome.text)
          }
          .toggleStyle(.checkbox)
          if let size = row.size {
            Text(size).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
          }
          if let note = row.note {
            Text(note).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).lineLimit(1)
          }
        }
      }
    }
  }

  private func pairs(
    _ list: Binding<[ProjectSetupModel.Pair]>, namePlaceholder: String, valuePlaceholder: String,
    problem: @escaping (ProjectSetupModel.Pair) -> String?
  ) -> some View {
    ForEach(list) { $pair in
      VStack(alignment: .leading, spacing: 2) {
        HStack(spacing: 8) {
          TextField(namePlaceholder, text: $pair.name).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 200)
          TextField(valuePlaceholder, text: $pair.value).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
          ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
            let id = pair.id
            list.wrappedValue.removeAll { $0.id == id }
          }
        }
        self.problem(problem(pair))
      }
    }
  }

  /// Why the entry above can't be saved, if it can't.
  @ViewBuilder private func problem(_ message: String?) -> some View {
    if let message {
      Text(message).font(ChromeFont.ui(11)).foregroundStyle(model.palette.danger).fixedSize(horizontal: false, vertical: true)
    }
  }

  private func script(_ title: String, _ help: String, _ text: Binding<String>, suggested: String?) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 3) {
      HStack(spacing: 6) {
        Text(title).font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
        Text(help).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
      }
      TextField("", text: text, axis: .vertical).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).lineLimit(1...4)
      if let suggested, text.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        suggestion(suggested, button: "Use") { text.wrappedValue = suggested }
      }
    }
  }

  /// The suggested pairs whose names `list` lacks, offered to add.
  @ViewBuilder
  private func suggested(_ found: [String: String], in list: Binding<[ProjectSetupModel.Pair]>) -> some View {
    let names = Set(list.wrappedValue.map { $0.name.trimmingCharacters(in: .whitespaces) })
    let missing = found.keys.sorted().filter { !names.contains($0) }
    if !missing.isEmpty {
      suggestion(missing.map { "\($0) = \(found[$0]!)" }.joined(separator: ", "), button: "Add") {
        list.wrappedValue += missing.map { .init(name: $0, value: found[$0]!) }
      }
    }
  }

  /// A suggestion offered beside a field, not filled into it.
  private func suggestion(_ text: String, button: String, apply: @escaping () -> Void) -> some View {
    let chrome = model.palette
    return HStack(alignment: .firstTextBaseline, spacing: 6) {
      Text("Suggested:").font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
      Text(text).font(ChromeFont.mono(11)).foregroundStyle(chrome.textSecondary).lineLimit(1).truncationMode(.tail)
      ChromeButton(title: button, kind: .ghost) { apply() }
    }
  }
}
