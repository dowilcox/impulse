import Foundation

// Ported from impulse-core/src/shell_parser.rs.
//
// A hand-written shell tokenizer for the terminal input bar: quote states,
// token roles (assignment/command/argument/redirection), redirections, and
// cursor-position -> completion-kind classification.
//
// All offsets (spans, cursor) are UTF-8 BYTE offsets, matching the Rust
// implementation's `&str` indices, so multibyte inputs produce identical
// results. JSON encoding mirrors the Rust serde serialization (snake_case
// enum values and field names).

public enum ShellQuoteState: String, Codable, Equatable, Sendable {
  case none
  case single
  case double
}

public enum ShellTokenRole: String, Codable, Equatable, Sendable {
  case assignment
  case command
  case argument
  case redirectionOperator = "redirection_operator"
  case redirectionTarget = "redirection_target"
  case pipelineSeparator = "pipeline_separator"
  case controlOperator = "control_operator"
}

public struct ShellToken: Codable, Equatable, Sendable {
  public var text: String
  public var span: TextSpan
  public var role: ShellTokenRole
  public var quoteState: ShellQuoteState
  public var terminated: Bool
  public var quoted: Bool

  public init(
    text: String, span: TextSpan, role: ShellTokenRole, quoteState: ShellQuoteState,
    terminated: Bool, quoted: Bool
  ) {
    self.text = text
    self.span = span
    self.role = role
    self.quoteState = quoteState
    self.terminated = terminated
    self.quoted = quoted
  }

  enum CodingKeys: String, CodingKey {
    case text, span, role, terminated, quoted
    case quoteState = "quote_state"
  }
}

public enum ShellCompletionKind: String, Codable, Equatable, Sendable {
  case command
  case argument
  case envAssignment = "env_assignment"
  case redirectTarget = "redirect_target"
}

public struct ShellCompletion: Codable, Equatable, Sendable {
  public var kind: ShellCompletionKind
  public var prefix: String
  public var span: TextSpan
  public var quoteState: ShellQuoteState
  public var command: String?
  public var commandSpan: TextSpan?
  public var argumentIndex: Int
  public var previousWord: String?

  public init(
    kind: ShellCompletionKind, prefix: String, span: TextSpan, quoteState: ShellQuoteState,
    command: String?, commandSpan: TextSpan?, argumentIndex: Int, previousWord: String?
  ) {
    self.kind = kind
    self.prefix = prefix
    self.span = span
    self.quoteState = quoteState
    self.command = command
    self.commandSpan = commandSpan
    self.argumentIndex = argumentIndex
    self.previousWord = previousWord
  }

  enum CodingKeys: String, CodingKey {
    case kind, prefix, span, command
    case quoteState = "quote_state"
    case commandSpan = "command_span"
    case argumentIndex = "argument_index"
    case previousWord = "previous_word"
  }
}

public struct ShellRedirection: Codable, Equatable, Sendable {
  public var operatorText: String
  public var operatorSpan: TextSpan
  public var target: String?
  public var targetSpan: TextSpan?

  public init(operatorText: String, operatorSpan: TextSpan, target: String?, targetSpan: TextSpan?)
  {
    self.operatorText = operatorText
    self.operatorSpan = operatorSpan
    self.target = target
    self.targetSpan = targetSpan
  }

  enum CodingKeys: String, CodingKey {
    case target
    case operatorText = "operator"
    case operatorSpan = "operator_span"
    case targetSpan = "target_span"
  }
}

public struct ShellParseResult: Codable, Equatable, Sendable {
  public var input: String
  public var cursor: Int
  public var tokens: [ShellToken]
  public var assignments: [ShellToken]
  public var redirects: [ShellRedirection]
  public var completion: ShellCompletion
  public var pipelineIndex: Int
  public var incomplete: Bool

  public init(
    input: String, cursor: Int, tokens: [ShellToken], assignments: [ShellToken],
    redirects: [ShellRedirection], completion: ShellCompletion, pipelineIndex: Int,
    incomplete: Bool
  ) {
    self.input = input
    self.cursor = cursor
    self.tokens = tokens
    self.assignments = assignments
    self.redirects = redirects
    self.completion = completion
    self.pipelineIndex = pipelineIndex
    self.incomplete = incomplete
  }

  enum CodingKeys: String, CodingKey {
    case input, cursor, tokens, assignments, redirects, completion, incomplete
    case pipelineIndex = "pipeline_index"
  }
}

