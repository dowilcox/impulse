#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct KnownAgentsTests {
    @Test func matchesNativeBinariesAndScripts() {
      #expect(KnownAgents.match(executablePath: "/opt/homebrew/bin/claude", arguments: ["claude"])?.id == "claude")
      #expect(
        KnownAgents.match(
          executablePath: "/usr/local/bin/node",
          arguments: ["node", "/usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js", "--resume"]
        )?.id == "claude")
      #expect(
        KnownAgents.match(
          executablePath: "/Users/me/.nvm/versions/node/v22/bin/node",
          arguments: ["node", "--no-warnings", "/Users/me/.nvm/versions/node/v22/bin/codex"])?.id == "codex")
      #expect(
        KnownAgents.match(
          executablePath: "/opt/homebrew/Cellar/python@3.12/3.12.4/bin/python3.12",
          arguments: ["python3.12", "/Users/me/.local/bin/aider", "--model", "x"])?.id == "aider")
      #expect(KnownAgents.match(executablePath: "/usr/local/bin/gemini", arguments: ["gemini"])?.displayName == "Gemini CLI")
      #expect(
        KnownAgents.match(executablePath: "/bin/sh", arguments: ["/bin/sh", "/Users/me/.claude/local/claude"])?.id
          == "claude")
      #expect(KnownAgents.match(executablePath: "/bin/bash", arguments: ["bash", "deploy.sh"]) == nil)
    }

    @Test func ignoresOrdinaryPrograms() {
      #expect(KnownAgents.match(executablePath: "/usr/bin/vim", arguments: ["vim", "claude.md"]) == nil)
      #expect(KnownAgents.match(executablePath: "/usr/local/bin/node", arguments: ["node", "server.js"]) == nil)
      #expect(KnownAgents.match(executablePath: "/bin/zsh", arguments: ["-zsh"]) == nil)
    }

    @Test func resumeCommands() {
      #expect(KnownAgents.resumeCommand(agentID: "claude", session: "3f2a-91bc") == "claude --resume 3f2a-91bc")
      #expect(KnownAgents.resumeCommand(agentID: "codex", session: "abc_1") == "codex resume abc_1")
      #expect(KnownAgents.resumeCommand(agentID: "claude", session: "x; rm -rf ~") == nil)
      #expect(KnownAgents.resumeCommand(agentID: "aider", session: "s") == nil)
    }

    @Test func userAgentsExtendTheTable() {
      let custom = AgentKind(id: "mine", displayName: "Mine", names: ["my-agent"])
      #expect(
        KnownAgents.match(executablePath: "/x/my-agent", arguments: ["my-agent"], extra: [custom])?.id
          == "mine")
    }
  }

  struct AgentStateMachineTests {
    private let t0 = Date(timeIntervalSince1970: 1000)
    private func at(_ seconds: TimeInterval) -> Date { t0.addingTimeInterval(seconds) }

    /// Output events every 100 ms over a span.
    private func stream(_ machine: inout AgentStateMachine, from start: TimeInterval, to end: TimeInterval) {
      var t = start
      while t <= end {
        machine.handle(.output, at: at(t))
        t += 0.1
      }
    }

    @Test func submittingAPromptStartsWorkAndSilenceEndsIt() {
      var machine = AgentStateMachine(now: t0)
      #expect(machine.state == .idle)
      machine.handle(.submit, at: at(1))
      #expect(machine.state == .working)
      stream(&machine, from: 1.2, to: 6)
      let changed1 = machine.tick(at: at(8))
      #expect(changed1 == false)
      #expect(machine.nextDeadline == at(6).addingTimeInterval(5))
      let changed2 = machine.tick(at: at(11.1))
      #expect(changed2)
      #expect(machine.state == .done)
      #expect(machine.since == at(11.1))
      // Looking at it clears "done".
      machine.handle(.acknowledged, at: at(12))
      #expect(machine.state == .idle)
    }

    @Test func sustainedOutputWithoutTypingMeansWorking() {
      var machine = AgentStateMachine(now: t0)
      stream(&machine, from: 1, to: 2.0)
      #expect(machine.state == .idle, "a short burst isn't work")
      stream(&machine, from: 2.1, to: 3.0)
      #expect(machine.state == .working)
    }

    @Test func echoOfTypingIsNotWork() {
      var machine = AgentStateMachine(now: t0)
      var t = 1.0
      while t < 5 {
        machine.handle(.keystroke, at: at(t))
        machine.handle(.output, at: at(t + 0.05))
        t += 0.3
      }
      #expect(machine.state == .idle)
    }

    @Test func progressReportsAreTrustedOverSilence() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.progress(active: true), at: at(1))
      #expect(machine.state == .working)
      let changed3 = machine.tick(at: at(30))
      #expect(changed3 == false, "long thinking without output")
      machine.handle(.progress(active: false), at: at(40))
      #expect(machine.state == .done)
      // Later output alone doesn't restart work once progress is known.
      stream(&machine, from: 41, to: 45)
      #expect(machine.state == .done)
    }

    @Test func permissionNotificationsNeedInputOthersFinish() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.submit, at: at(1))
      machine.handle(.notification("Claude needs your permission to use Bash"), at: at(2))
      #expect(machine.state == .needsInput)
      // Output while waiting (the prompt redraws) keeps it waiting.
      stream(&machine, from: 2.5, to: 5)
      #expect(machine.state == .needsInput)
      machine.handle(.submit, at: at(6))
      #expect(machine.state == .working)
      machine.handle(.notification("Claude is waiting for your input"), at: at(9))
      #expect(machine.state == .done)
    }

    @Test func bellEndsATurn() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.bell, at: at(1))
      #expect(machine.state == .idle)
      machine.handle(.submit, at: at(2))
      machine.handle(.bell, at: at(3))
      #expect(machine.state == .done)
    }

    @Test func spinnerTitlesMeanWorking() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.title("✳ Claude Code"), at: at(1))
      #expect(machine.state == .idle)
      machine.handle(.title("⠋ Refactoring parser"), at: at(2))
      #expect(machine.state == .working)
    }

    @Test func hooksOverrideHeuristics() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.sessionStarted), at: at(0.5))
      machine.handle(.hook(.promptSubmitted), at: at(1))
      #expect(machine.state == .working)
      // Heuristics are ignored once hooks report.
      machine.handle(.bell, at: at(2))
      let changed4 = machine.tick(at: at(100))
      #expect(changed4 == false)
      #expect(machine.nextDeadline == nil)
      #expect(machine.state == .working)
      machine.handle(.hook(.notification("Claude needs your permission to use Edit")), at: at(101))
      #expect(machine.state == .needsInput)
      machine.handle(.hook(.stopped), at: at(120))
      #expect(machine.state == .done)
    }

    @Test func exitIsFinal() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.submit, at: at(1))
      let changed5 = machine.handle(.exited, at: at(2))
      #expect(changed5)
      let changed6 = machine.handle(.submit, at: at(3))
      #expect(changed6 == false)
      #expect(machine.state == .exited)
      #expect(AgentState.done.wantsUser && AgentState.needsInput.wantsUser && !AgentState.working.wantsUser)
    }
  }
#endif
