import Foundation

/// Fuzzy subsequence matching for the command palette and quick open.
///
/// A candidate matches when every query character appears in order. Scores
/// reward what people mean when they type a few letters:
/// - matches at word starts (after `/ _ - . space`, or a lower→upper camel
///   hump) and at the very start;
/// - runs of consecutive matched characters;
/// - for paths, matches inside the last component (the file name);
/// - shorter candidates (less noise) as a tiebreak.
///
/// Matching is case-insensitive unless the query contains an uppercase
/// letter ("smart case").
public enum FuzzyMatcher {
  public struct Match: Equatable, Sendable {
    public let score: Int
    /// UTF-16 offsets of the matched characters in the candidate, ascending.
    public let positions: [Int]

    public init(score: Int, positions: [Int]) {
      self.score = score
      self.positions = positions
    }
  }

  // Scoring weights.
  static let matchBase = 16
  static let boundaryBonus = 16
  static let startBonus = 30
  static let consecutiveBonus = 20
  static let fileNameBonus = 12
  static let gapPenalty = 1
  static let maxStarts = 12

  /// A query prepared once for matching many candidates.
  struct Query {
    /// Its UTF-16 code units, lowercased unless it's case-sensitive.
    let units: [UInt16]
    let caseSensitive: Bool

    init(_ query: String) {
      caseSensitive = query.contains { $0.isUppercase }
      units = caseSensitive ? Array(query.utf16) : query.utf16.map(FuzzyMatcher.lower)
    }
  }

  /// Score `candidate` against `query`, or nil when it doesn't match.
  /// `isPath` enables the file-name bonus for matches after the last `/`.
  public static func match(_ query: String, in candidate: String, isPath: Bool = false) -> Match? {
    match(Query(query), in: candidate, isPath: isPath)
  }

  static func match(_ query: Query, in candidate: String, isPath: Bool) -> Match? {
    let qn = query.units
    guard !qn.isEmpty else { return Match(score: 0, positions: []) }

    // Quick reject, before copying anything: the whole query must appear as
    // a subsequence (most candidates stop here).
    var qi = 0
    for unit in candidate.utf16 {
      if (query.caseSensitive ? unit : lower(unit)) == qn[qi] {
        qi += 1
        if qi == qn.count { break }
      }
    }
    guard qi == qn.count else { return nil }

    let c = Array(candidate.utf16)
    let cn = query.caseSensitive ? c : c.map(lower)

    let fileNameStart = isPath ? (c.lastIndex(of: 0x2F).map { $0 + 1 } ?? 0) : 0

    // Try greedy alignments starting at each occurrence of the first query
    // character (bounded) and keep the best — cheap and close to optimal for
    // palette-sized inputs.
    var best: Match?
    var starts = 0
    var start = 0
    while start < cn.count, starts < maxStarts {
      guard cn[start] == qn[0] else {
        start += 1
        continue
      }
      starts += 1
      if let m = align(
        qn, cn, original: c, from: start, fileNameStart: fileNameStart, isPath: isPath),
        best == nil || m.score > best!.score
      {
        best = m
      }
      start += 1
    }
    guard var result = best else { return nil }
    // Prefer shorter candidates when everything else is equal.
    result = Match(score: result.score - min(c.count / 8, 20), positions: result.positions)
    return result
  }

  private static func align(
    _ q: [UInt16], _ c: [UInt16], original: [UInt16], from start: Int, fileNameStart: Int,
    isPath: Bool
  ) -> Match? {
    var positions: [Int] = []
    positions.reserveCapacity(q.count)
    var score = 0
    var qi = 0
    var ci = start
    var previous = -2
    while qi < q.count, ci < c.count {
      if c[ci] == q[qi] {
        // Prefer a later word-boundary occurrence of this character when it
        // scores better than taking this one and the rest still fits after it
        // (e.g. "tb" → the "B" in "TabBar", not the "b").
        var pick = ci
        let here = stepScore(c, original, ci, previous, fileNameStart, isPath)
        if ci == 0 || !isBoundary(original, ci) {
          var j = ci + 1
          while j < c.count {
            if c[j] == q[qi], isBoundary(original, j),
              isSubsequence(q, from: qi + 1, of: c, after: j)
            {
              if stepScore(c, original, j, previous, fileNameStart, isPath) > here {
                pick = j
              }
              break
            }
            j += 1
          }
        }
        score += stepScore(c, original, pick, previous, fileNameStart, isPath)
        positions.append(pick)
        previous = pick
        qi += 1
        ci = pick + 1
        continue
      }
      ci += 1
    }
    guard qi == q.count else { return nil }
    return Match(score: score, positions: positions)
  }

