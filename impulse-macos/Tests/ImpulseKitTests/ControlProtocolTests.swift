#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseProtocol

  struct ControlProtocolTests {
    private let env = [ControlProtocol.tokenKey: "TOKEN-1"]

    private func parse(_ args: [String], stdin: String? = nil) -> ControlRequest? {
      try? ControlProtocol.request(
        arguments: args, environment: env, cwd: "/repo/sub", stdin: { stdin.map { Data($0.utf8) } }
      ).get()
    }

    @Test func openAndEditResolvePathsAndPositions() {
      let open = parse(["open", "../src/a.swift:12:3"])
      #expect(open?.command == "open")
      #expect(open?.arguments == ["path": "/repo/src/a.swift", "line": "12", "column": "3"])
      #expect(open?.token == "TOKEN-1")
      #expect(open?.wait == false)
      let edit = parse(["edit", "~/notes.md"])
      #expect(edit?.command == "open")
      #expect(edit?.wait == true)
      #expect(edit?.arguments["path"] == NSHomeDirectory() + "/notes.md")
    }

    @Test func paneCommands() {
      #expect(parse(["split", "down", "npm", "test"])?.arguments == ["direction": "down", "command": "npm test"])
      #expect(parse(["split"])?.arguments == ["direction": "right"])
      #expect(parse(["tab", "htop"])?.arguments == ["command": "htop"])
      #expect(parse(["review", "last-turn"])?.arguments == ["scope": "last-turn"])
      #expect(parse(["review", "everything"]) == nil)
      #expect(parse(["status", "waiting", "need", "a", "key"])?.arguments == ["state": "waiting", "message": "need a key"])
      #expect(parse(["status", "busy"]) == nil)
      #expect(parse(["notify", "Build", "done"])?.arguments == ["title": "Build", "message": "done"])
    }

    @Test func claudeHooksReadStdin() {
      let stop = parse(["hook", "claude", "Stop"], stdin: #"{"session_id":"s1","hook_event_name":"Stop"}"#)
      #expect(stop?.arguments["event"] == "Stop")
      #expect(stop?.arguments["session"] == "s1")
      let notification = parse(
        ["hook", "claude"],
        stdin: #"{"hook_event_name":"Notification","message":"Claude needs your permission to use Bash"}"#)
      #expect(notification?.arguments["event"] == "Notification")
      #expect(notification?.arguments["message"] == "Claude needs your permission to use Bash")
    }

    @Test func toolHooksCarryTheToolAndWhatItActsOn() {
      let bash = parse(
        ["hook", "claude"],
        stdin: #"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git merge upgrade"},"cwd":"/repo"}"#)
      #expect(bash?.arguments["event"] == "PreToolUse")
      #expect(bash?.arguments["tool"] == "Bash")
      #expect(bash?.arguments["toolCommand"] == "git merge upgrade")
      #expect(bash?.arguments["hookCwd"] == "/repo")
      let edit = parse(
        ["hook", "claude"],
        stdin: #"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{"file_path":"/repo/src/a.ts"}}"#)
      #expect(edit?.arguments["toolFile"] == "/repo/src/a.ts")
    }

    @Test func tasksListsAndWaits() {
      #expect(parse(["tasks"])?.arguments == [:])
      #expect(parse(["tasks", "--json"])?.arguments == ["json": "1"])
      let wait = parse(["tasks", "wait", "interia-upgrade"])
      #expect(wait?.arguments == ["wait": "interia-upgrade"])
      #expect(wait?.wait == true)
      #expect(parse(["tasks", "wait"]) == nil)
      #expect(parse(["tasks", "everything"]) == nil)
    }

    @Test func tasksShowNewFinishAndArchive() {
      #expect(parse(["tasks", "show", "fix-elevation"])?.arguments == ["show": "fix-elevation"])
      #expect(parse(["tasks", "show", "fix-elevation", "--json"])?.arguments == ["show": "fix-elevation", "json": "1"])
      #expect(parse(["tasks", "show"]) == nil)
      #expect(parse(["tasks", "finish", "x"])?.wait == true)
      #expect(parse(["tasks", "archive", "x"])?.arguments == ["archive": "x"])
      let new = parse(["tasks", "new", "Fix", "the", "units", "--from", "origin/main", "--agent", "claude", "--move-changes"])
      #expect(new?.arguments == ["new": "Fix the units", "base": "origin/main", "agent": "claude", "move": "1"])
      #expect(new?.wait == true)
      let piped = parse(["tasks", "new", "Docs", "--brief", "-"], stdin: "Write the docs.")
      #expect(piped?.arguments["brief"] == "Write the docs.")
      #expect(parse(["tasks", "new", "Docs", "--brief-file", "notes.md"])?.arguments["briefFile"] == "/repo/sub/notes.md")
      #expect(parse(["tasks", "new", "Docs", "--brief", "notes.md"]) == nil)
      #expect(parse(["tasks", "new", "--agent", "claude"]) == nil)
      #expect(parse(["tasks", "new", "x", "--force"]) == nil)
      #expect(parse(["tasks", "new", "--", "--odd", "title"])?.arguments["new"] == "--odd title")
      let large = String(repeating: "x", count: ControlProtocol.briefLimit + 1)
      #expect(parse(["tasks", "new", "x", "--brief", "-"], stdin: large) == nil)
    }

    @Test func workspaceCommands() {
      #expect(parse(["turns", "--json"])?.arguments == ["json": "1"])
      #expect(parse(["turns", "extra"]) == nil)
      #expect(parse(["comments"])?.arguments == [:])
      #expect(parse(["comments", "../src/a.ts", "--json"])?.arguments == ["path": "/repo/src/a.ts", "json": "1"])
      #expect(parse(["comments", "resolve", "c1", "Fixed", "it"])?.arguments == ["resolve": "c1", "note": "Fixed it"])
      #expect(parse(["comments", "resolve"]) == nil)
      #expect(parse(["problems", "--errors"])?.arguments == ["errors": "1"])
      #expect(parse(["problems", "a.ts", "b.ts"]) == nil)
      #expect(parse(["blocks", "show", "ab12:7", "--lines", "20"])?.arguments == ["show": "ab12:7", "lines": "20"])
      #expect(parse(["blocks", "show", "ab12:7", "--lines", "none"]) == nil)
      #expect(parse(["blocks", "everything"]) == nil)
      #expect(parse(["unsaved"])?.command == "unsaved")
      #expect(parse(["project", "--json"])?.arguments == ["json": "1"])
    }

    @Test func responsesCarryStructuredData() throws {
      let response = ControlResponse(ok: true, message: "text", data: Data(#"{"a":1}"#.utf8))
      let line = try ControlProtocol.encodeLine(response)
      #expect(try ControlProtocol.decode(ControlResponse.self, from: line.dropLast()) == response)
      // An older app's answer has no data.
      let old = try ControlProtocol.decode(ControlResponse.self, from: Data(#"{"ok":true,"message":"hi"}"#.utf8))
      #expect(old.data == nil)
    }

    @Test func codexNotifyPassesJSONAsAnArgument() {
      let turn = parse(["hook", "codex", #"{"type":"agent-turn-complete","last-assistant-message":"All done"}"#])
      #expect(turn?.arguments == ["agent": "codex", "event": "agent-turn-complete", "message": "All done"])
    }

    @Test func usageErrors() {
      #expect(parse([]) == nil)
      #expect(parse(["open"]) == nil)
      #expect(parse(["frobnicate"]) == nil)
      if case .failure(let error) = ControlProtocol.request(arguments: ["--help"], environment: [:], cwd: "/") {
        #expect(error.message.contains("impulse open"))
      } else {
        Issue.record("--help should produce usage")
      }
    }

    @Test func unknownOptionsAreUsageErrors() {
      func error(_ args: [String]) -> ControlProtocol.UsageError? {
        if case .failure(let error) = ControlProtocol.request(arguments: args, environment: env, cwd: "/repo") {
          return error
        }
        return nil
      }
      // There's no `edit --wait`: it mustn't open a file named "--wait".
      #expect(error(["edit", "--wait", "notes.md"])?.message.hasPrefix("impulse edit: unknown option '--wait'") == true)
      #expect(error(["open", "notes.md", "-n"])?.isHelp == false)
      #expect(error(["tab", "--new"]) != nil)
      #expect(error(["split", "down", "-x"]) != nil)
      #expect(error(["notify", "-t", "Build"]) != nil)
      #expect(error(["checkpoint", "--all"]) != nil)
      // Help after a command is a request for usage, not a mistake.
      #expect(error(["edit", "--help"])?.isHelp == true)
      #expect(error(["review", "-h"])?.isHelp == true)
      // `--` ends options; a command's own options pass through.
      #expect(parse(["open", "--", "-odd-name.txt"])?.arguments["path"] == "/repo/sub/-odd-name.txt")
      #expect(parse(["tab", "ls", "-la"])?.arguments == ["command": "ls -la"])
      #expect(parse(["split", "right", "npm", "test", "--", "--watch"])?.arguments["command"] == "npm test -- --watch")
      #expect(parse(["checkpoint", "--", "-v2 rework"])?.arguments["message"] == "-v2 rework")
    }

    @Test func linesRoundTrip() throws {
      let request = ControlRequest(command: "open", token: "t", cwd: "/x", arguments: ["path": "/x/y"], wait: true)
      let line = try ControlProtocol.encodeLine(request)
      #expect(line.last == 0x0A)
      #expect(try ControlProtocol.decode(ControlRequest.self, from: line.dropLast()) == request)
      #expect(ControlProtocol.splitLocation("a:b.swift") == ("a:b.swift", nil, nil))
    }
  }
#endif
