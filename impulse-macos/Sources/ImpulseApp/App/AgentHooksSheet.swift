import AppKit
import ImpulseKit
import SwiftUI

/// Installs (or removes) the hooks that let agents report their state to
/// Impulse, showing the exact change to the agent's config first.
@Observable
final class AgentHooksModel {
  enum Agent: String, CaseIterable, Identifiable {
    case claude = "Claude Code"
    case codex = "Codex"
    var id: String { rawValue }
  }

  enum Scope: String, CaseIterable, Identifiable {
    case user = "All projects"
    case project = "This project only"
    var id: String { rawValue }
  }

  var agent: Agent = .claude { didSet { reload() } }
  var scope: Scope = .user { didSet { reload() } }
  /// The project for project-local Claude hooks, if any.
  let projectRoot: String?
  /// The repository's main checkout, when `projectRoot` is a task worktree
  /// (whose project hooks go away when it's archived).
  let mainCheckout: String?

  private(set) var path = ""
  private(set) var before = ""
  private(set) var after: String?
  private(set) var installed = false
  private(set) var error: String?

  init(projectRoot: String?, mainCheckout: String? = nil) {
    self.projectRoot = projectRoot
    self.mainCheckout = mainCheckout
    reload()
  }

  var diff: [String] {
    guard let after else { return [] }
    return AgentHookInstaller.diffLines(before: before, after: after)
  }

  func reload() {
    let home = NSHomeDirectory()
    switch agent {
    case .claude:
      path =
        scope == .project && projectRoot != nil
        ? (projectRoot! as NSString).appendingPathComponent(".claude/settings.local.json")
        : home + "/.claude/settings.json"
    case .codex:
      path = home + "/.codex/config.toml"
    }
    let data = FileManager.default.contents(atPath: path)
    before = data.map { String(decoding: $0, as: UTF8.self) } ?? ""
    error = nil
    switch agent {
    case .claude:
      installed = AgentHookInstaller.claudeHooksInstalled(data)
      switch installed
        ? (data.map { AgentHookInstaller.removingClaudeHooks(from: $0) } ?? .success(Data()))
        : AgentHookInstaller.installingClaudeHooks(into: data)
      {
      case .success(let result): after = String(decoding: result, as: UTF8.self)
      case .failure(let failure): fail(failure)
      }
    case .codex:
      installed = AgentHookInstaller.codexNotifyInstalled(before)
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

  private func fail(_ failure: AgentHookInstaller.InstallError) {
    after = nil
    switch failure {
    case .unreadable(let message), .notifyInUse(let message): error = message
    }
  }

  /// Write the previewed change. Returns an error message on failure.
  func apply() -> String? {
    guard let after else { return error ?? "Nothing to change." }
    do {
      try FileManager.default.createDirectory(
        atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
      // The agent's own config: write through a symlink (dotfile managers
      // link these) rather than replacing it, and keep the previous version.
      let target = TextFile.target(of: path)
      if FileManager.default.fileExists(atPath: target) {
        let backup = target + ".impulse-backup"
        try? FileManager.default.removeItem(atPath: backup)
        try FileManager.default.copyItem(atPath: target, toPath: backup)
      }
      try Data(after.utf8).write(to: URL(fileURLWithPath: target), options: .atomic)
      return nil
    } catch {
      return error.localizedDescription
    }
  }
}

struct AgentHooksSheetView: View {
  @Environment(\.chrome) private var chrome
  @Bindable var model: AgentHooksModel
  let onClose: (_ message: String?) -> Void

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text("Agent Hooks")
        .font(ChromeFont.ui(15, weight: .semibold))
        .foregroundStyle(chrome.text)
      Text("Hooks make the agent tell Impulse exactly when it starts working, needs you, and finishes — no guessing from output. They only do anything inside Impulse terminals.")
        .font(ChromeFont.ui(11.5))
        .foregroundStyle(chrome.textSecondary)
        .fixedSize(horizontal: false, vertical: true)

      HStack(spacing: 12) {
        Picker("", selection: $model.agent) {
          ForEach(AgentHooksModel.Agent.allCases) { Text($0.rawValue).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 220)
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
        StatusDot(color: model.installed ? chrome.success : chrome.textTertiary, size: 7)
        Text(model.installed ? "Installed" : "Not installed")
          .font(ChromeFont.ui(11.5, weight: .medium))
          .foregroundStyle(chrome.textSecondary)
        Text(TabManager.abbreviateHomePath(model.path))
          .font(ChromeFont.mono(11))
          .foregroundStyle(chrome.textTertiary)
          .lineLimit(1)
          .truncationMode(.middle)
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
        if model.after != nil {
          ChromeButton(
            title: model.installed ? "Remove Hooks" : "Install Hooks",
            icon: model.installed ? .trash2 : .check, kind: model.installed ? .secondary : .primary
          ) {
            let wasInstalled = model.installed
            if let failure = model.apply() {
              onClose("Couldn't update \(TabManager.abbreviateHomePath(model.path)): \(failure)")
            } else {
              onClose(
                wasInstalled
                  ? "Removed Impulse's \(model.agent.rawValue) hooks."
                  : "Installed \(model.agent.rawValue) hooks. Restart running agents to pick them up.")
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
  func presentAgentHooksSheet() {
    guard let window else { return }
    let projectRoot = windowModel.repository?.root
    let mainCheckout = projectRoot.map(Self.mainCheckoutRoot(of:))
    let model = AgentHooksModel(
      projectRoot: projectRoot, mainCheckout: mainCheckout == projectRoot ? nil : mainCheckout)
    let palette = windowModel.palette
    let sheet = NSWindow.themedSheet(palette: palette)
    window.beginThemedSheet(
      sheet, palette: palette,
      content: AgentHooksSheetView(model: model) { [weak self, weak window, weak sheet] message in
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
