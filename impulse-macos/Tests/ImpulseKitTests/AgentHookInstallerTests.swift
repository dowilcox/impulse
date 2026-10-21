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

    @Test func opencodePluginIsReplacedWholeAndOnlyIfItsImpulses() throws {
      let plugin = AgentHookInstaller.opencodePlugin
      #expect(plugin.hasPrefix(AgentHookInstaller.opencodePluginMarker))
      #expect(AgentHookInstaller.opencodePluginStatus(nil) == .notInstalled)
      #expect(AgentHookInstaller.opencodePluginStatus(plugin) == .installed)
      let older = AgentHookInstaller.opencodePluginMarker + ", from an older Impulse\nexport const X = 1\n"
      #expect(AgentHookInstaller.opencodePluginStatus(older) == .outdated)
      #expect(try AgentHookInstaller.installingOpencodePlugin(over: older).get() == plugin)
      #expect(try AgentHookInstaller.installingOpencodePlugin(over: nil).get() == plugin)
      let theirs = "export const Mine = async () => ({})\n"
      #expect(AgentHookInstaller.opencodePluginStatus(theirs) == .notInstalled)
      #expect((try? AgentHookInstaller.installingOpencodePlugin(over: theirs).get()) == nil)
    }

    @Test func opencodePluginSendsTheEventsImpulseKnows() {
      let plugin = AgentHookInstaller.opencodePlugin
      // A no-op outside Impulse, calling the CLI by its path (no shell).
      #expect(plugin.contains("process.env.IMPULSE_CLI"))
      #expect(plugin.contains(#"if (!cli) return {};"#))
      #expect(plugin.contains(#"spawn(cli, ["hook", "opencode", event]"#))
      for event in [
        "start", "session.start", "chat.message", "session.idle", "permission.asked", "permission.replied",
        "question.asked", "question.replied", "question.rejected", "tool.execute.before", "tool.execute.after",
      ] {
        #expect(plugin.contains("\"\(event)\""), "\(event)")
        let known =
          AgentHookEvents.hook(agent: "opencode", event: event, tool: "bash", message: nil) != nil
          || AgentHookEvents.reply(agent: "opencode", event: event, tool: "bash") != nil
        #expect(known, "\(event)")
      }
      for tool in AgentHookEvents.opencodeEditTools { #expect(plugin.contains("\"\(tool)\""), "\(tool)") }
      // Only one export: opencode calls every export as a plugin.
      #expect(plugin.components(separatedBy: "export ").count == 2)
    }

    @Test func diffShowsAddedAndRemovedLines() {
      let lines = AgentHookInstaller.diffLines(before: "a\nb\nc", after: "a\nB\nc\nd")
      #expect(lines == ["  a", "- b", "+ B", "  c", "+ d"])
    }
  }
#endif
