import AppKit
import SwiftUI

/// Keyboard shortcuts as a tab: every command with its shortcut, search by
/// name or by pressing the keys, click a shortcut to record a new one,
/// conflicts flagged, reset per command, and shortcuts that run shell
/// commands in the terminal.
@Observable
final class KeybindingsModel {
  var query = ""
  /// The shortcut typed into the search (symbol form, "⌘D").
  var keyFilter: String?
  /// What the next key press is recorded for: a command id, "custom:N",
  /// or "search".
  var recording: String?
  var palette: ChromePalette

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var overrides: [String: String] { SettingsStore.shared.settings.keybindingOverrides }
  var custom: [CustomKeybinding] { SettingsStore.shared.settings.customKeybindings }

  var conflicts: [String: [String]] {
    Keybindings.conflicts(
      overrides: overrides,
      extra: custom.enumerated().map { ("custom:\($0.offset)", $0.element.key) })
  }

  /// Commands shown, in their categories.
  var groups: [(category: String, items: [BuiltinKeybinding])] {
    let q = query.trimmingCharacters(in: .whitespaces).lowercased()
    var order: [String] = []
    var byCategory: [String: [BuiltinKeybinding]] = [:]
    for binding in Keybindings.builtins {
      let symbol = Keybindings.symbolDisplay(forId: binding.id, overrides: overrides) ?? ""
      if let keyFilter, symbol != keyFilter { continue }
      if !q.isEmpty,
        !(binding.description.lowercased().contains(q) || binding.id.contains(q)
          || binding.category.lowercased().contains(q) || symbol.lowercased().contains(q))
      {
        continue
      }
      if byCategory[binding.category] == nil { order.append(binding.category) }
      byCategory[binding.category, default: []].append(binding)
    }
    return order.map { ($0, byCategory[$0] ?? []) }
  }

  func displayName(for id: String) -> String {
    if id.hasPrefix("custom:"), let index = Int(id.dropFirst(7)), custom.indices.contains(index) {
      return custom[index].name.isEmpty ? "Custom shortcut" : custom[index].name
    }
    return Keybindings.builtins.first { $0.id == id }?.description ?? id
  }

  // MARK: Edits

  func setShortcut(_ shortcut: String?, for id: String) {
    SettingsStore.shared.update { settings in
      if id.hasPrefix("custom:"), let index = Int(id.dropFirst(7)),
        settings.customKeybindings.indices.contains(index)
      {
        settings.customKeybindings[index].key = shortcut ?? ""
        return
      }
      guard let builtin = Keybindings.builtins.first(where: { $0.id == id }) else { return }
      let value = shortcut ?? Keybindings.unbound
      let isDefault =
        shortcut.flatMap(Keybindings.symbolDisplay(shortcut:))
        == Keybindings.symbolDisplay(shortcut: builtin.defaultShortcut)
        || (shortcut == nil && builtin.defaultShortcut.isEmpty)
      if isDefault {
        settings.keybindingOverrides.removeValue(forKey: id)
      } else {
        settings.keybindingOverrides[id] = value
      }
    }
  }

  func reset(_ id: String) {
    SettingsStore.shared.update { $0.keybindingOverrides.removeValue(forKey: id) }
  }

  func addCustom() {
    SettingsStore.shared.update { $0.customKeybindings.append(CustomKeybinding(name: "New shortcut")) }
  }

  func updateCustom(_ index: Int, _ change: (inout CustomKeybinding) -> Void) {
    SettingsStore.shared.update { settings in
      guard settings.customKeybindings.indices.contains(index) else { return }
      change(&settings.customKeybindings[index])
    }
  }

  func removeCustom(_ index: Int) {
    SettingsStore.shared.update { settings in
      guard settings.customKeybindings.indices.contains(index) else { return }
      settings.customKeybindings.remove(at: index)
    }
  }
}

final class KeybindingsSurface: NSView, ToolSurface {
  let model: KeybindingsModel
  private var monitor: Any?

  var toolKind: String { "keybindings" }
  var toolTitle: String { "Keyboard Shortcuts" }
  var toolSymbol: String { "keyboard" }

