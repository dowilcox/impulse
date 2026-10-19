// The tools `impulse mcp` offers agents. Each one is a control request
// the `impulse` command-line tool can also send (`list_tasks` is `impulse
// tasks`, `create_task` is `impulse tasks new`), so the app answers both the
// same way; this file only names them, describes them to the agent and
// turns their arguments into requests.

import Foundation

public struct MCPTool: Sendable {
  public let name: String
  public let title: String
  public let description: String
  public let inputSchema: JSONValue
  /// Changes nothing (MCP's readOnlyHint).
  public let readOnly: Bool
  /// May remove or overwrite something (destructiveHint); every such tool
  /// asks you in Impulse first.
  public let destructive: Bool
  /// Calling it twice with the same arguments does no more than once.
  public let idempotent: Bool
  /// Waits for something (an agent to stop, you to answer a sheet), so the
  /// server reports progress while it does.
  public let waits: Bool
  let makeRequest: @Sendable ([String: JSONValue], MCPContext) -> Result<ControlRequest, MCPToolError>

  /// How `tools/list` describes it.
  public var listing: JSONValue {
    [
      "name": .string(name),
      "title": .string(title),
      "description": .string(description),
      "inputSchema": inputSchema,
      // No outputSchema: with one, clients reject a result without
      // structured content, and some answers are only text (an older app's,
      // or "Done.").
      "annotations": [
        "title": .string(title),
        "readOnlyHint": .bool(readOnly),
        "destructiveHint": .bool(destructive),
        "idempotentHint": .bool(idempotent),
        "openWorldHint": false,
      ],
    ]
  }

  public func request(_ arguments: [String: JSONValue], context: MCPContext) -> Result<ControlRequest, MCPToolError> {
    makeRequest(arguments, context)
  }
}

/// Who is calling: the pane's token and the server's folder.
public struct MCPContext: Sendable {
  public var token: String?
  public var cwd: String

  public init(token: String?, cwd: String) {
    self.token = token
    self.cwd = cwd
  }
}

public struct MCPToolError: Error, Equatable, Sendable {
  public let message: String
  public init(_ message: String) { self.message = message }
}

public enum MCPTools {
  /// The tools, in the order `tools/list` gives them.
  public static let all: [MCPTool] = tasks + extras + context + taskActions

  public static func named(_ name: String) -> MCPTool? {
    all.first { $0.name == name }
  }

  /// The longest `wait_for_task` waits by default and at most, in seconds.
  /// The default stays under the MCP SDKs' 60-second request timeout, for
  /// agents that don't let progress extend it.
  public static let defaultWait = 50
  public static let maxWait = 600

  // MARK: Tasks

  static let tasks: [MCPTool] = [
    tool(
      "list_tasks", "List tasks",
      """
      The repository's workspaces: its main checkout and every task (a branch in its own worktree folder) \
      Impulse made, with each one's branch, base, how far ahead and behind its base it is, its agent and \
      that agent's state (working, needs input, finished, idle), how many files are uncommitted, and the files it \
      changes that your workspace changes too. Yours is marked isCaller. Check it before merging a branch \
      or making sweeping edits: another agent may be working on the same files.
      """,
      properties: [:], readOnly: true
    ) { _, context in .success(request("tasks", context)) },
    tool(
      "get_task", "Look at a task",
      """
      One workspace in detail: what list_tasks says about it, plus the files it changes against its base, \
      its commits ahead of its base, whether it's merged into its base (squash merges included), its slot \
      and port values, and the brief it was started with. Name it by its folder or branch, as list_tasks shows.
      """,
      properties: ["task": string("The task's folder name or branch.")], required: ["task"], readOnly: true
    ) { arguments, context in
      required("task", arguments).map { task in request("tasks", context, ["show": task]) }
    },
    tool(
      "wait_for_task", "Wait for a task's agent",
      """
      Wait until no agent is working (or waiting for input) in a task, so its branch has stopped moving \
      before you merge it. Answers when that happens or when timeout_seconds pass, saying which; call it \
      again to keep waiting.
      """,
      properties: [
        "task": string("The task's folder name or branch."),
        "timeout_seconds": [
          "type": "integer", "minimum": 1, "maximum": .int(maxWait),
          "description": .string("How long to wait at most (default \(defaultWait))."),
        ],
      ], required: ["task"], readOnly: true, waits: true
    ) { arguments, context in
      required("task", arguments).map { task in
        let timeout = min(maxWait, max(1, arguments["timeout_seconds"]?.int ?? defaultWait))
        return request("tasks", context, ["wait": task, "timeout": String(timeout)], wait: true)
      }
    },
  ]

