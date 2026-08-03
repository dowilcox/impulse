// GitClient — public entry points of the Swift git layer, ported from
// impulse-core/src/git.rs (plus the git-status functions of filesystem.rs).
//
// Behavior parity with the Rust implementation is verified against the golden
// fixtures in Tests/ImpulseGitTests/Fixtures.

import Clibgit2
import Foundation

/// Error carrying the raw git error text, mirroring the Rust
/// `Result<T, String>` convention used throughout impulse-core.
public struct GitError: Error, Equatable, CustomStringConvertible {
  public let message: String

  public init(_ message: String) {
    self.message = message
  }

  public var description: String { message }
}

/// Namespace for the ported git API. All operations are stateless; the only
/// shared state is the repo-root LRU cache (`RepoCache`).
public enum GitClient {}

// MARK: - Size / complexity guards (ported constants from git.rs)

/// Maximum file/blob size (bytes) for which we read full diff contents.
let maxDiffContentSize: UInt64 = 1_048_576

/// Maximum single-line length (bytes) before a file is treated as too complex
/// to diff inline.
let maxDiffLineLength = 20_000

/// Maximum number of hunks emitted per file before the diff is marked truncated.
let maxDiffHunks = 1_500

/// Maximum number of diff lines emitted per file before the diff is marked
/// truncated.
let maxDiffTotalLines = 30_000

/// Skip intra-line word-diffing when the combined old+new line length (UTF-16
/// units) exceeds this.
let maxWordDiffLineLen = 2_000
