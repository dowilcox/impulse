// The symbols in a file, from LSP textDocument/documentSymbol — either the
// hierarchical DocumentSymbol[] or the flat SymbolInformation[] — flattened
// in document order with their nesting, for the palette's @ mode and the
// outline.

import Foundation

public struct OutlineSymbol: Equatable, Sendable {
  public let name: String
  public let detail: String?
  /// LSP SymbolKind (1 File … 26 TypeParameter).
  public let kind: Int
  /// 1-based position of the name.
  public let line: Int
  public let column: Int
  /// 0 for top level.
  public let depth: Int
  /// Enclosing symbol names, outermost first.
  public let container: [String]

  public init(
    name: String, detail: String? = nil, kind: Int, line: Int, column: Int, depth: Int = 0,
    container: [String] = []
  ) {
    self.name = name
    self.detail = detail
    self.kind = kind
    self.line = line
    self.column = column
    self.depth = depth
    self.container = container
  }

  public var kindName: String { DocumentSymbols.kindName(kind) }
}

public enum DocumentSymbols {
  public static func parse(_ json: Data) -> [OutlineSymbol] {
    guard let array = try? JSONSerialization.jsonObject(with: json) as? [[String: Any]] else { return [] }
    var out: [OutlineSymbol] = []
    if array.first?["location"] != nil {
      // SymbolInformation: flat, with an optional container name.
      for item in array {
        guard let name = item["name"] as? String, let kind = item["kind"] as? Int,
          let location = item["location"] as? [String: Any], let (line, column) = start(location["range"])
        else { continue }
        let container = (item["containerName"] as? String).map { [$0] } ?? []
        out.append(
          OutlineSymbol(
            name: name, kind: kind, line: line, column: column, depth: container.count, container: container))
      }
      return out.sorted { ($0.line, $0.column) < ($1.line, $1.column) }
    }
    func walk(_ items: [[String: Any]], container: [String]) {
      let ordered = items.sorted { (start($0["range"]) ?? (0, 0)) < (start($1["range"]) ?? (0, 0)) }
      for item in ordered {
        guard let name = item["name"] as? String, let kind = item["kind"] as? Int,
          let (line, column) = start(item["selectionRange"]) ?? start(item["range"])
        else { continue }
        out.append(
          OutlineSymbol(
            name: name, detail: (item["detail"] as? String).flatMap { $0.isEmpty ? nil : $0 }, kind: kind,
            line: line, column: column, depth: container.count, container: container))
        if let children = item["children"] as? [[String: Any]], !children.isEmpty {
          walk(children, container: container + [name])
        }
      }
    }
    walk(array, container: [])
    return out
  }

  /// `workspace/symbol` results (SymbolInformation or WorkspaceSymbol):
  /// each symbol with the file it's in.
  public static func parseWorkspace(_ json: Data) -> [(symbol: OutlineSymbol, path: String)] {
    guard let array = try? JSONSerialization.jsonObject(with: json) as? [[String: Any]] else { return [] }
    return array.compactMap { item in
      guard let name = item["name"] as? String, let kind = item["kind"] as? Int,
        let location = item["location"] as? [String: Any], let uri = location["uri"] as? String,
        let url = URL(string: uri), url.isFileURL
      else { return nil }
      // WorkspaceSymbol may leave the range out until resolved: line 1.
      let (line, column) = start(location["range"]) ?? (1, 1)
      let container = (item["containerName"] as? String).flatMap { $0.isEmpty ? nil : [$0] } ?? []
      return (OutlineSymbol(name: name, kind: kind, line: line, column: column, container: container), url.path)
    }
  }

  /// A range's start as 1-based (line, column).
  private static func start(_ range: Any?) -> (Int, Int)? {
    guard let range = range as? [String: Any], let start = range["start"] as? [String: Any],
      let line = (start["line"] as? NSNumber)?.intValue, let character = (start["character"] as? NSNumber)?.intValue
    else { return nil }
    return (line + 1, character + 1)
  }

  public static func kindName(_ kind: Int) -> String {
    let names = [
      "file", "module", "namespace", "package", "class", "method", "property", "field", "constructor", "enum",
      "interface", "function", "variable", "constant", "string", "number", "boolean", "array", "object", "key",
      "null", "enum member", "struct", "event", "operator", "type parameter",
    ]
    return (1...names.count).contains(kind) ? names[kind - 1] : "symbol"
  }
}