  // MARK: Showing you things, status, checkpoints

  static let extras: [MCPTool] = [
    tool(
      "open_file", "Open a file for the user",
      """
      Open a file in Impulse's editor, in your window, at a line if given: to show the user where \
      something is. It doesn't take focus from another app. The file must exist.
      """,
      properties: [
        "path": string("The file, absolute or relative to your working directory."),
        "line": ["type": "integer", "minimum": 1],
        "column": ["type": "integer", "minimum": 1],
      ], required: ["path"], idempotent: true
    ) { arguments, context in
      required("path", arguments).map { path in
        var values = ["path": ControlProtocol.absolute(path, cwd: context.cwd), "background": "1"]
        if let line = arguments["line"]?.int, line > 0 { values["line"] = String(line) }
        if let column = arguments["column"]?.int, column > 0 { values["column"] = String(column) }
        return request("open", context, values)
      }
    },
    tool(
      "open_review", "Open Review for the user",
      """
      Open Impulse's Review on changes in your repository, to hand your work to the user: last-turn \
      (what your last turn changed), uncommitted (default), staged or unstaged.
      """,
      properties: [
        "scope": [
          "type": "string", "enum": ["last-turn", "uncommitted", "staged", "unstaged"],
          "description": "Which changes (default uncommitted).",
        ]
      ], idempotent: true
    ) { arguments, context in
      let scope = arguments["scope"]?.string ?? "uncommitted"
      guard ["last-turn", "uncommitted", "staged", "unstaged"].contains(scope) else {
        return .failure(MCPToolError("scope must be last-turn, uncommitted, staged or unstaged."))
      }
      return .success(request("review", context, ["scope": scope, "background": "1"]))
    },
    tool(
      "set_status", "Report your state",
      """
      Tell Impulse what you're doing, for your terminal's tab and the agent list: working, waiting (you \
      need the user), done or idle, with a short message. Only for agents without Impulse's hooks; when \
      hooks report your state, they win and this changes nothing.
      """,
      properties: [
        "state": ["type": "string", "enum": ["working", "waiting", "done", "idle"]],
        "message": string("Shown with the state; empty clears it."),
      ], required: ["state"], idempotent: true
    ) { arguments, context in
      required("state", arguments).flatMap { state in
        guard ["working", "waiting", "done", "idle"].contains(state) else {
          return .failure(MCPToolError("state must be working, waiting, done or idle."))
        }
        return .success(
          request("status", context, ["state": state, "message": arguments["message"]?.string ?? ""]))
      }
    },
    tool(
      "notify", "Notify the user",
      "Flag your terminal for the user's attention and send a desktop notification.",
      properties: ["title": string("The notification's title."), "message": string("Its text.")],
      required: ["title"]
    ) { arguments, context in
      required("title", arguments).map { title in
        request("notify", context, ["title": title, "message": arguments["message"]?.string ?? ""])
      }
    },
    tool(
      "create_checkpoint", "Checkpoint the repository",
      """
      Snapshot the repository (tracked and untracked files, without touching them or the index) under \
      refs/impulse/checkpoints/manual/, before a risky step. Answers with the ref; the user can review or \
      restore it from Impulse.
      """,
      properties: ["message": string("What the checkpoint is for.")]
    ) { arguments, context in
      .success(request("checkpoint", context, ["message": arguments["message"]?.string ?? ""]))
    },
    tool(
      "list_turns", "List your turns",
      """
      The agent turns Impulse recorded in your terminal, oldest first: when each started and ended, and \
      its start and end checkpoint refs, so `git diff <start> <end>` shows what that turn changed.
      """,
      properties: [:], readOnly: true
    ) { _, context in .success(request("turns", context)) },
  ]

  // MARK: What Impulse knows about the workspace

