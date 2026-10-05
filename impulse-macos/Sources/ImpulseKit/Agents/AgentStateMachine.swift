// Tracks what a coding agent in a terminal is doing — working, waiting on
// the user, finished — from the signals a terminal can see. Hooks (when the
// agent is set up to call Impulse) are authoritative; without them the
// machine combines progress reports (OSC 9;4), notifications, bells, and
// how output flows relative to the user's typing.

import Foundation

public enum AgentState: String, Codable, Sendable {
  /// Running, nothing asked of it yet (or its result was seen).
  case idle
  case working
  /// Blocked on the user mid-turn (a permission prompt, a question).
  case needsInput
  /// Finished a turn the user hasn't looked at yet.
  case done
  case exited

  /// Whether the user is wanted.
  public var wantsUser: Bool { self == .needsInput || self == .done }
}

/// What an agent reported through its hooks (Claude Code hooks, Codex
/// notify, the `impulse` CLI).
public enum AgentHook: Equatable, Sendable {
  case sessionStarted
  case promptSubmitted
  case stopped
  case notification(String)
}

public enum AgentEvent: Equatable, Sendable {
  /// The program wrote to the terminal.
  case output
  /// The user typed into the program.
  case keystroke
  /// The user pressed Return (sent a prompt or answered a question).
  case submit
  /// OSC 9;4 progress shown (true) or cleared (false).
  case progress(active: Bool)
  case title(String)
  /// A desktop-notification request (OSC 9/777/99), title and body joined.
  case notification(String)
  case bell
  case hook(AgentHook)
  /// The user looked at the pane: a finished turn has been seen.
  case acknowledged
  case exited
}

public struct AgentStateMachine: Sendable {
  public private(set) var state: AgentState = .idle
  /// When `state` last changed.
  public private(set) var since: Date

  /// Output must keep coming this long (a spinner, streaming text) before
  /// it counts as working…
  public var streakToWork: TimeInterval = 1.5
  /// …and gaps longer than this end a streak.
  public var streakGap: TimeInterval = 0.6
  /// Output right after a keystroke is just echo.
  public var echoWindow: TimeInterval = 1.0
  /// Working with no output for this long means the turn ended.
  public var silenceToDone: TimeInterval = 5
  /// With progress reports, wait this long before trusting silence.
  public var silenceWithProgress: TimeInterval = 120

  private var lastOutput: Date?
  private var streakStart: Date?
  private var lastKeystroke: Date?
  private var progressActive = false
  private var sawProgress = false
  private var hooksActive = false

  public init(now: Date = Date()) {
    since = now
  }

  /// Apply an event. Returns true when `state` changed.
  @discardableResult
  public mutating func handle(_ event: AgentEvent, at now: Date) -> Bool {
    guard state != .exited || event == .exited else { return false }
    switch event {
    case .exited:
      return transition(.exited, at: now)

    case .hook(let hook):
      hooksActive = true
      switch hook {
      case .sessionStarted: return transition(.idle, at: now)
      case .promptSubmitted: return transition(.working, at: now)
      case .stopped: return transition(.done, at: now)
      case .notification(let text):
        return transition(Self.isPermissionRequest(text) ? .needsInput : .done, at: now)
      }

    case .acknowledged:
      return state == .done ? transition(.idle, at: now) : false

    case .submit:
      lastKeystroke = now
      streakStart = nil
      return transition(.working, at: now)

    case .keystroke:
      lastKeystroke = now
      streakStart = nil
      return false

    case .progress(let active):
      sawProgress = true
      progressActive = active
      if hooksActive { return false }
      if active { return transition(.working, at: now) }
      return state == .working ? transition(.done, at: now) : false

    case .notification(let text):
      if Self.isPermissionRequest(text) { return transition(.needsInput, at: now) }
      if hooksActive { return false }
      return state == .working || state == .idle ? transition(.done, at: now) : false

    case .bell:
      if hooksActive { return false }
      return state == .working ? transition(.done, at: now) : false

    case .title(let title):
      guard !hooksActive, Self.isSpinner(title.unicodeScalars.first) else { return false }
      if let lastKeystroke, now.timeIntervalSince(lastKeystroke) < echoWindow { return false }
      return state == .working ? false : transition(.working, at: now)

    case .output:
      defer { lastOutput = now }
      if let lastKeystroke, now.timeIntervalSince(lastKeystroke) < echoWindow {
        streakStart = nil
        return false
      }
      if let lastOutput, now.timeIntervalSince(lastOutput) <= streakGap, streakStart != nil {
        // Streak continues.
      } else {
        streakStart = now
      }
      guard !hooksActive, !sawProgress, state != .working, state != .needsInput,
        let streakStart, now.timeIntervalSince(streakStart) >= streakToWork
      else { return false }
      return transition(.working, at: now)
    }
  }

  /// Evaluate timeouts. Returns true when `state` changed.
  @discardableResult
  public mutating func tick(at now: Date) -> Bool {
    guard state == .working, !hooksActive else { return false }
    let quiet = now.timeIntervalSince(lastOutput ?? since)
    let limit = progressActive || sawProgress ? silenceWithProgress : silenceToDone
    guard quiet >= limit else { return false }
    progressActive = false
    return transition(.done, at: now)
  }

  /// When `tick` next needs to run, if ever.
  public var nextDeadline: Date? {
    guard state == .working, !hooksActive else { return nil }
    let limit = progressActive || sawProgress ? silenceWithProgress : silenceToDone
    return (lastOutput ?? since).addingTimeInterval(limit)
  }

  private mutating func transition(_ next: AgentState, at now: Date) -> Bool {
    guard next != state else { return false }
    state = next
    since = now
    if next != .working { streakStart = nil }
    return true
  }

  /// "Claude needs your permission to use Bash", "Allow edit?", …
  static func isPermissionRequest(_ text: String) -> Bool {
    let lower = text.lowercased()
    return ["permission", "approve", "approval", "allow ", "confirm", "needs your input", "question"]
      .contains { lower.contains($0) }
  }

  /// Braille spinner glyphs some agents put at the front of the title while
  /// they work.
  static func isSpinner(_ scalar: Unicode.Scalar?) -> Bool {
    guard let scalar else { return false }
    return (0x2801...0x28FF).contains(scalar.value)
  }
}
