// What a shell command line does with git, for agent hooks: which branches
// it merges, rebases onto or pulls (`git merge interia-upgrade`,
// `git -C ../x rebase origin/main && npm test`), and whether it moves HEAD.
// Read with the shell parser, so quoting and `&&` chains don't fool it.

import Foundation

public enum GitCommandInspector {
  /// One git invocation in a command line: its subcommand and arguments
  /// (global options like `-C dir` removed).
  public struct Invocation: Equatable, Sendable {
    public let subcommand: String
    public let arguments: [String]
  }

  /// The git invocations in `commandLine`, in order.
  public static func invocations(in commandLine: String) -> [Invocation] {
    let parsed = parseShellInput(commandLine, cursor: commandLine.utf16.count)
    var result: [Invocation] = []
    var words: [String] = []
    func finish() {
      defer { words = [] }
      guard let first = words.first, first == "git" || first.hasSuffix("/git") else { return }
      var rest = Array(words.dropFirst())
      // Global options before the subcommand.
      while let option = rest.first, option.hasPrefix("-") {
        rest.removeFirst()
        if ["-C", "-c", "--git-dir", "--work-tree", "--namespace"].contains(option), !rest.isEmpty { rest.removeFirst() }
      }
      guard let subcommand = rest.first else { return }
      result.append(Invocation(subcommand: subcommand, arguments: Array(rest.dropFirst())))
    }
    for token in parsed.tokens {
      switch token.role {
      case .command, .argument: words.append(token.text)
      case .controlOperator, .pipelineSeparator: finish()
      default: break
      }
    }
    finish()
    return result
  }

  /// The refs a command line merges into the current branch: `merge`'s
  /// commits, `rebase`'s upstream, `pull`'s branches (as `remote/branch`,
  /// or the branch itself when pulling from `.`).
  public static func mergedRefs(in commandLine: String) -> [String] {
    var refs: [String] = []
    for invocation in invocations(in: commandLine) {
      let positional = positionals(invocation.arguments, valued: valuedOptions[invocation.subcommand] ?? [])
      switch invocation.subcommand {
      case "merge":
        refs += positional
      case "rebase":
        if let onto = value(of: "--onto", in: invocation.arguments) { refs.append(onto) }
        if let upstream = positional.first { refs.append(upstream) }
      case "pull":
        guard let remote = positional.first else { continue }
        refs += positional.dropFirst().map { remote == "." ? $0 : "\(remote)/\($0)" }
      default:
        continue
      }
    }
    return refs
  }

  /// Whether the command line can move HEAD to another commit (so
  /// dependency files may have changed under the checkout).
  public static func movesHead(_ commandLine: String) -> Bool {
    invocations(in: commandLine).contains {
      ["merge", "pull", "rebase", "checkout", "switch", "reset", "cherry-pick", "revert"].contains($0.subcommand)
    }
  }

  /// Options that take a value, per subcommand, so the value isn't read as
  /// a branch.
  static let valuedOptions: [String: Set<String>] = [
    "merge": ["-m", "-F", "-s", "-X", "--strategy", "--strategy-option", "--file", "--into-name", "--cleanup"],
    "rebase": ["--onto", "-s", "-X", "--strategy", "--strategy-option", "-x", "--exec"],
    "pull": ["-s", "-X", "--strategy", "--strategy-option", "--depth", "--deepen", "--shallow-since", "-j", "--jobs"],
  ]

  static func positionals(_ arguments: [String], valued: Set<String>) -> [String] {
    var result: [String] = []
    var skipNext = false
    var optionsEnded = false
    for argument in arguments {
      if skipNext {
        skipNext = false
        continue
      }
      if !optionsEnded, argument == "--" {
        optionsEnded = true
        continue
      }
      if !optionsEnded, argument.hasPrefix("-") {
        if valued.contains(argument) { skipNext = true }
        continue
      }
      result.append(argument)
    }
    return result
  }

  static func value(of option: String, in arguments: [String]) -> String? {
    for (index, argument) in arguments.enumerated() {
      if argument == option, index + 1 < arguments.count { return arguments[index + 1] }
      if argument.hasPrefix(option + "=") { return String(argument.dropFirst(option.count + 1)) }
    }
    return nil
  }
}
