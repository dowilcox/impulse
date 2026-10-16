// Tracks what a coding agent in a terminal is doing — working, waiting on
// the user, finished — from the signals a terminal can see. Hooks (when the
// agent is set up to call Impulse) are authoritative; without them the
// machine combines progress reports (OSC 9;4), notifications, bells, and
// how output flows relative to the user's typing. Even with hooks, a long
// silence ends a turn or a question: one dismissed with Esc never reports
// that it stopped. State reported with `impulse status` is left alone.

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
  /// A tool finished (named, when the hook says). The answer to a pending
  /// approval, even one given without Return; otherwise no change.
  case usedTool(String?)
  case stopped
  case notification(String)
  /// What `impulse status` reported, kept until the next report.
  case status(AgentState)
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
  /// With hooks, which report the end of a turn, this long without output
  /// or a report means the turn was interrupted (Esc sends no Stop).
  public var silenceWithHooks: TimeInterval = 300
  /// …and this long means a question was dismissed. Much longer: one left
  /// unanswered while you're away should still say it needs you.
  public var silenceWhileAsking: TimeInterval = 1800

  private var lastOutput: Date?
  private var lastHook: Date?
  private var streakStart: Date?
  private var lastKeystroke: Date?
  private var progressActive = false
  private var sawProgress = false
  private var hooksActive = false
  /// The state came from `impulse status`, which reports its own changes.
  private var reportedByStatus = false
  /// The tool the pending permission request names ("…to use Bash").
  private var requestedTool: String?

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
      lastHook = now
      reportedByStatus = false
      switch hook {
      case .sessionStarted: return transition(.idle, at: now)
      case .promptSubmitted: return transition(.working, at: now)
      case .stopped: return transition(.done, at: now)
      case .usedTool(let tool):
        // Tools also run in parallel subagents and after Stop (background
        // tasks): only the one that was waiting for approval answers it.
        guard state == .needsInput else { return false }
        if let tool, let requestedTool, requestedTool.caseInsensitiveCompare(tool) != .orderedSame { return false }
        return transition(.working, at: now)
      case .notification(let text):
        if Self.isPermissionRequest(text) { return ask(text, at: now) }
        // Other notifications end a turn only while one runs: Claude Code's
        // "waiting for your input" reminder comes long after it finished.
        return state == .working ? transition(.done, at: now) : false
      case .status(let reported):
        reportedByStatus = true
        requestedTool = nil
        return transition(reported, at: now)
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
      if Self.isPermissionRequest(text) { return ask(text, at: now) }
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
    guard let silenceLimit, now.timeIntervalSince(quietSince) >= silenceLimit else { return false }
    progressActive = false
    // With hooks, a turn that finished would have said so: this one (or its
    // question) was dismissed, by the user, so there's nothing to flag.
    return transition(hooksActive ? .idle : .done, at: now)
  }

  /// When `tick` next needs to run, if ever.
  public var nextDeadline: Date? {
    silenceLimit.map { quietSince.addingTimeInterval($0) }
  }

  /// Silence counts from the last output or hook report, or from when
  /// work started if that's later.
  private var quietSince: Date { max(lastOutput ?? since, lastHook ?? since, since) }

  /// How long silence may last in this state before it ends; nil: as long
  /// as it likes.
  private var silenceLimit: TimeInterval? {
    switch state {
    case .working where reportedByStatus: return nil
    case .working:
      return hooksActive ? silenceWithHooks : progressActive || sawProgress ? silenceWithProgress : silenceToDone
    case .needsInput where hooksActive && !reportedByStatus: return silenceWhileAsking
    default: return nil
    }
  }

  private mutating func ask(_ text: String, at now: Date) -> Bool {
    requestedTool = Self.requestedTool(text)
    return transition(.needsInput, at: now)
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

  /// The tool a permission request names: "Claude needs your permission to
  /// use Bash" → "Bash".
  static func requestedTool(_ text: String) -> String? {
    guard let range = text.range(of: "to use ", options: .caseInsensitive) else { return nil }
    let name = text[range.upperBound...].prefix { !$0.isWhitespace && !".,?!:;".contains($0) }
    return name.isEmpty ? nil : String(name)
  }

  /// Braille spinner glyphs some agents put at the front of the title while
  /// they work.
  static func isSpinner(_ scalar: Unicode.Scalar?) -> Bool {
    guard let scalar else { return false }
    return (0x2801...0x28FF).contains(scalar.value)
  }
}
