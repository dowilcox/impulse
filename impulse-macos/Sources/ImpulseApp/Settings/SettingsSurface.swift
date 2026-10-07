import AppKit
import ImpulseKit
import SwiftUI

/// Settings as a tab: categories on the left, the settings on the right,
/// search across all of them, a marker on everything changed from its
/// default and a per-setting reset. Edits apply immediately.
@Observable
final class SettingsSurfaceModel {
  var category: SettingItem.Category = .general
  var query = ""
  var onlyModified = false
  var palette: ChromePalette
  /// Bumped to focus the search field.
  var searchFocusToken = 0

  @ObservationIgnored var onOpenSettingsFile: (() -> Void)?
  @ObservationIgnored var onOpenKeybindings: (() -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  /// What the right side lists: search results across categories, else the
  /// selected category.
  var visibleItems: [SettingItem] {
    let settings = SettingsStore.shared.settings
    let trimmed = query.trimmingCharacters(in: .whitespaces)
    return SettingsCatalog.items.filter { item in
      if onlyModified, !item.isModified(settings) { return false }
      if !trimmed.isEmpty { return item.matches(trimmed) }
      return onlyModified || item.category == category
    }
  }

  var isSearching: Bool { !query.trimmingCharacters(in: .whitespaces).isEmpty || onlyModified }

  func modifiedCount(in category: SettingItem.Category) -> Int {
    let settings = SettingsStore.shared.settings
    return SettingsCatalog.items.filter { $0.category == category && $0.isModified(settings) }.count
  }
}

final class SettingsSurface: NSView, ToolSurface {
  let model: SettingsSurfaceModel
  private var host: NSView!

  var toolKind: String { "settings" }
  var toolTitle: String { "Settings" }
  var toolSymbol: String { "gearshape" }

  init(palette: ChromePalette) {
    model = SettingsSurfaceModel(palette: palette)
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
    let hosting = WorkbenchHosting.make(SettingsView(model: model))
    hosting.translatesAutoresizingMaskIntoConstraints = false
    addSubview(hosting)
    host = hosting
    NSLayoutConstraint.activate([
      hosting.topAnchor.constraint(equalTo: topAnchor),
      hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
      hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  func focusTool() {
    model.searchFocusToken += 1
  }

  func applyToolTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
  }

  /// Show one setting (palette deep link): search for it.
  func reveal(query: String) {
    model.query = query
    model.searchFocusToken += 1
  }
}

// MARK: - Views

struct SettingsView: View {
  var model: SettingsSurfaceModel
  @FocusState private var searchFocused: Bool

  var body: some View {
    let chrome = model.palette
    HStack(spacing: 0) {
      sidebar
        .frame(width: 210)
        .background(chrome.panel)
      Rectangle().fill(chrome.hairline).frame(width: 1)
      content
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(chrome.content)
    }
    .environment(\.chrome, chrome)
    .onChange(of: model.searchFocusToken) { _, _ in searchFocused = true }
  }

  private var sidebar: some View {
    let chrome = model.palette
    return VStack(alignment: .leading, spacing: 2) {
      HStack(spacing: 6) {
        Icon(.search, size: 12).foregroundStyle(chrome.textTertiary)
        TextField("Search settings", text: Binding(get: { model.query }, set: { model.query = $0 }))
          .textFieldStyle(.plain)
          .font(ChromeFont.ui(12.5))
          .focused($searchFocused)
          .onExitCommand { model.query = "" }
      }
      .padding(.horizontal, 8)
      .frame(height: 28)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.raised))
      .padding(.bottom, 8)

      ForEach(SettingItem.Category.allCases) { category in
        categoryRow(category)
      }
      Spacer()
      Toggle(isOn: Binding(get: { model.onlyModified }, set: { model.onlyModified = $0 })) {
        Text("Only changed settings").font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
      }
      .toggleStyle(.checkbox)
      .controlSize(.small)
      Button {
        model.onOpenSettingsFile?()
      } label: {
        HStack(spacing: 5) {
          Icon(.fileCode, size: 11)
          Text("Open settings.json").font(ChromeFont.ui(11.5))
        }
        .foregroundStyle(chrome.accent)
      }
      .buttonStyle(.plain)
      .padding(.top, 4)
    }
    .padding(12)
  }

