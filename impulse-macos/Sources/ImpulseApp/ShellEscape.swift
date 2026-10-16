import Foundation
import ImpulseKit

// MARK: - Shell Escaping

extension String {

    /// This string as one word for a POSIX shell (sh, bash, zsh): as is when
    /// it needs no quoting, otherwise in single quotes with internal single
    /// quotes written `'\''`. Text typed into a terminal uses
    /// `shellEscaped(for:)` with that terminal's shell.
    var shellEscaped: String {
        shellEscaped(for: .bash)
    }

    /// This string as one word for `shell` (fish quotes differently).
    /// Control characters other than tab and newline are dropped.
    func shellEscaped(for shell: ShellType) -> String {
        ShellQuoting.quoted(self, for: shell)
    }
}
