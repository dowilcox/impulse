import AppKit
import ImpulseKit

/// Keeps each repository's pull request (for its current branch) fresh by
/// asking the GitHub CLI, when it's installed. Quietly does nothing without
/// `gh`, without a GitHub remote, or when signed out.
final class PullRequestMonitor {
  static let shared = PullRequestMonitor()

  private let queue = DispatchQueue(label: "impulse.pr", qos: .utility)
  private var lastFetch: [String: (branch: String?, at: Date)] = [:]
  private var inFlight: Set<String> = []
  private lazy var ghPath: String? = LoginShell.which("gh")

  /// Refresh a repository's PR, at most every 60 s per branch unless forced
  /// (a branch switch always refetches).
  func refresh(_ repository: GitRepositoryState, force: Bool = false) {
    // Headless snapshot runs never talk to GitHub.
    guard AppState.persistenceEnabled else { return }
    let root = repository.root
    let branch = repository.snapshot?.branch
    guard !inFlight.contains(root) else { return }
    if !force, let last = lastFetch[root], last.branch == branch, Date().timeIntervalSince(last.at) < 60 {
      return
    }
    guard let branch, !(repository.snapshot?.isDetached ?? true) else {
      repository.pullRequest = nil
      return
    }
    inFlight.insert(root)
    lastFetch[root] = (branch, Date())
    queue.async { [weak self] in
      let info = self?.fetch(root: root)
      DispatchQueue.main.async {
        guard let self else { return }
        self.inFlight.remove(root)
        // Ignore an answer for a branch we've since left.
        guard repository.snapshot?.branch == branch else { return }
        repository.pullRequest = info
      }
    }
  }

  private func fetch(root: String) -> PullRequestInfo? {
    run(["pr", "view", "--json", PullRequestInfo.ghFields], root: root).flatMap(PullRequestInfo.parse)
  }

  /// Unresolved review threads on `pullRequest`, as review comments (nil
  /// when gh failed: signed out, offline, not GitHub).
  func reviewThreads(
    root: String, pullRequest: PullRequestInfo, completion: @escaping ([ReviewComment]?) -> Void
  ) {
    guard let coordinates = PullRequestThreads.coordinates(fromURL: pullRequest.url) else {
      return completion(nil)
    }
    queue.async { [weak self] in
      let data = self?.run(
        [
          "api", "graphql", "-f", "query=\(PullRequestThreads.query)",
          "-F", "owner=\(coordinates.owner)", "-F", "name=\(coordinates.name)",
          "-F", "number=\(coordinates.number)",
        ], root: root)
      let comments = data.flatMap { PullRequestThreads.parse($0) }
      DispatchQueue.main.async { completion(comments) }
    }
  }

  /// Run gh in `root` and return its stdout when it succeeds.
  private func run(_ arguments: [String], root: String) -> Data? {
    guard let gh = ghPath else { return nil }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: gh)
    process.arguments = arguments
    process.currentDirectoryURL = URL(fileURLWithPath: root)
    var environment = ProcessInfo.processInfo.environment
    environment["PATH"] = LoginShell.loginPath()
    environment["GH_PROMPT_DISABLED"] = "1"
    environment["NO_COLOR"] = "1"
    process.environment = environment
    let output = Pipe()
    process.standardOutput = output
    process.standardError = Pipe()
    let done = DispatchSemaphore(value: 0)
    process.terminationHandler = { _ in done.signal() }
    do { try process.run() } catch { return nil }
    let data = output.fileHandleForReading.readDataToEndOfFile()
    if done.wait(timeout: .now() + 20) == .timedOut {
      process.terminate()
      return nil
    }
    return process.terminationStatus == 0 ? data : nil
  }

  /// `gh pr create --web` (fills in the branch; opens the browser).
  func createInBrowser(root: String, completion: @escaping (Bool) -> Void) {
    guard let gh = ghPath else { return completion(false) }
    queue.async {
      let process = Process()
      process.executableURL = URL(fileURLWithPath: gh)
      process.arguments = ["pr", "create", "--web"]
      process.currentDirectoryURL = URL(fileURLWithPath: root)
      var environment = ProcessInfo.processInfo.environment
      environment["PATH"] = LoginShell.loginPath()
      environment["GH_PROMPT_DISABLED"] = "1"
      process.environment = environment
      process.standardOutput = Pipe()
      process.standardError = Pipe()
      let ok = (try? process.run()) != nil
      if ok { process.waitUntilExit() }
      DispatchQueue.main.async { completion(ok && process.terminationStatus == 0) }
    }
  }

  var isAvailable: Bool { ghPath != nil }
}
