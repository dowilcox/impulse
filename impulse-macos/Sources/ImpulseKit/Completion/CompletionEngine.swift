// The completion menu for the terminal input: what can come next at the
// cursor — commands, subcommands, options, and argument values from specs
// (git branches, package scripts, make targets, ssh hosts), falling back to
// paths.

import Foundation

/// What the engine can't work out from the input alone.
public struct CompletionContext {
  public var cwd: String?
  public var home: String
  public var gitBranches: () -> [String]
  public var gitRemotes: () -> [String]
  public var gitTags: () -> [String]

  public init(
    cwd: String?, home: String = NSHomeDirectory(), gitBranches: @escaping () -> [String] = { [] },
    gitRemotes: @escaping () -> [String] = { [] }, gitTags: @escaping () -> [String] = { [] }
  ) {
    self.cwd = cwd
    self.home = home
    self.gitBranches = gitBranches
    self.gitRemotes = gitRemotes
    self.gitTags = gitTags
  }
}

public enum CompletionEngine {
  /// Candidates for the token at the end of `input`. Paths keep their
  /// `"path"` kind; everything else is `"command"`, `"subcommand"`,
  /// `"option"` or the generator's kind (`"branch"`, `"script"`, …), with a
  /// description in `detail`.
  public static func candidates(input: String, context: CompletionContext, limit: Int = 50) -> CompletionResult {
    let parsed = parseShellInput(input, cursor: input.utf8.count)
    let completion = parsed.completion
    let span = completion.span
    if parsed.incomplete { return CompletionResult(span: span, candidates: []) }
    let prefix = completion.prefix

    switch completion.kind {
    case .command:
      guard !prefix.isEmpty else { return CompletionResult(span: span, candidates: []) }
      return CompletionResult(span: span, candidates: Array(commands(prefix).prefix(limit)))
    case .redirectTarget:
      return paths(input: input, context: context, limit: limit)
    case .envAssignment:
      return CompletionResult(span: span, candidates: [])
    case .argument:
      break
    }

    guard let command = completion.command,
      let spec = CompletionSpecs.spec(for: InputCompletion.commandBasename(command))
    else { return paths(input: input, context: context, limit: limit) }

    // Walk the words before the cursor through the spec.
    let segment = argumentsBefore(span: span, in: parsed.tokens)
    var current = spec
    var positional = 0
    var pendingOption: CompletionOption?
    for word in segment {
      if let option = pendingOption {
        _ = option
        pendingOption = nil
        continue
      }
      if word.hasPrefix("-") {
        if let option = current.option(named: word), option.argument != nil, !word.contains("=") {
          pendingOption = option
        }
        continue
      }
      if positional == 0, let sub = current.subcommands.first(where: { $0.name == word }) {
        current = sub
        continue
      }
      positional += 1
    }

    var out: [CompletionCandidate] = []
    if let option = pendingOption, let generator = option.argument {
      out = values(generator, prefix: prefix, context: context, input: input, limit: limit)
      if case .paths = generator { return paths(input: input, context: context, limit: limit) }
      if case .directories = generator { return paths(input: input, context: context, limit: limit, directoriesOnly: true) }
      return CompletionResult(span: span, candidates: Array(out.prefix(limit)))
    }
    if prefix.hasPrefix("-") {
      for option in current.options {
        for name in option.names where name.hasPrefix(prefix) && name != prefix {
          out.append(
            CompletionCandidate(value: name, display: name, kind: "option", isDir: false, gitStatus: nil, detail: option.description))
        }
      }
      return CompletionResult(span: span, candidates: Array(out.prefix(limit)))
    }
    if positional == 0 {
      for sub in current.subcommands where sub.name.hasPrefix(prefix) && !sub.name.hasPrefix("-") {
        out.append(
          CompletionCandidate(
            value: sub.name, display: sub.name, kind: "subcommand", isDir: false, gitStatus: nil,
            detail: sub.description.isEmpty ? nil : sub.description))
      }
    }
    let generator = current.generator(at: positional)
    switch generator {
    case .paths?:
      return CompletionResult(span: span, candidates: out + paths(input: input, context: context, limit: limit).candidates)
    case .directories?:
      return CompletionResult(
        span: span, candidates: out + paths(input: input, context: context, limit: limit, directoriesOnly: true).candidates)
    case let generator?:
      out += values(generator, prefix: prefix, context: context, input: input, limit: limit)
    case nil:
      // No subcommand matched and nothing is known about the arguments:
      // paths are the safest guess.
      if out.isEmpty && current.subcommands.isEmpty {
        return paths(input: input, context: context, limit: limit)
      }
    }
    return CompletionResult(span: span, candidates: Array(out.prefix(limit)))
  }

