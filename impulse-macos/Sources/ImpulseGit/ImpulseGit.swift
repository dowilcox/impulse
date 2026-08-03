// ImpulseGit — Swift port of the Rust git layer (impulse-core/src/git.rs and
// the git-status parts of filesystem.rs), backed by a vendored static libgit2
// built without network transports (scripts/build-libgit2.sh).
//
// Behavior parity is verified against the golden fixtures in
// Tests/ImpulseGitTests/Fixtures, generated from the Rust implementation.

import Clibgit2
import Foundation

/// One-time libgit2 global initialization.
enum LibGit2 {
  static let initialized: Bool = {
    git_libgit2_init() >= 0
  }()
}
