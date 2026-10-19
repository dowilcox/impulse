import AppKit
import ImpulseKit
import SwiftUI

/// Installs (or removes) the hooks that let agents report their state to
/// Impulse, and Impulse's MCP server (`impulse mcp`), showing the exact
/// change to the agent's config first.
@Observable
final class AgentHooksModel {
  enum Agent: String, CaseIterable, Identifiable {
    case claude = "Claude Code"
    case codex = "Codex"
    case opencode = "opencode"
    /// Agents Impulse doesn't configure: the MCP entry to copy.
    case other = "Other"
    var id: String { rawValue }
  }

  /// What's being set up.
  enum Part: String, CaseIterable, Identifiable {
    case hooks = "Hooks"
    case mcp = "MCP Server"
    var id: String { rawValue }
  }

  enum Scope: String, CaseIterable, Identifiable {
    case user = "All projects"
    case project = "This project only"
    var id: String { rawValue }
  }

  var part: Part = .hooks {
    didSet {
      if part == .hooks, agent == .other { agent = .claude } else { reload() }
    }
  }
  var agent: Agent = .claude { didSet { reload() } }
  var scope: Scope = .user { didSet { reload() } }
  /// The bundled CLI's path, for the MCP entry's fallback outside Impulse.
  let fallbackCLI: String
  /// For Claude Code's MCP server: the `claude` commands that make the
  /// change, in order (Claude Code rewrites its own config file while it
  /// runs, so Impulse doesn't edit it); nil when `claude` isn't on PATH.
  private(set) var claudeCommands: [[String]]?
  @ObservationIgnored private var claudeExecutable: String?
  /// The project for project-local Claude hooks, if any.
  let projectRoot: String?
  /// The repository's main checkout, when `projectRoot` is a task worktree
  /// (whose project hooks go away when it's archived).
  let mainCheckout: String?

  private(set) var path = ""
  private(set) var before = ""
  /// The file's bytes when the preview was made (nil: no file), to tell
  /// whether it changed since.
  @ObservationIgnored private var previewed: Data?
  private(set) var after: String?
  private(set) var installed = false
  /// Some of Impulse's hooks, from an older Impulse: Install adds the rest.
  private(set) var outdated = false
  private(set) var error: String?
  /// The file changed after the preview was made, which now shows the
  /// new content.
  private(set) var refreshed = false

  init(projectRoot: String?, mainCheckout: String? = nil, fallbackCLI: String = AgentMCPInstaller.defaultCLI) {
    self.projectRoot = projectRoot
    self.mainCheckout = mainCheckout
    self.fallbackCLI = fallbackCLI
    reload()
  }

  var diff: [String] {
    if part == .mcp, agent == .other {
      return AgentMCPInstaller.genericSnippet(fallbackCLI: fallbackCLI).components(separatedBy: "\n")
    }
    if part == .mcp, agent == .claude {
      return (claudeCommands ?? []).map {
        "$ " + (["claude"] + $0).map { ShellQuoting.quoted($0, for: .zsh) }.joined(separator: " ")
      }
    }
    guard let after else { return [] }
    return AgentHookInstaller.diffLines(before: before, after: after)
  }

  /// What the sheet installs, in its messages.
  var what: String { part == .hooks ? "hooks" : "MCP server" }

  /// opencode's hooks are a plugin file of Impulse's own: Remove deletes it.
  private var removesFile: Bool { part == .hooks && agent == .opencode && installed }

  private static var opencodeDirectory: String {
    AgentMCPInstaller.opencodeConfigDirectory(environment: ProcessInfo.processInfo.environment, home: NSHomeDirectory())
  }

  func reload() {
    // Other agents only have the MCP entry to copy.
    if part == .mcp || agent == .other { return reloadMCP() }
    let home = NSHomeDirectory()
    switch agent {
    case .claude:
      path =
        scope == .project && projectRoot != nil
        ? (projectRoot! as NSString).appendingPathComponent(".claude/settings.local.json")
        : home + "/.claude/settings.json"
    case .opencode:
      path = Self.opencodeDirectory + "/plugins/" + AgentHookInstaller.opencodePluginName
    case .codex, .other:
      path = home + "/.codex/config.toml"
    }
    let data = FileManager.default.contents(atPath: path)
    previewed = data
    before = data.map { String(decoding: $0, as: UTF8.self) } ?? ""
    error = nil
    refreshed = false
    switch agent {
    case .claude:
      let status = AgentHookInstaller.claudeHookStatus(data)
      installed = status == .installed
      outdated = status == .outdated
      switch installed
        ? (data.map { AgentHookInstaller.removingClaudeHooks(from: $0) } ?? .success(Data()))
        : AgentHookInstaller.installingClaudeHooks(into: data)
      {
      case .success(let result): after = String(decoding: result, as: UTF8.self)
      case .failure(let failure): fail(failure)
      }
    case .opencode:
      let status = AgentHookInstaller.opencodePluginStatus(data == nil ? nil : before)
      installed = status == .installed
      outdated = status == .outdated
      if installed {
        after = ""
      } else {
        switch AgentHookInstaller.installingOpencodePlugin(over: data == nil ? nil : before) {
        case .success(let text): after = text
        case .failure(let failure): fail(failure)
        }
      }
    case .codex, .other:
      installed = AgentHookInstaller.codexNotifyInstalled(before)
      outdated = false
      if installed {
        let removed = AgentHookInstaller.removingCodexNotify(from: before)
        after = removed == before ? nil : removed
        if after == nil { error = "Impulse's notify isn't on a line of its own; remove it from config.toml by hand." }
      } else {
        switch AgentHookInstaller.installingCodexNotify(into: data == nil ? nil : before) {
        case .success(let text): after = text
        case .failure(let failure): fail(failure)
        }
      }
    }
  }

