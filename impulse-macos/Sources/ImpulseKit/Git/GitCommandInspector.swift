// What a shell command line does with git, for agent hooks: which branches
// it merges, rebases onto or pulls (`git merge interia-upgrade`,
// `git -C ../x rebase origin/main && npm test`), and whether it moves HEAD.
// Read with the shell parser, so quoting and `&&` chains don't fool it, and
// neither do scripts: several lines, subshells and groups, `if`/`while`
// bodies, and wrappers such as `env`, `sudo` and `time`.

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
    let script = separatingCommands(commandLine)
    let parsed = parseShellInput(script, cursor: script.utf8.count)
    var result: [Invocation] = []
    var words: [String] = []
    func finish() {
      defer { words = [] }
      let command = words.dropFirst(leadingWords(words))
      guard let first = command.first, first == "git" || first.hasSuffix("/git") else { return }
      var rest = Array(command.dropFirst())
      // Global options before the subcommand.
      while let option = rest.first, option.hasPrefix("-") {
        rest.removeFirst()
        if ["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env"].contains(option), !rest.isEmpty {
          rest.removeFirst()
        }
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

  /// `commandLine` with each unquoted newline, parenthesis and backtick
  /// made a `;`, so every command in a script, subshell or command
  /// substitution ends up a command of its own for the parser, and line
  /// continuations (backslash, newline) made spaces.
  static func separatingCommands(_ commandLine: String) -> String {
    var scalars: [Unicode.Scalar] = []
    var quote: Unicode.Scalar?
    var escaped = false
    for scalar in commandLine.unicodeScalars {
      if escaped {
        escaped = false
        if scalar == "\n" {
          scalars[scalars.count - 1] = " "
          scalars.append(" ")
        } else {
          scalars.append(scalar)
        }
        continue
      }
      if let open = quote {
        if scalar == open {
          quote = nil
        } else if open == "\"", scalar == "\\" {
          escaped = true
        }
      } else if scalar == "\\" {
        escaped = true
      } else if scalar == "'" || scalar == "\"" {
        quote = scalar
      } else if ["\n", "(", ")", "`"].contains(scalar) {
        scalars.append(";")
        continue
      }
      scalars.append(scalar)
    }
    var result = String.UnicodeScalarView()
    result.append(contentsOf: scalars)
    return String(result)
  }

  /// How many of a command's words come before the program it runs: shell
  /// keywords (`if`, `then`, `do`, `!`, `{` …), assignments, and wrappers
  /// that run the rest as a command (`env A=b`, `command`, `time -p`,
  /// `sudo -u x`, `nice -n 5`, `nohup`) with their options.
  static func leadingWords(_ words: [String]) -> Int {
    var index = 0
    while index < words.count {
      let word = words[index]
      let name = word.split(separator: "/").last.map(String.init) ?? word
      if keywords.contains(word) || isAssignment(word) {
        index += 1
      } else if let valued = wrappers[name] {
        index += 1
        while index < words.count, words[index].hasPrefix("-") {
          let option = words[index]
          index += valued.contains(option) ? 2 : 1
          if option == "--" { break }
        }
      } else {
        break
      }
    }
    return min(index, words.count)
  }

  /// Words that start a command without being one.
  static let keywords: Set<String> = ["!", "{", "}", "if", "then", "else", "elif", "fi", "do", "done", "while", "until"]

  /// Commands that run the rest of the line as a command, with those of
  /// their options that take a value.
  static let wrappers: [String: Set<String>] = [
    "env": ["-u", "-C", "-P", "-S", "--unset", "--chdir"],
    "command": [],
    "time": [],
    "sudo": ["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T", "--user", "--group", "--chdir", "--prompt"],
    "nice": ["-n", "--adjustment"],
    "nohup": [],
    "exec": ["-a"],
  ]

  /// `NAME=value`, the way a shell recognizes an assignment.
  private static func isAssignment(_ word: String) -> Bool {
    guard let equals = word.firstIndex(of: "="), let first = word.first, first == "_" || (first.isASCII && first.isLetter)
    else { return false }
    return word[..<equals].allSatisfy { $0 == "_" || ($0.isASCII && ($0.isLetter || $0.isNumber)) }
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