  // MARK: Pieces

  /// Words of the current command (after any pipe or `&&`) before the token
  /// being completed, without the command itself.
  static func argumentsBefore(span: TextSpan, in tokens: [ShellToken]) -> [String] {
    var words: [String] = []
    for token in tokens where token.span.start < span.start {
      switch token.role {
      case .pipelineSeparator, .controlOperator: words = []
      case .command: words = []
      case .argument: words.append(token.text)
      default: break
      }
    }
    return words
  }

  static func commands(_ prefix: String) -> [CompletionCandidate] {
    var seen = Set<String>()
    var out: [CompletionCandidate] = []
    func add(_ name: String, _ detail: String?) {
      guard name.hasPrefix(prefix), seen.insert(name).inserted else { return }
      out.append(CompletionCandidate(value: name, display: name, kind: "command", isDir: false, gitStatus: nil, detail: detail))
    }
    for spec in CompletionSpecs.all { add(spec.name, spec.description) }
    for name in InputCompletion.commonCommands { add(name, nil) }
    for name in InputCompletion.pathExecutables { add(name, nil) }
    // Spec'd and common commands first (in that order), then the rest by length.
    let ranked = out.enumerated().sorted { a, b in
      let aKnown = a.element.detail != nil || InputCompletion.commonCommands.contains(a.element.value)
      let bKnown = b.element.detail != nil || InputCompletion.commonCommands.contains(b.element.value)
      if aKnown != bKnown { return aKnown }
      if aKnown { return a.offset < b.offset }
      return (a.element.value.count, a.element.value) < (b.element.value.count, b.element.value)
    }
    return ranked.map(\.element)
  }

  private static func paths(
    input: String, context: CompletionContext, limit: Int, directoriesOnly: Bool = false
  ) -> CompletionResult {
    var result = InputCompletion.completeCandidates(input: input, cwd: context.cwd, history: [], limit: limit)
    if directoriesOnly { result.candidates = result.candidates.filter(\.isDir) }
    return result
  }

  static func values(
    _ generator: CompletionGenerator, prefix: String, context: CompletionContext, input: String, limit: Int
  ) -> [CompletionCandidate] {
    func make(_ items: [(String, String?)], kind: String) -> [CompletionCandidate] {
      items.filter { $0.0.hasPrefix(prefix) && $0.0 != prefix }.prefix(limit).map {
        CompletionCandidate(value: $0.0, display: $0.0, kind: kind, isDir: false, gitStatus: nil, detail: $0.1)
      }
    }
    switch generator {
    case .paths, .directories:
      return []
    case .gitBranches:
      return make(context.gitBranches().map { ($0, nil) }, kind: "branch")
    case .gitRemotes:
      return make(context.gitRemotes().map { ($0, nil) }, kind: "remote")
    case .gitTags:
      return make(context.gitTags().map { ($0, nil) }, kind: "tag")
    case .gitRefs:
      return make(context.gitBranches().map { ($0, "branch") } + context.gitTags().map { ($0, "tag") }, kind: "branch")
    case .npmScripts:
      return make(CompletionSources.packageScripts(in: context.cwd), kind: "script")
    case .makeTargets:
      return make(CompletionSources.makeTargets(in: context.cwd).map { ($0, nil) }, kind: "target")
    case .justRecipes:
      return make(CompletionSources.justRecipes(in: context.cwd), kind: "recipe")
    case .sshHosts:
      return make(CompletionSources.sshHosts(home: context.home).map { ($0, nil) }, kind: "host")
    case .values(let list):
      return make(list.map { ($0, nil) }, kind: "value")
    }
  }
}

