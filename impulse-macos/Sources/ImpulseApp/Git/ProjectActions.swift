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

  /// The project's config if it has one and it parses (no trust needed to read).
  func projectConfig(root: String) -> ProjectConfig? {
    guard let loaded = ProjectConfig.load(root: root) else { return nil }
    switch loaded.config {
    case .success(let config): return config
    case .failure(let error):
      if case .invalid(let message) = error {
        toasts.show(Toast(kind: .warning, message: "\(ProjectConfig.relativePath): \(message)", lifetime: 12))
      }
      return nil
    }
  }

  /// Trust is per main repository, so a task worktree with the same file
  /// doesn't ask again.
  static func trustKey(for root: String) -> String {
    GitClient.commonGitDirectory(forPath: root).map { ($0 as NSString).deletingLastPathComponent } ?? root
  }

  /// Run `body` with the project's config once the user trusts this exact
  /// file (asking when it's new or changed). Nothing runs otherwise.
  func withTrustedProjectConfig(root: String, then body: @escaping (ProjectConfig) -> Void) {
    trustProjectConfig(root: root) { config in
      if let config { body(config) }
    }
  }

  /// The project's config once trusted; nil when there's none, it doesn't
  /// parse, or the user declined.
  func trustProjectConfig(root: String, completion: @escaping (ProjectConfig?) -> Void) {
    guard let loaded = ProjectConfig.load(root: root), case .success(let config) = loaded.config else {
      return completion(nil)
    }
    let key = Self.trustKey(for: root)
    if config.commands.isEmpty || ProjectTrustStore.current.isTrusted(root: key, digest: loaded.digest) {
      return completion(config)
    }
    let list = config.commands.prefix(8).map { "  \($0)" }.joined(separator: "\n")
    let more = config.commands.count > 8 ? "\n  …and \(config.commands.count - 8) more" : ""
    gitConfirm(
      title: "Trust \(ProjectConfig.relativePath) in \((root as NSString).lastPathComponent)?",
      message: "It can run these commands on your Mac:\n\(list)\(more)\n\nYou'll be asked again if the file changes.",
      confirmTitle: "Trust and Run", destructive: false
    ) { trusted in
      guard trusted else { return completion(nil) }
      var trust = ProjectTrustStore.current
      trust.trust(root: key, digest: loaded.digest)
      ProjectTrustStore.current = trust
      completion(config)
    }
  }

  /// Run a project script synchronously in `directory` (off the main
  /// thread); false when it fails or takes over two minutes.
  static func runScript(_ script: String, in directory: String) -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sh")
    process.arguments = ["-c", script]
    process.currentDirectoryURL = URL(fileURLWithPath: directory)
    var environment = ProcessInfo.processInfo.environment
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

  /// Create or open `.impulse/project.toml` with an example.
  func editProjectConfig() {
    guard let root = projectRoot else {
      toasts.show(Toast(kind: .info, message: "Open a folder in a git repository first."))
      return
    }
    let path = (root as NSString).appendingPathComponent(ProjectConfig.relativePath)
    if !FileManager.default.fileExists(atPath: path) {
      try? FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      let example = """
        # Impulse project settings. Commands here only run after you trust
        # this file, and you're asked again whenever it changes.

        # Palette actions (a: in the palette). open = "tab" | "right" | "down".
        [[actions]]
        name = "Dev server"
        command = "npm run dev"
        open = "right"

        [[actions]]
        name = "Tests"
        command = "npm test"

        # Task worktrees (New Task…).
        [scripts]
        # setup = "npm ci"      # runs in a new task's first terminal
        # archive = ""          # runs before a task's folder is removed

        [worktrees]
        # Untracked files to copy into new tasks, in addition to .worktreeinclude.
        copy = []

        """
      try? example.write(toFile: path, atomically: true, encoding: .utf8)
    }
    openFile(path: path)
  }
}