  private func reloadMCP() {
    let home = NSHomeDirectory()
    error = nil
    refreshed = false
    outdated = false
    claudeCommands = nil
    switch agent {
    case .claude:
      path = home + "/.claude.json"
      let data = FileManager.default.contents(atPath: path)
      previewed = data
      before = ""
      let scope: AgentMCPInstaller.ClaudeScope = self.scope == .project && projectRoot != nil ? .local : .user
      let status = AgentMCPInstaller.claudeStatus(
        data, scope: scope, project: projectRoot, fallbackCLI: fallbackCLI)
      installed = status == .installed
      outdated = status == .outdated
      claudeExecutable = LoginShell.which("claude")
      if claudeExecutable == nil {
        after = nil
        error = "The claude command isn't on PATH. Install Claude Code, or run: claude "
          + AgentMCPInstaller.claudeAddArguments(scope: scope, fallbackCLI: fallbackCLI)
          .map { ShellQuoting.quoted($0, for: .zsh) }.joined(separator: " ")
      } else {
        let remove = AgentMCPInstaller.claudeRemoveArguments(scope: scope)
        let add = AgentMCPInstaller.claudeAddArguments(scope: scope, fallbackCLI: fallbackCLI)
        // `add-json` refuses a name that's taken: an older entry goes first.
        claudeCommands = installed ? [remove] : outdated ? [remove, add] : [add]
        after = ""
      }
    case .codex:
      path = home + "/.codex/config.toml"
      let data = FileManager.default.contents(atPath: path)
      previewed = data
      before = data.map { String(decoding: $0, as: UTF8.self) } ?? ""
      let status = AgentMCPInstaller.codexStatus(data == nil ? nil : before, fallbackCLI: fallbackCLI)
      installed = status == .installed
      outdated = status == .outdated
      if installed {
        after = AgentMCPInstaller.removingCodexMCP(from: before)
        if after == nil { error = "The [mcp_servers.impulse] table isn't the one Impulse wrote; remove it from config.toml by hand." }
      } else {
        switch AgentMCPInstaller.installingCodexMCP(into: data == nil ? nil : before, fallbackCLI: fallbackCLI) {
        case .success(let text): after = text
        case .failure(let failure): fail(failure)
        }
      }
    case .opencode:
      path = AgentMCPInstaller.opencodeConfigFile(directory: Self.opencodeDirectory)
      let data = FileManager.default.contents(atPath: path)
      previewed = data
      before = data.map { String(decoding: $0, as: UTF8.self) } ?? ""
      let status = AgentMCPInstaller.opencodeStatus(data == nil ? nil : before, fallbackCLI: fallbackCLI)
      installed = status == .installed
      outdated = status == .outdated
      if installed {
        after = AgentMCPInstaller.removingOpencodeMCP(from: before)
        if after == nil { error = "The \"impulse\" server isn't the one Impulse wrote; remove it from the file by hand." }
      } else {
        switch AgentMCPInstaller.installingOpencodeMCP(into: data == nil ? nil : before, fallbackCLI: fallbackCLI) {
        case .success(let text): after = text
        case .failure(let failure): fail(failure)
        }
      }
    case .other:
      path = ""
      previewed = nil
      before = ""
      installed = false
      after = nil
    }
  }

  private func fail(_ failure: AgentHookInstaller.InstallError) {
    after = nil
    switch failure {
    case .unreadable(let message), .notifyInUse(let message): error = message
    }
  }

  enum Outcome: Equatable {
    case written
    /// The file changed since the preview: it's been redone, for another
    /// look.
    case changed
    case failed(String)
  }

