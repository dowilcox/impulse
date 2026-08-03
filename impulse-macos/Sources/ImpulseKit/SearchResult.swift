import Foundation

// Mirrors the Rust `impulse_core::search::SearchResult` serialization
// (snake_case keys). Moved from the FFI bridge; the search implementation
// itself is ported in a later phase.
public struct SearchResult: Codable, Equatable {
  public let path: String
  public let name: String
  public let lineNumber: UInt32?
  public let lineContent: String?
  public let columnStart: UInt32?
  public let columnEnd: UInt32?
  public let matchType: String

  public init(
    path: String,
    name: String,
    lineNumber: UInt32? = nil,
    lineContent: String? = nil,
    columnStart: UInt32? = nil,
    columnEnd: UInt32? = nil,
    matchType: String
  ) {
    self.path = path
    self.name = name
    self.lineNumber = lineNumber
    self.lineContent = lineContent
    self.columnStart = columnStart
    self.columnEnd = columnEnd
    self.matchType = matchType
  }

  enum CodingKeys: String, CodingKey {
    case path
    case name
    case lineNumber = "line_number"
    case lineContent = "line_content"
    case columnStart = "column_start"
    case columnEnd = "column_end"
    case matchType = "match_type"
  }
}