  private func categoryRow(_ category: SettingItem.Category) -> some View {
    let chrome = model.palette
    let selected = !model.isSearching && model.category == category
    let modified = model.modifiedCount(in: category)
    return Button {
      model.query = ""
      model.onlyModified = false
      model.category = category
    } label: {
      HStack(spacing: 8) {
        Icon(category.icon, size: 13).foregroundStyle(selected ? chrome.accent : chrome.textSecondary)
        Text(category.rawValue).font(ChromeFont.ui(12.5, weight: selected ? .semibold : .regular))
          .foregroundStyle(chrome.text)
        Spacer()
        if modified > 0 {
          Text(verbatim: "\(modified)").font(ChromeFont.mono(10)).foregroundStyle(chrome.accent)
            .help("\(modified) changed from the default")
        }
      }
      .padding(.horizontal, 8)
      .frame(height: 26)
      .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(selected ? chrome.selection : .clear))
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }

  @ViewBuilder
  private var content: some View {
    let chrome = model.palette
    if !model.isSearching && model.category == .advanced {
      AdvancedSettingsView(model: model)
    } else if !model.isSearching && model.category == .automation {
      AutomationSettingsView()
    } else if !model.isSearching && model.category == .languageServers {
      LanguageServersView()
    } else {
      let items = model.visibleItems
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          Text(model.isSearching ? "Results" : model.category.rawValue)
            .font(ChromeFont.ui(18, weight: .semibold))
            .foregroundStyle(chrome.text)
            .padding(.bottom, 10)
          if items.isEmpty {
            Text(model.onlyModified ? "Everything is at its default." : "No settings match “\(model.query)”.")
              .font(ChromeFont.ui(12.5))
              .foregroundStyle(chrome.textSecondary)
          }
          ForEach(Array(sections(items).enumerated()), id: \.offset) { _, group in
            Text(group.title.uppercased())
              .font(ChromeFont.ui(10.5, weight: .semibold))
              .tracking(0.6)
              .foregroundStyle(chrome.textTertiary)
              .padding(.top, 14)
              .padding(.bottom, 4)
            ForEach(group.items) { item in
              SettingRow(item: item)
              Rectangle().fill(chrome.hairline).frame(height: 1)
            }
          }
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 22)
        .frame(maxWidth: 820, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }

  /// Items grouped by section (with the category in search results).
  private func sections(_ items: [SettingItem]) -> [(title: String, items: [SettingItem])] {
    var groups: [(title: String, items: [SettingItem])] = []
    for item in items {
      let title = model.isSearching ? "\(item.category.rawValue) › \(item.section)" : item.section
      if groups.last?.title == title {
        groups[groups.count - 1].items.append(item)
      } else {
        groups.append((title, [item]))
      }
    }
    return groups
  }
}

/// One setting: title, key and description, its control, and a reset when
/// it differs from the default.
private struct SettingRow: View {
  @Environment(\.chrome) private var chrome
  let item: SettingItem

  var body: some View {
    let store = SettingsStore.shared
    let modified = item.isModified(store.settings)
    HStack(alignment: .center, spacing: 12) {
      Rectangle()
        .fill(modified ? chrome.accent : .clear)
        .frame(width: 2)
        .help(modified ? "Changed from the default" : "")
      VStack(alignment: .leading, spacing: 2) {
        HStack(spacing: 6) {
          Text(item.title).font(ChromeFont.ui(12.5, weight: .medium)).foregroundStyle(chrome.text)
          Text(item.key).font(ChromeFont.mono(10)).foregroundStyle(chrome.textTertiary)
            .textSelection(.enabled)
        }
        if let detail = item.detail {
          Text(detail).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
        }
      }
      Spacer(minLength: 12)
      SettingControl(item: item)
      // A theme change applies through the settings broadcast (AppDelegate).
      ChromeIconButton(icon: .rotateCcw, help: "Reset to default", size: 22, iconSize: 12) {
        store.update { item.reset(&$0) }
      }
      .opacity(modified ? 1 : 0)
      .disabled(!modified)
    }
    .padding(.vertical, 9)
    .accessibilityElement(children: .contain)
    .accessibilityLabel(item.title)
  }
}

private struct SettingControl: View {
  @Environment(\.chrome) private var chrome
  let item: SettingItem

