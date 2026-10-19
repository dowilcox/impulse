#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit
  import ImpulseProtocol

  /// What Impulse answers agents with (the MCP tools and their `impulse`
  /// commands), whether it starts a task an agent asks for, and how it
  /// registers its MCP server.
  struct AgentToolTests {
    private let task = TaskListing.Workspace(
      name: "fix-elevation", path: "/code/trailhead.worktrees/fix-elevation", branch: "fix-elevation",
      base: "origin/main", head: "9b1d4e07", isMainCheckout: false, isCaller: false, agent: "Claude Code",
      agentState: "working", uncommitted: 1, ahead: 3, behind: 0, createdBy: "agent",
      requestedBy: "Codex in trailhead (main checkout)", brief: "Fix elevation units")

    // MARK: Answers

    @Test func taskDetailText() {
      let detail = TaskDetail(
        workspace: task, changedFiles: ["src/lib/units.ts"], uncommittedFiles: ["notes.md"],
        commits: [.init(id: "9b1d4e07aa", subject: "Use feet")], moreCommits: 2, merged: false, slot: 1,
        ports: ["APP_PORT": 8100], brief: "Fix elevation units\nOn the trail page.")
      let text = detail.text()
      #expect(text.hasPrefix("fix-elevation (task)"))
      #expect(text.contains("  started at the request of Codex in trailhead (main checkout)"))
      #expect(text.contains("  not merged into its base yet"))
      #expect(text.contains("  slot 1: APP_PORT 8100"))
      #expect(text.contains("  9b1d4e0 Use feet\n  …and 2 more"))
      #expect(text.contains("Changed against its base:\n  src/lib/units.ts"))
      #expect(text.contains("Brief:\n  Fix elevation units\n  On the trail page."))
      let decoded = try? JSONDecoder().decode(TaskDetail.self, from: AgentToolJSON.data(detail))
      #expect(decoded == detail)
    }

    @Test func listingsCarryProvenance() throws {
      let listing = TaskListing(repository: "trailhead", workspaces: [task])
      #expect(listing.text().contains("  brief: Fix elevation units"))
      let decoded = try JSONDecoder().decode(TaskListing.self, from: Data(listing.json().utf8))
      #expect(decoded.workspaces[0].createdBy == "agent")
      // A listing from before provenance reads as before.
      let old = #"{"repository":"r","workspaces":[{"name":"r","path":"/r","isMainCheckout":true,"isCaller":true,"uncommitted":0,"sharedWithCaller":[]}]}"#
      #expect(try JSONDecoder().decode(TaskListing.self, from: Data(old.utf8)).workspaces[0].createdBy == nil)
    }

    @Test func taskRecordsWithoutProvenanceStillDecode() throws {
      let old = #"{"tasks":[{"path":"/t","branch":"t","created":"2026-10-01T00:00:00Z"}]}"#
      let decoder = JSONDecoder()
      decoder.dateDecodingStrategy = .iso8601
      let registry = try decoder.decode(TaskRegistry.self, from: Data(old.utf8))
      #expect(registry.tasks[0].createdBy == nil)
      #expect(registry.agentTaskCount == 0)
      var withAgent = registry
      withAgent.add(TaskRecord(path: "/u", branch: "u", createdBy: "agent", requestedBy: "Claude Code in r"))
      #expect(withAgent.agentTaskCount == 1)
      #expect(withAgent.record(named: "u")?.branch == "u")
      #expect(withAgent.record(named: "t")?.path == "/t")
    }

    @Test func commentsListWithResolutionAndFreshness() throws {
      var comment = ReviewComment(
        id: "c1", path: "src/a.ts", side: .new, line: 2, endLine: 2, snippet: "let x = 1", text: "Why 1?")
      #expect(CommentFreshness.isOutdated(comment, currentText: "a\nlet x = 1\nb") == false)
      #expect(CommentFreshness.isOutdated(comment, currentText: "a\nlet x = 2\nb") == true)
      #expect(CommentFreshness.isOutdated(comment, currentText: nil) == true)
      let removed = ReviewComment(path: "src/a.ts", side: .old, line: 1, endLine: 1, snippet: "y", text: "Gone?")
      #expect(CommentFreshness.isOutdated(removed, currentText: "z") == nil)

      comment.resolution = .init(by: "Claude Code", note: "Made it a constant.")
      let list = CommentList(repository: "trailhead", comments: [.init(comment, outdated: false)])
      let text = list.text()
      #expect(text.contains("[c1] src/a.ts:2 (addressed by Claude Code)"))
      #expect(text.contains("Addressed: Made it a constant."))
      // Comments saved before resolutions existed.
      let decoder = JSONDecoder()
      decoder.dateDecodingStrategy = .iso8601
      let saved = #"[{"id":"x","path":"p","side":"new","line":1,"endLine":1,"snippet":"","text":"t","createdAt":"2026-10-01T00:00:00Z"}]"#
      #expect(try decoder.decode([ReviewComment].self, from: Data(saved.utf8))[0].resolution == nil)
    }

    @Test func problemsAreScopedSortedAndLimited() {
      var problems = [
        Problem(path: "/w/b.ts", line: 3, column: 1, severity: .warning, message: "unused"),
        Problem(path: "/w/a.ts", line: 9, column: 2, severity: .error, message: "type", source: "ts", code: "2322"),
        Problem(path: "/other/c.ts", line: 1, column: 1, severity: .error, message: "elsewhere"),
      ]
      var list = ProblemList(problems: problems, root: "/w", checkedFiles: ["/w/a.ts", "/w/b.ts"])
      #expect(list.problems.map(\.path) == ["a.ts", "b.ts"])
      #expect(list.counts.errors == 1 && list.counts.warnings == 1)
      #expect(list.text().contains("a.ts:9:2 error [ts 2322]: type"))
      list = ProblemList(problems: problems, root: "/w", minimum: .error, checkedFiles: [])
      #expect(list.problems.count == 1)
      for index in 0..<(ProblemList.limit + 5) {
        problems.append(Problem(path: "/w/many.ts", line: index, column: 1, severity: .hint, message: "h"))
      }
      list = ProblemList(problems: problems, root: "/w", checkedFiles: [])
      #expect(list.problems.count == ProblemList.limit)
      #expect(list.truncated == 7)
      let empty = ProblemList(problems: [], root: "/w", checkedFiles: [], note: "Language servers don't run here.")
      #expect(empty.text() == "Language servers don't run here.")
    }

    @Test func blockOutputKeepsTheTail() {
      let output = (1...300).map { "line \($0)" }.joined(separator: "\n") + "\n"
      let block = BlockOutput(id: "ab:1", command: "npm test", folder: "/w", exitCode: 1, running: false, output: output, lines: 200)
      #expect(block.totalLines == 300)
      #expect(block.shownLines == 200)
      #expect(block.output.hasPrefix("line 101\n"))
      #expect(block.text().hasPrefix("`npm test` in /w (exit status 1), last 200 of 300 lines:"))
      #expect(BlockOutput(id: "x", command: nil, folder: nil, exitCode: 0, running: false, output: "", lines: 5).text()
        == "`a command` (succeeded): no output.")
      #expect(BlockList.parse("ab12cd34:1042")?.terminal == "ab12cd34")
      #expect(BlockList.parse("ab12cd34:1042")?.block == 1042)
      #expect(BlockList.parse("1042") == nil)
    }

    @Test func projectSummaryListsCommandsOnlyWhenTrusted() {
      var config = ProjectConfig()
      config.checkScript = "npm test"
      config.onChange = ["package-lock.json": "npm ci"]
      config.ports = ["APP_PORT": 8000]
      let detected = ProjectSummary.Detected(lockFiles: ["package-lock.json"], install: ["npm ci"], composeFile: nil)
      let trusted = ProjectSummary(config: config, trusted: true, files: [".impulse/project.toml"], slot: 2, detected: detected)
      #expect(trusted.check == "npm test")
      #expect(trusted.onChange == [.init(file: "package-lock.json", command: "npm ci")])
      #expect(trusted.ports["APP_PORT"] == 8200)
      let untrusted = ProjectSummary(config: config, trusted: false, files: [".impulse/project.toml"], slot: 2, detected: detected)
      #expect(untrusted.check == nil && untrusted.onChange.isEmpty && untrusted.ports.isEmpty)
      #expect(untrusted.text().contains("lock files: package-lock.json (install: npm ci)"))
    }

    // MARK: Agents' task requests

    @Test func whenAnAgentsTaskAsksFirst() {
      func decide(
        _ setting: AgentTaskPolicy.Setting, trusted: Bool = true, project: Bool = true, tasks: Int = 0, limit: Int = 4,
        chain: Bool = false, moves: Bool = false
      ) -> AgentTaskPolicy.Decision {
        AgentTaskPolicy.decide(
          setting: setting, repositoryTrusted: trusted, projectTrusted: project, agentTasks: tasks, limit: limit,
          fromAgentTask: chain, movesChanges: moves)
      }
      #expect(decide(.ask) == .ask)
      #expect(decide(.trustedProjects) == .create)
      #expect(decide(.trustedProjects, trusted: false) == .ask)
      #expect(decide(.trustedProjects, project: false) == .ask)
      #expect(decide(.trustedProjects, chain: true) == .ask)
      #expect(decide(.trustedProjects, moves: true) == .ask)
      if case .refuse = decide(.off) {} else { Issue.record("off refuses") }
      if case .refuse(let why) = decide(.ask, tasks: 4) {
        #expect(why.contains("the limit is 4"))
      } else {
        Issue.record("the limit refuses")
      }
      #expect(decide(.ask, tasks: 40, limit: 0) == .ask, "0: no limit")
      #expect(AgentTaskPolicy.Setting(setting: "nonsense") == .ask)
    }

    @Test func theBriefStartsTheAgentFromItsFile() {
      let claude = KnownAgents.builtIn.first { $0.id == "claude" }!
      let aider = KnownAgents.builtIn.first { $0.id == "aider" }!
      let path = "/repo/.git/impulse/tasks/fix it/brief.md"
      #expect(AgentTaskPolicy.command(agent: claude, briefPath: path, shell: .zsh) == #"claude -- "$(command cat '/repo/.git/impulse/tasks/fix it/brief.md')""#)
      #expect(AgentTaskPolicy.command(agent: claude, briefPath: path, shell: .fish) == "claude -- (command cat '/repo/.git/impulse/tasks/fix it/brief.md' | string collect)")
      #expect(AgentTaskPolicy.command(agent: claude, briefPath: nil, shell: .zsh) == "claude")
      #expect(AgentTaskPolicy.command(agent: aider, briefPath: path, shell: .zsh) == nil, "typed in once it's up")
    }

    @Test func briefsAreKeptBesideTheRegistry() throws {
      let common = NSTemporaryDirectory() + "impulse-brief-\(UUID().uuidString)/.git"
      defer { try? FileManager.default.removeItem(atPath: (common as NSString).deletingLastPathComponent) }
      #expect(TaskBrief.write("Do the thing.\nCarefully.", task: "t", commonGitDirectory: common))
      #expect(TaskBrief.path(task: "t", commonGitDirectory: common) == common + "/impulse/tasks/t/brief.md")
      #expect(TaskBrief.read(task: "t", commonGitDirectory: common) == "Do the thing.\nCarefully.")
      #expect(TaskBrief.summary("\n\n  First line  \nsecond") == "First line")
      #expect(TaskBrief.summary(String(repeating: "x", count: 200), limit: 10) == "xxxxxxxxx…")
    }

    // MARK: Registering the server

    private let cli = "/Applications/Impulse.app/Contents/Resources/bin/impulse"

    @Test func theLauncherFallsBackToTheAppsCLI() {
      #expect(AgentMCPInstaller.launcher(fallbackCLI: cli) == #"exec "${IMPULSE_CLI:-/Applications/Impulse.app/Contents/Resources/bin/impulse}" mcp"#)
      #expect(AgentMCPInstaller.launcher(fallbackCLI: #"/A "b"/$x"#) == #"exec "${IMPULSE_CLI:-/A \"b\"/\$x}" mcp"#)
      let args = AgentMCPInstaller.claudeAddArguments(scope: .user, fallbackCLI: cli)
      #expect(Array(args.prefix(5)) == ["mcp", "add-json", "--scope", "user", "impulse"])
      let entry = try? JSONSerialization.jsonObject(with: Data(args[5].utf8)) as? [String: Any]
      #expect(entry?["command"] as? String == "sh")
      #expect((entry?["env"] as? [String: String])?["IMPULSE_PANE_TOKEN"] == "${IMPULSE_PANE_TOKEN:-}")
    }

    @Test func claudeStatusReadsItsConfig() {
      let current = AgentMCPInstaller.claudeServer(fallbackCLI: cli)
      func config(_ servers: [String: Any], project: String? = nil) -> Data {
        var object: [String: Any] = ["numStartups": 3]
        if let project {
          let entry: [String: Any] = ["mcpServers": servers]
          object["projects"] = [project: entry]
        } else {
          object["mcpServers"] = servers
        }
        return try! JSONSerialization.data(withJSONObject: object)
      }
      #expect(AgentMCPInstaller.claudeStatus(nil, scope: .user, project: nil, fallbackCLI: cli) == .notInstalled)
      #expect(AgentMCPInstaller.claudeStatus(config(["impulse": current]), scope: .user, project: nil, fallbackCLI: cli) == .installed)
      #expect(AgentMCPInstaller.claudeStatus(config(["impulse": current]), scope: .user, project: nil, fallbackCLI: "/elsewhere") == .outdated)
      #expect(AgentMCPInstaller.claudeStatus(config(["impulse": current], project: "/r"), scope: .local, project: "/r", fallbackCLI: cli) == .installed)
      #expect(AgentMCPInstaller.claudeStatus(config(["impulse": current], project: "/r"), scope: .user, project: "/r", fallbackCLI: cli) == .notInstalled)
    }

    @Test func codexTableInstallsAndRemovesCleanly() throws {
      let original = "model = \"o3\"\n\n[profiles.fast]\nmodel = \"o4-mini\"\n"
      let installed = try AgentMCPInstaller.installingCodexMCP(into: original, fallbackCLI: cli).get()
      #expect(installed.hasPrefix(original))
      #expect(installed.contains("[mcp_servers.impulse]\ncommand = \"sh\""))
      #expect(installed.contains(#"env_vars = ["IMPULSE_SOCKET", "IMPULSE_PANE_TOKEN", "IMPULSE_CLI"]"#))
      #expect(AgentMCPInstaller.codexStatus(installed, fallbackCLI: cli) == .installed)
      #expect(AgentMCPInstaller.codexStatus(installed, fallbackCLI: "/elsewhere") == .outdated)
      #expect(AgentMCPInstaller.removingCodexMCP(from: installed) == original)
      // Installing again replaces Impulse's own table.
      let again = try AgentMCPInstaller.installingCodexMCP(into: installed, fallbackCLI: "/elsewhere").get()
      #expect(again.components(separatedBy: "[mcp_servers.impulse]").count == 2)
      // From nothing, and back.
      let fresh = try AgentMCPInstaller.installingCodexMCP(into: nil, fallbackCLI: cli).get()
      #expect(AgentMCPInstaller.removingCodexMCP(from: fresh) == "")
      // A table the user wrote is theirs.
      let theirs = "[mcp_servers.impulse]\ncommand = \"impulse\"\n"
      #expect((try? AgentMCPInstaller.installingCodexMCP(into: theirs, fallbackCLI: cli).get()) == nil)
      #expect(AgentMCPInstaller.removingCodexMCP(from: theirs) == nil)
      #expect(AgentMCPInstaller.codexStatus(theirs, fallbackCLI: cli) == .outdated)
    }

    @Test func codexServerDefinedOtherwiseIsLeftAlone() throws {
      // Another spelling of the table, a dotted key, or a key in
      // [mcp_servers]: a second definition would stop Codex reading the file.
      for theirs in [
        "[mcp_servers.\"impulse\"]\ncommand = \"impulse\"\n",
        "[ mcp_servers.impulse ] # mine\ncommand = \"impulse\"\n",
        "mcp_servers.impulse.command = \"impulse\"\n",
        "[mcp_servers]\nimpulse = { command = \"impulse\" }\n",
      ] {
        #expect((try? AgentMCPInstaller.installingCodexMCP(into: theirs, fallbackCLI: cli).get()) == nil, "\(theirs)")
        #expect(AgentMCPInstaller.codexStatus(theirs, fallbackCLI: cli) == .outdated, "\(theirs)")
        #expect(AgentMCPInstaller.removingCodexMCP(from: theirs) == nil, "\(theirs)")
      }
      // Other servers are someone else's business.
      let others = "[mcp_servers]\nimpulsive = { command = \"x\" }\n\n[mcp_servers.docs]\ncommand = \"d\"\n"
      #expect(AgentMCPInstaller.codexStatus(others, fallbackCLI: cli) == .notInstalled)
      #expect((try? AgentMCPInstaller.installingCodexMCP(into: others, fallbackCLI: cli).get()) != nil)
    }

    @Test func codexTableInACRLFFile() throws {
      let original = "model = \"o3\"\r\n"
      let installed = try AgentMCPInstaller.installingCodexMCP(into: original, fallbackCLI: cli).get()
      let crlf = installed.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\n", with: "\r\n")
      #expect(AgentMCPInstaller.codexStatus(crlf, fallbackCLI: cli) == .installed)
      let removed = try #require(AgentMCPInstaller.removingCodexMCP(from: crlf))
      #expect(!removed.contains("mcp_servers"))
      #expect(removed.hasPrefix("model = \"o3\""))
      // Installing again doesn't add a second table.
      let again = try AgentMCPInstaller.installingCodexMCP(into: crlf, fallbackCLI: "/elsewhere").get()
      #expect(again.components(separatedBy: "[mcp_servers.impulse]").count == 2)
    }

    @Test func createTaskNamesEveryKnownAgent() {
      let description = MCPTools.named("create_task")?.inputSchema["properties"]?["agent"]?["description"]?.string ?? ""
      for agent in KnownAgents.builtIn {
        #expect(description.contains(agent.id), "\(agent.id)")
      }
    }
  }
#endif
