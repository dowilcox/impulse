import AppKit
import ImpulseKit
import SwiftUI

/// Project Setup as a tab: what a repository's tasks need (files to copy,
/// folders and a database to clone, ports and values of their own, scripts,
/// actions), proposed from what's in the repository and saved as its
/// project settings, on this Mac (`.git/impulse/project.toml`) or committed
/// (`.impulse/project.toml`).
@Observable
final class ProjectSetupModel {
  struct Row: Identifiable, Equatable {
    let id = UUID()
    var path: String
    var isOn: Bool
    var note: String?
    var size: String?
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

  enum Location: Hashable { case local, project }
  enum Database: Hashable { case none, clone, dump, empty }

  var palette: ChromePalette
  /// The repository's main checkout.
  var root = ""
  var isLoading = true
  var isSaving = false
  var location: Location = .local
  var composeFileName: String?
  var composeWarnings: [String] = []
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
  /// What the Database row offers, and what's chosen.
  var databaseOptions: [Database] = []
  var database: Database = .none
  var databaseFolder: String?
  var databaseService: String?
  var dumpCommand: String?
  var emptyCommand: String?
  /// Tasks Impulse made that have a slot, for Re-apply.
  var taskCount = 0
  /// Things to know before saving ("Edited by hand…").
  var notes: [String] = []
  /// Bumped to scroll to a section ("actions").
  var revealSection: String?
  var revealToken = 0

  @ObservationIgnored var onSave: (() -> Void)?
  @ObservationIgnored var onTry: (() -> Void)?
  @ObservationIgnored var onReapply: (() -> Void)?
  @ObservationIgnored var onShowCompose: (() -> Void)?
  @ObservationIgnored var onOpenFile: (() -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var name: String { (root as NSString).lastPathComponent }

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

  private func removing(_ command: String, from script: String) -> String {
    script.replacingOccurrences(of: " && \(command)", with: "").replacingOccurrences(of: command, with: "")
      .trimmingCharacters(in: .whitespaces)
  }

  /// The settings as the screen has them.
  func config() -> ProjectConfig {
    func text(_ value: String) -> String? {
      let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
      return trimmed.isEmpty ? nil : trimmed
    }
    var config = ProjectConfig(
      actions: actions.filter(\.isOn).map(\.action).filter { !$0.name.isEmpty && !$0.command.isEmpty },
      setupScript: text(setup), archiveScript: text(archive), worktreeCopy: copies.filter(\.isOn).map(\.path))
    config.checkScript = text(check)
    config.worktreeClone = clones.filter(\.isOn).map(\.path)
    config.envFile = text(envFile) ?? ProjectConfig().envFile
    config.portOffset = Int(portOffset.trimmingCharacters(in: .whitespaces)).flatMap { $0 > 0 ? $0 : nil } ?? 100
    for pair in ports {
      let name = pair.name.trimmingCharacters(in: .whitespaces)
      if !name.isEmpty, let port = Int(pair.value.trimmingCharacters(in: .whitespaces)) { config.ports[name] = port }
    }
    for pair in values {
      let name = pair.name.trimmingCharacters(in: .whitespaces)
      if !name.isEmpty { config.worktreeEnv[name] = pair.value }
    }
    if database == .clone {
      config.databaseFolder = databaseFolder
      config.databaseService = databaseService
    }
    config.composeOverride = composeOverride
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
        Text("What new tasks get: files, folders, ports and values of their own, and scripts.")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      }
      Spacer()
      Text("Save to").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      ChromeSegmented(
        options: [(ProjectSetupModel.Location.local, "This Mac"), (.project, "The project")],
        selection: Binding(get: { model.location }, set: { model.location = $0 }),
        help: "This Mac: .git/impulse/project.toml, never committed. The project: .impulse/project.toml, to commit.")
      ChromeButton(title: "Open File", kind: .ghost) { model.onOpenFile?() }
      ChromeButton(title: model.isSaving ? "Saving…" : "Save", icon: .check, kind: .primary) { model.onSave?() }
        .disabled(model.isLoading || model.isSaving)
    }
    .padding(.horizontal, 20)
    .frame(height: 58)
    .background(model.palette.panel)
  }