/// Parse `input` up to `cursor` (a UTF-8 byte offset, clamped to the nearest
/// character boundary at or below it, like the Rust `clamp_cursor`).
public func parseShellInput(_ input: String, cursor: Int) -> ShellParseResult {
  let inputBytes = Array(input.utf8)
  let cursor = clampCursor(inputBytes, cursor)
  let prefix = Array(inputBytes[..<cursor])
  var (tokens, quoteState, escapedAtEnd) = lexPrefix(prefix)
  classifyTokens(&tokens)

  let currentIndex = tokens.firstIndex { tokenContainsCursor($0, cursor) }
  let segmentStart = currentSegmentStart(tokens)
  let pipelineStart = currentPipelineStart(tokens, segmentStart)
  let pipelineIndex = tokens[pipelineStart..<segmentStart]
    .filter { $0.role == .pipelineSeparator }
    .count

  let assignments = tokens[segmentStart...].filter { $0.role == .assignment }
  let redirects = collectRedirects(Array(tokens[segmentStart...]))
  let completion = buildCompletion(
    tokens: tokens, currentIndex: currentIndex, segmentStart: segmentStart, cursor: cursor,
    quoteState: quoteState)

  return ShellParseResult(
    input: input,
    cursor: cursor,
    tokens: tokens,
    assignments: assignments,
    redirects: redirects,
    completion: completion,
    pipelineIndex: pipelineIndex,
    incomplete: escapedAtEnd || quoteState != .none)
}

private func clampCursor(_ input: [UInt8], _ cursor: Int) -> Int {
  var cursor = min(cursor, input.count)
  while cursor > 0 && !isCharBoundary(input, cursor) {
    cursor -= 1
  }
  return cursor
}

/// Mirrors Rust `str::is_char_boundary`: true at the ends and wherever the
/// byte is not a UTF-8 continuation byte.
private func isCharBoundary(_ input: [UInt8], _ index: Int) -> Bool {
  if index == 0 || index == input.count { return true }
  return input[index] & 0xC0 != 0x80
}

/// Decode the Unicode scalar starting at byte `pos`. Input comes from a Swift
/// `String`, so the bytes are valid UTF-8; the fallbacks are belt-and-braces.
private func decodeScalar(_ bytes: [UInt8], at pos: Int) -> (Unicode.Scalar, Int) {
  let b0 = bytes[pos]
  if b0 < 0x80 {
    return (Unicode.Scalar(b0), 1)
  }
  if b0 & 0xE0 == 0xC0, pos + 1 < bytes.count {
    let value = (UInt32(b0 & 0x1F) << 6) | UInt32(bytes[pos + 1] & 0x3F)
    return (Unicode.Scalar(value) ?? "\u{FFFD}", 2)
  }
  if b0 & 0xF0 == 0xE0, pos + 2 < bytes.count {
    let value =
      (UInt32(b0 & 0x0F) << 12) | (UInt32(bytes[pos + 1] & 0x3F) << 6)
      | UInt32(bytes[pos + 2] & 0x3F)
    return (Unicode.Scalar(value) ?? "\u{FFFD}", 3)
  }
  if b0 & 0xF8 == 0xF0, pos + 3 < bytes.count {
    let value =
      (UInt32(b0 & 0x07) << 18) | (UInt32(bytes[pos + 1] & 0x3F) << 12)
      | (UInt32(bytes[pos + 2] & 0x3F) << 6) | UInt32(bytes[pos + 3] & 0x3F)
    return (Unicode.Scalar(value) ?? "\u{FFFD}", 4)
  }
  return (Unicode.Scalar(b0), 1)
}

private func lexPrefix(_ input: [UInt8]) -> ([ShellToken], ShellQuoteState, Bool) {
  var tokens: [ShellToken] = []
  var current: ShellToken? = nil
  var quoteState = ShellQuoteState.none
  var pos = 0
  var escaped = false
  var escapedAtEnd = false

  while pos < input.count {
    let (ch, chLen) = decodeScalar(input, at: pos)
    let nextPos = pos + chLen

    if escaped {
      appendLiteral(&current, start: pos, end: nextPos, scalar: ch)
      escaped = false
      pos = nextPos
      continue
    }

    switch quoteState {
    case .none:
      if ch.properties.isWhitespace {
        pushCurrent(&tokens, &current, terminated: true, quoteState: quoteState)
        pos = nextPos
        continue
      }

      if ch == "\\" {
        ensureToken(&current, start: pos)
        current!.span.end = nextPos
        if nextPos >= input.count {
          escapedAtEnd = true
        } else {
          escaped = true
        }
        pos = nextPos
        continue
      }

      if ch == "'" {
        ensureToken(&current, start: pos)
        current!.quoted = true
        current!.span.end = nextPos
        quoteState = .single
        pos = nextPos
        continue
      }

      if ch == "\"" {
        ensureToken(&current, start: pos)
        current!.quoted = true
        current!.span.end = nextPos
        quoteState = .double
        pos = nextPos
        continue
      }

      if let (op, end, role) = controlOperatorAt(input, pos) {
        pushCurrent(&tokens, &current, terminated: true, quoteState: quoteState)
        tokens.append(operatorToken(op, start: pos, end: end, role: role))
        pos = end
        continue
      }

      if let (op, end) = redirectOperatorAt(input, pos) {
        if let existing = current, !existing.quoted, isAsciiDigits(existing.text) {
          var token = existing
          current = nil
          token.text += op
          token.span.end = end
          token.role = .redirectionOperator
          token.terminated = true
          tokens.append(token)
        } else {
          pushCurrent(&tokens, &current, terminated: true, quoteState: quoteState)
          tokens.append(operatorToken(op, start: pos, end: end, role: .redirectionOperator))
        }
        pos = end
        continue
      }

      appendLiteral(&current, start: pos, end: nextPos, scalar: ch)

    case .single:
      if ch == "'" {
        current?.span.end = nextPos
        quoteState = .none
      } else {
        appendLiteral(&current, start: pos, end: nextPos, scalar: ch)
      }

    case .double:
      if ch == "\"" {
        current?.span.end = nextPos
        quoteState = .none
      } else if ch == "\\" {
        ensureToken(&current, start: pos)
        current!.span.end = nextPos
        if nextPos >= input.count {
          escapedAtEnd = true
        } else {
          escaped = true
        }
      } else {
        appendLiteral(&current, start: pos, end: nextPos, scalar: ch)
      }
    }

    pos = nextPos
  }

  pushCurrent(&tokens, &current, terminated: false, quoteState: quoteState)
  return (tokens, quoteState, escapedAtEnd)
}