  /// `apply`, or for Claude Code's MCP server the `claude` command, off
  /// the main thread.
  func applyInBackground(completion: @escaping (Outcome) -> Void) {
    guard part == .mcp, agent == .claude else { return completion(apply()) }
    guard let executable = claudeExecutable, let commands = claudeCommands, !commands.isEmpty else {
      return completion(.failed(error ?? "Nothing to change."))
    }
    let directory = projectRoot ?? NSHomeDirectory()
    DispatchQueue.global(qos: .userInitiated).async {
      var environment = ProcessInfo.processInfo.environment
      environment["PATH"] = LoginShell.loginPath()
      var outcome = Outcome.written
      for arguments in commands {
        do {
          let output = try ChildProcess.run(
            executable, arguments, in: directory, environment: environment, timeout: 30)
          guard output.status != 0 else { continue }
          let message = String(decoding: output.stderr.isEmpty ? output.stdout : output.stderr, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
          outcome = .failed(message.isEmpty ? "claude exited with status \(output.status)" : message)
        } catch {
          outcome = .failed(error.localizedDescription)
        }
        break
      }
      DispatchQueue.main.async { completion(outcome) }
    }
  }

  /// Write the previewed change, unless the file changed since the preview
  /// was made (the agent rewrote its settings, say): then the preview is
  /// made again from what's there now, and nothing is written.
  func apply() -> Outcome {
    guard let after else { return .failed(error ?? "Nothing to change.") }
    guard FileManager.default.contents(atPath: path) == previewed else {
      reload()
      refreshed = true
      return .changed
    }
    do {
      if removesFile {
        try FileManager.default.removeItem(atPath: path)
        return .written
      }
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      // The agent's own config: write through a symlink (dotfile managers
      // link these) rather than replacing it, and keep the previous version
      // (not of Impulse's own plugin file).
      let target = TextFile.target(of: path)
      if FileManager.default.fileExists(atPath: target), !(part == .hooks && agent == .opencode) {
        let backup = target + ".impulse-backup"
        try? FileManager.default.removeItem(atPath: backup)
        try FileManager.default.copyItem(atPath: target, toPath: backup)
      }
      try Data(after.utf8).write(to: URL(fileURLWithPath: target), options: .atomic)
      return .written
    } catch {
      return .failed(error.localizedDescription)
    }
  }
}

struct AgentHooksSheetView: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: AgentHooksModel
  let onClose: (_ message: String?) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text("Agent Integration")
          .font(ChromeFont.ui(15, weight: .semibold))
          .foregroundStyle(chrome.text)
        Spacer()
        Picker("", selection: $model.part) {
          ForEach(AgentHooksModel.Part.allCases) { Text($0.rawValue).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 200)
      }
      Text(
        model.part == .hooks
          ? "Hooks make the agent tell Impulse exactly when it starts working, needs you, and finishes — no guessing from output. They only do anything inside Impulse terminals."
          : "The MCP server gives the agent Impulse's tools: the repository's tasks (and starting one, which asks you first), your review comments, the language servers' problems, other terminals' output, unsaved files and the project's scripts. It only has tools inside Impulse terminals."
      )
        .font(ChromeFont.ui(11.5))
        .foregroundStyle(chrome.textSecondary)
        .fixedSize(horizontal: false, vertical: true)

