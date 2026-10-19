// Setting or removing one member of an object in a JSONC file (JSON with
// comments and trailing commas, as opencode reads its config) without
// rewriting the rest: the user's comments, key order and formatting stay
// byte for byte. Text that isn't JSONC this can follow is refused rather
// than guessed at.

import Foundation

enum JSONCEdit {
  struct Malformed: Error {}

  /// An object member, by UTF-8 offsets into the text.
  struct Member {
    let key: String
    let keyStart: Int
    let valueStart: Int
    let valueEnd: Int
    /// The comma after the value (a separator, or a trailing comma).
    var comma: Int?
  }

  struct Object {
    let open: Int
    let close: Int
    var members: [Member]

    /// The last member named `key` (the one a JSON reader keeps).
    func member(_ key: String) -> Member? {
      members.last { $0.key == key }
    }
  }

  // MARK: Reading

  /// The top-level object of `bytes`.
  static func root(_ bytes: [UInt8]) throws -> Object {
    var start = 0
    if bytes.starts(with: [0xEF, 0xBB, 0xBF]) { start = 3 }
    let open = try skip(bytes, start)
    guard open < bytes.count, bytes[open] == UInt8(ascii: "{") else { throw Malformed() }
    let object = try object(bytes, open)
    guard try skip(bytes, object.close + 1) == bytes.count else { throw Malformed() }
    return object
  }

  /// The object that starts at `open`.
  static func object(_ bytes: [UInt8], _ open: Int) throws -> Object {
    var members: [Member] = []
    var index = try skip(bytes, open + 1)
    while true {
      guard index < bytes.count else { throw Malformed() }
      if bytes[index] == UInt8(ascii: "}") { return Object(open: open, close: index, members: members) }
      guard bytes[index] == UInt8(ascii: "\"") else { throw Malformed() }
      let keyEnd = try stringEnd(bytes, index)
      guard
        let key = try? JSONSerialization.jsonObject(with: Data(bytes[index..<keyEnd]), options: .fragmentsAllowed)
          as? String
      else { throw Malformed() }
      let colon = try skip(bytes, keyEnd)
      guard colon < bytes.count, bytes[colon] == UInt8(ascii: ":") else { throw Malformed() }
      let valueStart = try skip(bytes, colon + 1)
      let valueEnd = try self.valueEnd(bytes, valueStart)
      var member = Member(key: key, keyStart: index, valueStart: valueStart, valueEnd: valueEnd)
      let next = try skip(bytes, valueEnd)
      guard next < bytes.count else { throw Malformed() }
      if bytes[next] == UInt8(ascii: ",") {
        member.comma = next
        members.append(member)
        index = try skip(bytes, next + 1)
      } else {
        guard bytes[next] == UInt8(ascii: "}") else { throw Malformed() }
        members.append(member)
        return Object(open: open, close: next, members: members)
      }
    }
  }

  /// Where the value that starts at `start` ends.
  static func valueEnd(_ bytes: [UInt8], _ start: Int) throws -> Int {
    guard start < bytes.count else { throw Malformed() }
    switch bytes[start] {
    case UInt8(ascii: "\""):
      return try stringEnd(bytes, start)
    case UInt8(ascii: "{"):
      return try object(bytes, start).close + 1
    case UInt8(ascii: "["):
      var index = try skip(bytes, start + 1)
      while true {
        guard index < bytes.count else { throw Malformed() }
        if bytes[index] == UInt8(ascii: "]") { return index + 1 }
        let next = try skip(bytes, try valueEnd(bytes, index))
        guard next < bytes.count else { throw Malformed() }
        if bytes[next] == UInt8(ascii: ",") {
          index = try skip(bytes, next + 1)
        } else {
          guard bytes[next] == UInt8(ascii: "]") else { throw Malformed() }
          return next + 1
        }
      }
    default:
      // A number, true, false or null.
      var end = start
      while end < bytes.count, isBareCharacter(bytes[end]) { end += 1 }
      guard end > start else { throw Malformed() }
      return end
    }
  }

