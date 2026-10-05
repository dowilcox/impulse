import AppKit
import ImpulseKit
import SwiftUI

/// State for the terminal's find bar: the query with its case / regex /
/// whole-word toggles, and the match count the terminal reports back.
@Observable
final class TerminalFindModel {
  var query = TerminalFindQuery(text: "") {
    didSet { if query != oldValue { onChange?(query) } }
  }
  private(set) var current = 0
  private(set) var total = 0
  private(set) var capped = false
  private(set) var invalid = false
  /// Bumped to put the cursor in the field.
  var focusToken = 0
  var palette: ChromePalette

  @ObservationIgnored var onChange: ((TerminalFindQuery) -> Void)?
  @ObservationIgnored var onNext: (() -> Void)?
  @ObservationIgnored var onPrevious: (() -> Void)?
  @ObservationIgnored var onClose: (() -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  func update(_ stats: TerminalBackend.SearchStats?) {
    current = stats?.current ?? 0
    total = stats?.total ?? 0
    capped = stats?.capped ?? false
    invalid = stats?.invalid ?? false
  }

  var status: String {
    if query.text.isEmpty { return "" }
    if invalid { return "Invalid pattern" }
    if total == 0 { return "No results" }
    let count = capped ? "\(total)+" : "\(total)"
    return current > 0 ? "\(current) of \(count)" : "\(count) found"
  }
}

struct TerminalFindBar: View {
  var model: TerminalFindModel
  @FocusState private var focused: Bool

  var body: some View {
    let chrome = model.palette
    HStack(spacing: 6) {
      Icon(.search, size: 12).foregroundStyle(chrome.textTertiary)
      TextField(
        "Find in terminal",
        text: Binding(get: { model.query.text }, set: { model.query.text = $0 })
      )
      .textFieldStyle(.plain)
      .font(ChromeFont.ui(12.5))
      .foregroundStyle(chrome.text)
      .focused($focused)
      .onSubmit { model.onNext?() }
      .onKeyPress(.return, phases: .down) { press in
        guard press.modifiers.contains(.shift) else { return .ignored }
        model.onPrevious?()
        return .handled
      }
      .onExitCommand { model.onClose?() }
      .frame(minWidth: 120)
      toggle("Aa", help: "Match case", isOn: model.query.caseSensitive) {
        model.query.caseSensitive.toggle()
      }
      toggle("ab", help: "Whole word", isOn: model.query.wholeWord, underline: true) {
        model.query.wholeWord.toggle()
      }
      toggle(".*", help: "Regular expression", isOn: model.query.regex) {
        model.query.regex.toggle()
      }
      Text(verbatim: model.status)
        .font(ChromeFont.ui(11.5))
        .monospacedDigit()
        .foregroundStyle(model.invalid || (model.total == 0 && !model.query.text.isEmpty) ? chrome.danger : chrome.textSecondary)
        .frame(minWidth: 84, alignment: .trailing)
        .lineLimit(1)
      ChromeIconButton(icon: .chevronUp, help: "Previous match (⇧↩)", size: 22, iconSize: 13) {
        model.onPrevious?()
      }
      .disabled(model.total == 0)
      ChromeIconButton(icon: .chevronDown, help: "Next match (↩)", size: 22, iconSize: 13) {
        model.onNext?()
      }
      .disabled(model.total == 0)
      ChromeIconButton(icon: .x, help: "Close (esc)", size: 22, iconSize: 13) {
        model.onClose?()
      }
    }
    .padding(.horizontal, 10)
    .frame(height: 32)
    .background(chrome.panel)
    .overlay(alignment: .bottom) { Rectangle().fill(chrome.hairline).frame(height: 1) }
    .environment(\.chrome, chrome)
    .onChange(of: model.focusToken) { _, _ in focused = true }
    .onAppear { focused = true }
  }

  private func toggle(
    _ label: String, help: String, isOn: Bool, underline: Bool = false, action: @escaping () -> Void
  ) -> some View {
    let chrome = model.palette
    return Button(action: action) {
      Text(label)
        .font(ChromeFont.mono(11, weight: .semibold))
        .underline(underline)
        .foregroundStyle(isOn ? chrome.accent : chrome.textSecondary)
        .frame(width: 24, height: 20)
        .background(
          RoundedRectangle(cornerRadius: 4)
            .fill(isOn ? chrome.accent.opacity(0.16) : .clear))
        .overlay(
          RoundedRectangle(cornerRadius: 4)
            .strokeBorder(isOn ? chrome.accent.opacity(0.5) : .clear, lineWidth: 1))
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .help(help)
    .accessibilityLabel(help)
    .accessibilityAddTraits(isOn ? .isSelected : [])
  }
}
