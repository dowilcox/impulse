import Foundation

// Ported from impulse-core/src/command_palette.rs.

public struct CommandPaletteItem: Codable, Hashable {
  public let id: String
  public let title: String
  public let category: String
  public let keywords: [String]
  public let source: String
  public let shortcut: String?
  public let payload: [String: String]?

  public init(
    id: String,
    title: String,
    category: String,
    keywords: [String] = [],
    source: String,
    shortcut: String? = nil,
    payload: [String: String]? = nil
  ) {
    self.id = id
    self.title = title
    self.category = category
    self.keywords = keywords
    self.source = source
    self.shortcut = shortcut
    self.payload = payload
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    id = try container.decode(String.self, forKey: .id)
    title = try container.decode(String.self, forKey: .title)
    category = try container.decode(String.self, forKey: .category)
    keywords = try container.decodeIfPresent([String].self, forKey: .keywords) ?? []
    source = try container.decode(String.self, forKey: .source)
    shortcut = try container.decodeIfPresent(String.self, forKey: .shortcut)
    payload = try container.decodeIfPresent([String: String].self, forKey: .payload)
  }
}

public struct RecentCommandItem: Codable, Hashable {
  public var id: String
  public var title: String
  public var lastUsedMs: UInt64
  public var useCount: UInt32

  public init(id: String, title: String, lastUsedMs: UInt64, useCount: UInt32) {
    self.id = id
    self.title = title
    self.lastUsedMs = lastUsedMs
    self.useCount = useCount
  }

  enum CodingKeys: String, CodingKey {
    case id
    case title
    case lastUsedMs = "last_used_ms"
    case useCount = "use_count"
  }
}

public struct RecentCommandStore: Codable, Hashable {
  public var items: [RecentCommandItem]

  public init(items: [RecentCommandItem] = []) {
    self.items = items
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    items = try container.decodeIfPresent([RecentCommandItem].self, forKey: .items) ?? []
  }

  enum CodingKeys: String, CodingKey {
    case items
  }

  public mutating func record(_ item: CommandPaletteItem, nowMs: UInt64, maxItems: Int) {
    if let index = items.firstIndex(where: { $0.id == item.id }) {
      items[index].title = item.title
      items[index].lastUsedMs = nowMs
      items[index].useCount = items[index].useCount &+ (items[index].useCount == .max ? 0 : 1)
    } else {
      items.append(
        RecentCommandItem(id: item.id, title: item.title, lastUsedMs: nowMs, useCount: 1))
    }
    items.sort { a, b in
      if a.lastUsedMs != b.lastUsedMs { return a.lastUsedMs > b.lastUsedMs }
      if a.useCount != b.useCount { return a.useCount > b.useCount }
      return a.title < b.title
    }
    if items.count > maxItems {
      items.removeLast(items.count - maxItems)
    }
  }

  public func score(id: String) -> Int64 {
    guard let index = items.firstIndex(where: { $0.id == id }) else { return 0 }
    let recent = items[index]
    return 10_000 - (Int64(index) * 250) + Int64(min(recent.useCount, 100))
  }
}

public enum CommandPalette {
  private struct BuiltinCommand {
    let id: String
    let title: String
    let category: String
    let keywords: [String]
  }

  private static let builtinCommands: [BuiltinCommand] = [
    BuiltinCommand(
      id: "new_tab", title: "New Terminal Tab", category: "Tabs", keywords: ["terminal", "shell"]),
    BuiltinCommand(id: "close_tab", title: "Close Tab", category: "Tabs", keywords: ["remove"]),
    BuiltinCommand(
      id: "reopen_tab", title: "Reopen Closed Tab", category: "Tabs", keywords: ["restore", "undo"]),
    BuiltinCommand(id: "next_tab", title: "Next Tab", category: "Tabs", keywords: ["navigate"]),
    BuiltinCommand(id: "prev_tab", title: "Previous Tab", category: "Tabs", keywords: ["navigate"]),
    BuiltinCommand(id: "copy", title: "Copy", category: "Terminal", keywords: ["clipboard"]),
    BuiltinCommand(id: "paste", title: "Paste", category: "Terminal", keywords: ["clipboard"]),
    BuiltinCommand(
      id: "review_changes", title: "Review Changes", category: "Navigation",
      keywords: ["git", "diff", "commit", "review"]),
    BuiltinCommand(id: "new_file", title: "New File", category: "Editor", keywords: ["editor"]),
    BuiltinCommand(id: "save", title: "Save File", category: "Editor", keywords: ["write"]),
    BuiltinCommand(id: "find", title: "Find", category: "Editor", keywords: ["search"]),
    BuiltinCommand(
      id: "go_to_line", title: "Go to Line", category: "Editor", keywords: ["jump", "navigate"]),
    BuiltinCommand(
      id: "toggle_markdown_preview", title: "Toggle Preview", category: "Editor",
      keywords: ["markdown", "preview"]),
    BuiltinCommand(
      id: "toggle_sidebar", title: "Toggle Sidebar", category: "Navigation", keywords: ["files"]),
    BuiltinCommand(
      id: "quick_open", title: "Quick Open File", category: "Navigation",
      keywords: ["file", "finder"]),
    BuiltinCommand(
      id: "project_search", title: "Find in Project", category: "Navigation",
      keywords: ["search", "files"]),
    BuiltinCommand(
      id: "command_palette", title: "Command Palette", category: "Navigation",
      keywords: ["commands"]),
    BuiltinCommand(
      id: "open_settings", title: "Open Settings", category: "Navigation",
      keywords: ["preferences"]),
    BuiltinCommand(
      id: "font_increase", title: "Increase Font Size", category: "Font", keywords: ["zoom"]),
    BuiltinCommand(
      id: "font_decrease", title: "Decrease Font Size", category: "Font", keywords: ["zoom"]),
    BuiltinCommand(
      id: "font_reset", title: "Reset Font Size", category: "Font", keywords: ["zoom"]),
    BuiltinCommand(id: "new_window", title: "New Window", category: "App", keywords: ["window"]),
    BuiltinCommand(
      id: "fullscreen", title: "Toggle Fullscreen", category: "App", keywords: ["window"]),
    BuiltinCommand(
      id: "install_lsp", title: "Install Web LSP Servers", category: "Language Servers",
      keywords: ["typescript", "php", "html", "css"]),
  ]