  static let context: [MCPTool] = [
    tool(
      "list_review_comments", "List review comments",
      """
      The user's review comments on this repository's changes, written in Impulse's Review: each one's \
      file and lines, the code it was written on, its text, whether it's outdated (those lines changed \
      since), and whether it's been marked addressed. Use it when asked to address the review.
      """,
      properties: [
        "path": string("Only this file's comments (absolute or relative to your working directory)."),
        "include_outdated": ["type": "boolean", "description": "Include outdated comments (default true)."],
      ], readOnly: true
    ) { arguments, context in
      var values: [String: String] = [:]
      if let path = arguments["path"]?.string, !path.isEmpty {
        values["path"] = ControlProtocol.absolute(path, cwd: context.cwd)
      }
      if arguments["include_outdated"]?.bool == false { values["current"] = "1" }
      return .success(request("comments", context, values))
    },
    tool(
      "resolve_review_comment", "Mark a review comment addressed",
      """
      Mark a review comment addressed, with a note saying what you did. The comment stays: the user \
      sees your note in Review and accepts it (which deletes the comment) or reopens it.
      """,
      properties: [
        "id": string("The comment's id, from list_review_comments."),
        "note": string("What you changed, or why you didn't."),
      ], required: ["id", "note"], idempotent: true
    ) { arguments, context in
      required("id", arguments).flatMap { id in
        required("note", arguments).map { note in request("comments", context, ["resolve": id, "note": note]) }
      }
    },
    tool(
      "get_diagnostics", "Get language-server problems",
      """
      The problems Impulse's language servers report in your workspace (what its Problems panel shows): \
      path, line, column, severity, message, source and code, errors first, at most 200. It names the \
      files whose diagnostics are current; a file it doesn't name is unknown, not clean. It never starts \
      a server: in a folder the user hasn't trusted, none run, and the answer says so.
      """,
      properties: [
        "path": string("Only this file (absolute or relative to your working directory)."),
        "severity": [
          "type": "string", "enum": ["error", "warning", "info", "hint"],
          "description": "This severity and worse (default: all).",
        ],
      ], readOnly: true
    ) { arguments, context in
      var values: [String: String] = [:]
      if let path = arguments["path"]?.string, !path.isEmpty {
        values["path"] = ControlProtocol.absolute(path, cwd: context.cwd)
      }
      if let severity = arguments["severity"]?.string {
        guard ["error", "warning", "info", "hint"].contains(severity) else {
          return .failure(MCPToolError("severity must be error, warning, info or hint."))
        }
        values["severity"] = severity
      }
      return .success(request("problems", context, values))
    },
    tool(
      "list_command_blocks", "List terminal commands",
      """
      The terminals in your workspace (other than yours, and yours too) with their recent commands: each \
      command's block id, text, folder, exit status, start time and duration, and whether it's still \
      running. No output: get it with get_block_output. Use it to see a dev server, test watcher or build \
      the user runs beside you.
      """,
      properties: [:], readOnly: true
    ) { _, context in .success(request("blocks", context)) },
    tool(
      "get_block_output", "Get a command's output",
      "One command's output, from list_command_blocks: the last 200 lines by default.",
      properties: [
        "block": string("The block's id, from list_command_blocks."),
        "lines": ["type": "integer", "minimum": 1, "maximum": 2000, "description": "Last lines to return (default 200)."],
      ], required: ["block"], readOnly: true
    ) { arguments, context in
      required("block", arguments).map { block in
        var values = ["show": block]
        if let lines = arguments["lines"]?.int { values["lines"] = String(min(2000, max(1, lines))) }
        return request("blocks", context, values)
      }
    },
    tool(
      "list_unsaved_files", "List unsaved files",
      """
      Files in your workspace open in Impulse's editor with edits the user hasn't saved, and whether each \
      also changed on disk. Don't rewrite one: the user would have to reconcile it with their edits. Ask \
      them to save it first.
      """,
      properties: [:], readOnly: true
    ) { _, context in .success(request("unsaved", context)) },
    tool(
      "get_project_config", "Get the project's scripts",
      """
      How this repository is set up and checked, from its Impulse project settings: the check and setup \
      scripts, its actions, the commands to run when dependency files change, the env file's name, and \
      for a task its slot and port values. Only from settings the user has trusted; before that, only \
      what Impulse detected (package manager, lock files, a Compose file).
      """,
      properties: [:], readOnly: true
    ) { _, context in .success(request("project", context)) },
  ]

  // MARK: Starting and ending tasks