  private static func isBareCharacter(_ byte: UInt8) -> Bool {
    (byte >= UInt8(ascii: "a") && byte <= UInt8(ascii: "z")) || (byte >= UInt8(ascii: "0") && byte <= UInt8(ascii: "9"))
      || [UInt8(ascii: "-"), UInt8(ascii: "+"), UInt8(ascii: "."), UInt8(ascii: "E")].contains(byte)
  }

  private static func stringEnd(_ bytes: [UInt8], _ start: Int) throws -> Int {
    var index = start + 1
    while index < bytes.count {
      switch bytes[index] {
      case UInt8(ascii: "\\"): index += 2
      case UInt8(ascii: "\""): return index + 1
      case 0x0A: throw Malformed()
      default: index += 1
      }
    }
    throw Malformed()
  }

  /// The first offset from `index` that isn't whitespace or a comment.
  static func skip(_ bytes: [UInt8], _ index: Int) throws -> Int {
    var index = index
    while index < bytes.count {
      switch bytes[index] {
      case 0x20, 0x09, 0x0A, 0x0D:
        index += 1
      case UInt8(ascii: "/") where index + 1 < bytes.count && bytes[index + 1] == UInt8(ascii: "/"):
        while index < bytes.count, bytes[index] != 0x0A { index += 1 }
      case UInt8(ascii: "/") where index + 1 < bytes.count && bytes[index + 1] == UInt8(ascii: "*"):
        var end = index + 2
        while end + 1 < bytes.count, !(bytes[end] == UInt8(ascii: "*") && bytes[end + 1] == UInt8(ascii: "/")) {
          end += 1
        }
        guard end + 1 < bytes.count else { throw Malformed() }
        index = end + 2
      default:
        return index
      }
    }
    return index
  }

  /// The value at `start..<end` as Foundation objects (comments and
  /// trailing commas dropped).
  static func value(_ bytes: [UInt8], _ start: Int, _ end: Int) -> Any? {
    var plain: [UInt8] = []
    var index = start
    while index < end {
      let byte = bytes[index]
      if byte == UInt8(ascii: "\""), let close = try? stringEnd(bytes, index) {
        plain += bytes[index..<close]
        index = close
      } else if byte == UInt8(ascii: "/"), let next = try? skip(bytes, index), next > index {
        plain.append(0x20)
        index = next
      } else if byte == UInt8(ascii: ","), let next = try? skip(bytes, index + 1), next < end,
        [UInt8(ascii: "}"), UInt8(ascii: "]")].contains(bytes[next])
      {
        index = next
      } else {
        plain.append(byte)
        index += 1
      }
    }
    return try? JSONSerialization.jsonObject(with: Data(plain), options: .fragmentsAllowed)
  }

  // MARK: Writing

  /// One member as JSONC text: `"key": value`, the value's lines indented
  /// from `indent` by `unit`. Values are strings, string arrays (on one
  /// line) and objects with their keys in order.
  static func format(_ key: String, _ value: Any, indent: String, unit: String) -> String {
    "\(quoted(key)): \(format(value, indent: indent, unit: unit))"
  }

  static func format(_ value: Any, indent: String, unit: String) -> String {
    switch value {
    case let members as KeyValuePairs<String, Any>:
      let inner = indent + unit
      let lines = members.map { inner + format($0.key, $0.value, indent: inner, unit: unit) }
      return "{\n" + lines.joined(separator: ",\n") + "\n" + indent + "}"
    case let strings as [String]:
      return "[" + strings.map(quoted).joined(separator: ", ") + "]"
    case let string as String:
      return quoted(string)
    default:
      return "null"
    }
  }

  private static func quoted(_ string: String) -> String {
    (try? JSONSerialization.data(withJSONObject: string, options: [.fragmentsAllowed, .withoutEscapingSlashes]))
      .map { String(decoding: $0, as: UTF8.self) } ?? "\"\""
  }