  init(palette: ChromePalette) {
    model = KeybindingsModel(palette: palette)
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
    let hosting = WorkbenchHosting.make(KeybindingsView(model: model))
    hosting.translatesAutoresizingMaskIntoConstraints = false
    addSubview(hosting)
    NSLayoutConstraint.activate([
      hosting.topAnchor.constraint(equalTo: topAnchor),
      hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
      hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
    // While recording, the next key press (in this window) is the shortcut.
    monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
      guard let self, let target = self.model.recording, event.window === self.window else { return event }
      self.record(event, for: target)
      return nil
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  deinit {
    if let monitor { NSEvent.removeMonitor(monitor) }
  }

  func applyToolTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
  }

  func cleanupTool() {
    model.recording = nil
    if let monitor { NSEvent.removeMonitor(monitor) }
    monitor = nil
  }

  /// Esc cancels; ⌫ alone removes the shortcut; anything with a key becomes it.
  private func record(_ event: NSEvent, for target: String) {
    let modifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
    defer { model.recording = nil }
    if event.keyCode == 53, modifiers.isEmpty { return }
    if target == "search" {
      model.keyFilter = Keybindings.shortcutString(
        keyCode: event.keyCode, characters: event.charactersIgnoringModifiers, modifiers: modifiers
      ).flatMap(Keybindings.symbolDisplay(shortcut:))
      return
    }
    if event.keyCode == 51, modifiers.isEmpty {
      model.setShortcut(nil, for: target)
      return
    }
    guard
      let shortcut = Keybindings.shortcutString(
        keyCode: event.keyCode, characters: event.charactersIgnoringModifiers, modifiers: modifiers)
    else { return }
    model.setShortcut(shortcut, for: target)
  }
}

struct KeybindingsView: View {
  var model: KeybindingsModel

  var body: some View {
    let chrome = model.palette
    let conflicts = model.conflicts
    VStack(spacing: 0) {
      header
      Rectangle().fill(chrome.hairline).frame(height: 1)
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          ForEach(model.groups, id: \.category) { group in
            Text(group.category.uppercased())
              .font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
              .foregroundStyle(chrome.textTertiary)
              .padding(.top, 16).padding(.bottom, 4)
            ForEach(group.items, id: \.id) { binding in
              CommandShortcutRow(model: model, binding: binding, conflicts: conflicts)
              Rectangle().fill(chrome.hairline).frame(height: 1)
            }
          }
          customSection(conflicts: conflicts)
        }
        .padding(.horizontal, 28)
        .padding(.bottom, 24)
        .frame(maxWidth: 860, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
    .background(chrome.content)
    .environment(\.chrome, chrome)
  }

  private var header: some View {
    let chrome = model.palette
    return HStack(spacing: 10) {
      Text("Keyboard Shortcuts").font(ChromeFont.ui(16, weight: .semibold)).foregroundStyle(chrome.text)
      Spacer()
      HStack(spacing: 6) {
        Icon(.search, size: 12).foregroundStyle(chrome.textTertiary)
        if let keyFilter = model.keyFilter {
          Text(keyFilter).font(ChromeFont.mono(12, weight: .semibold)).foregroundStyle(chrome.accent)
          Button {
            model.keyFilter = nil
          } label: {
            Icon(.x, size: 10).foregroundStyle(chrome.textTertiary)
          }
          .buttonStyle(.plain)
        }
        TextField("Search commands", text: Binding(get: { model.query }, set: { model.query = $0 }))
          .textFieldStyle(.plain)
          .font(ChromeFont.ui(12.5))
          .frame(width: 200)
      }
      .padding(.horizontal, 8)
      .frame(height: 28)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.raised))
      ChromeButton(
        title: model.recording == "search" ? "Press keys…" : "Search by keys",
        kind: model.recording == "search" ? .primary : .secondary
      ) {
        model.recording = model.recording == "search" ? nil : "search"
      }
    }
    .padding(.horizontal, 28)
    .frame(height: 52)
    .background(chrome.panel)
  }

  @ViewBuilder
  private func customSection(conflicts: [String: [String]]) -> some View {
    let chrome = model.palette
    HStack {
      Text("RUN IN TERMINAL").font(ChromeFont.ui(10.5, weight: .semibold)).tracking(0.6)
        .foregroundStyle(chrome.textTertiary)
      Spacer()
      Button {
        model.addCustom()
      } label: {
        HStack(spacing: 4) {
          Icon(.plus, size: 11)
          Text("Add").font(ChromeFont.ui(11.5))
        }
        .foregroundStyle(chrome.accent)
      }
      .buttonStyle(.plain)
    }
    .padding(.top, 22).padding(.bottom, 4)
    Text("Shortcuts that type a command into the focused terminal and run it.")
      .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary).padding(.bottom, 6)
    ForEach(Array(model.custom.enumerated()), id: \.offset) { index, custom in
      CustomShortcutRow(model: model, index: index, binding: custom, conflicts: conflicts)
      Rectangle().fill(chrome.hairline).frame(height: 1)
    }
  }
}