  var body: some View {
    switch item.control {
    case .toggle(let kp):
      Toggle("", isOn: binding(kp)).toggleStyle(.switch).labelsHidden().controlSize(.small).tint(chrome.accent)
    case .integer(let kp, let range, let step, let zeroLabel):
      IntegerField(value: binding(kp), range: range, step: step, zeroLabel: zeroLabel)
    case .decimal(let kp, let range, let step):
      HStack(spacing: 4) {
        Text(verbatim: String(format: "%.1f", SettingsStore.shared.settings[keyPath: kp]))
          .font(ChromeFont.mono(12)).frame(width: 40, alignment: .trailing)
        Stepper("", value: binding(kp), in: range, step: step).labelsHidden()
      }
    case .text(let kp, let placeholder):
      CommitTextField(value: binding(kp), placeholder: placeholder)
    case .folder(let kp, let placeholder):
      FolderField(value: binding(kp), placeholder: placeholder)
    case .font(let kp):
      FontPicker(value: binding(kp))
    case .choice(let kp, let options):
      Picker("", selection: binding(kp)) {
        ForEach(options, id: \.value) { option in Text(option.label).tag(option.value) }
        // Keep an unknown value from settings.json selectable.
        if !options.contains(where: { $0.value == SettingsStore.shared.settings[keyPath: kp] }) {
          Text(SettingsStore.shared.settings[keyPath: kp]).tag(SettingsStore.shared.settings[keyPath: kp])
        }
      }
      .labelsHidden()
      .frame(width: 210)
    case .theme(let kp):
      Picker(
        "",
        selection: Binding(
          // Names typed by hand ("Tokyo_Night") select their theme.
          get: { ThemeManager.canonicalID(SettingsStore.shared.settings[keyPath: kp]) },
          set: { name in SettingsStore.shared.update { $0[keyPath: kp] = name } })
      ) {
        ForEach(ThemeManager.availableThemes(), id: \.self) { id in
          Text(ThemeManager.displayName(for: id)).tag(id)
        }
      }
      .labelsHidden()
      .frame(width: 210)
    }
  }

  private func binding<V>(_ kp: WritableKeyPath<Settings, V>) -> Binding<V> {
    Binding(
      get: { SettingsStore.shared.settings[keyPath: kp] },
      set: { value in SettingsStore.shared.update { $0[keyPath: kp] = value } })
  }
}

/// A number with a stepper; typing commits on Return or when focus leaves.
private struct IntegerField: View {
  @Environment(\.chrome) private var chrome
  @Binding var value: Int
  let range: ClosedRange<Int>
  let step: Int
  let zeroLabel: String?
  @State private var text = ""
  @FocusState private var focused: Bool

  var body: some View {
    HStack(spacing: 4) {
      TextField(zeroLabel ?? "", text: $text)
        .textFieldStyle(.roundedBorder)
        .font(ChromeFont.mono(12))
        .multilineTextAlignment(.trailing)
        .frame(width: 84)
        .focused($focused)
        .onSubmit(commit)
        .onChange(of: focused) { _, isFocused in if !isFocused { commit() } }
      Stepper("", value: $value, in: range, step: step).labelsHidden()
    }
    .onAppear(perform: sync)
    .onChange(of: value) { _, _ in sync() }
  }

  private func sync() {
    text = value == 0 && zeroLabel != nil ? "" : String(value)
  }

