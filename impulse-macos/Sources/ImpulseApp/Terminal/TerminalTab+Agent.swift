import AppKit
import ImpulseKit

// Coding-agent tracking for a terminal: notices when the foreground program
// is a known agent (Claude Code, Codex, …) and follows what it's doing with
// `AgentStateMachine`, fed from the terminal's own signals.

extension TerminalTab {
  /// Look for an agent soon after a command starts (the shell forks before
  /// it execs), then every 2 s while the command runs.
  func startAgentProbe() {
    stopAgentProbe()
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { [weak self] in
      self?.probeForegroundAgent()
    }
    agentProbeTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
      self?.probeForegroundAgent()
    }
  }

  func stopAgentProbe() {
    agentProbeTimer?.invalidate()
    agentProbeTimer = nil
  }

  /// Pass a signal to the agent's state machine.
  func agentEvent(_ event: AgentEvent) {
    guard agent != nil else { return }
    let before = agentMachine.state
    if agentMachine.handle(event, at: Date()) {
      agentStateChanged(from: before, cause: event)
    }
    scheduleAgentTick()
  }

  /// The agent left (its command ended or another program took over).
  func endAgent() {
    guard agent != nil else { return }
    agentMachine.handle(.exited, at: Date())
    agent = nil
    agentTickTimer?.invalidate()
    agentTickTimer = nil
    agentTickDeadline = nil
    NotificationCenter.default.post(name: .terminalAgentChanged, object: self)
  }

  private func probeForegroundAgent() {
    guard isCommandRunning, let backend, let leader = backend.foregroundPid(),
      leader != pid_t(backend.childPid())
    else {
      endAgent()
      return
    }
    // The group leader, or one level down for wrappers (`caffeinate claude`).
    var found: AgentKind?
    for pid in [leader] + Self.children(of: leader) {
      if let details = ProcessInspector.details(of: pid),
        let kind = KnownAgents.match(
          executablePath: details.executablePath, arguments: details.arguments)
      {
        found = kind
        break
      }
    }
    guard found != agent else { return }
    endAgent()
    guard let found else { return }
    agent = found
    agentMachine = AgentStateMachine(now: Date())
    NotificationCenter.default.post(name: .terminalAgentChanged, object: self)
  }

  private static func children(of pid: pid_t) -> [pid_t] {
    let count = proc_listchildpids(pid, nil, 0)
    guard count > 0 else { return [] }
    var pids = [pid_t](repeating: 0, count: Int(count))
    let bytes = pids.withUnsafeMutableBufferPointer {
      proc_listchildpids(pid, $0.baseAddress, Int32(Int(count) * MemoryLayout<pid_t>.size))
    }
    return Array(pids.prefix(Int(bytes) / MemoryLayout<pid_t>.size)).filter { $0 > 0 }
  }

  /// Keep one timer armed for the machine's next timeout. Output pushes the
  /// deadline later many times a second; the timer only moves earlier, and
  /// when it fires early it just re-arms for the new deadline.
  private func scheduleAgentTick() {
    guard let deadline = agentMachine.nextDeadline else {
      agentTickTimer?.invalidate()
      agentTickTimer = nil
      agentTickDeadline = nil
      return
    }
    if agentTickTimer != nil, let scheduled = agentTickDeadline, scheduled <= deadline { return }
    agentTickTimer?.invalidate()
    agentTickDeadline = deadline
    agentTickTimer = Timer.scheduledTimer(
      withTimeInterval: max(0.1, deadline.timeIntervalSinceNow), repeats: false
    ) { [weak self] _ in
      guard let self else { return }
      self.agentTickTimer = nil
      self.agentTickDeadline = nil
      let before = self.agentMachine.state
      if self.agentMachine.tick(at: Date()) {
        self.agentStateChanged(from: before, cause: nil)
      }
      self.scheduleAgentTick()
    }
  }

  private func agentStateChanged(from before: AgentState, cause: AgentEvent?) {
    let state = agentMachine.state
    if state.wantsUser, let agent {
      if isWatched, state == .done {
        // The user saw it finish; nothing to flag.
        agentMachine.handle(.acknowledged, at: Date())
      } else {
        setAttentionFromAgent()
        // Notifications and bells already notified on their own.
        if cause != .bell, !Self.isNotification(cause) {
          let headline =
            state == .needsInput ? "\(agent.displayName) needs your input" : "\(agent.displayName) finished"
          NotificationCenter.default.post(
            name: .terminalWantsNotification, object: self,
            userInfo: ["title": headline, "body": tabTitle])
        }
      }
    }
    NotificationCenter.default.post(name: .terminalAgentChanged, object: self)
  }

  private static func isNotification(_ event: AgentEvent?) -> Bool {
    if case .notification? = event { return true }
    return false
  }
}