      HStack(spacing: 12) {
        let agents = AgentHooksModel.Agent.allCases.filter { model.part == .mcp || $0 != .other }
        Picker("", selection: $model.agent) {
          ForEach(agents) { Text($0.rawValue).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        // Its own width, at the leading edge (a set width centers it, or
        // lets it spill over the scope menu), and a new control when the
        // segments change: the native one keeps its old segment widths.
        .fixedSize()
        .id(model.part)
        if model.agent == .claude {
          Picker("", selection: $model.scope) {
            ForEach(AgentHooksModel.Scope.allCases) { scope in
              Text(scope.rawValue).tag(scope)
            }
          }
          .labelsHidden()
          .frame(width: 170)
          .disabled(model.projectRoot == nil)
        }
        Spacer()
      }

      HStack(spacing: 6) {
        StatusDot(color: model.installed ? chrome.success : model.outdated ? chrome.warning : chrome.textTertiary, size: 7)
        Text(
          model.agent == .other
            ? "Add this to the agent's own MCP settings (each agent keeps them in its own file)"
            : model.installed
              ? "Installed"
              : model.outdated
                ? (model.part == .hooks ? "Out of date: install again to add the newer hooks" : "Out of date: install again to update it")
                : "Not installed")
          .font(ChromeFont.ui(11.5, weight: .medium))
          .foregroundStyle(chrome.textSecondary)
        Text(TabManager.abbreviateHomePath(model.path))
          .font(ChromeFont.mono(11))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .truncationMode(.middle)
      }

      if model.refreshed {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          Icon(.info, size: 11).foregroundStyle(chrome.warning)
          Text("The file changed since this preview, so nothing was written. Check the change again.")
            .font(ChromeFont.ui(11.5))
            .foregroundStyle(chrome.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
        }
      }

      if model.agent == .claude, model.scope == .project, let main = model.mainCheckout {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
          Icon(.info, size: 11).foregroundStyle(chrome.warning)
          Text(
            "This is a task folder: project hooks installed here are deleted when the task is archived. To keep them, install them in the main checkout (\(TabManager.abbreviateHomePath(main))); new tasks copy them from there unless a .worktreeinclude leaves them out."
          )
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(chrome.textSecondary)
          .fixedSize(horizontal: false, vertical: true)
        }
      }

      ScrollView {
        VStack(alignment: .leading, spacing: 0) {
          ForEach(Array(model.diff.enumerated()), id: \.offset) { _, line in
            Text(line.isEmpty ? " " : line)
              .font(ChromeFont.mono(11))
              .foregroundStyle(
                line.hasPrefix("+ ") ? chrome.gitAdded : line.hasPrefix("- ") ? chrome.gitDeleted : chrome.textTertiary)
              .frame(maxWidth: .infinity, alignment: .leading)
          }
          if model.diff.isEmpty {
            Text(model.error ?? "No change needed.")
              .font(ChromeFont.ui(11.5))
              .foregroundStyle(model.error == nil ? chrome.textTertiary : chrome.danger)
          }
        }
        .padding(8)
      }
      .frame(height: 220)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.content))

      HStack {
        Spacer()
        ChromeButton(title: "Close", kind: .secondary) { onClose(nil) }
          .keyboardShortcut(.cancelAction)
        if model.agent == .other {
          ChromeButton(title: "Copy", icon: .copy, kind: .primary) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(model.diff.joined(separator: "\n"), forType: .string)
            onClose("Copied Impulse's MCP server entry.")
          }
          .keyboardShortcut(.defaultAction)
        } else if model.after != nil {
          ChromeButton(
            title: "\(model.installed ? "Remove" : "Install") \(model.part == .hooks ? "Hooks" : "MCP Server")",
            icon: model.installed ? .trash2 : .check, kind: model.installed ? .secondary : .primary
          ) {
            let wasInstalled = model.installed
            let what = model.what
            model.applyInBackground { outcome in
              switch outcome {
              case .failed(let failure):
                onClose("Couldn't update \(TabManager.abbreviateHomePath(model.path)): \(failure)")
              case .changed:
                break  // the sheet stays open on the new preview
              case .written:
                onClose(
                  wasInstalled
                    ? "Removed Impulse's \(model.agent.rawValue) \(what)."
                    : "Installed the \(model.agent.rawValue) \(what). Restart running agents to pick it up.")
              }
            }
          }
          .keyboardShortcut(.defaultAction)
        }
      }
    }
    .padding(20)
    .frame(width: 620)
    .background(chrome.overlay)
  }
}

extension MainWindowController {
  func presentAgentHooksSheet(part: AgentHooksModel.Part = .hooks) {
    guard let window else { return }
    let projectRoot = windowModel.repository?.root
    let mainCheckout = projectRoot.map(Self.mainCheckoutRoot(of:))
    let model = AgentHooksModel(
      projectRoot: projectRoot, mainCheckout: mainCheckout == projectRoot ? nil : mainCheckout,
      fallbackCLI: TerminalTab.cliPath ?? AgentMCPInstaller.defaultCLI)
    model.part = part
    let palette = windowModel.palette
    let sheet = NSWindow.themedSheet(palette: palette)
    if DebugSnapshot.isActive {
      DebugSnapshot.agentSheet = (model, { [weak window, weak sheet] in
        DebugSnapshot.agentSheet = nil
        if let sheet { window?.endSheet(sheet) }
      })
    }
    window.beginThemedSheet(
      sheet, palette: palette,
      content: AgentHooksSheetView(model: model) { [weak self, weak window, weak sheet] message in
        DebugSnapshot.agentSheet = nil
        if let sheet { window?.endSheet(sheet) }
        if let message { self?.toasts.show(Toast(kind: .success, message: message)) }
      })
  }

  /// Toggle "Impulse as $EDITOR" for new terminals.
  func toggleEditorIntegration() {
    var settings = SettingsStore.shared.settings
    settings.terminalEditorIntegration.toggle()
    SettingsStore.shared.settings = settings
    toasts.show(
      Toast(
        kind: .success,
        message: settings.terminalEditorIntegration
          ? "New terminals use Impulse as $EDITOR (git commit opens a tab)."
          : "New terminals keep your own $EDITOR."))
  }
}
