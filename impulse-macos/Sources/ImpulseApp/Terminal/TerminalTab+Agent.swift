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

  /// Type `text` into the agent's prompt as a paste (so newlines don't
  /// submit), without pressing Return. While the agent is mid-turn it waits
  /// until the turn ends. Returns false when there's no agent.
  @discardableResult
  func sendToAgent(_ text: String) -> Bool {
    guard agent != nil else { return false }
    if agentMachine.state == .working || agentMachine.state == .needsInput {
      pendingAgentText.append(text)
      return true
    }
    pasteToAgent(text)
    return true
  }

  /// Paste text into the program now (bracketed when it asks for that),
  /// optionally pressing Return after it.
  func paste(_ text: String, submit: Bool) {
    pasteToAgent(text)
    guard submit else { return }
    // Give the program a moment to take the paste before Return.
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
      self?.backend?.write("\r")
      self?.agentEvent(.submit)
    }
  }

  private func pasteToAgent(_ text: String) {
    guard let backend else { return }
    if backend.mode()?.bracketedPaste == true {
      backend.write("\u{1b}[200~" + text + "\u{1b}[201~")
    } else {
      backend.write(text)
    }
  }

  /// Deliver text queued while the agent was busy.
  private func flushPendingAgentText() {
    guard !pendingAgentText.isEmpty else { return }
    let text = pendingAgentText.joined(separator: "\n\n")
    pendingAgentText = []
    pasteToAgent(text)
  }

  /// The agent left (its command ended or another program took over).
  func endAgent() {
    guard agent != nil else { return }
    let before = agentMachine.state
    agentMachine.handle(.exited, at: Date())
    recordTurnBoundary(from: before, to: .exited)
    agent = nil
    agentFromHooks = false
    agentSession = nil
    agentTickTimer?.invalidate()
    agentTickTimer = nil
    agentTickDeadline = nil
    NotificationCenter.default.post(name: .terminalAgentChanged, object: self)
  }

  /// An agent's hook (or `impulse status`) reported in. Starts tracking
  /// the agent if it wasn't recognized yet.
  func agentHook(_ hook: AgentHook, agentID: String?) {
    if agent == nil {
      agent =
        KnownAgents.builtIn.first { $0.id == agentID }
        ?? AgentKind(id: agentID ?? "agent", displayName: agentID?.capitalized ?? "Agent", names: [])
      agentMachine = AgentStateMachine(now: Date())
      agentFromHooks = true
      NotificationCenter.default.post(name: .terminalAgentChanged, object: self)
    }
    agentEvent(.hook(hook))
  }

  private func probeForegroundAgent() {
    // Hooks vouch for the agent while its command runs.
    if agentFromHooks, isCommandRunning { return }
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
    // Returns a count of PIDs (not bytes).
    let filled = pids.withUnsafeMutableBufferPointer {
      proc_listchildpids(pid, $0.baseAddress, Int32(Int(count) * MemoryLayout<pid_t>.size))
    }
    return Array(pids.prefix(max(0, min(Int(filled), pids.count)))).filter { $0 > 0 }
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

  /// Checkpoint the repository when a turn starts (work begins from idle or
  /// done) and when it ends (done, idle, exit). A mid-turn question
  /// (needs input) doesn't split the turn.
  private func recordTurnBoundary(from before: AgentState, to state: AgentState) {
    if state == .working, before == .idle || before == .done, let agent {
      AgentCheckpoints.shared.turnStarted(
        terminalID: id, agentName: agent.displayName, cwd: currentWorkingDirectory)
    } else if before == .working || before == .needsInput,
      state == .done || state == .idle || state == .exited
    {
      AgentCheckpoints.shared.turnEnded(terminalID: id)
    }
  }

  private func agentStateChanged(from before: AgentState, cause: AgentEvent?) {
    let state = agentMachine.state
    recordTurnBoundary(from: before, to: state)
    if state == .done || state == .idle { flushPendingAgentText() }
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
