// GitCLI — the write path of the git layer.
//
// libgit2 (the rest of ImpulseGit) is used for reads: status, diffs, blame,
// log. Mutations go through the user's `git` binary instead so they behave
// exactly like the git the user (and their agents) run in the terminal:
// hooks run, commits are signed per config, LFS/clean-smudge filters apply,
// credential helpers and SSH agents work for remote operations.

import Foundation
import ImpulseKit

/// Output of a finished `git` invocation.
public struct GitCLIResult: Equatable, Sendable {
  public let status: Int32
  public let stdout: String
  public let stderr: String

  public init(status: Int32, stdout: String, stderr: String) {
    self.status = status
    self.stdout = stdout
    self.stderr = stderr
  }
}

/// A failed `git` invocation, classified so the UI can show a plain-English
/// explanation while keeping the raw output available for details.
public struct GitCLIError: Error, Equatable, CustomStringConvertible, Sendable {
  public enum Kind: String, Equatable, Sendable {
    case gitNotFound
    case commandLineToolsMissing
    case notARepository
    case identityNotConfigured
    case indexLocked
    case nonFastForward
    case noUpstream
    case authenticationFailed
    case localChangesWouldBeOverwritten
    case mergeConflict
    case nothingToCommit
    case unknownRevision
    case branchAlreadyExists
    case hookFailed
    case timedOut
    case cancelled
    case other
  }

  public let kind: Kind
  /// The arguments passed to git (without the binary path).
  public let arguments: [String]
  public let status: Int32
  /// Raw stderr (or stdout when stderr was empty) from git.
  public let output: String

  public init(kind: Kind, arguments: [String], status: Int32, output: String) {
    self.kind = kind
    self.arguments = arguments
    self.status = status
    self.output = output
  }

  /// One sentence describing what went wrong and what to do about it.
  public var message: String {
    switch kind {
    case .gitNotFound:
      return "Git isn't installed or isn't on your PATH. Install it with Homebrew or the Xcode Command Line Tools."
    case .commandLineToolsMissing:
      return "Git needs the Xcode Command Line Tools. Run `xcode-select --install` in a terminal."
    case .notARepository:
      return "This folder isn't inside a git repository."
    case .identityNotConfigured:
      return "Git doesn't know who you are yet. Set `git config --global user.name` and `user.email`."
    case .indexLocked:
      return "Another git process is using this repository (index.lock exists). Wait for it to finish, then try again."
    case .nonFastForward:
      return "The remote has commits you don't have. Pull before pushing."
    case .noUpstream:
      return "This branch has no upstream branch. Publish it first."
    case .authenticationFailed:
      return "Git couldn't authenticate with the remote. Run the command in a terminal to sign in."
    case .localChangesWouldBeOverwritten:
      return "Your uncommitted changes would be overwritten. Commit or stash them first."
    case .mergeConflict:
      return "The operation stopped with conflicts. Resolve them, then continue."
    case .nothingToCommit:
      return "There's nothing to commit."
    case .unknownRevision:
      return "Git doesn't know that branch or revision."
    case .branchAlreadyExists:
      return "A branch with that name already exists."
    case .hookFailed:
      return "A git hook rejected the operation. See the hook output for details."
    case .timedOut:
      return "Git took too long and was stopped."
    case .cancelled:
      return "The git operation was cancelled."
    case .other:
      let firstLine = output.split(separator: "\n").first.map(String.init) ?? ""
      return firstLine.isEmpty ? "Git failed (exit \(status))." : firstLine
    }
  }

  public var description: String { "git \(arguments.joined(separator: " ")): \(message)" }
}

/// Runs the user's `git` binary.
public enum GitCLI {
  private static let lock = NSLock()
  nonisolated(unsafe) private static var cachedGitPath: String??

  /// Absolute path of the git binary resolved from the login-shell PATH, or
  /// nil when git is unavailable. Cached after the first lookup.
  public static func gitPath() -> String? {
    lock.lock()
    defer { lock.unlock() }
    if let cachedGitPath { return cachedGitPath }
    let resolved = LoginShell.which("git")
    cachedGitPath = .some(resolved)
    return resolved
  }

