import AppKit
import SwiftUI

// The Settings tab's list-valued sections: commands run on save, per-file-
// type overrides, and language servers.

/// Shell-style split for a "command arg arg" field (quotes group words).
func splitCommandLine(_ text: String) -> [String] {
  var parts: [String] = []
  var current = ""
  var quote: Character?
  var inToken = false
  for character in text {
    if let open = quote {
      if character == open { quote = nil } else { current.append(character) }
    } else if character == "\"" || character == "'" {
      quote = character
      inToken = true
    } else if character.isWhitespace {
      if inToken { parts.append(current) }
      current = ""
      inToken = false
    } else {
      current.append(character)
      inToken = true
    }
  }
  if inToken { parts.append(current) }
  return parts
}

/// The inverse of `splitCommandLine` for display.
func joinCommandLine(_ parts: [String]) -> String {
  parts.map { $0.contains(" ") || $0.isEmpty ? "\"\($0)\"" : $0 }.joined(separator: " ")
}

struct AutomationSettingsView: View {
  @Environment(\.chrome) private var chrome

  var body: some View {
    let settings = SettingsStore.shared.settings
    ScrollView {
      VStack(alignment: .leading, spacing: 0) {
        Text("Automation").font(ChromeFont.ui(18, weight: .semibold)).foregroundStyle(chrome.text)
          .padding(.bottom, 4)

        sectionHeader(
          "Commands on save", detail: "Run after saving files that match a pattern, e.g. a linter or code generator."
        ) {
          SettingsStore.shared.update { $0.commandsOnSave.append(CommandOnSave(name: "New command")) }
        }
        if settings.commandsOnSave.isEmpty { empty("No commands yet.") }
        ForEach(Array(settings.commandsOnSave.enumerated()), id: \.offset) { index, entry in
          HStack(spacing: 8) {
            SectionField(placeholder: "Name", value: entry.name, width: 130) { value in
              update(command: index) { $0.name = value }
            }
            SectionField(placeholder: "*.swift", value: entry.filePattern, width: 110) { value in
              update(command: index) { $0.filePattern = value }
            }
            SectionField(placeholder: "command --flag {file}", value: joinCommandLine([entry.command] + entry.args)) {
              value in
              let parts = splitCommandLine(value)
              update(command: index) {
                $0.command = parts.first ?? ""
                $0.args = Array(parts.dropFirst())
              }
            }
            Toggle(isOn: Binding(
              get: { entry.reloadFile },
              set: { value in update(command: index) { $0.reloadFile = value } })
            ) {
              Text("Reload").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
            }
            .toggleStyle(.checkbox)
            .help("Reload the file afterwards (for commands that rewrite it)")
            ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
              SettingsStore.shared.update { $0.commandsOnSave.remove(at: index) }
            }
          }
          .padding(.vertical, 6)
          Rectangle().fill(chrome.hairline).frame(height: 1)
        }

        sectionHeader(
          "File types", detail: "Indentation and a formatter per file pattern, overriding the editor defaults."
        ) {
          SettingsStore.shared.update { $0.fileTypeOverrides.append(FileTypeOverride(pattern: "*.ext")) }
        }
        if settings.fileTypeOverrides.isEmpty { empty("No overrides yet.") }
        ForEach(Array(settings.fileTypeOverrides.enumerated()), id: \.offset) { index, entry in
          HStack(spacing: 8) {
            SectionField(placeholder: "*.py", value: entry.pattern, width: 110) { value in
              update(fileType: index) { $0.pattern = value }
            }
            Picker(
              "",
              selection: Binding(
                get: { entry.tabWidth ?? 0 },
                set: { value in update(fileType: index) { $0.tabWidth = value == 0 ? nil : value } })
            ) {
              Text("Tab width: default").tag(0)
              ForEach([2, 4, 8], id: \.self) { Text("Tab width: \($0)").tag($0) }
            }
            .labelsHidden()
            .frame(width: 150)
            Picker(
              "",
              selection: Binding(
                get: { entry.useSpaces.map { $0 ? 1 : 2 } ?? 0 },
                set: { value in update(fileType: index) { $0.useSpaces = value == 0 ? nil : value == 1 } })
            ) {
              Text("Indent: default").tag(0)
              Text("Spaces").tag(1)
              Text("Tabs").tag(2)
            }
            .labelsHidden()
            .frame(width: 130)
            SectionField(
              placeholder: "formatter (optional)",
              value: entry.formatOnSave.map { joinCommandLine([$0.command] + $0.args) } ?? ""
            ) { value in
              let parts = splitCommandLine(value)
              update(fileType: index) {
                $0.formatOnSave =
                  parts.isEmpty ? nil : FormatOnSave(command: parts[0], args: Array(parts.dropFirst()))
              }
            }
            ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
              SettingsStore.shared.update { $0.fileTypeOverrides.remove(at: index) }
            }
          }
          .padding(.vertical, 6)
          Rectangle().fill(chrome.hairline).frame(height: 1)
        }
      }
      .padding(.horizontal, 28)
      .padding(.vertical, 22)
      .frame(maxWidth: 900, alignment: .leading)
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }

  private func update(command index: Int, _ change: (inout CommandOnSave) -> Void) {
    SettingsStore.shared.update { settings in
      guard settings.commandsOnSave.indices.contains(index) else { return }
      change(&settings.commandsOnSave[index])
    }
  }

  private func update(fileType index: Int, _ change: (inout FileTypeOverride) -> Void) {
    SettingsStore.shared.update { settings in
      guard settings.fileTypeOverrides.indices.contains(index) else { return }
      change(&settings.fileTypeOverrides[index])
    }
  }

  private func sectionHeader(_ title: String, detail: String, add: @escaping () -> Void) -> some View {
    VStack(alignment: .leading, spacing: 2) {
      HStack {
        Text(title.uppercased()).font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
          .foregroundStyle(chrome.textTertiary)
        Spacer()
        Button(action: add) {
          HStack(spacing: 4) {
            Icon(.plus, size: 11)
            Text("Add").font(ChromeFont.ui(11.5))
          }
          .foregroundStyle(chrome.accent)
        }
        .buttonStyle(.plain)
      }
      Text(detail).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
    }
    .padding(.top, 18)
    .padding(.bottom, 6)
  }

  private func empty(_ text: String) -> some View {
    Text(text).font(ChromeFont.ui(12)).foregroundStyle(chrome.textTertiary).padding(.vertical, 6)
  }
}

