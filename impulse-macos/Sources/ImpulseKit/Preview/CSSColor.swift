import Foundation

// Ported from impulse-editor/src/css.rs.

/// Sanitise a CSS color value. Accepts `#hex`, `rgb(…)`, `rgba(…)`.
/// Anything else is replaced by the fallback.
public func sanitizeCSSColor(_ value: String, fallback: String) -> String {
  let v = value.trimmingCharacters(in: .whitespacesAndNewlines)
  // Hex: #abc, #aabbcc, #aabbccdd
  if v.hasPrefix("#"),
    v.count == 4 || v.count == 7 || v.count == 9,
    v.dropFirst().allSatisfy({ $0.isHexDigit && $0.isASCII })
  {
    return v
  }
  // rgb(…) / rgba(…)
  if (v.hasPrefix("rgb(") || v.hasPrefix("rgba(")) && v.hasSuffix(")") {
    let openIndex = v.firstIndex(of: "(")!
    let inner = v[v.index(after: openIndex)..<v.index(before: v.endIndex)]
    if inner.allSatisfy({ ($0.isNumber && $0.isASCII) || ", .%".contains($0) }) {
      return v
    }
  }
  return fallback
}
