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

  /// Startup files (file name → contents) for a zsh started with ZDOTDIR
  /// pointing at a folder holding them. Each reads the user's own file from
  /// where zsh would have — their ZDOTDIR (`userZdotdir`, from before
  /// Impulse set it) or home — with the user's ZDOTDIR in place while it
  /// runs, and `integration` loads after the user's .zshrc.
  ///
  /// A .zshenv (or .zprofile) that sets ZDOTDIR moves the rest of the
  /// user's files: Impulse notes the new folder and puts its own ZDOTDIR
  /// back so zsh keeps reading these files. The .zshrc gives ZDOTDIR back
  /// to the user for good, so .zlogin comes straight from their folder and
  /// programs started in the shell see their value.
  public static func zshStartupFiles(integration: String, userZdotdir: String?) -> [String: String] {
    // Files run at the top level, never inside a function: a `typeset` in
    // them would otherwise be local to it.
    let useUsers = """
      if (( ${+__impulse_user_zdotdir} )); then export ZDOTDIR=$__impulse_user_zdotdir; else unset ZDOTDIR; fi

      """
    let backToImpulse = """
      if (( ${+ZDOTDIR} )); then __impulse_user_zdotdir=$ZDOTDIR; else unset __impulse_user_zdotdir; fi
      export ZDOTDIR=$__impulse_zdotdir

      """
    func sourceUsers(_ name: String) -> String {
      """
      if [[ -f "${ZDOTDIR:-$HOME}/\(name)" ]]; then
          source "${ZDOTDIR:-$HOME}/\(name)"
      fi

      """
    }
    let initial =
      userZdotdir.map { "__impulse_user_zdotdir=\(singleQuoted($0))\n" } ?? "unset __impulse_user_zdotdir\n"
    // macOS's /etc/zshrc sets HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history while
    // ZDOTDIR is still Impulse's folder, which is deleted with the terminal:
    // history goes where it would have without Impulse.
    let userHistory = """
      if [[ -n $__impulse_zdotdir && $HISTFILE == "$__impulse_zdotdir"/* ]]; then
          HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history
      fi

      """
    return [
      ".zshenv": "__impulse_zdotdir=$ZDOTDIR\n" + initial + useUsers + sourceUsers(".zshenv") + backToImpulse,
      ".zprofile": useUsers + sourceUsers(".zprofile") + backToImpulse,
      ".zshrc": useUsers + userHistory + "unset __impulse_zdotdir __impulse_user_zdotdir\n" + sourceUsers(".zshrc")
        + integration,
    ]
  }

  private static func singleQuoted(_ value: String) -> String {
    "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
  }
}
