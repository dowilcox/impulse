// `.impulse/project.toml`: what a repository tells Impulse about itself —
// actions to run from the palette, scripts for new and archived task
// worktrees, and files to copy into them. Commands from it only run after
// the user trusts that exact file.
//
// The same settings can also live in `.git/impulse/project.toml`, inside the
// repository's shared git folder: settings for this Mac only, never
// committed, seen by every worktree at once. Where both files set a key, the
// local one wins.

import CryptoKit
import Foundation
import TOMLKit

public struct ProjectConfig: Equatable, Sendable {
  public struct Action: Equatable, Sendable, Decodable {
    public var name: String
    public var command: String
    /// Folder to run in, relative to the repository root.
    public var cwd: String?
    /// Where the terminal opens: "tab" (default), "right" or "down".
    public var open: String?

    public init(name: String, command: String, cwd: String? = nil, open: String? = nil) {
      self.name = name
      self.command = command
      self.cwd = cwd
      self.open = open
    }
  }

  public var actions: [Action] = []
  /// Run in a new task worktree after it's created.
  public var setupScript: String?
  /// Run in a task worktree before it's archived.
  public var archiveScript: String?
  /// Untracked files to copy into new task worktrees (globs allowed).
  public var worktreeCopy: [String] = []
  /// The dotenv file a new task's own values are written into
  /// (`[worktrees] env_file`): the main checkout's copy, then updated.
  public var envFile = ".env"
  /// What task n adds to every port: n × this (`[worktrees] port_offset`).
  public var portOffset = 100
  /// The main checkout's ports, by their name in the env file
  /// (`[worktrees.ports]`).
  public var ports: [String: Int] = [:]
  /// Values that differ per task, with placeholders (`[worktrees.env]`).
  public var worktreeEnv: [String: String] = [:]

  public init(
    actions: [Action] = [], setupScript: String? = nil, archiveScript: String? = nil, worktreeCopy: [String] = []
  ) {
    self.actions = actions
    self.setupScript = setupScript
    self.archiveScript = archiveScript
    self.worktreeCopy = worktreeCopy
  }

  /// Every command the file can run, for the trust prompt.
  public var commands: [String] {
    actions.map(\.command) + [setupScript, archiveScript].compactMap { $0 }
  }

  /// Whether new tasks get values of their own (ports, env).
  public var hasTaskValues: Bool { !ports.isEmpty || !worktreeEnv.isEmpty }

  /// The names a new task's values are written under, for the trust prompt.
  public var taskValueNames: [String] { ports.keys.sorted() + worktreeEnv.keys.sorted() }

  /// The committed file, relative to the repository root.
  public static let relativePath = ".impulse/project.toml"

  /// The local file: settings for this Mac only, in the repository's shared
  /// git folder (`.git/impulse/project.toml`).
  public static func localPath(commonGitDirectory: String) -> String {
    (TaskRegistry.directory(commonGitDirectory: commonGitDirectory) as NSString)
      .appendingPathComponent("project.toml")
  }

  /// One file's settings. Keys the file doesn't set stay nil, so a later
  /// file only overrides what it sets.
  struct Layer: Decodable {
    struct Scripts: Decodable {
      var setup: String?
      var archive: String?
    }
    struct Worktrees: Decodable {
      var copy: [String]?
      var envFile: String?
      var portOffset: Int?
      var ports: [String: Int]?
      var env: [String: String]?

      enum CodingKeys: String, CodingKey {
        case copy, ports, env
        case envFile = "env_file"
        case portOffset = "port_offset"
      }
    }
    var actions: [Action]?
    var scripts: Scripts?
    var worktrees: Worktrees?
  }

  public enum LoadError: Error, Equatable {
    case invalid(String)
  }

  public static func parse(_ text: String) -> Result<ProjectConfig, LoadError> {
    parseLayer(text).map { resolve([$0]) }
  }

  static func parseLayer(_ text: String) -> Result<Layer, LoadError> {
    do {
      return .success(try TOMLDecoder().decode(Layer.self, from: text))
    } catch {
      return .failure(.invalid(String(describing: error)))
    }
  }

