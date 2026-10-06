import AppKit
import Foundation
import JavaScriptCore

/// Syntax coloring for native code views (the review): highlight.js running
/// in a JavaScriptCore context, no web view. Results are token kinds per
/// line; colors come from the theme at draw time.
final class SyntaxHighlighter: @unchecked Sendable {
  static let shared = SyntaxHighlighter()

  enum Token: UInt8, Sendable {
    case keyword, string, number, comment, function, type, variable, constant, operatorToken
    case attribute, tag, regexp, escape, link, delimiter
  }

  /// A colored range within one line (UTF-16 offsets).
  struct Span: Equatable, Sendable {
    let location: Int
    let length: Int
    let token: Token
  }

  private let queue = DispatchQueue(label: "impulse.syntax-highlighter", qos: .userInitiated)
  private var context: JSContext?
  private var loadFailed = false
  /// Where highlight.min.js lives; nil means the app bundle's copy (tests
  /// point it at vendor/).
  var scriptURL: URL?

  /// highlight.js's name for a diff's language id (or the path's extension),
  /// nil when it has no grammar for it.
  static func hljsLanguage(_ language: String, path: String) -> String? {
    let byId: [String: String] = [
      "bash": "bash", "shellscript": "bash", "sh": "bash", "zsh": "bash", "fish": "bash",
      "c": "c", "cpp": "cpp", "csharp": "csharp", "css": "css", "less": "less", "scss": "scss",
      "go": "go", "graphql": "graphql", "html": "xml", "xml": "xml", "vue": "xml", "svelte": "xml",
      "java": "java", "javascript": "javascript", "javascriptreact": "javascript",
      "typescript": "typescript", "typescriptreact": "typescript", "json": "json", "jsonc": "json",
      "kotlin": "kotlin", "lua": "lua", "makefile": "makefile", "markdown": "markdown",
      "objective-c": "objectivec", "perl": "perl", "php": "php", "python": "python", "r": "r",
      "ruby": "ruby", "rust": "rust", "sql": "sql", "swift": "swift", "yaml": "yaml", "ini": "ini",
      "diff": "diff",
    ]
    if let name = byId[language.lowercased()] { return name }
    let byExtension: [String: String] = [
      "swift": "swift", "kt": "kotlin", "kts": "kotlin", "m": "objectivec", "mm": "objectivec",
      "sql": "sql", "r": "r", "pl": "perl", "pm": "perl", "toml": "ini", "ini": "ini", "cfg": "ini",
      "conf": "ini", "xml": "xml", "plist": "xml", "svg": "xml", "md": "markdown", "markdown": "markdown",
      "cs": "csharp", "vb": "vbnet", "diff": "diff", "patch": "diff", "wat": "wasm",
      "dockerfile": "bash", "env": "bash",
    ]
    let name = (path as NSString).lastPathComponent.lowercased()
    if name == "dockerfile" || name == "makefile" { return name == "makefile" ? "makefile" : "bash" }
    return byExtension[(path as NSString).pathExtension.lowercased()]
  }

  /// Token spans for each of `lines`, highlighted as one text (so strings and
  /// comments spanning lines come out right). nil without a grammar or when
  /// highlight.js isn't available. Call off the main thread.
  func highlight(lines: [String], language: String) -> [[Span]]? {
    queue.sync { () -> [[Span]]? in
      guard let context = loadContext() else { return nil }
      let code = lines.joined(separator: "\n")
      guard let function = context.objectForKeyedSubscript("__impulseHighlight"),
        let html = function.call(withArguments: [code, language])?.toString(),
        html != "undefined"
      else { return nil }
      return Self.parse(html: html, lineCount: lines.count)
    }
  }

  private func loadContext() -> JSContext? {
    if let context { return context }
    guard !loadFailed,
      let url = scriptURL ?? EditorAssets.monacoDirectory?.appendingPathComponent("highlight/highlight.min.js"),
      let script = try? String(contentsOf: url, encoding: .utf8),
      let context = JSContext()
    else {
      loadFailed = true
      return nil
    }
    context.exceptionHandler = { _, exception in
      NSLog("SyntaxHighlighter: %@", exception?.toString() ?? "unknown error")
    }
    context.evaluateScript(script)
    context.evaluateScript(
      """
      function __impulseHighlight(code, language) {
        try {
          if (!hljs.getLanguage(language)) return undefined;
          return hljs.highlight(code, { language: language, ignoreIllegals: true }).value;
        } catch (e) { return undefined; }
      }
      """)
    guard context.objectForKeyedSubscript("hljs")?.isUndefined == false else {
      loadFailed = true
      return nil
    }
    self.context = context
    return context
  }

  // MARK: Parsing highlight.js output

