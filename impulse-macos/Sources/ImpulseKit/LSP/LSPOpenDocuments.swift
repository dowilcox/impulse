// Which documents the language servers have open, for every window at once.

import Foundation

/// Documents open with the language servers. The servers are shared by
/// every window, so a file shown in two windows is opened once, closed when
/// the last window lets go of it, and has one version sequence (a server
/// keeps one version per document).
public struct LSPOpenDocuments {
  private var documents: [String: (holders: Int, version: Int32)] = [:]

  public init() {}

  /// A window shows `uri`. True for the first one: send didOpen (version 1).
  public mutating func open(_ uri: String) -> Bool {
    if let document = documents[uri] {
      documents[uri] = (document.holders + 1, document.version)
      return false
    }
    documents[uri] = (1, 1)
    return true
  }

  /// A window stopped showing `uri`. True for the last one: send didClose.
  @discardableResult
  public mutating func close(_ uri: String) -> Bool {
    guard let document = documents[uri] else { return false }
    if document.holders > 1 {
      documents[uri] = (document.holders - 1, document.version)
      return false
    }
    documents.removeValue(forKey: uri)
    return true
  }

  /// The version for a change just made to `uri` (nil when it isn't open).
  public mutating func nextVersion(_ uri: String) -> Int32? {
    guard let document = documents[uri] else { return nil }
    documents[uri] = (document.holders, document.version + 1)
    return document.version + 1
  }

  /// The version the servers have been sent for `uri`.
  public func version(_ uri: String) -> Int32? {
    documents[uri]?.version
  }

  /// How many windows show `uri`.
  public func holders(_ uri: String) -> Int {
    documents[uri]?.holders ?? 0
  }
}