  @ViewBuilder private var sections: some View {
    let chrome = model.palette
    VStack(alignment: .leading, spacing: 22) {
      ForEach(model.notes, id: \.self) { note in
        Label(note, systemImage: "info.circle").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      }

      if !model.composeWarnings.isEmpty {
        section("Docker Compose", detail: "\(model.composeFileName ?? "The Compose file") keeps two checkouts' stacks from running at once:") {
          ForEach(model.composeWarnings, id: \.self) { warning in
            HStack(spacing: 6) {
              Icon(.triangleAlert, size: 12).foregroundStyle(chrome.warning)
              Text(warning).font(ChromeFont.mono(11.5)).foregroundStyle(chrome.text)
            }
          }
          HStack(spacing: 10) {
            Toggle(isOn: Binding(get: { model.composeOverride }, set: { model.composeOverride = $0 })) {
              Text("Handle in tasks").font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
            }
            .toggleStyle(.checkbox)
            ChromeButton(title: "Show", kind: .ghost) { model.onShowCompose?() }
          }
          caption("Each new task gets an override (in .git/impulse, never committed) that renames its containers and moves these ports by its slot. Needs Docker Compose 2.24 or later.")
        }
      }

      section("Copy into tasks", detail: "Ignored files a fresh checkout lacks, copied from the main checkout.") {
        if model.copies.isEmpty { caption("No ignored files found.") }
        ForEach(Binding(get: { model.copies }, set: { model.copies = $0 })) { $row in
          checkRow($row)
        }
      }

      section("Clone into tasks", detail: "Folders cloned in at once, with no extra disk until they change.") {
        if model.clones.isEmpty { caption("No ignored dependency or build folders found.") }
        ForEach(Binding(get: { model.clones }, set: { model.clones = $0 })) { $row in
          checkRow($row)
        }
      }

      if !model.databaseOptions.isEmpty {
        section("Database", detail: "Each task's own stack starts with an empty database unless it gets one.") {
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
          }
        }
      }

      section("Ports", detail: "The main checkout's ports. Task n adds n × the offset to each, written into its \(model.envFile).") {
        pairs(Binding(get: { model.ports }, set: { model.ports = $0 }), namePlaceholder: "APP_PORT", valuePlaceholder: "8000")
        HStack(spacing: 8) {
          ChromeButton(title: "Add Port", icon: .plus, kind: .ghost) { model.ports.append(.init(name: "", value: "")) }
          Spacer()
          Text("Offset per task").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
          TextField("100", text: Binding(get: { model.portOffset }, set: { model.portOffset = $0 }))
            .textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 70)
        }
      }

      section("Values for each task", detail: "Written into each new task's \(model.envFile). Use {task}, {task_}, {slot} and port names.") {
        pairs(Binding(get: { model.values }, set: { model.values = $0 }), namePlaceholder: "DB_DATABASE", valuePlaceholder: "myapp_{task_}")
        ChromeButton(title: "Add Value", icon: .plus, kind: .ghost) { model.values.append(.init(name: "", value: "")) }
      }

      section("Scripts", detail: "Commands run in tasks: once trusted, and asked again whenever they change.") {
        script("Setup", "Runs in a new task's first terminal, before the agent", Binding(get: { model.setup }, set: { model.setup = $0 }))
        script("Check", "Checks a task is ready to land (types, tests)", Binding(get: { model.check }, set: { model.check = $0 }))
        script("Archive", "Runs before a task's folder is removed", Binding(get: { model.archive }, set: { model.archive = $0 }))
      }

      section("Actions", detail: "Commands in the palette's a: list.") {
        if model.actions.isEmpty { caption("No actions yet.") }
        ForEach(Binding(get: { model.actions }, set: { model.actions = $0 })) { $row in
          HStack(spacing: 8) {
            Toggle("", isOn: $row.isOn).toggleStyle(.checkbox).labelsHidden()
            TextField("Name", text: $row.action.name).textFieldStyle(.roundedBorder).font(ChromeFont.ui(12)).frame(width: 160)
            TextField("Command", text: $row.action.command).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
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

  private func title(of option: ProjectSetupModel.Database) -> String {
    switch option {
    case .clone: return "Clone the data folder"
    case .dump: return "Dump and load"
    case .empty: return "Empty, with migrations and seed data"
    case .none: return "Nothing"
    }
  }

  private func detail(of option: ProjectSetupModel.Database) -> String {
    switch option {
    case .clone:
      let service = model.databaseService.map { ", stopping \($0) in the main checkout for a second or two" } ?? ""
      return "\(model.databaseFolder ?? "") is cloned into each task\(service)."
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

  private func caption(_ text: String) -> some View {
    Text(text).font(ChromeFont.ui(11)).foregroundStyle(model.palette.textTertiary).fixedSize(horizontal: false, vertical: true)
  }

  private func checkRow(_ row: Binding<ProjectSetupModel.Row>) -> some View {
    let chrome = model.palette
    return HStack(spacing: 8) {
      Toggle(isOn: row.isOn) {
        Text(row.wrappedValue.path).font(ChromeFont.mono(12)).foregroundStyle(chrome.text)
      }
      .toggleStyle(.checkbox)
      if let size = row.wrappedValue.size {
        Text(size).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
      }
      if let note = row.wrappedValue.note {
        Text(note).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).lineLimit(1)
      }
    }
  }

  private func pairs(_ list: Binding<[ProjectSetupModel.Pair]>, namePlaceholder: String, valuePlaceholder: String) -> some View {
    ForEach(list) { $pair in
      HStack(spacing: 8) {
        TextField(namePlaceholder, text: $pair.name).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).frame(width: 200)
        TextField(valuePlaceholder, text: $pair.value).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12))
        ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
          list.wrappedValue.removeAll { $0.id == pair.id }
        }
      }
    }
  }

  private func script(_ title: String, _ help: String, _ text: Binding<String>) -> some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 3) {
      HStack(spacing: 6) {
        Text(title).font(ChromeFont.ui(12, weight: .medium)).foregroundStyle(chrome.text)
        Text(help).font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary)
      }
      TextField("", text: text, axis: .vertical).textFieldStyle(.roundedBorder).font(ChromeFont.mono(12)).lineLimit(1...4)
    }
  }
}