  static let taskActions: [MCPTool] = [
    tool(
      "create_task", "Start a task",
      """
      Ask Impulse to start a task: a new branch in its own worktree folder beside the repository, opened \
      as a workspace, optionally with an agent started there on a brief you write. Use it to split work \
      into independent pieces that can run in parallel. Impulse shows the user its New Task sheet filled \
      in from this, and waits: the answer is the task they created (with any changes they made) or that \
      they declined. Setup scripts and copied files are the project's own, as for any new task.
      """,
      properties: [
        "title": string("What the task is about; becomes its branch and folder name."),
        "base": string("The branch or commit to start from (default: what New Task would pick, usually origin/main)."),
        "agent": string("The agent to start there: claude, codex, gemini, aider, opencode, amp, copilot, cursor, goose, qwen or crush. Omit for a plain terminal."),
        "brief": string("The agent's first prompt, and the task's description (at most 16 KB)."),
        "move_changes": ["type": "boolean", "description": "Take the main checkout's uncommitted files into the task."],
      ], required: ["title"], waits: true
    ) { arguments, context in
      required("title", arguments).flatMap { title in
        var values = ["new": title]
        if let base = arguments["base"]?.string, !base.isEmpty { values["base"] = base }
        if let agent = arguments["agent"]?.string, !agent.isEmpty { values["agent"] = agent }
        if let brief = arguments["brief"]?.string, !brief.isEmpty {
          guard brief.utf8.count <= ControlProtocol.briefLimit else {
            return .failure(MCPToolError("The brief holds more than 16 KB."))
          }
          values["brief"] = brief
        }
        if arguments["move_changes"]?.bool == true { values["move"] = "1" }
        return .success(request("tasks", context, values, wait: true))
      }
    },
    tool(
      "finish_task", "Finish a task",
      """
      Open Impulse's Finish for a task and run its steps: everything committed, synced with its base, \
      the project's check passed, then landed (merged and pushed, or pushed for review) and cleaned up. \
      Landing (for a task already merged, the clean-up) always waits for the user. Answers with the step \
      it stopped at and why, so you can fix a failing check and call again.
      """,
      properties: ["task": string("The task's folder name or branch.")], required: ["task"], destructive: true,
      waits: true
    ) { arguments, context in
      required("task", arguments).map { task in request("tasks", context, ["finish": task], wait: true) }
    },
    tool(
      "archive_task", "Archive a task",
      """
      Archive a task: close its workspace, snapshot any uncommitted work and remove its folder, keeping \
      its branch. Impulse asks the user first, and offers Undo.
      """,
      properties: ["task": string("The task's folder name or branch.")], required: ["task"], destructive: true,
      waits: true
    ) { arguments, context in
      required("task", arguments).map { task in request("tasks", context, ["archive": task], wait: true) }
    },
  ]

  // MARK: Building blocks

  private static func tool(
    _ name: String, _ title: String, _ description: String, properties: [String: JSONValue],
    required: [String] = [], readOnly: Bool = false, destructive: Bool = false, idempotent: Bool = false,
    waits: Bool = false,
    _ make: @escaping @Sendable ([String: JSONValue], MCPContext) -> Result<ControlRequest, MCPToolError>
  ) -> MCPTool {
    var schema: [String: JSONValue] = [
      "type": "object", "properties": .object(properties), "additionalProperties": false,
    ]
    if !required.isEmpty { schema["required"] = .array(required.map { .string($0) }) }
    return MCPTool(
      name: name, title: title, description: description, inputSchema: .object(schema), readOnly: readOnly,
      destructive: destructive, idempotent: idempotent || readOnly, waits: waits, makeRequest: make)
  }

  private static func string(_ description: String) -> JSONValue {
    ["type": "string", "description": .string(description)]
  }

  private static func required(_ key: String, _ arguments: [String: JSONValue]) -> Result<String, MCPToolError> {
    guard let value = arguments[key]?.string?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty
    else { return .failure(MCPToolError("\(key) is required.")) }
    return .success(value)
  }

  private static func request(
    _ command: String, _ context: MCPContext, _ arguments: [String: String] = [:], wait: Bool = false
  ) -> ControlRequest {
    var arguments = arguments
    // The app tells tool calls from commands typed in a shell by this.
    arguments["via"] = "mcp"
    return ControlRequest(command: command, token: context.token, cwd: context.cwd, arguments: arguments, wait: wait)
  }
}