private func ensureToken(_ current: inout ShellToken?, start: Int) {
  if current == nil {
    current = ShellToken(
      text: "",
      span: TextSpan(start: start, end: start),
      role: .argument,
      quoteState: .none,
      terminated: false,
      quoted: false)
  }
}

private func appendLiteral(
  _ current: inout ShellToken?, start: Int, end: Int, scalar: Unicode.Scalar
) {
  ensureToken(&current, start: start)
  current!.text.unicodeScalars.append(scalar)
  current!.span.end = end
}

private func pushCurrent(
  _ tokens: inout [ShellToken], _ current: inout ShellToken?, terminated: Bool,
  quoteState: ShellQuoteState
) {
  if var token = current {
    current = nil
    token.terminated = terminated
    token.quoteState = quoteState
    tokens.append(token)
  }
}

private func operatorToken(_ text: String, start: Int, end: Int, role: ShellTokenRole) -> ShellToken
{
  ShellToken(
    text: text,
    span: TextSpan(start: start, end: end),
    role: role,
    quoteState: .none,
    terminated: true,
    quoted: false)
}

private func controlOperatorAt(_ input: [UInt8], _ pos: Int) -> (String, Int, ShellTokenRole)? {
  let b = input[pos]
  let pipe = UInt8(ascii: "|")
  let amp = UInt8(ascii: "&")
  let semi = UInt8(ascii: ";")
  if (b == pipe || b == amp), pos + 1 < input.count, input[pos + 1] == b {
    let text = String(UnicodeScalar(b)) + String(UnicodeScalar(b))
    return (text, pos + 2, .controlOperator)
  }
  if b == pipe {
    return ("|", pos + 1, .pipelineSeparator)
  }
  if b == semi || b == amp {
    return (String(UnicodeScalar(b)), pos + 1, .controlOperator)
  }
  return nil
}

private func redirectOperatorAt(_ input: [UInt8], _ pos: Int) -> (String, Int)? {
  var consumed = 0
  while pos + consumed < input.count, input[pos + consumed].isAsciiDigit {
    consumed += 1
  }

  guard pos + consumed < input.count else { return nil }
  let byte = input[pos + consumed]
  let gt = UInt8(ascii: ">")
  let lt = UInt8(ascii: "<")
  guard byte == gt || byte == lt else { return nil }
  consumed += 1

  if pos + consumed < input.count {
    let next = input[pos + consumed]
    if next == byte || next == UInt8(ascii: "&") {
      consumed += 1
      if byte == lt, next == lt, pos + consumed < input.count, input[pos + consumed] == lt {
        consumed += 1
      }
    }
  }

  let text = String(decoding: input[pos..<(pos + consumed)], as: UTF8.self)
  return (text, pos + consumed)
}

private func classifyTokens(_ tokens: inout [ShellToken]) {
  var commandSeen = false
  var expectRedirectionTarget = false

  for index in tokens.indices {
    switch tokens[index].role {
    case .pipelineSeparator, .controlOperator:
      commandSeen = false
      expectRedirectionTarget = false
    case .redirectionOperator:
      expectRedirectionTarget = true
    default:
      if expectRedirectionTarget {
        tokens[index].role = .redirectionTarget
        expectRedirectionTarget = false
      } else if !commandSeen, isAssignmentWord(tokens[index].text) {
        tokens[index].role = .assignment
      } else if !commandSeen {
        tokens[index].role = .command
        commandSeen = true
      } else {
        tokens[index].role = .argument
      }
    }
  }
}

