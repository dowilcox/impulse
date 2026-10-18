import AppKit
import ImpulseGit
import ImpulseKit

/// Trusted project.toml files, remembered across launches.
enum ProjectTrustStore {
  private static let key = "projectTrust"
  /// Snapshot runs start with nothing trusted and keep what they trust in
  /// memory: never the dev build's own answers.
  nonisolated(unsafe) private static var snapshotTrust = ProjectTrust()

  static var current: ProjectTrust {
    get {
      if DebugSnapshot.isActive { return snapshotTrust }
      guard let data = UserDefaults.standard.data(forKey: key),
        let trust = try? JSONDecoder().decode(ProjectTrust.self, from: data)
      else { return ProjectTrust() }
      return trust
    }
    set {
      if DebugSnapshot.isActive {
        snapshotTrust = newValue
        return
      }
      if let data = try? JSONEncoder().encode(newValue) { UserDefaults.standard.set(data, forKey: key) }
    }
  }
}

extension MainWindowController {
  /// The repository root for project actions: the active workspace's
  /// repository, else the window's.
  var projectRoot: String? {
    tabManager.activeWorkspace.repository?.root ?? windowModel.repository?.root
  }

  /// The settings for the checkout at `root`, which are its repository's:
  /// the main checkout's `.impulse/project.toml`, committed or not, so every
  /// task gets what Project Setup saved, then the local
  /// `.git/impulse/project.toml`.
  static func loadProjectConfig(root: String) -> ProjectConfig.Loaded? {
    ProjectConfig.load(root: mainCheckoutRoot(of: root), commonGitDirectory: GitClient.commonGitDirectory(forPath: root))
  }

  /// The project's config if it has one and it parses (no trust needed to read).
  func projectConfig(root: String) -> ProjectConfig? {
    guard let loaded = Self.loadProjectConfig(root: root) else { return nil }
    switch loaded.config {
    case .success(let config): return config
    case .failure(.invalid(let message)):
      toasts.show(Toast(kind: .warning, message: message, lifetime: 12))
      return nil
    }
  }

  /// Trust is per main repository, so a task worktree with the same file
  /// doesn't ask again.
  static func trustKey(for root: String) -> String {
    GitClient.commonGitDirectory(forPath: root).map { ($0 as NSString).deletingLastPathComponent } ?? root
  }

  /// What a settings file is trusted under: the committed file per main
  /// repository (`trustKey`), the local file by its own path.
  static func trustKey(for source: ProjectConfig.Source, in loaded: ProjectConfig.Loaded, root: String)
    -> String
  {
    source == loaded.local ? source.path : trustKey(for: root)
  }

  private static func displayName(of source: ProjectConfig.Source, in loaded: ProjectConfig.Loaded) -> String {
    source == loaded.local ? ".git/impulse/project.toml" : ProjectConfig.relativePath
  }

  /// Run `body` with the project's config once the user trusts this exact
  /// file (asking when it's new or changed). Nothing runs otherwise.
  func withTrustedProjectConfig(root: String, then body: @escaping (ProjectConfig) -> Void) {
    trustProjectConfig(root: root) { config in
      if let config { body(config) }
    }
  }

  /// The project's config once trusted; nil when there's none, it doesn't
  /// parse, or the user declined. Each settings file with commands is
  /// trusted on its own, for exactly its content.
  func trustProjectConfig(root: String, completion: @escaping (ProjectConfig?) -> Void) {
    guard let loaded = Self.loadProjectConfig(root: root), case .success(let config) = loaded.config else {
      return completion(nil)
    }
    let store = ProjectTrustStore.current
    let untrusted = loaded.sources.filter {
      !store.isTrusted(root: Self.trustKey(for: $0, in: loaded, root: root), digest: $0.digest)
    }
    if untrusted.isEmpty { return completion(config) }
    let commands = untrusted.flatMap(\.commands)
    let values = Array(Set(untrusted.flatMap(\.values))).sorted()
    let name = (root as NSString).lastPathComponent
    let title =
      untrusted.count == 1
      ? "Trust \(Self.displayName(of: untrusted[0], in: loaded)) in \(name)?"
      : "Trust the project settings in \(name)?"
    let what = untrusted.count == 1 ? "It" : "\(ProjectConfig.relativePath) and .git/impulse/project.toml"
    var message = ""
    if !commands.isEmpty {
      message = "\(what) can run \(commands.count == 1 ? "the command" : "all \(commands.count) commands") below on your Mac. "
    }
    if !values.isEmpty {
      message += "\(commands.isEmpty ? what : "It also") sets \(values.joined(separator: ", ")) in each new task's env file. "
    }
    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = message + "You'll be asked again if the " + (untrusted.count == 1 ? "file changes." : "files change.")
    alert.addButton(withTitle: "Trust and Run")
    alert.addButton(withTitle: "Cancel")
    // Every command it trusts, however many or long: one left out of
    // sight would run unseen.
    if !commands.isEmpty { alert.accessoryView = Self.commandList(commands) }
    presentWhenNoSheet { window in
      alert.beginSheetModal(for: window) { response in
        guard response == .alertFirstButtonReturn else { return completion(nil) }
        var trust = ProjectTrustStore.current
        for source in untrusted {
          trust.trust(root: Self.trustKey(for: source, in: loaded, root: root), digest: source.digest)
        }
        ProjectTrustStore.current = trust
        completion(config)
      }
    }
  }

