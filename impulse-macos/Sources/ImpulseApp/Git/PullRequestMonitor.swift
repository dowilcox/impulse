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
  /// While checks run, poll again after a growing delay (30 s … 5 min).
  private var pollDelay: [String: TimeInterval] = [:]
  private var pollWork: [String: DispatchWorkItem] = [:]

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
        let previous = repository.pullRequest
        repository.pullRequest = info
        if let previous, let info, previous.number == info.number, previous.checks == .pending,
          info.checks == .passed || info.checks == .failed
        {
          self.announceChecks(info, repository: repository)
        }
        self.schedulePoll(repository, pending: info?.checks == .pending)
      }
    }
  }

  private func schedulePoll(_ repository: GitRepositoryState, pending: Bool) {
    let root = repository.root
    pollWork.removeValue(forKey: root)?.cancel()
    guard pending else {
      pollDelay[root] = nil
      return
    }
    let delay = min((pollDelay[root] ?? 15) * 2, 300)
    pollDelay[root] = delay
    let work = DispatchWorkItem { [weak self, weak repository] in
      guard let self, let repository else { return }
      self.refresh(repository, force: true)
    }
    pollWork[root] = work
    DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
  }

  /// Checks just finished: a desktop notification when Impulse is in the
  /// background, otherwise a toast in the window showing the repository.
  private func announceChecks(_ info: PullRequestInfo, repository: GitRepositoryState) {
    let title = info.checks == .passed ? "Checks passed" : "Checks failed"
    let body = "#\(info.number) \(info.title)"
    if NSApp.isActive {
      NotificationCenter.default.post(
        name: .pullRequestChecksFinished, object: repository,
        userInfo: ["title": title, "body": body, "url": info.url, "passed": info.checks == .passed])
    } else {
      DesktopNotifier.shared.post(
        title: title, subtitle: (repository.root as NSString).lastPathComponent, body: body, url: info.url,
        thread: "pr:\(repository.root)")
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

  /// `gh pr create --draft --fill`: a draft PR titled and described from the
  /// branch's commits. Completes with the PR's URL, or gh's error.
  func createDraft(root: String, completion: @escaping (Result<String, String>) -> Void) {
    guard ghPath != nil else { return completion(.failure("The GitHub CLI (gh) isn't installed.")) }
    queue.async { [weak self] in
      let result = self?.runCapturingErrors(["pr", "create", "--draft", "--fill"], root: root)
      DispatchQueue.main.async {
        switch result {
        case .success(let output)?:
          let url = output.split(separator: "\n").last { $0.hasPrefix("https://") }.map(String.init) ?? ""
          completion(.success(url))
        case .failure(let message)?:
          completion(.failure(message))
        case nil:
          completion(.failure("gh didn't run."))
        }
      }
    }
  }

  /// Run gh and return stdout, or stderr's last line as the failure.
  private func runCapturingErrors(_ arguments: [String], root: String) -> Result<String, String> {
    guard let gh = ghPath else { return .failure("The GitHub CLI (gh) isn't installed.") }
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
    let errors = Pipe()
    process.standardOutput = output
    process.standardError = errors
    do { try process.run() } catch { return .failure(error.localizedDescription) }
    let data = output.fileHandleForReading.readDataToEndOfFile()
    let errorData = errors.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    let stdout = String(decoding: data, as: UTF8.self)
    guard process.terminationStatus == 0 else {
      let message = String(decoding: errorData, as: UTF8.self)
        .split(separator: "\n").last { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
      return .failure(message.map(String.init) ?? "gh exited with status \(process.terminationStatus).")
    }
    return .success(stdout)
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