private func currentSegmentStart(_ tokens: [ShellToken]) -> Int {
  if let index = tokens.lastIndex(where: {
    $0.role == .pipelineSeparator || $0.role == .controlOperator
  }) {
    return index + 1
  }
  return 0
}

private func currentPipelineStart(_ tokens: [ShellToken], _ segmentStart: Int) -> Int {
  if let index = tokens[..<segmentStart].lastIndex(where: { $0.role == .controlOperator }) {
    return index + 1
  }
  return 0
}

private func collectRedirects(_ tokens: [ShellToken]) -> [ShellRedirection] {
  var redirects: [ShellRedirection] = []
  var index = 0
  while index < tokens.count {
    let token = tokens[index]
    index += 1
    guard token.role == .redirectionOperator else { continue }
    var target: ShellToken? = nil
    if index < tokens.count, tokens[index].role == .redirectionTarget {
      target = tokens[index]
    }
    redirects.append(
      ShellRedirection(
        operatorText: token.text,
        operatorSpan: token.span,
        target: target?.text,
        targetSpan: target?.span))
  }
  return redirects
}

private func buildCompletion(
  tokens: [ShellToken], currentIndex: Int?, segmentStart: Int, cursor: Int,
  quoteState: ShellQuoteState
) -> ShellCompletion {
  let current = currentIndex.map { tokens[$0] }
  let command = tokens[segmentStart...].first { $0.role == .command }
  let previous = previousWord(tokens, currentIndex, segmentStart)
  let expectsRedirectTarget =
    previousRole(tokens, currentIndex, segmentStart) == .redirectionOperator

  let kind: ShellCompletionKind
  if current?.role == .redirectionTarget || expectsRedirectTarget {
    kind = .redirectTarget
  } else if current?.role == .assignment {
    kind = .envAssignment
  } else if command == nil || current?.role == .command {
    kind = .command
  } else {
    kind = .argument
  }

  let nonOperator = current.flatMap { isOperatorRole($0.role) ? nil : $0 }
  let prefix = nonOperator?.text ?? ""
  let span = nonOperator?.span ?? TextSpan(start: cursor, end: cursor)
  let argumentCountEnd = currentIndex ?? tokens.count
  let argumentIndex = tokens[segmentStart..<argumentCountEnd]
    .filter { $0.role == .argument || $0.role == .redirectionTarget }
    .count

  return ShellCompletion(
    kind: kind,
    prefix: prefix,
    span: span,
    quoteState: current?.quoteState ?? quoteState,
    command: command?.text,
    commandSpan: command?.span,
    argumentIndex: argumentIndex,
    previousWord: previous)
}

private func tokenContainsCursor(_ token: ShellToken, _ cursor: Int) -> Bool {
  !token.terminated && !isOperatorRole(token.role) && token.span.start <= cursor
    && cursor <= token.span.end
}

private func previousRole(_ tokens: [ShellToken], _ currentIndex: Int?, _ segmentStart: Int)
  -> ShellTokenRole?
{
  let end = currentIndex ?? tokens.count
  return tokens[segmentStart..<end].last?.role
}

private func previousWord(_ tokens: [ShellToken], _ currentIndex: Int?, _ segmentStart: Int)
  -> String?
{
  let end = currentIndex ?? tokens.count
  return tokens[segmentStart..<end].last { !isOperatorRole($0.role) }?.text
}

private func isOperatorRole(_ role: ShellTokenRole) -> Bool {
  role == .redirectionOperator || role == .pipelineSeparator || role == .controlOperator
}

private func isAssignmentWord(_ word: String) -> Bool {
  let scalars = Array(word.unicodeScalars)
  guard let eq = scalars.firstIndex(of: "=") else { return false }
  let name = scalars[..<eq]
  guard let first = name.first else { return false }
  guard first == "_" || first.isAsciiAlphabetic else { return false }
  return name.dropFirst().allSatisfy { $0 == "_" || $0.isAsciiAlphanumeric }
}

private func isAsciiDigits(_ value: String) -> Bool {
  !value.isEmpty && value.utf8.allSatisfy { $0.isAsciiDigit }
}

extension UInt8 {
  fileprivate var isAsciiDigit: Bool {
    self >= UInt8(ascii: "0") && self <= UInt8(ascii: "9")
  }
}

extension Unicode.Scalar {
  fileprivate var isAsciiAlphabetic: Bool {
    (self >= "a" && self <= "z") || (self >= "A" && self <= "Z")
  }

  fileprivate var isAsciiAlphanumeric: Bool {
    isAsciiAlphabetic || (self >= "0" && self <= "9")
  }
}