  /// The trust prompt's list of commands: all of them, selectable, in a
  /// box that scrolls once they don't fit.
  private static func commandList(_ commands: [String]) -> NSView {
    let font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
    let text = commands.map(listedCommand).joined(separator: "\n")
    let lines = text.components(separatedBy: "\n").count
    let lineHeight = NSLayoutManager().defaultLineHeight(for: font)
    let scroll = NSTextView.scrollableTextView()
    scroll.frame = NSRect(x: 0, y: 0, width: 440, height: CGFloat(min(max(lines, 2), 14)) * lineHeight + 12)
    scroll.borderType = .bezelBorder
    scroll.hasVerticalScroller = true
    if let view = scroll.documentView as? NSTextView {
      view.frame.size.width = scroll.contentSize.width
      view.isEditable = false
      view.isSelectable = true
      view.font = font
      view.textContainerInset = NSSize(width: 4, height: 4)
      view.string = text
    }
    return scroll
  }

  /// A command as the trust prompt lists it: `$ ` before its first line,
  /// its other lines indented under it, and characters that would hide
  /// what it does (control characters, direction overrides, zero-width
  /// ones) written out as `\u{202E}`.
  static func listedCommand(_ command: String) -> String {
    command.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: .newlines).enumerated()
      .map { index, line in
        var shown = index == 0 ? "$ " : "  "
        for scalar in line.unicodeScalars {
          let category = scalar.properties.generalCategory
          if scalar != "\t", category == .control || category == .format {
            shown += "\\u{\(String(scalar.value, radix: 16, uppercase: true))}"
          } else {
            shown.unicodeScalars.append(scalar)
          }
        }
        return shown
      }
      .joined(separator: "\n")
  }

  /// The project's config when every settings file that needs trusting is
  /// trusted already; nil otherwise. Never asks.
  func alreadyTrustedProjectConfig(root: String) -> ProjectConfig? {
    guard let loaded = Self.loadProjectConfig(root: root), case .success(let config) = loaded.config else {
      return nil
    }
    let store = ProjectTrustStore.current
    let trusted = loaded.sources.allSatisfy {
      store.isTrusted(root: Self.trustKey(for: $0, in: loaded, root: root), digest: $0.digest)
    }
    return trusted ? config : nil
  }

  /// Run a project script synchronously in `directory` (off the main
  /// thread); false when it fails or takes over two minutes. `extra` is
  /// added to its environment (a task's `IMPULSE_TASK`, say).
  static func runScript(_ script: String, in directory: String, extra: [String: String] = [:]) -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sh")
    process.arguments = ["-c", script]
    process.currentDirectoryURL = URL(fileURLWithPath: directory)
    var environment = ProcessInfo.processInfo.environment
    environment.merge(extra) { _, new in new }
    environment["PATH"] = LoginShell.loginPath()
    process.environment = environment
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    let done = DispatchSemaphore(value: 0)
    process.terminationHandler = { _ in done.signal() }
    guard (try? process.run()) != nil else { return false }
    if done.wait(timeout: .now() + 120) == .timedOut {
      process.terminate()
      return false
    }
    return process.terminationStatus == 0
  }

  /// Run a project action in a new terminal (tab, or split right/down).
  /// What runs is the trusted settings' action of that name, not the one
  /// the palette listed: the file can change in between.
  func runProjectAction(_ listed: ProjectConfig.Action, root: String) {
    withTrustedProjectConfig(root: root) { [weak self] config in
      guard let self else { return }
      guard let action = config.actions.first(where: { $0.name == listed.name }) else {
        self.toasts.show(Toast(kind: .info, message: "The project settings have no action named \(listed.name) anymore."))
        return
      }
      let directory = action.cwd.map { (root as NSString).appendingPathComponent($0) } ?? root
      switch action.open {
      case "right", "down":
        let container = self.tabManager.makeTerminalContainer(directory: directory, initialCommand: action.command)
        self.tabManager.splitSelectedTab(
          with: .terminal(container), axis: action.open == "down" ? .vertical : .horizontal)
      default:
        self.tabManager.addTerminalTab(directory: directory, initialCommand: action.command)
      }
    }
  }

  /// "Edit Project Actions": Project Setup at its Actions, where they're
  /// saved on this Mac or in the project.
  func editProjectConfig() {
    openProjectSetup(section: "actions")
  }
}