  /// highlight.js HTML (nested `<span class="hljs-…">`, escaped text) → spans
  /// per line. A span crossing a line break is split at it.
  static func parse(html: String, lineCount: Int) -> [[Span]] {
    var result = [[Span]](repeating: [], count: max(lineCount, 1))
    var stack: [Token?] = []
    var line = 0
    var column = 0  // UTF-16 offset within the line
    var runStart = 0
    var runToken: Token?

    func flush() {
      if let token = runToken, column > runStart, line < result.count {
        result[line].append(Span(location: runStart, length: column - runStart, token: token))
      }
      runStart = column
    }
    func setToken(_ token: Token?) {
      if token != runToken {
        flush()
        runToken = token
      }
    }

    let scalars = Array(html.unicodeScalars)
    var i = 0
    while i < scalars.count {
      let c = scalars[i]
      if c == "<" {
        // Tag: <span class="…"> or </span>.
        var j = i + 1
        while j < scalars.count, scalars[j] != ">" { j += 1 }
        let tag = String(String.UnicodeScalarView(scalars[(i + 1)..<min(j, scalars.count)]))
        if tag.hasPrefix("/") {
          if !stack.isEmpty { stack.removeLast() }
        } else if tag.hasPrefix("span") {
          let token = token(forTag: tag) ?? stack.last ?? nil
          stack.append(token)
        }
        setToken(stack.last ?? nil)
        i = j + 1
        continue
      }
      var text: Character
      if c == "&" {
        var j = i + 1
        while j < scalars.count, j - i < 10, scalars[j] != ";" { j += 1 }
        let entity = String(String.UnicodeScalarView(scalars[(i + 1)..<min(j, scalars.count)]))
        text = decode(entity: entity) ?? "&"
        i = (j < scalars.count && scalars[j] == ";" && decode(entity: entity) != nil) ? j + 1 : i + 1
      } else {
        text = Character(c)
        i += 1
      }
      if text == "\n" {
        flush()
        line += 1
        column = 0
        runStart = 0
        continue
      }
      column += text.utf16.count
    }
    flush()
    return Array(result.prefix(lineCount))
  }

  private static func decode(entity: String) -> Character? {
    switch entity {
    case "amp": return "&"
    case "lt": return "<"
    case "gt": return ">"
    case "quot": return "\""
    case "#x27", "#39", "apos": return "'"
    default:
      if entity.hasPrefix("#x"), let value = UInt32(entity.dropFirst(2), radix: 16), let s = Unicode.Scalar(value) {
        return Character(s)
      }
      if entity.hasPrefix("#"), let value = UInt32(entity.dropFirst()), let s = Unicode.Scalar(value) {
        return Character(s)
      }
      return nil
    }
  }

  /// The token for `span class="hljs-title function_"` and friends.
  private static func token(forTag tag: String) -> Token? {
    guard let start = tag.range(of: "class=\"") else { return nil }
    let rest = tag[start.upperBound...]
    let classes = rest.prefix { $0 != "\"" }.split(separator: " ").map(String.init)
    guard let first = classes.first else { return nil }
    let modifiers = Set(classes.dropFirst())
    switch first {
    case "hljs-keyword", "hljs-meta", "hljs-bullet", "hljs-selector-pseudo": return .keyword
    case "hljs-built_in", "hljs-type": return .type
    case "hljs-title":
      return modifiers.contains("class_") || modifiers.contains("class") ? .type : .function
    case "hljs-string", "hljs-char", "hljs-template-tag", "hljs-template-string": return .string
    case "hljs-number": return .number
    case "hljs-literal", "hljs-symbol", "hljs-selector-id", "hljs-selector-class": return .constant
    case "hljs-comment", "hljs-quote", "hljs-doctag": return .comment
    case "hljs-variable", "hljs-template-variable", "hljs-params", "hljs-subst": return .variable
    case "hljs-attr", "hljs-attribute", "hljs-property", "hljs-selector-attr": return .attribute
    case "hljs-tag", "hljs-name", "hljs-selector-tag": return .tag
    case "hljs-regexp": return .regexp
    case "hljs-operator": return .operatorToken
    case "hljs-punctuation": return .delimiter
    case "hljs-link": return .link
    case "hljs-section": return .function
    case "hljs-addition", "hljs-deletion", "hljs-emphasis", "hljs-strong": return nil
    default: return nil
    }
  }
}

extension Theme {
  /// The color for a syntax token in native code views.
  func syntaxColor(_ token: SyntaxHighlighter.Token) -> NSColor {
    let hex: String
    switch token {
    case .keyword: hex = syntaxKeyword
    case .string: hex = syntaxString
    case .number: hex = syntaxNumber
    case .comment: hex = syntaxComment
    case .function: hex = syntaxFunction
    case .type: hex = syntaxType
    case .variable: hex = syntaxVariable
    case .constant: hex = syntaxConstant
    case .operatorToken: hex = syntaxOperator
    case .attribute: hex = syntaxAttribute
    case .tag: hex = syntaxTag
    case .regexp: hex = syntaxRegexp
    case .escape: hex = syntaxEscape
    case .link: hex = syntaxLink
    case .delimiter: hex = syntaxDelimiter
    }
    return NSColor(hex: hex)
  }
}