  /// `/usr/bin/git` is an xcrun shim: without the Command Line Tools (or
  /// Xcode) installed, running it pops a system install dialog instead of
  /// running git. Detect that case so we never trigger the dialog by surprise.
  static func isMissingToolsShim(_ path: String) -> Bool {
    guard path == "/usr/bin/git" else { return false }
    let fm = FileManager.default
    if fm.fileExists(atPath: "/Library/Developer/CommandLineTools/usr/bin/git") { return false }
    if fm.fileExists(atPath: "/Applications/Xcode.app/Contents/Developer/usr/bin/git") { return false }
    return true
  }

  /// Run `git <arguments>` in `directory` and wait for it to finish.
  ///
  /// - Parameters:
  ///   - stdin: bytes written to git's standard input (e.g. a commit message
  ///     for `commit -F -`, or a patch for `apply --cached`).
  ///   - environment: extra variables layered over the inherited environment.
  ///   - timeout: wall-clock limit; the process is terminated when exceeded.
  ///   - onOutputLine: called (on a background queue) for each stderr line as
  ///     it arrives — git writes progress for fetch/push there.
  /// - Returns: the result on exit status 0, otherwise a classified error.
  @discardableResult
  public static func run(
    _ arguments: [String],
    in directory: String,
    stdin: Data? = nil,
    environment: [String: String] = [:],
    timeout: TimeInterval = 120,
    onOutputLine: ((String) -> Void)? = nil
  ) -> Result<GitCLIResult, GitCLIError> {
    guard let git = gitPath() else {
      return .failure(GitCLIError(kind: .gitNotFound, arguments: arguments, status: -1, output: ""))
    }
    if isMissingToolsShim(git) {
      return .failure(
        GitCLIError(kind: .commandLineToolsMissing, arguments: arguments, status: -1, output: ""))
    }

    let process = Process()
    process.executableURL = URL(fileURLWithPath: git)
    process.arguments = arguments
    process.currentDirectoryURL = URL(fileURLWithPath: directory)
    process.environment = mergedEnvironment(extra: environment)

    let outPipe = Pipe()
    let errPipe = Pipe()
    process.standardOutput = outPipe
    process.standardError = errPipe
    let inPipe: Pipe? = stdin == nil ? nil : Pipe()
    process.standardInput = inPipe ?? FileHandle.nullDevice

    // Drain both pipes concurrently so a chatty command can't fill a pipe
    // buffer and deadlock against waitUntilExit.
    let group = DispatchGroup()
    var outData = Data()
    var errData = Data()
    let errLines = LineSplitter(onLine: onOutputLine)

    // Signal exit from the termination handler rather than waitUntilExit():
    // waitUntilExit on a background thread can miss the exit of a child that
    // finished quickly and then block forever.
    let exited = DispatchSemaphore(value: 0)
    process.terminationHandler = { _ in exited.signal() }

    do {
      try process.run()
    } catch {
      return .failure(
        GitCLIError(
          kind: .gitNotFound, arguments: arguments, status: -1,
          output: error.localizedDescription))
    }

    group.enter()
    DispatchQueue.global(qos: .userInitiated).async {
      outData = outPipe.fileHandleForReading.readDataToEndOfFile()
      group.leave()
    }
    group.enter()
    DispatchQueue.global(qos: .userInitiated).async {
      let handle = errPipe.fileHandleForReading
      while true {
        let chunk = handle.availableData
        if chunk.isEmpty { break }
        errData.append(chunk)
        errLines.feed(chunk)
      }
      errLines.flush()
      group.leave()
    }

    if let inPipe, let stdin {
      DispatchQueue.global(qos: .userInitiated).async {
        try? inPipe.fileHandleForWriting.write(contentsOf: stdin)
        try? inPipe.fileHandleForWriting.close()
      }
    }

    var timedOut = false
    if exited.wait(timeout: .now() + timeout) == .timedOut {
      timedOut = true
      process.terminate()
      if exited.wait(timeout: .now() + 5) == .timedOut {
        kill(process.processIdentifier, SIGKILL)
        _ = exited.wait(timeout: .now() + 5)
      }
    }
    // Pipes close when the process (and any children holding them) exit.
    _ = group.wait(timeout: .now() + 5)

    let stdout = String(decoding: outData, as: UTF8.self)
    let stderr = String(decoding: errData, as: UTF8.self)
    let status = process.terminationStatus

    if timedOut {
      return .failure(
        GitCLIError(kind: .timedOut, arguments: arguments, status: status, output: stderr))
    }
    if status == 0 {
      return .success(GitCLIResult(status: status, stdout: stdout, stderr: stderr))
    }
    let output = stderr.isEmpty ? stdout : stderr
    return .failure(
      GitCLIError(
        kind: classify(output: stdout + "\n" + stderr, arguments: arguments),
        arguments: arguments, status: status, output: output))
  }

