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

    @Test func matchesOpencodeHoweverItsInstalled() {
      func id(_ executable: String, _ arguments: [String]) -> String? {
        KnownAgents.match(executablePath: executable, arguments: arguments)?.id
      }
      // npm (opencode-ai): its postinstall links the native binary in as bin/opencode.exe.
      #expect(id("/Users/me/.npm-global/lib/node_modules/opencode-ai/bin/opencode.exe", ["opencode"]) == "opencode")
      // Older npm releases: a node script that starts the platform package's binary.
      #expect(id("/usr/local/bin/node", ["node", "/usr/local/lib/node_modules/opencode-ai/bin/opencode"]) == "opencode")
      #expect(
        id("/usr/local/lib/node_modules/opencode-ai/node_modules/opencode-darwin-arm64/bin/opencode", ["opencode"])
          == "opencode")
      // Homebrew, and the curl installer.
      #expect(id("/opt/homebrew/Cellar/opencode/1.18.34/bin/opencode", ["opencode"]) == "opencode")
      #expect(id("/Users/me/.opencode/bin/opencode", ["/Users/me/.opencode/bin/opencode", "--continue"]) == "opencode")
      // Crush (once the Go project called opencode) is its own agent.
      #expect(id("/opt/homebrew/bin/crush", ["crush"]) == "crush")
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
      #expect(machine.nextDeadline == at(301), "only a long silence ends a turn")
      #expect(machine.state == .working)
      machine.handle(.hook(.notification("Claude needs your permission to use Edit")), at: at(101))
      #expect(machine.state == .needsInput)
      machine.handle(.hook(.stopped), at: at(120))
      #expect(machine.state == .done)
    }

    @Test func aToolRunningAnswersItsOwnApproval() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.promptSubmitted), at: at(1))
      machine.handle(.hook(.notification("Claude needs your permission to use Bash")), at: at(3))
      #expect(machine.state == .needsInput)
      // A parallel subagent's edit doesn't answer it.
      let sibling = machine.handle(.hook(.usedTool("Edit")), at: at(4))
      #expect(sibling == false)
      #expect(machine.state == .needsInput)
      // Approved with a number key, no Return: the command runs, and says so.
      machine.handle(.keystroke, at: at(5))
      #expect(machine.state == .needsInput)
      machine.handle(.hook(.usedTool("Bash")), at: at(6))
      #expect(machine.state == .working)
      // A tool the hook doesn't name answers any approval.
      machine.handle(.hook(.notification("Claude needs your permission to use Write")), at: at(7))
      machine.handle(.hook(.usedTool(nil)), at: at(8))
      #expect(machine.state == .working)

      #expect(AgentStateMachine.requestedTool("Claude needs your permission to use Bash") == "Bash")
      #expect(AgentStateMachine.requestedTool("Allow mcp__docs__read? It wants to use mcp__docs__read.") == "mcp__docs__read")
      #expect(AgentStateMachine.requestedTool("Claude needs your input") == nil)
    }

    @Test func toolsAfterTheTurnChangeNothing() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.promptSubmitted), at: at(1))
      machine.handle(.hook(.stopped), at: at(10))
      // A background task's tool finishing after Stop.
      let afterStop = machine.handle(.hook(.usedTool("Bash")), at: at(12))
      #expect(afterStop == false)
      #expect(machine.state == .done, "still unseen")
      machine.handle(.acknowledged, at: at(13))
      let afterSeen = machine.handle(.hook(.usedTool("Bash")), at: at(14))
      #expect(afterSeen == false)
      #expect(machine.state == .idle)
      #expect(machine.nextDeadline == nil)
    }

    @Test func aDismissedQuestionEndsAfterAWhile() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.promptSubmitted), at: at(1))
      machine.handle(.hook(.notification("Claude needs your permission to use Bash")), at: at(2))
      // Esc on the prompt sends nothing; someone away is still asked for a
      // good while.
      let soon = machine.tick(at: at(600))
      #expect(soon == false)
      #expect(machine.state == .needsInput)
      #expect(machine.nextDeadline == at(1802))
      let later = machine.tick(at: at(1900))
      #expect(later)
      #expect(machine.state == .idle)
    }

    @Test func impulseStatusReportsAreKept() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.status(.working)), at: at(1))
      #expect(machine.state == .working)
      #expect(machine.nextDeadline == nil, "a silent wrapper keeps working")
      let ticked = machine.tick(at: at(5000))
      #expect(ticked == false)
      machine.handle(.hook(.status(.needsInput)), at: at(5001))
      #expect(machine.state == .needsInput)
      #expect(machine.nextDeadline == nil)
      machine.handle(.hook(.status(.done)), at: at(5002))
      #expect(machine.state == .done)
    }

    @Test func idleRemindersDontFinishATurn() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.promptSubmitted), at: at(1))
      machine.handle(.hook(.stopped), at: at(10))
      machine.handle(.acknowledged, at: at(11))
      #expect(machine.state == .idle)
      // Claude Code's reminder a minute after the turn.
      let changed = machine.handle(.hook(.notification("Claude is waiting for your input")), at: at(71))
      #expect(changed == false)
      #expect(machine.state == .idle)
      // Mid-turn, a notification that asks nothing still ends the turn.
      machine.handle(.hook(.promptSubmitted), at: at(80))
      machine.handle(.hook(.notification("Claude is waiting for your input")), at: at(90))
      #expect(machine.state == .done)
    }

    @Test func aLongSilenceEndsAnInterruptedHookTurn() {
      var machine = AgentStateMachine(now: t0)
      machine.handle(.hook(.promptSubmitted), at: at(1))
      stream(&machine, from: 1, to: 20)
      // Esc mid-turn: no Stop comes, and the spinner stops.
      let early = machine.tick(at: at(200))
      #expect(early == false)
      let late = machine.tick(at: at(330))
      #expect(late)
      #expect(machine.state == .idle, "interrupted, not finished: nothing to flag")

      // Silence from before the turn started doesn't count.
      var quiet = AgentStateMachine(now: t0)
      stream(&quiet, from: 0, to: 1)
      quiet.handle(.hook(.promptSubmitted), at: at(600))
      let ended = quiet.tick(at: at(601))
      #expect(ended == false)
      #expect(quiet.nextDeadline == at(900))
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