/// Saves on Return or focus loss.
struct SectionField: View {
  let placeholder: String
  let value: String
  var width: CGFloat? = nil
  let commit: (String) -> Void
  @State private var text = ""
  @FocusState private var focused: Bool

  var body: some View {
    TextField(placeholder, text: $text)
      .textFieldStyle(.roundedBorder)
      .font(ChromeFont.mono(12))
      .focused($focused)
      .onSubmit { if text != value { commit(text) } }
      .onChange(of: focused) { _, isFocused in if !isFocused, text != value { commit(text) } }
      .onAppear { text = value }
      .onChange(of: value) { _, newValue in text = newValue }
      .frame(width: width)
      .frame(maxWidth: width == nil ? .infinity : nil)
  }
}

struct LanguageServersView: View {
  @Environment(\.chrome) private var chrome
  @State private var managed: [[String: Any]] = []
  @State private var system: [[String: Any]] = []
  @State private var npm = true
  @State private var installing = false
  @State private var message: (text: String, failed: Bool)?

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 0) {
        Text("Language Servers").font(ChromeFont.ui(18, weight: .semibold)).foregroundStyle(chrome.text)
        Text("Completions, diagnostics, go to definition and formatting come from these.")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).padding(.top, 2)

        HStack {
          Text("MANAGED BY IMPULSE").font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
            .foregroundStyle(chrome.textTertiary)
          Spacer()
          if installing {
            ProgressRing(progress: nil, color: chrome.textTertiary, size: 12, lineWidth: 1.5)
          }
          ChromeButton(title: "Install All", kind: .secondary) { install() }
            .disabled(!npm || installing)
            .help(npm ? "Install or update the web language servers with npm" : "npm is not installed")
        }
        .padding(.top, 18).padding(.bottom, 4)
        Text(npm ? "Installed with npm into Impulse's own folder." : "Install Node.js (npm) to manage these.")
          .font(ChromeFont.ui(11.5)).foregroundStyle(npm ? chrome.textSecondary : chrome.warning)
          .padding(.bottom, 6)
        if let message {
          Text(message.text).font(ChromeFont.ui(11.5))
            .foregroundStyle(message.failed ? chrome.danger : chrome.success).padding(.bottom, 6)
        }
        ForEach(Array(managed.enumerated()), id: \.offset) { _, status in row(status) }

        Text("FROM YOUR SYSTEM").font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
          .foregroundStyle(chrome.textTertiary)
          .padding(.top, 18).padding(.bottom, 4)
        Text("Install these with your package manager (Homebrew, rustup, go, …).")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).padding(.bottom, 6)
        ForEach(Array(system.enumerated()), id: \.offset) { _, status in row(status) }
      }
      .padding(.horizontal, 28)
      .padding(.vertical, 22)
      .frame(maxWidth: 820, alignment: .leading)
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .onAppear(perform: reload)
  }

  private func row(_ status: [String: Any]) -> some View {
    let command = status["command"] as? String ?? "?"
    let installed = status["installed"] as? Bool ?? false
    let path = status["resolvedPath"] as? String
    return VStack(spacing: 0) {
      HStack(spacing: 10) {
        Icon(installed ? .circleCheck : .circle, size: 13)
          .foregroundStyle(installed ? chrome.success : chrome.textTertiary)
        Text(command).font(ChromeFont.mono(12.5)).foregroundStyle(chrome.text)
        Spacer()
        Text(installed ? (path.map { TabManager.abbreviateHomePath($0) } ?? "Installed") : "Not installed")
          .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textTertiary).lineLimit(1).truncationMode(.middle)
      }
      .padding(.vertical, 7)
      Rectangle().fill(chrome.hairline).frame(height: 1)
    }
  }

  private func reload() {
    DispatchQueue.global(qos: .userInitiated).async {
      let managed = ImpulseCore.lspCheckStatus()
      let system = ImpulseCore.systemLspStatus()
      let npm = ImpulseCore.npmIsAvailable()
      DispatchQueue.main.async {
        self.managed = managed
        self.system = system
        self.npm = npm
      }
    }
  }

  private func install() {
    installing = true
    message = nil
    DispatchQueue.global(qos: .userInitiated).async {
      let result = ImpulseCore.lspInstall()
      DispatchQueue.main.async {
        installing = false
        switch result {
        case .success: message = ("Language servers installed.", false)
        case .failure(let error): message = (error.message, true)
        }
        reload()
      }
    }
  }
}