  /// Score for matching the character at `i` given the previous match.
  private static func stepScore(
    _ c: [UInt16], _ original: [UInt16], _ i: Int, _ previous: Int, _ fileNameStart: Int,
    _ isPath: Bool
  ) -> Int {
    var s = matchBase
    if i == 0 {
      s += startBonus
    } else if isPath && i == fileNameStart {
      // First letter of the file name: a strong word start.
      s += boundaryBonus + 8
    } else if isBoundary(original, i) {
      s += boundaryBonus
    }
    if previous == i - 1 {
      s += consecutiveBonus
    } else if previous >= 0 {
      s -= min(i - previous - 1, 10) * gapPenalty
    }
    if isPath, i >= fileNameStart {
      s += fileNameBonus
    }
    return s
  }

  /// Whether q[qi...] is a subsequence of c[(after + 1)...].
  private static func isSubsequence(_ q: [UInt16], from qi: Int, of c: [UInt16], after: Int)
    -> Bool
  {
    var k = qi
    var j = after + 1
    while k < q.count, j < c.count {
      if c[j] == q[k] { k += 1 }
      j += 1
    }
    return k == q.count
  }

  private static func isBoundary(_ c: [UInt16], _ i: Int) -> Bool {
    let prev = c[i - 1]
    switch prev {
    case 0x2F, 0x5F, 0x2D, 0x2E, 0x20, 0x3A, 0x5C:  // / _ - . space : \
      return true
    default:
      // camelCase hump: lowercase followed by uppercase.
      return isLowerASCII(prev) && isUpperASCII(c[i])
    }
  }

  private static func lower(_ u: UInt16) -> UInt16 {
    isUpperASCII(u) ? u + 32 : u
  }

  private static func isUpperASCII(_ u: UInt16) -> Bool { u >= 0x41 && u <= 0x5A }
  private static func isLowerASCII(_ u: UInt16) -> Bool { u >= 0x61 && u <= 0x7A }

  /// Rank `items` by fuzzy score against `query` (best first), keeping at most
  /// `limit`. Items that don't match are dropped; an empty query keeps the
  /// input order.
  public static func rank<T>(
    _ items: [T], query: String, limit: Int = .max, isPath: Bool = false,
    text: (T) -> String
  ) -> [(item: T, match: Match)] {
    let trimmed = query.trimmingCharacters(in: .whitespaces)
    guard !trimmed.isEmpty else {
      return items.prefix(limit).map { ($0, Match(score: 0, positions: [])) }
    }
    return best(matches(items, query: trimmed, isPath: isPath, text: text) ?? [], limit: limit)
  }

  /// Every item that matches `query` (trimmed), in input order, or nil when
  /// `shouldStop` returned true (asked every few hundred items, so a
  /// superseded search over a big list ends early). An empty query matches
  /// everything. With `best`, this is `rank` in two steps: the matches of a
  /// query are all a longer query that starts with it can match, so they
  /// can be searched again instead of the whole list.
  public static func matches<T>(
    _ items: [T], query: String, isPath: Bool = false, shouldStop: () -> Bool = { false },
    text: (T) -> String
  ) -> [(item: T, match: Match)]? {
    let prepared = Query(query.trimmingCharacters(in: .whitespaces))
    var found: [(item: T, match: Match)] = []
    found.reserveCapacity(min(items.count, 4096))
    for (order, item) in items.enumerated() {
      if order % 512 == 0, shouldStop() { return nil }
      if let match = match(prepared, in: text(item), isPath: isPath) {
        found.append((item, match))
      }
    }
    return found
  }

  /// The `limit` best of `matches` (in input order), best first; equal
  /// scores keep their input order.
  public static func best<T>(_ matches: [(item: T, match: Match)], limit: Int = .max)
    -> [(item: T, match: Match)]
  {
    var scored = matches.enumerated().map { (order: $0.offset, entry: $0.element) }
    scored.sort {
      $0.entry.match.score != $1.entry.match.score
        ? $0.entry.match.score > $1.entry.match.score : $0.order < $1.order
    }
    return scored.prefix(limit).map(\.entry)
  }
}