/// The recorded/recording shortcut chip; click to record.
private struct ShortcutChip: View {
  @Environment(\.chrome) private var chrome
  var model: KeybindingsModel
  let target: String
  let shortcut: String?

  var body: some View {
    let recording = model.recording == target
    Button {
      model.recording = recording ? nil : target
    } label: {
      Text(recording ? "Press keys… (⌫ removes, esc cancels)" : (shortcut ?? "Unbound"))
        .font(recording || shortcut == nil ? ChromeFont.ui(11.5) : ChromeFont.mono(12, weight: .medium))
        .foregroundStyle(recording ? chrome.accent : shortcut == nil ? chrome.textTertiary : chrome.text)
        .padding(.horizontal, 8)
        .frame(minWidth: 70, minHeight: 24)
        .background(
          RoundedRectangle(cornerRadius: 5).fill(recording ? chrome.accent.opacity(0.14) : chrome.raised))
        .overlay(
          RoundedRectangle(cornerRadius: 5)
            .strokeBorder(recording ? chrome.accent : chrome.hairline, lineWidth: 1))
    }
    .buttonStyle(.plain)
    .help("Click, then press the new shortcut")
  }
}

private struct ConflictBadge: View {
  @Environment(\.chrome) private var chrome
  var model: KeybindingsModel
  let id: String
  let symbol: String?
  let conflicts: [String: [String]]

  var body: some View {
    if let symbol, let others = conflicts[symbol]?.filter({ $0 != id }), !others.isEmpty {
      Icon(.triangleAlert, size: 12)
        .foregroundStyle(chrome.warning)
        .help("Also bound to " + others.map(model.displayName(for:)).joined(separator: ", "))
    }
  }
}

private struct CommandShortcutRow: View {
  @Environment(\.chrome) private var chrome
  var model: KeybindingsModel
  let binding: BuiltinKeybinding
  let conflicts: [String: [String]]

  var body: some View {
    let symbol = Keybindings.symbolDisplay(forId: binding.id, overrides: model.overrides)
    let modified = model.overrides[binding.id] != nil
    HStack(spacing: 10) {
      Rectangle().fill(modified ? chrome.accent : .clear).frame(width: 2, height: 22)
      VStack(alignment: .leading, spacing: 1) {
        Text(binding.description).font(ChromeFont.ui(12.5)).foregroundStyle(chrome.text)
        Text(binding.id).font(ChromeFont.mono(10)).foregroundStyle(chrome.textTertiary)
      }
      Spacer(minLength: 10)
      ConflictBadge(model: model, id: binding.id, symbol: symbol, conflicts: conflicts)
      ShortcutChip(model: model, target: binding.id, shortcut: symbol)
      ChromeIconButton(icon: .rotateCcw, help: "Reset to default", size: 22, iconSize: 12) {
        model.reset(binding.id)
      }
      .opacity(modified ? 1 : 0)
      .disabled(!modified)
    }
    .padding(.vertical, 6)
  }
}

private struct CustomShortcutRow: View {
  @Environment(\.chrome) private var chrome
  var model: KeybindingsModel
  let index: Int
  let binding: CustomKeybinding
  let conflicts: [String: [String]]

  var body: some View {
    let symbol = Keybindings.symbolDisplay(shortcut: binding.key)
    HStack(spacing: 8) {
      field("Name", binding.name, width: 150) { value in model.updateCustom(index) { $0.name = value } }
      field("Command", ([binding.command] + binding.args).joined(separator: " "), width: nil) { value in
        let parts = value.split(separator: " ").map(String.init)
        model.updateCustom(index) {
          $0.command = parts.first ?? ""
          $0.args = Array(parts.dropFirst())
        }
      }
      ConflictBadge(model: model, id: "custom:\(index)", symbol: symbol, conflicts: conflicts)
      ShortcutChip(model: model, target: "custom:\(index)", shortcut: symbol)
      ChromeIconButton(icon: .trash2, help: "Remove", size: 22, iconSize: 12) {
        model.removeCustom(index)
      }
    }
    .padding(.vertical, 6)
  }

  private func field(_ placeholder: String, _ value: String, width: CGFloat?, commit: @escaping (String) -> Void)
    -> some View
  {
    CommitField(placeholder: placeholder, value: value, commit: commit)
      .frame(width: width)
      .frame(maxWidth: width == nil ? .infinity : nil)
  }
}

/// A text field that saves on Return or when focus leaves.
private struct CommitField: View {
  let placeholder: String
  let value: String
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
  }
}
