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
