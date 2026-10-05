import Foundation

/// What a diff compares. Shared by the review UI (scope picker), the git
/// layer (which builds the diff) and the review protocol (Codable).
public enum DiffScope: Codable, Hashable, Sendable {
  /// HEAD → working tree (staged and unstaged together).
  case uncommitted
  /// Index → working tree (not yet staged), including untracked files.
  case unstaged
  /// HEAD → index (what the next commit contains).
  case staged
  /// merge-base(base, HEAD) → working tree: everything this branch changed
  /// relative to `base` (e.g. "main" or "origin/main"), including
  /// uncommitted work.
  case branch(base: String)
  /// A single commit against its first parent.
  case commit(sha: String)
  /// Tree of `from` → tree of `to`.
  case range(from: String, to: String)
  /// A stash entry against the commit it was made on.
  case stash(index: Int)
  /// A snapshot ref (e.g. an agent-turn checkpoint) → another ref, or the
  /// working tree when `to` is nil.
  case snapshot(from: String, to: String?)

  /// Whether the right-hand side is the live working tree (so the diff
  /// changes as files are edited, and hunks can be reverted on disk).
  public var includesWorkingTree: Bool {
    switch self {
    case .uncommitted, .unstaged, .branch: return true
    case .snapshot(_, let to): return to == nil
    case .staged, .commit, .range, .stash: return false
    }
  }

  /// Whether hunks/lines in this diff can be staged (unstaged) or unstaged
  /// (staged) against the index.
  public var supportsStaging: Bool {
    switch self {
    case .unstaged, .staged: return true
    default: return false
    }
  }

  /// Short label for pickers and headers.
  public var title: String {
    switch self {
    case .uncommitted: return "Uncommitted changes"
    case .unstaged: return "Unstaged"
    case .staged: return "Staged"
    case .branch(let base): return "vs \(base)"
    case .commit(let sha): return "Commit \(sha.prefix(7))"
    case .range(let from, let to): return "\(from.prefix(10))…\(to.prefix(10))"
    case .stash(let index): return "stash@{\(index)}"
    case .snapshot(let from, let to):
      if let reviewed = Self.checkpointDate(from, folder: "reviews") {
        let time = DateFormatter.localizedString(from: reviewed, dateStyle: .none, timeStyle: .short)
        return "Since your review at \(time)"
      }
      if let started = Self.checkpointDate(from) {
        let time = DateFormatter.localizedString(from: started, dateStyle: .none, timeStyle: .short)
        return to == nil ? "Agent turn since \(time)" : "Agent turn at \(time)"
      }
      return to == nil ? "Since \(Self.shortRef(from))" : "\(Self.shortRef(from))…\(Self.shortRef(to!))"
    }
  }

  /// When an agent-turn checkpoint ref (…/checkpoints/<id>/<millis>-…) was
  /// taken.
  static func checkpointDate(_ ref: String, folder: String = "checkpoints") -> Date? {
    guard ref.contains("/impulse/\(folder)/"),
      let millis = ref.split(separator: "/").last?.split(separator: "-").first.flatMap({ Double($0) })
    else { return nil }
    return Date(timeIntervalSince1970: millis / 1000)
  }

  private static func shortRef(_ ref: String) -> String {
    ref.split(separator: "/").last.map(String.init) ?? ref
  }
}

/// The kind of change to a file.
public enum ChangeStatus: String, Codable, Equatable, Sendable {
  case added
  case modified
  case deleted
  case renamed
  case typeChanged = "type_changed"
  case untracked
  case conflicted

  /// One-letter badge, git-status style.
  public var letter: String {
    switch self {
    case .added: return "A"
    case .modified: return "M"
    case .deleted: return "D"
    case .renamed: return "R"
    case .typeChanged: return "T"
    case .untracked: return "U"
    case .conflicted: return "C"
    }
  }
}

/// One changed file in a scope or status section.
public struct FileChange: Codable, Equatable, Hashable, Sendable {
  /// Repo-relative path (new side for renames).
  public let path: String
  /// Repo-relative previous path for renames/copies.
  public let oldPath: String?
  public let status: ChangeStatus
  /// Line counts; nil when not computed (binary, too large, or skipped for
  /// very large change sets).
  public let added: Int?
  public let removed: Int?
  public let isBinary: Bool

  public init(
    path: String, oldPath: String? = nil, status: ChangeStatus, added: Int? = nil,
    removed: Int? = nil, isBinary: Bool = false
  ) {
    self.path = path
    self.oldPath = oldPath
    self.status = status
    self.added = added
    self.removed = removed
    self.isBinary = isBinary
  }
}

/// An in-progress multi-step operation (git's "repository state").
public enum RepoOperation: Codable, Equatable, Sendable {
  case merge
  case rebase(step: Int?, total: Int?)
  case cherryPick
  case revert
  case bisect
  case applyMailbox

  public var title: String {
    switch self {
    case .merge: return "Merging"
    case .rebase(let step, let total):
      if let step, let total { return "Rebasing \(step)/\(total)" }
      return "Rebasing"
    case .cherryPick: return "Cherry-picking"
    case .revert: return "Reverting"
    case .bisect: return "Bisecting"
    case .applyMailbox: return "Applying patches"
    }
  }

  /// Whether `--skip` is meaningful.
  public var canSkip: Bool {
    switch self {
    case .rebase, .cherryPick, .revert, .applyMailbox: return true
    case .merge, .bisect: return false
    }
  }
}
