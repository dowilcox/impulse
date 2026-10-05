// `.impulse/project.toml`: what a repository tells Impulse about itself —
// actions to run from the palette, scripts for new and archived task
// worktrees, and files to copy into them. Commands from it only run after
// the user trusts that exact file.

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

  public static let relativePath = ".impulse/project.toml"

  private struct File: Decodable {
    struct Scripts: Decodable {
      var setup: String?
      var archive: String?
    }
    struct Worktrees: Decodable {
      var copy: [String]?
    }
    var actions: [Action]?
    var scripts: Scripts?
    var worktrees: Worktrees?
  }

  public enum LoadError: Error, Equatable {
    case invalid(String)
  }

  public static func parse(_ text: String) -> Result<ProjectConfig, LoadError> {
    do {
      let file = try TOMLDecoder().decode(File.self, from: text)
      return .success(
        ProjectConfig(
          actions: (file.actions ?? []).filter { !$0.name.isEmpty && !$0.command.isEmpty },
          setupScript: file.scripts?.setup.flatMap { $0.isEmpty ? nil : $0 },
          archiveScript: file.scripts?.archive.flatMap { $0.isEmpty ? nil : $0 },
          worktreeCopy: file.worktrees?.copy ?? []))
    } catch {
      return .failure(.invalid(String(describing: error)))
    }
  }

  /// The config in `root`, nil when there's no file.
  public static func load(root: String) -> (config: Result<ProjectConfig, LoadError>, digest: String)? {
    let path = (root as NSString).appendingPathComponent(relativePath)
    guard let data = FileManager.default.contents(atPath: path) else { return nil }
    let text = String(decoding: data, as: UTF8.self)
    return (parse(text), digest(data))
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
}
