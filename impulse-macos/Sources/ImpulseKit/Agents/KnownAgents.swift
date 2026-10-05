// Recognizing coding agents (Claude Code, Codex, …) from the process a
// terminal is running. Many are scripts, so the interpreter's arguments are
// checked too.

import Foundation

public struct AgentKind: Equatable, Hashable, Codable, Sendable {
  /// Stable id ("claude", "codex", …).
  public let id: String
  public let displayName: String
  /// Executable or script basenames that identify it.
  public let names: [String]
  /// Path fragments of its installed package (for `node …/cli.js` and the
  /// like).
  public let packages: [String]

  public init(id: String, displayName: String, names: [String], packages: [String] = []) {
    self.id = id
    self.displayName = displayName
    self.names = names
    self.packages = packages
  }
}

public enum KnownAgents {
  public static let builtIn: [AgentKind] = [
    AgentKind(
      id: "claude", displayName: "Claude Code", names: ["claude"],
      packages: ["@anthropic-ai/claude-code"]),
    AgentKind(id: "codex", displayName: "Codex", names: ["codex"], packages: ["@openai/codex"]),
    AgentKind(
      id: "gemini", displayName: "Gemini CLI", names: ["gemini"], packages: ["@google/gemini-cli"]),
    AgentKind(id: "aider", displayName: "Aider", names: ["aider"], packages: ["aider-chat", "/aider/"]),
    AgentKind(id: "opencode", displayName: "opencode", names: ["opencode"], packages: ["opencode-ai"]),
    AgentKind(id: "amp", displayName: "Amp", names: ["amp"], packages: ["@sourcegraph/amp"]),
    AgentKind(id: "copilot", displayName: "Copilot CLI", names: ["copilot"], packages: ["@github/copilot"]),
    AgentKind(id: "cursor", displayName: "Cursor Agent", names: ["cursor-agent"]),
    AgentKind(id: "goose", displayName: "Goose", names: ["goose"]),
    AgentKind(id: "qwen", displayName: "Qwen Code", names: ["qwen"], packages: ["@qwen-code/qwen-code"]),
    AgentKind(id: "crush", displayName: "Crush", names: ["crush"]),
  ]

  /// Interpreters whose script argument names the real program.
  static let interpreters: Set<String> = [
    "node", "bun", "deno", "python", "python3", "ruby", "uv", "uvx", "npx", "pnpm", "bunx",
    // Wrapper scripts (e.g. ~/.claude/local/claude) run under a shell.
    "sh", "bash", "zsh", "dash",
  ]

  /// The agent a process is, given its executable path and argv (argv[0]
  /// included). `extra` extends the built-in table (user settings).
  public static func match(
    executablePath: String, arguments: [String], extra: [AgentKind] = []
  ) -> AgentKind? {
    let kinds = extra + builtIn
    let executable = basename(executablePath)
    var candidates = [executable]
    if let first = arguments.first { candidates.append(basename(first)) }
    let isInterpreter =
      interpreters.contains(executable) || executable.hasPrefix("python")
      || arguments.first.map { interpreters.contains(basename($0)) } == true
    if isInterpreter {
      // The first argument that isn't an option is the script.
      if let script = arguments.dropFirst().first(where: { !$0.hasPrefix("-") }) {
        candidates.append(basename(script))
        if let kind = kinds.first(where: { kind in kind.packages.contains { script.contains($0) } }) {
          return kind
        }
      }
    }
    for candidate in candidates {
      let name = stripExtension(candidate)
      if let kind = kinds.first(where: { $0.names.contains(name) }) { return kind }
    }
    // A package path anywhere in the executable (e.g. a node_modules shim).
    return kinds.first { kind in kind.packages.contains { executablePath.contains($0) } }
  }

  private static func basename(_ path: String) -> String {
    (path as NSString).lastPathComponent
  }

  private static func stripExtension(_ name: String) -> String {
    for ext in [".js", ".mjs", ".cjs", ".py", ".exe"] where name.hasSuffix(ext) {
      return String(name.dropLast(ext.count))
    }
    return name
  }
}