  /// The file's indent step: the first indented line's leading whitespace
  /// (two spaces when nothing is indented yet).
  static func indentUnit(_ bytes: [UInt8]) -> String {
    var index = 0
    while index < bytes.count {
      var end = index
      while end < bytes.count, bytes[end] == 0x20 || bytes[end] == 0x09 { end += 1 }
      if end > index, end < bytes.count, bytes[end] == UInt8(ascii: "\"") {
        return String(decoding: bytes[index..<end], as: UTF8.self)
      }
      while end < bytes.count, bytes[end] != 0x0A { end += 1 }
      index = end + 1
    }
    return "  "
  }

  /// The leading whitespace of the line holding `offset`, and whether only
  /// whitespace comes before `offset` on it.
  static func lineIndent(_ bytes: [UInt8], _ offset: Int) -> (indent: String, first: Bool) {
    var start = offset
    while start > 0, bytes[start - 1] != 0x0A { start -= 1 }
    var end = start
    while end < offset, bytes[end] == 0x20 || bytes[end] == 0x09 { end += 1 }
    return (String(decoding: bytes[start..<end], as: UTF8.self), end == offset)
  }

  /// The indent for a member of `object`: its members', else one step in
  /// from the line it opens on.
  static func memberIndent(_ bytes: [UInt8], _ object: Object, unit: String) -> String {
    if let last = object.members.last {
      let line = lineIndent(bytes, last.keyStart)
      if line.first { return line.indent }
    }
    return lineIndent(bytes, object.open).indent + unit
  }

  /// A change: replace `range` with `text`. Apply from the end backwards.
  typealias Change = (range: Range<Int>, text: String)

  static func applying(_ changes: [Change], to bytes: [UInt8]) -> [UInt8] {
    var result = bytes
    for change in changes.sorted(by: { $0.range.lowerBound > $1.range.lowerBound }) {
      result.replaceSubrange(change.range, with: Array(change.text.utf8))
    }
    return result
  }

  /// Add `member` (formatted with `memberText(indent)`) as the last member
  /// of `object`.
  static func inserting(
    into object: Object, _ bytes: [UInt8], unit: String, memberText: (String) -> String
  ) -> [Change] {
    let indent = memberIndent(bytes, object, unit: unit)
    guard let last = object.members.last else {
      let closeIndent = lineIndent(bytes, object.open).indent
      let inside = object.open + 1..<object.close
      if bytes[inside].allSatisfy({ [0x20, 0x09, 0x0A, 0x0D].contains($0) }) {
        return [(inside, "\n" + indent + memberText(indent) + "\n" + closeIndent)]
      }
      // Only comments inside: they stay after the new member.
      return [(inside.lowerBound..<inside.lowerBound, "\n" + indent + memberText(indent))]
    }
    // After the last thing before the closing brace (a comment, perhaps),
    // keeping a trailing comma's style.
    var end = object.close
    while end > object.open + 1, [0x20, 0x09, 0x0A, 0x0D].contains(bytes[end - 1]) { end -= 1 }
    var changes: [Change] = [(end..<end, "\n" + indent + memberText(indent) + (last.comma == nil ? "" : ","))]
    if last.comma == nil { changes.append((last.valueEnd..<last.valueEnd, ",")) }
    return changes
  }

  /// Take `member` out of `object`: its lines when it has them to itself,
  /// with the comma that separated it.
  static func removing(_ member: Member, from object: Object, _ bytes: [UInt8]) -> [Change] {
    var start = member.keyStart
    var end = member.comma.map { $0 + 1 } ?? member.valueEnd
    let line = lineIndent(bytes, start)
    var rest = end
    while rest < bytes.count, bytes[rest] == 0x20 || bytes[rest] == 0x09 || bytes[rest] == 0x0D { rest += 1 }
    if line.first, rest < bytes.count, bytes[rest] == 0x0A {
      start -= line.indent.utf8.count
      end = rest + 1
    }
    var changes: [Change] = [(start..<end, "")]
    // The last member, without a trailing comma: the one before it loses
    // its separator.
    if member.comma == nil, let index = object.members.firstIndex(where: { $0.keyStart == member.keyStart }),
      index > 0, let comma = object.members[index - 1].comma
    {
      changes.append((comma..<comma + 1, ""))
    }
    return changes
  }
}
