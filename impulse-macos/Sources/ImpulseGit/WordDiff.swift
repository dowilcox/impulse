// Port of `assign_word_spans` / `word_spans` from impulse-core/src/git.rs.
//
// The Rust code uses the `similar` crate's `TextDiff::from_words` (Myers diff
// over alternating whitespace/non-whitespace token runs) and records the
// changed UTF-16 ranges on each side, merging adjacent spans. This file
// replicates that: the same tokenization (Unicode `White_Space` runs, matching
// Rust's `char::is_whitespace`), a Myers diff over the tokens, and the same
// span-merging rule.

import Foundation

enum WordDiff {
  /// Compute intra-line word-diff spans for paired removed/added lines within
  /// a hunk. A maximal run of consecutive removed lines immediately followed
  /// by added lines is paired index-for-index.
  static func assignWordSpans(_ lines: inout [DiffLine]) {
    var i = 0
    while i < lines.count {
      if lines[i].kind != .removed {
        i += 1
        continue
      }
      let removedStart = i
      while i < lines.count, lines[i].kind == .removed { i += 1 }
      let removedEnd = i
      let addedStart = i
      while i < lines.count, lines[i].kind == .added { i += 1 }
      let addedEnd = i

      let pairs = min(removedEnd - removedStart, addedEnd - addedStart)
      for k in 0..<pairs {
        let oldIndex = removedStart + k
        let newIndex = addedStart + k
        let oldContent = lines[oldIndex].content
        let newContent = lines[newIndex].content
        if oldContent.utf16.count + newContent.utf16.count > maxWordDiffLineLen {
          continue
        }
        let (oldSpans, newSpans) = wordSpans(old: oldContent, new: newContent)
        lines[oldIndex].spans = oldSpans
        lines[newIndex].spans = newSpans
      }
    }
  }

  /// Word-level diff of two lines, returning the changed UTF-16 ranges on the
  /// old and new side respectively.
  static func wordSpans(old: String, new: String) -> ([WordSpan], [WordSpan]) {
    let oldTokens = tokenize(old)
    let newTokens = tokenize(new)
    let (oldChanged, newChanged) = changedFlags(oldTokens, newTokens)
    return (spans(tokens: oldTokens, changed: oldChanged),
            spans(tokens: newTokens, changed: newChanged))
  }

  /// Split into alternating runs of whitespace / non-whitespace scalars, using
  /// the Unicode `White_Space` property (Rust's `char::is_whitespace`).
  static func tokenize(_ text: String) -> [String] {
    var tokens: [String] = []
    var current = String.UnicodeScalarView()
    var currentIsWhitespace: Bool?
    for scalar in text.unicodeScalars {
      let isWhitespace = scalar.properties.isWhitespace
      if currentIsWhitespace != nil, currentIsWhitespace != isWhitespace {
        tokens.append(String(current))
        current = String.UnicodeScalarView()
      }
      current.append(scalar)
      currentIsWhitespace = isWhitespace
    }
    if currentIsWhitespace != nil {
      tokens.append(String(current))
    }
    return tokens
  }

  /// Merge changed tokens into UTF-16 spans, matching the Rust `push` closure
  /// (adjacent spans coalesce when the previous one ends where the next starts).
  static func spans(tokens: [String], changed: [Bool]) -> [WordSpan] {
    var result: [WordSpan] = []
    var offset: UInt32 = 0
    for (token, isChanged) in zip(tokens, changed) {
      let length = UInt32(token.utf16.count)
      if isChanged, length > 0 {
        if let last = result.last, last.end == offset {
          result[result.count - 1] = WordSpan(start: last.start, end: offset + length)
        } else {
          result.append(WordSpan(start: offset, end: offset + length))
        }
      }
      offset += length
    }
    return result
  }

  /// Myers diff (greedy O(ND) with backtracking) over two token sequences.
  /// Returns per-token changed flags: `true` = deleted (old) / inserted (new).
  static func changedFlags(_ old: [String], _ new: [String]) -> ([Bool], [Bool]) {
    let n = old.count
    let m = new.count
    var oldChanged = [Bool](repeating: false, count: n)
    var newChanged = [Bool](repeating: false, count: m)
    if n == 0 || m == 0 {
      for i in 0..<n { oldChanged[i] = true }
      for j in 0..<m { newChanged[j] = true }
      return (oldChanged, newChanged)
    }

    let maxD = n + m
    let offset = maxD
    var v = [Int](repeating: 0, count: 2 * maxD + 1)
    var trace: [[Int]] = []
    var endD = 0

    outer: for d in 0...maxD {
      trace.append(v)
      var k = -d
      while k <= d {
        var x: Int
        if k == -d || (k != d && v[offset + k - 1] < v[offset + k + 1]) {
          x = v[offset + k + 1]
        } else {
          x = v[offset + k - 1] + 1
        }
        var y = x - k
        while x < n, y < m, old[x] == new[y] {
          x += 1
          y += 1
        }
        v[offset + k] = x
        if x >= n, y >= m {
          endD = d
          break outer
        }
        k += 2
      }
    }

    // Backtrack from (n, m) to (0, 0), marking deletions and insertions.
    var x = n
    var y = m
    var d = endD
    while d > 0 {
      let previous = trace[d]
      let k = x - y
      let previousK: Int
      if k == -d || (k != d && previous[offset + k - 1] < previous[offset + k + 1]) {
        previousK = k + 1
      } else {
        previousK = k - 1
      }
      let previousX = previous[offset + previousK]
      let previousY = previousX - previousK
      // Walk back through the snake (equal tokens).
      while x > previousX, y > previousY {
        x -= 1
        y -= 1
      }
      if previousK == k + 1 {
        // Downward move: insertion of new[previousY].
        y -= 1
        newChanged[y] = true
      } else {
        // Rightward move: deletion of old[previousX].
        x -= 1
        oldChanged[x] = true
      }
      d -= 1
    }
    return (oldChanged, newChanged)
  }
}