  /// Environment for git: the app's environment with the login PATH, no
  /// interactive prompts (a GUI can't answer them), and English messages so
  /// `classify` can recognise them.
  static func mergedEnvironment(extra: [String: String]) -> [String: String] {
    var env = ProcessInfo.processInfo.environment
    env["PATH"] = LoginShell.loginPath()
    env["GIT_TERMINAL_PROMPT"] = "0"
    env["GIT_ASKPASS"] = env["GIT_ASKPASS"] ?? ""
    env["SSH_ASKPASS"] = env["SSH_ASKPASS"] ?? ""
    env["GCM_INTERACTIVE"] = "never"
    env["LC_MESSAGES"] = "C"
    env["GIT_EDITOR"] = "true"
    for (key, value) in extra { env[key] = value }
    return env
  }

  /// Classify git's output into a known failure kind. Pure; unit-tested.
  public static func classify(output: String, arguments: [String]) -> GitCLIError.Kind {
    let text = output.lowercased()
    func has(_ needle: String) -> Bool { text.contains(needle) }

    if has("not a git repository") { return .notARepository }
    if has("please tell me who you are") || has("unable to auto-detect email address")
      || has("empty ident name")
    {
      return .identityNotConfigured
    }
    if has("index.lock") && (has("file exists") || has("another git process")) {
      return .indexLocked
    }
    if has("would be overwritten by") || has("your local changes to the following files") {
      return .localChangesWouldBeOverwritten
    }
    if has("[rejected]") || has("non-fast-forward") || has("updates were rejected")
      || has("fetch first")
    {
      return .nonFastForward
    }
    if has("has no upstream branch") || has("no upstream configured")
      || has("there is no tracking information")
    {
      return .noUpstream
    }
    if has("authentication failed") || has("could not read username")
      || has("permission denied (publickey") || has("terminal prompts disabled")
      || has("could not read from remote repository")
    {
      return .authenticationFailed
    }
    if has("conflict (") || has("merge conflict") || has("fix conflicts and then")
      || has("needs merge") || has("you have unmerged paths")
    {
      return .mergeConflict
    }
    if has("nothing to commit") || has("no changes added to commit") {
      return .nothingToCommit
    }
    if has("already exists") && (arguments.first == "branch" || arguments.first == "switch"
      || arguments.first == "checkout" || arguments.first == "worktree")
    {
      return .branchAlreadyExists
    }
    if has("did not match any file(s) known to git") || has("invalid reference")
      || has("unknown revision") || has("not a valid object name")
      || has("invalid branch name")
    {
      return .unknownRevision
    }
    if has("hook") && (has("declined") || has("failed") || has("exit"))
      || (arguments.first == "commit" && has("pre-commit"))
    {
      return .hookFailed
    }
    return .other
  }
}

/// Splits streamed bytes into lines on `\n` and `\r` (git progress rewrites
/// one line with carriage returns).
private final class LineSplitter {
  private var buffer = Data()
  private let onLine: ((String) -> Void)?

  init(onLine: ((String) -> Void)?) {
    self.onLine = onLine
  }

  func feed(_ data: Data) {
    guard let onLine else { return }
    buffer.append(data)
    while let index = buffer.firstIndex(where: { $0 == 0x0A || $0 == 0x0D }) {
      let line = String(decoding: buffer[buffer.startIndex..<index], as: UTF8.self)
      buffer.removeSubrange(buffer.startIndex...index)
      if !line.isEmpty { onLine(line) }
    }
  }

  func flush() {
    guard let onLine, !buffer.isEmpty else { return }
    onLine(String(decoding: buffer, as: UTF8.self))
    buffer.removeAll()
  }
}
