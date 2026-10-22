#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct AgentHookInstallerTests {
    private func object(_ data: Data) -> [String: Any] {
      (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    @Test func installsClaudeHooksIntoEmptyOrMissingSettings() throws {
      let fresh = try AgentHookInstaller.installingClaudeHooks(into: nil).get()
      #expect(AgentHookInstaller.claudeHooksInstalled(fresh))
      let hooks = object(fresh)["hooks"] as? [String: Any]
      #expect(Set(hooks?.keys.map { $0 } ?? []) == Set(AgentHookInstaller.claudeEvents))
      #expect(AgentHookInstaller.claudeHooksInstalled(nil) == false)
    }

    @Test func keepsOtherSettingsAndHooksAndDoesNotDuplicate() throws {
      let existing = Data(
        #"""
        {"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
         "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "audit"}]}]}}
        """#.utf8)
      let once = try AgentHookInstaller.installingClaudeHooks(into: existing).get()
      let twice = try AgentHookInstaller.installingClaudeHooks(into: once).get()
      #expect(once == twice)
      let settings = object(twice)
      #expect(settings["model"] as? String == "opus")
      let hooks = settings["hooks"] as? [String: Any]
      #expect((hooks?["Stop"] as? [[String: Any]])?.count == 2)
      // Impulse's own PreToolUse hook (for shell commands) goes beside the user's.
      let preToolUse = hooks?["PreToolUse"] as? [[String: Any]]
      #expect(preToolUse?.count == 2)
      #expect((preToolUse?.first?["hooks"] as? [[String: Any]])?.first?["command"] as? String == "audit")
      #expect(preToolUse?.last?["matcher"] as? String == "Bash")
      #expect((hooks?["PostToolUse"] as? [[String: Any]])?.first?["matcher"] as? String == "Edit|Write|MultiEdit|NotebookEdit|Bash")
    }

    @Test func hooksFromAnOlderImpulseAreOutOfDate() throws {
      let command = AgentHookInstaller.claudeCommand.replacingOccurrences(of: "\"", with: "\\\"")
      let older = Data(
        """
        {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "\(command)"}]}],
         "Stop": [{"hooks": [{"type": "command", "command": "\(command)"}]}]}}
        """.utf8)
      #expect(AgentHookInstaller.claudeHookStatus(older) == .outdated)
      #expect(AgentHookInstaller.claudeHookStatus(nil) == .notInstalled)
      let updated = try AgentHookInstaller.installingClaudeHooks(into: older).get()
      #expect(AgentHookInstaller.claudeHookStatus(updated) == .installed)
      // Everything but StopFailure (Impulse 0.35 and before).
      var settings = object(updated)
      var hooks = settings["hooks"] as? [String: Any] ?? [:]
      hooks["StopFailure"] = nil
      settings["hooks"] = hooks
      let beforeStopFailure = try JSONSerialization.data(withJSONObject: settings)
      #expect(AgentHookInstaller.claudeHookStatus(beforeStopFailure) == .outdated)
    }

    @Test func uninstallRestoresTheRest() throws {
      let existing = Data(#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}, "x": 1}"#.utf8)
      let installed = try AgentHookInstaller.installingClaudeHooks(into: existing).get()
      let removed = try AgentHookInstaller.removingClaudeHooks(from: installed).get()
      let settings = object(removed)
      #expect(settings["x"] as? Int == 1)
      let hooks = settings["hooks"] as? [String: Any]
      #expect(hooks?.keys.sorted() == ["Stop"])
      #expect(AgentHookInstaller.claudeHooksInstalled(removed) == false)
      // Removing from a file that only had ours drops "hooks" entirely.
      let onlyOurs = try AgentHookInstaller.installingClaudeHooks(into: nil).get()
      #expect(object(try AgentHookInstaller.removingClaudeHooks(from: onlyOurs).get())["hooks"] == nil)
    }

    @Test func rejectsUnexpectedShapes() {
      #expect(AgentHookInstaller.installingClaudeHooks(into: Data("[1,2]".utf8)) == .failure(.unreadable("settings.json isn't a JSON object")))
      #expect(AgentHookInstaller.installingClaudeHooks(into: Data(#"{"hooks": 3}"#.utf8)) == .failure(.unreadable("\"hooks\" isn't an object")))
    }

    @Test func codexNotifyGoesAtTheTopAndRespectsExistingPrograms() throws {
      let config = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"mini\"\n"
      let installed = try AgentHookInstaller.installingCodexNotify(into: config).get()
      #expect(installed.hasPrefix("# Tells Impulse"))
      #expect(installed.contains(#"notify = ["sh", "-c", "[ -n \"$IMPULSE_CLI\" ]"#))
      #expect(installed.hasSuffix(config))
      #expect(AgentHookInstaller.codexNotifyInstalled(installed))
      #expect(try AgentHookInstaller.installingCodexNotify(into: installed).get() == installed)
      #expect(
        AgentHookInstaller.installingCodexNotify(into: "notify = [\"growl\"]\n")
          == .failure(.notifyInUse("config.toml already sets notify; add Impulse's command by hand")))
    }

    @Test func codexNotifyRemovalRestoresTheFile() throws {
      let config = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"mini\"\n"
      let installed = try AgentHookInstaller.installingCodexNotify(into: config).get()
      let removed = AgentHookInstaller.removingCodexNotify(from: installed)
      #expect(removed == config)
      #expect(!AgentHookInstaller.codexNotifyInstalled(removed))
      // A file that only had Impulse's notify ends up empty.
      let onlyOurs = try AgentHookInstaller.installingCodexNotify(into: nil).get()
      #expect(AgentHookInstaller.removingCodexNotify(from: onlyOurs) == "")
      // Someone else's notify program stays.
      let other = "notify = [\"growl\"]\nmodel = \"o3\"\n"
      #expect(AgentHookInstaller.removingCodexNotify(from: other) == other)
    }

    @Test func opencodePluginIsAFolderReplacedWholeAndOnlyIfItsImpulses() throws {
      typealias Found = AgentHookInstaller.OpencodePlugin
      let files = AgentHookInstaller.opencodePluginFiles
      #expect(files.keys.sorted() == ["rpc.js", "server.js", "tui.js"])
      for (name, text) in files { #expect(AgentHookInstaller.isOpencodePluginFile(text), "\(name)") }
      #expect(AgentHookInstaller.opencodePluginStatus(Found()) == .notInstalled)
      #expect(AgentHookInstaller.opencodePluginStatus(Found(files: files)) == .installed)
      // opencode 1's single file, from an older Impulse: replaced by the folder.
      let single = AgentHookInstaller.opencodePluginMarker + ", from an older Impulse\nexport const X = 1\n"
      #expect(AgentHookInstaller.opencodePluginStatus(Found(legacy: single)) == .outdated)
      #expect(AgentHookInstaller.opencodePluginStatus(Found(files: files, legacy: single)) == .outdated)
      let theirs = "export const Mine = async () => ({})\n"
      #expect(AgentHookInstaller.opencodePluginStatus(Found(files: files, legacy: theirs)) == .installed)
      // An older folder: updated whole.
      var older = files
      older["server.js"] = AgentHookInstaller.opencodePluginMarker + ", older\n"
      older["gone.js"] = AgentHookInstaller.opencodePluginMarker + ", a file this version doesn't have\n"
      #expect(AgentHookInstaller.opencodePluginStatus(Found(files: older)) == .outdated)
      #expect(try AgentHookInstaller.installingOpencodePlugin(over: Found(files: older)).get() == files)
      #expect(try AgentHookInstaller.installingOpencodePlugin(over: Found()).get() == files)
      // A folder holding someone else's file is left alone.
      #expect(AgentHookInstaller.opencodePluginStatus(Found(files: ["index.js": theirs])) == .notInstalled)
      #expect((try? AgentHookInstaller.installingOpencodePlugin(over: Found(files: ["index.js": theirs])).get()) == nil)
      // The preview names each file.
      let text = AgentHookInstaller.opencodePluginText(Found(files: files, legacy: single))
      for name in ["impulse/rpc.js", "impulse/server.js", "impulse/tui.js", "impulse.js"] {
        #expect(text.contains("// \(name)\n"), "\(name)")
      }
    }

    @Test func opencodePluginSendsTheEventsImpulseKnows() throws {
      let tui = try #require(AgentHookInstaller.opencodePluginFiles["tui.js"])
      let server = try #require(AgentHookInstaller.opencodePluginFiles["server.js"])
      // The terminal's half is a no-op outside Impulse and calls the CLI by
      // its path (no shell); the service's half never trusts its own
      // environment, which is whichever terminal started the service.
      #expect(tui.contains("process.env.IMPULSE_CLI"))
      #expect(tui.contains("if (!cli) return;"))
      #expect(tui.contains(#"spawn(cli, ["hook", "opencode", event]"#))
      #expect(!server.contains("process.env.IMPULSE"))
      #expect(!server.contains("spawn("), "only the terminal's half runs impulse")
      for event in [
        "start", "session.start", "session.execution.started", "session.execution.succeeded",
        "session.execution.failed", "session.execution.interrupted", "permission.asked", "permission.replied",
        "form.created", "form.replied", "form.cancelled", "tool.execute.before", "tool.execute.after",
      ] {
        #expect(tui.contains("\"\(event)\""), "\(event)")
        let known =
          AgentHookEvents.hook(agent: "opencode", event: event, tool: "shell", message: nil) != nil
          || AgentHookEvents.reply(agent: "opencode", event: event, tool: "shell") != nil
        #expect(known, "\(event)")
      }
      for tool in AgentHookEvents.opencodeEditTools { #expect(server.contains("\"\(tool)\""), "\(tool)") }
      // The variables a shell command gets from its terminal are the ones
      // the terminal sends.
      func variables(_ text: String) -> [String] {
        guard let start = text.range(of: "const paneVariables = ["), let end = text[start.upperBound...].range(of: "];")
        else { return [] }
        return text[start.upperBound..<end.lowerBound].components(separatedBy: "\"").enumerated()
          .filter { $0.offset % 2 == 1 }.map(\.element)
      }
      #expect(variables(server) == variables(tui))
      #expect(variables(server).contains("IMPULSE_PANE_TOKEN"))
      // opencode loads a plugin module's default export.
      for text in [server, tui] { #expect(text.components(separatedBy: "export default").count == 2) }
    }

    @Test func diffShowsAddedAndRemovedLines() {
      let lines = AgentHookInstaller.diffLines(before: "a\nb\nc", after: "a\nB\nc\nd")
      #expect(lines == ["  a", "- b", "+ B", "  c", "+ d"])
    }
  }
#endif