/// Values read from files in the project or home folder.
public enum CompletionSources {
  /// package.json scripts (name, command), nearest package.json up from `cwd`.
  public static func packageScripts(in cwd: String?) -> [(String, String?)] {
    guard let path = nearest("package.json", from: cwd),
      let data = FileManager.default.contents(atPath: path),
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let scripts = object["scripts"] as? [String: Any]
    else { return [] }
    return scripts.keys.sorted().map { ($0, scripts[$0] as? String) }
  }

  /// Makefile targets (not pattern rules or special targets), in file order.
  public static func makeTargets(in cwd: String?) -> [String] {
    guard let cwd else { return [] }
    for name in ["GNUmakefile", "makefile", "Makefile"] {
      let path = (cwd as NSString).appendingPathComponent(name)
      guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { continue }
      var targets: [String] = []
      for line in text.components(separatedBy: .newlines) {
        guard let first = line.first, !first.isWhitespace, first != "#", first != ".",
          let colon = line.firstIndex(of: ":")
        else { continue }
        // Not an assignment (`x := y`, `x ::= y`).
        let after = line[line.index(after: colon)...]
        if after.hasPrefix("=") || after.hasPrefix(":=") { continue }
        for target in line[..<colon].split(separator: " ") {
          let name = String(target)
          guard !name.contains("%"), !name.contains("$"), !name.contains("="), !targets.contains(name) else { continue }
          targets.append(name)
        }
      }
      return targets
    }
    return []
  }

  /// justfile recipes with the comment above each as its description.
  public static func justRecipes(in cwd: String?) -> [(String, String?)] {
    guard let path = ["justfile", "Justfile", ".justfile"].lazy.compactMap({ nearest($0, from: cwd) }).first,
      let text = try? String(contentsOfFile: path, encoding: .utf8)
    else { return [] }
    var recipes: [(String, String?)] = []
    var comment: String?
    for line in text.components(separatedBy: .newlines) {
      if line.hasPrefix("#") {
        comment = line.drop { $0 == "#" }.trimmingCharacters(in: .whitespaces)
        continue
      }
      defer { if !line.hasPrefix("#") { comment = nil } }
      guard let first = line.first, first.isLetter || first == "@" || first == "_",
        let colon = line.firstIndex(of: ":"), !line[line.index(after: colon)...].hasPrefix("=")
      else { continue }
      let head = line[..<colon].trimmingCharacters(in: .whitespaces)
      guard !head.hasPrefix("set "), !head.hasPrefix("alias "), !head.hasPrefix("export "), !head.contains(":=")
      else { continue }
      let name = head.split(separator: " ").first.map { String($0).trimmingCharacters(in: CharacterSet(charactersIn: "@")) } ?? ""
      if !name.isEmpty, !name.hasPrefix("_") { recipes.append((name, comment)) }
    }
    return recipes
  }

  /// `Host` names from ~/.ssh/config (wildcards left out).
  public static func sshHosts(home: String) -> [String] {
    let path = (home as NSString).appendingPathComponent(".ssh/config")
    guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return [] }
    var hosts: [String] = []
    for line in text.components(separatedBy: .newlines) {
      let words = line.split(whereSeparator: \.isWhitespace)
      guard words.first?.lowercased() == "host" else { continue }
      for word in words.dropFirst() where !word.contains("*") && !word.contains("?") && !word.hasPrefix("!") {
        let host = String(word)
        if !hosts.contains(host) { hosts.append(host) }
      }
    }
    return hosts
  }

  /// `name` in `cwd` or the nearest folder above it.
  static func nearest(_ name: String, from cwd: String?) -> String? {
    guard var dir = cwd, !dir.isEmpty else { return nil }
    while true {
      let path = (dir as NSString).appendingPathComponent(name)
      if FileManager.default.fileExists(atPath: path) { return path }
      let parent = (dir as NSString).deletingLastPathComponent
      if parent == dir || parent.isEmpty { return nil }
      dir = parent
    }
  }
}