  /// The settings of `layers`, later layers winning key by key. Actions
  /// are keyed by name: a later action replaces an earlier one of the same
  /// name. An empty script (`setup = ""`) clears an earlier one.
  static func resolve(_ layers: [Layer]) -> ProjectConfig {
    var config = ProjectConfig()
    var setup: String?
    var archive: String?
    for layer in layers {
      for action in layer.actions ?? [] where !action.name.isEmpty && !action.command.isEmpty {
        if let index = config.actions.firstIndex(where: { $0.name == action.name }) {
          config.actions[index] = action
        } else {
          config.actions.append(action)
        }
      }
      if let value = layer.scripts?.setup { setup = value }
      if let value = layer.scripts?.archive { archive = value }
      if let copy = layer.worktrees?.copy { config.worktreeCopy = copy }
      if let file = layer.worktrees?.envFile, !file.isEmpty { config.envFile = file }
      if let offset = layer.worktrees?.portOffset, offset > 0 { config.portOffset = offset }
      config.ports.merge(layer.worktrees?.ports ?? [:]) { _, later in later }
      config.worktreeEnv.merge(layer.worktrees?.env ?? [:]) { _, later in later }
    }
    config.setupScript = setup.flatMap { $0.isEmpty ? nil : $0 }
    config.archiveScript = archive.flatMap { $0.isEmpty ? nil : $0 }
    return config
  }

  /// One settings file as read: where it is, the SHA-256 of its bytes
  /// (trust is for exactly this content), the commands it can run and the
  /// per-task values it sets.
  public struct Source: Equatable, Sendable {
    public let path: String
    public let digest: String
    public let commands: [String]
    public var values: [String] = []
  }

  /// The settings for a checkout, from both files.
  public struct Loaded: Equatable, Sendable {
    /// Both files together, local winning; a failure names the file.
    public let config: Result<ProjectConfig, LoadError>
    /// `.impulse/project.toml` in the checkout.
    public let committed: Source?
    /// `.git/impulse/project.toml`.
    public let local: Source?

    /// The files that need trusting: they run commands or set values.
    public var sources: [Source] {
      [committed, local].compactMap { $0 }.filter { !$0.commands.isEmpty || !$0.values.isEmpty }
    }
  }

  /// The settings for the checkout at `root`: its own `.impulse/project.toml`
  /// and, given the repository's shared git folder, the local file. Nil
  /// when neither exists.
  public static func load(root: String, commonGitDirectory: String?) -> Loaded? {
    let committedPath = (root as NSString).appendingPathComponent(relativePath)
    let localPath = commonGitDirectory.map { localPath(commonGitDirectory: $0) }
    var layers: [Layer] = []
    var sources: [Source?] = [nil, nil]
    var failure: LoadError?
    for (index, path) in [committedPath, localPath].enumerated() {
      guard let path, let data = FileManager.default.contents(atPath: path) else { continue }
      switch parseLayer(String(decoding: data, as: UTF8.self)) {
      case .success(let layer):
        layers.append(layer)
        let alone = resolve([layer])
        sources[index] = Source(
          path: path, digest: digest(data), commands: alone.commands, values: alone.taskValueNames)
      case .failure(.invalid(let message)):
        let name = index == 0 ? relativePath : ".git/impulse/project.toml"
        failure = failure ?? .invalid("\(name): \(message)")
        sources[index] = Source(path: path, digest: digest(data), commands: [])
      }
    }
    guard sources.contains(where: { $0 != nil }) else { return nil }
    return Loaded(
      config: failure.map { .failure($0) } ?? .success(resolve(layers)), committed: sources[0], local: sources[1])
  }

  /// SHA-256 of the file's bytes: trust is for exactly this content.
  public static func digest(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
  }
}

/// Which project.toml files the user has trusted, by repository and
/// content digest. Editing the file asks again.
public struct ProjectTrust: Codable, Equatable, Sendable {
  public var trusted: [String: String] = [:]

  public init(trusted: [String: String] = [:]) {
    self.trusted = trusted
  }

  public func isTrusted(root: String, digest: String) -> Bool { trusted[root] == digest }

  public mutating func trust(root: String, digest: String) { trusted[root] = digest }

  public mutating func revoke(root: String) { trusted[root] = nil }

  /// Forget the files of `folder` and of every repository inside it.
  public mutating func revoke(within folder: String) {
    let folder = WorkspaceTrust.normalize(folder)
    trusted = trusted.filter { !WorkspaceTrust.contains(folder, WorkspaceTrust.normalize($0.key)) }
  }

  public mutating func revokeAll() { trusted = [:] }
}
