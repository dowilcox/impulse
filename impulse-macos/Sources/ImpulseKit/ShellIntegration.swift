import Foundation

/// Shell integration scripts emitting OSC 133 (command marks), OSC 7 (cwd),
/// and OSC 6973 (command text) escape sequences. The script files moved here
/// verbatim from impulse-core/src/shell_integration/.
public enum ShellIntegration {
  /// The integration script for the given shell type.
  public static func script(for shellType: ShellType) -> String {
    let name: String
    switch shellType {
    case .bash: name = "bash"
    case .zsh: name = "zsh"
    case .fish: name = "fish"
    }
    guard
      let url = Bundle.kitResources.url(
        forResource: name, withExtension: "sh", subdirectory: "Resources/ShellIntegration"),
      let script = try? String(contentsOf: url, encoding: .utf8)
    else {
      NSLog("Missing shell integration script for %@", name)
      return ""
    }
    return script
  }

  /// The integration script for a shell name or path (e.g. "zsh", "/bin/bash").
  public static func script(forShell shell: String) -> String {
    script(for: LoginShell.detectShellType(shell))
  }
}