  private func commit() {
    let digits = text.filter(\.isNumber)
    let parsed = digits.isEmpty ? (zeroLabel != nil ? 0 : value) : (Int(digits) ?? value)
    let clamped = min(max(parsed, range.lowerBound), range.upperBound)
    if clamped != value { value = clamped } else { sync() }
  }
}

/// Text applied on Return or focus loss (not per keystroke).
/// A folder path: type it, or choose it in an open panel (stored with ~).
private struct FolderField: View {
  @Binding var value: String
  let placeholder: String

  var body: some View {
    HStack(spacing: 6) {
      CommitTextField(value: $value, placeholder: placeholder, width: 150)
      Button("Choose…") {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = URL(
          fileURLWithPath: ((value.isEmpty ? NSHomeDirectory() : value) as NSString).expandingTildeInPath)
        if panel.runModal() == .OK, let url = panel.url {
          value = (url.path as NSString).abbreviatingWithTildeInPath
        }
      }
      .controlSize(.small)
    }
  }
}

private struct CommitTextField: View {
  @Binding var value: String
  let placeholder: String
  var width: CGFloat = 210
  @State private var text = ""
  @FocusState private var focused: Bool

  var body: some View {
    TextField(placeholder, text: $text)
      .textFieldStyle(.roundedBorder)
      .frame(width: width)
      .focused($focused)
      .onSubmit { value = text }
      .onChange(of: focused) { _, isFocused in if !isFocused, text != value { value = text } }
      .onAppear { text = value }
      .onChange(of: value) { _, newValue in text = newValue }
  }
}

/// Monospaced font families installed on this Mac, plus whatever is set.
private struct FontPicker: View {
  @Binding var value: String

  private static let monospaced: [String] = {
    let manager = NSFontManager.shared
    return manager.availableFontFamilies.filter { family in
      guard let font = NSFont(name: family, size: 12) ?? manager.font(
        withFamily: family, traits: [], weight: 5, size: 12)
      else { return false }
      return font.isFixedPitch || manager.traits(of: font).contains(.fixedPitchFontMask)
    }
  }()

  var body: some View {
    Picker("", selection: $value) {
      if !Self.monospaced.contains(value) { Text(value).tag(value) }
      ForEach(Self.monospaced, id: \.self) { family in Text(family).tag(family) }
    }
    .labelsHidden()
    .frame(width: 210)
  }
}

/// Settings edited elsewhere: keyboard shortcuts and settings.json.
private struct AdvancedSettingsView: View {
  @Environment(\.chrome) private var chrome
  var model: SettingsSurfaceModel

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      Text("Advanced").font(ChromeFont.ui(18, weight: .semibold)).foregroundStyle(chrome.text)
        .padding(.bottom, 14)
      link(
        "Keyboard shortcuts", "Rebind any command, and add shortcuts that run shell commands.",
        icon: .keyboard
      ) { model.onOpenKeybindings?() }
      link(
        "settings.json", "Every setting as JSON, with completion and validation in the editor.",
        icon: .fileCode
      ) { model.onOpenSettingsFile?() }
      Spacer()
    }
    .padding(.horizontal, 28)
    .padding(.vertical, 22)
    .frame(maxWidth: 820, alignment: .leading)
  }

  private func link(_ title: String, _ detail: String, icon: LucideIcon, action: @escaping () -> Void) -> some View {
    Button(action: action) {
      HStack(spacing: 12) {
        Icon(icon, size: 15).foregroundStyle(chrome.accent).frame(width: 22)
        VStack(alignment: .leading, spacing: 2) {
          Text(title).font(ChromeFont.ui(12.5, weight: .medium)).foregroundStyle(chrome.text)
          Text(detail).font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textSecondary)
        }
        Spacer()
        Icon(.chevronRight, size: 12).foregroundStyle(chrome.textTertiary)
      }
      .padding(.vertical, 10)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .overlay(alignment: .bottom) { Rectangle().fill(chrome.hairline).frame(height: 1) }
  }
}