  public static func builtinItems() -> [CommandPaletteItem] {
    builtinCommands.map { command in
      CommandPaletteItem(
        id: command.id,
        title: command.title,
        category: command.category,
        keywords: command.keywords,
        source: "builtin"
      )
    }
  }

  public static func customCommandItem(
    name: String, shortcut: String?, command: String, args: [String]
  ) -> CommandPaletteItem {
    let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
    let title = trimmedName.isEmpty ? command : trimmedName
    let trimmedShortcut = shortcut?.trimmingCharacters(in: .whitespacesAndNewlines)
    return CommandPaletteItem(
      id: customCommandId(command: command, args: args),
      title: title,
      category: "Custom",
      keywords: [command],
      source: "custom",
      shortcut: (trimmedShortcut?.isEmpty ?? true) ? nil : trimmedShortcut
    )
  }

  public static func customCommandId(command: String, args: [String]) -> String {
    var value = command.trimmingCharacters(in: .whitespacesAndNewlines)
    value.append("\0")
    for arg in args {
      value.append(arg)
      value.append("\0")
    }
    return String(format: "custom:external:%016llx", stableHash(Array(value.utf8)))
  }

  public static func filterItems(
    _ items: [CommandPaletteItem], recents: RecentCommandStore, query: String
  ) -> [CommandPaletteItem] {
    let terms = query.split(whereSeparator: { $0.isWhitespace })
      .map { $0.lowercased() }
      .filter { !$0.isEmpty }

    var seenIds = Set<String>()
    var scored: [(score: Int64, index: Int, item: CommandPaletteItem)] = []
    for (index, item) in items.enumerated() {
      guard seenIds.insert(item.id).inserted else { continue }
      guard let queryScore = scoreQuery(item: item, terms: terms) else { continue }
      scored.append((queryScore + recents.score(id: item.id), index, item))
    }

    scored.sort { a, b in
      if a.score != b.score { return a.score > b.score }
      if a.index != b.index { return a.index < b.index }
      return a.item.title < b.item.title
    }
    return scored.map(\.item)
  }

  /// Builds palette items from raw search results, mirroring the Rust
  /// `search_result_items` (stable FNV-based ids, payload with path/line/column).
  public static func searchResultItems(root: String, results: [SearchResult])
    -> [CommandPaletteItem]
  {
    results.map { searchResultItem(root: root, result: $0) }
  }

  private static func searchResultItem(root: String, result: SearchResult) -> CommandPaletteItem {
    let relativePath = relativeDisplayPath(root: root, path: result.path)
    let isContent = result.matchType == "content"
    let kind = isContent ? "content" : "file"

    var idMaterial = kind
    idMaterial.append("\0")
    idMaterial.append(result.path)
    if let line = result.lineNumber {
      idMaterial.append("\0")
      idMaterial.append(String(line))
    }
    if let column = result.columnStart {
      idMaterial.append("\0")
      idMaterial.append(String(column))
    }

    var payload: [String: String] = ["kind": kind, "path": result.path]
    if let line = result.lineNumber {
      payload["line"] = String(line)
    }
    if let column = result.columnStart {
      payload["column"] = String(column)
    }

    var keywords = [result.name, relativePath, result.path]
    if let lineContent = result.lineContent {
      keywords.append(lineContent)
    }

    let title: String
    if isContent, let line = result.lineNumber {
      title = "\(relativePath):\(line)"
    } else {
      title = relativePath
    }

    return CommandPaletteItem(
      id: String(format: "%@:%016llx", kind, stableHash(Array(idMaterial.utf8))),
      title: title,
      category: isContent ? "Project Search" : "Files",
      keywords: keywords,
      source: "dynamic",
      payload: payload
    )
  }

  private static func scoreQuery(item: CommandPaletteItem, terms: [String]) -> Int64? {
    if terms.isEmpty { return 0 }

    let title = item.title.lowercased()
    let category = item.category.lowercased()
    let keywords = item.keywords.map { $0.lowercased() }

    var score: Int64 = 0
    for term in terms {
      if title == term {
        score += 2_000
      } else if title.hasPrefix(term) {
        score += 1_500
      } else if title.contains(term) {
        score += 1_000
      } else if category.contains(term) {
        score += 500
      } else if keywords.contains(where: { $0.contains(term) }) {
        score += 250
      } else {
        return nil
      }
    }
    return score
  }

  /// FNV-1a, matching the Rust `stable_hash`.
  static func stableHash(_ bytes: [UInt8]) -> UInt64 {
    var hash: UInt64 = 0xcbf2_9ce4_8422_2325
    for byte in bytes {
      hash ^= UInt64(byte)
      hash = hash &* 0x1_0000_0001b3
    }
    return hash
  }

  static func relativeDisplayPath(root: String, path: String) -> String {
    let normalizedRoot = root.hasSuffix("/") ? String(root.dropLast()) : root
    if path.hasPrefix(normalizedRoot + "/") {
      let relative = String(path.dropFirst(normalizedRoot.count + 1))
      if !relative.isEmpty { return relative }
    }
    return path
  }
}
