import AppKit
import SwiftUI

/// The palette's SwiftUI content: query field, results, and a hint footer.
struct PaletteView: View {
  @Bindable var model: PaletteModel
  let palette: ChromePalette
  @FocusState private var fieldFocused: Bool
  @State private var hoveredId: String? = nil

  static let width: CGFloat = 640
  static let maxListHeight: CGFloat = 380

  var body: some View {
    VStack(spacing: 0) {
      field
      Hairline()
      results
      Hairline()
      footer
    }
    .frame(width: Self.width)
    .background(palette.overlay)
    .clipShape(RoundedRectangle(cornerRadius: Metrics.radiusLarge + 2, style: .continuous))
    .overlay(
      RoundedRectangle(cornerRadius: Metrics.radiusLarge + 2, style: .continuous)
        .strokeBorder(palette.hairlineStrong, lineWidth: 1)
    )
    .environment(\.chrome, palette)
    .onAppear { fieldFocused = true }
  }

  // MARK: Field

  private var field: some View {
    HStack(spacing: 10) {
      Icon(model.mode.icon, size: 15)
        .foregroundStyle(palette.textTertiary)
      TextField(model.mode.placeholder, text: $model.query)
        .textFieldStyle(.plain)
        .font(ChromeFont.ui(15))
        .foregroundStyle(palette.text)
        .focused($fieldFocused)
        .onSubmit { model.runSelectedOrActivate() }
        .onKeyPress(.upArrow) {
          model.moveSelection(-1)
          return .handled
        }
        .onKeyPress(.downArrow) {
          model.moveSelection(1)
          return .handled
        }
        .onKeyPress(.escape) {
          model.onDismiss?()
          return .handled
        }
        .onKeyPress(phases: .down) { press in
          // Ctrl-N / Ctrl-P like most pickers.
          guard press.modifiers.contains(.control) else { return .ignored }
          if press.key == KeyEquivalent("n") {
            model.moveSelection(1)
            return .handled
          }
          if press.key == KeyEquivalent("p") {
            model.moveSelection(-1)
            return .handled
          }
          return .ignored
        }
        .accessibilityLabel("Palette query")
      if model.isBusy {
        ProgressRing(progress: nil, color: palette.textTertiary, size: 12, lineWidth: 1.5)
      }
    }
    .padding(.horizontal, 14)
    .frame(height: 46)
  }

  // MARK: Results

  @ViewBuilder
  private var results: some View {
    if model.rows.isEmpty {
      Text(model.emptyMessage)
        .font(ChromeFont.ui(12))
        .foregroundStyle(palette.textTertiary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .frame(height: 44)
    } else {
      ScrollViewReader { proxy in
        ScrollView {
          LazyVStack(spacing: 0) {
            ForEach(Array(model.rows.enumerated()), id: \.element.id) { index, row in
              rowView(row, selected: index == model.selectedIndex)
                .id(row.id)
                .onTapGesture { model.activate(row) }
                .onHover { inside in
                  if inside { hoveredId = row.id }
                }
            }
          }
          .padding(.vertical, 5)
        }
        .frame(height: min(Self.maxListHeight, CGFloat(model.rows.count) * 34 + 10))
        .onChange(of: model.selectedIndex) { _, index in
          guard model.rows.indices.contains(index) else { return }
          proxy.scrollTo(model.rows[index].id)
        }
      }
    }
  }

  private func rowView(_ row: PaletteRow, selected: Bool) -> some View {
    HStack(spacing: 10) {
      glyph(row.glyph)
        .frame(width: 18, height: 18)
      VStack(alignment: .leading, spacing: 1) {
        highlighted(row.title, positions: row.highlights)
          .font(ChromeFont.ui(13))
          .lineLimit(1)
          .truncationMode(.middle)
      }
      if let subtitle = row.subtitle {
        Text(subtitle)
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(palette.textTertiary)
          .lineLimit(1)
          .truncationMode(.head)
      }
      Spacer(minLength: 8)
      if let trailing = row.trailing {
        KeyHint(trailing)
      }
    }
    .padding(.horizontal, 12)
    .frame(height: 34)
    .background(
      RoundedRectangle(cornerRadius: Metrics.radius, style: .continuous)
        .fill(
          selected ? palette.selectionStrong : hoveredId == row.id ? palette.hover : .clear)
        .padding(.horizontal, 6)
    )
    .contentShape(Rectangle())
  }

  @ViewBuilder
  private func glyph(_ glyph: PaletteRow.Glyph?) -> some View {
    switch glyph {
    case .lucide(let icon):
      Icon(icon, size: 15).foregroundStyle(palette.textSecondary)
    case .image(let image):
      Image(nsImage: image).resizable().interpolation(.high).frame(width: 16, height: 16)
    case nil:
      Color.clear
    }
  }

  /// Title with fuzzy-matched characters emphasized.
  private func highlighted(_ text: String, positions: [Int]) -> Text {
    guard !positions.isEmpty else {
      return Text(text).foregroundColor(palette.text)
    }
    let marks = Set(positions)
    var result = Text("")
    var offset = 0
    var run = ""
    var runMatched = false
    func flush() {
      guard !run.isEmpty else { return }
      let piece = Text(run)
      result =
        result
        + (runMatched
          ? piece.foregroundColor(palette.accent).fontWeight(.semibold)
          : piece.foregroundColor(palette.text))
      run = ""
    }
    for character in text {
      let width = character.utf16.count
      let matched = marks.contains(offset)
      if matched != runMatched {
        flush()
        runMatched = matched
      }
      run.append(character)
      offset += width
    }
    flush()
    return result
  }

  // MARK: Footer

  private var footer: some View {
    HStack(spacing: 14) {
      hint("↑↓", "navigate")
      hint("⏎", enterLabel)
      hint("esc", "close")
      Spacer(minLength: 8)
      Text("> commands   : line   % text   b: branches   w: workspaces   h: history   ? more")
        .font(ChromeFont.ui(10.5))
        .foregroundStyle(palette.textTertiary)
        .lineLimit(1)
    }
    .padding(.horizontal, 14)
    .frame(height: 28)
  }

  private var enterLabel: String {
    switch model.mode {
    case .commands: return "run"
    case .history: return "insert"
    case .branches: return "switch"
    default: return "open"
    }
  }

  private func hint(_ key: String, _ label: String) -> some View {
    HStack(spacing: 5) {
      KeyHint(key)
      Text(label).font(ChromeFont.ui(10.5)).foregroundStyle(palette.textTertiary)
    }
  }
}

extension PaletteModel {
  /// Return pressed: help rows switch mode; everything else runs.
  func runSelectedOrActivate() {
    if rows.indices.contains(selectedIndex) {
      activate(rows[selectedIndex])
    } else {
      runSelected()
    }
  }
}
