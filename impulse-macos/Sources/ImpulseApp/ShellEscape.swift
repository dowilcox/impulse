import Foundation

// MARK: - Shell Escaping

extension String {

    /// Characters considered safe for shell arguments (no quoting needed).
    private static let shellSafeChars = CharacterSet.alphanumerics
        .union(CharacterSet(charactersIn: "/_.-"))

    /// Returns a shell-escaped version of this string suitable for use in a
    /// POSIX shell command. If the string contains only safe characters it is
    /// returned as-is; otherwise it is wrapped in single quotes with internal
    /// single quotes escaped using the `'\''` idiom.
    var shellEscaped: String {
        if unicodeScalars.allSatisfy({ Self.shellSafeChars.contains($0) }) {
            return self
        }
        let escaped = replacingOccurrences(of: "'", with: "'\\''")
        return "'\(escaped)'"
    }
}
