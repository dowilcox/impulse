import AppKit
import ImpulseGit
import ImpulseKit

/// Trusted project.toml files, remembered across launches.
enum ProjectTrustStore {
  private static let key = "projectTrust"

  static var current: ProjectTrust {
    get {
      guard let data = UserDefaults.standard.data(forKey: key),
        let trust = try? JSONDecoder().decode(ProjectTrust.self, from: data)
      else { return ProjectTrust() }
      return trust
    }
    set {
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

  /// The settings of the checkout at `root`: its `.impulse/project.toml` and
  /// the repository's local `.git/impulse/project.toml`.
  static func loadProjectConfig(root: String) -> ProjectConfig.Loaded? {
    ProjectConfig.load(root: root, commonGitDirectory: GitClient.commonGitDirectory(forPath: root))
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
    let list = commands.prefix(8).map { "  \($0)" }.joined(separator: "\n")
    let more = commands.count > 8 ? "\n  …and \(commands.count - 8) more" : ""
    let name = (root as NSString).lastPathComponent
    let title =
      untrusted.count == 1
      ? "Trust \(Self.displayName(of: untrusted[0], in: loaded)) in \(name)?"
      : "Trust the project settings in \(name)?"
    let what = untrusted.count == 1 ? "It" : "\(ProjectConfig.relativePath) and .git/impulse/project.toml"
    var message = commands.isEmpty ? "" : "\(what) can run these commands on your Mac:\n\(list)\(more)\n\n"
    if !values.isEmpty {
      message += "\(commands.isEmpty ? what : "It also") sets \(values.joined(separator: ", ")) in each new task's env file.\n\n"
    }
    gitConfirm(
      title: title,
      message: message + "You'll be asked again if the " + (untrusted.count == 1 ? "file changes." : "files change."),
      confirmTitle: "Trust and Run", destructive: false
    ) { trusted in
      guard trusted else { return completion(nil) }
      var trust = ProjectTrustStore.current
      for source in untrusted {
        trust.trust(root: Self.trustKey(for: source, in: loaded, root: root), digest: source.digest)
      }
      ProjectTrustStore.current = trust
      completion(config)
    }
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
  func runProjectAction(_ action: ProjectConfig.Action, root: String) {
    withTrustedProjectConfig(root: root) { [weak self] _ in
      guard let self else { return }
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
