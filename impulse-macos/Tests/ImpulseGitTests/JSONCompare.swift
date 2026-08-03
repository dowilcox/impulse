// JSON normalization + structural comparison helpers for the parity tests.
// Objects compare order-insensitively; arrays compare in order. On mismatch,
// the first differing key path plus both values is reported.

import Foundation

enum JSONCompare {
  /// Encode a value with JSONEncoder and reparse it into a JSON object graph.
  static func jsonObject<T: Encodable>(_ value: T) throws -> Any {
    let data = try JSONEncoder().encode(value)
    return try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
  }

  /// Replace every occurrence of the repo root in strings (keys and values)
  /// with "$ROOT".
  static func normalize(_ value: Any, root: String) -> Any {
    if let string = value as? String {
      return string.replacingOccurrences(of: root, with: "$ROOT")
    }
    if let array = value as? [Any] {
      return array.map { normalize($0, root: root) }
    }
    if let dictionary = value as? [String: Any] {
      var out: [String: Any] = [:]
      for (key, inner) in dictionary {
        out[key.replacingOccurrences(of: root, with: "$ROOT")] = normalize(inner, root: root)
      }
      return out
    }
    return value
  }

  /// First difference between two JSON object graphs, or nil when equal.
  static func firstDifference(actual: Any, expected: Any, path: String = "$") -> String? {
    switch (actual, expected) {
    case (let actualDict as [String: Any], let expectedDict as [String: Any]):
      for key in Set(actualDict.keys).union(expectedDict.keys).sorted() {
        let keyPath = "\(path).\(key)"
        switch (actualDict[key], expectedDict[key]) {
        case (nil, let expectedValue?):
          return "\(keyPath): missing in actual (expected \(expectedValue))"
        case (let actualValue?, nil):
          return "\(keyPath): unexpected in actual (\(actualValue))"
        case (let actualValue?, let expectedValue?):
          if let difference = firstDifference(
            actual: actualValue, expected: expectedValue, path: keyPath)
          {
            return difference
          }
        default:
          break
        }
      }
      return nil
    case (let actualArray as [Any], let expectedArray as [Any]):
      if actualArray.count != expectedArray.count {
        return "\(path): array count \(actualArray.count) != \(expectedArray.count)"
      }
      for (index, pair) in zip(actualArray, expectedArray).enumerated() {
        if let difference = firstDifference(
          actual: pair.0, expected: pair.1, path: "\(path)[\(index)]")
        {
          return difference
        }
      }
      return nil
    default:
      let actualObject = actual as AnyObject
      let expectedObject = expected as AnyObject
      if !actualObject.isEqual(expectedObject) {
        return "\(path): actual \(actual) != expected \(expected)"
      }
      return nil
    }
  }
}
