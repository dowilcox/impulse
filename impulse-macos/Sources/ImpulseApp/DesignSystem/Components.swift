import AppKit
import SwiftUI

// MARK: - Environment

private struct ChromePaletteKey: EnvironmentKey {
  static let defaultValue = ChromePalette(theme: ThemeManager.theme(forName: "nord"))
}

extension EnvironmentValues {
  /// The chrome palette for the window's current theme. Injected at the root
  /// of every hosting view by `WorkbenchHosting`.
  var chrome: ChromePalette {
    get { self[ChromePaletteKey.self] }
    set { self[ChromePaletteKey.self] = newValue }
  }
}

// MARK: - Icon button

/// Square, borderless icon button with a hover/pressed tint (no system bezel).
struct ChromeIconButton: View {
  @Environment(\.chrome) private var chrome
  let icon: LucideIcon
  let help: String
  var size: CGFloat = 24
  var iconSize: CGFloat = Metrics.icon
  var isActive: Bool = false
  var tint: Color? = nil
  let action: () -> Void

  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      Icon(icon, size: iconSize)
        .foregroundStyle(tint ?? (isActive ? chrome.text : chrome.textSecondary))
        .frame(width: size, height: size)
        .background(
          RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
            .fill(isActive ? chrome.pressed : hovering ? chrome.hover : .clear)
        )
        .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .help(help)
    .accessibilityLabel(help)
  }
}

/// Dims slightly while pressed; no system bezel or focus halo.
struct ChromePressStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.opacity(configuration.isPressed ? 0.7 : 1)
  }
}

// MARK: - Text button

/// Compact text button: `.primary` (accent fill), `.secondary` (raised fill),
/// `.ghost` (hover tint only), or `.danger`.
struct ChromeButton: View {
  enum Kind { case primary, secondary, ghost, danger }

  @Environment(\.chrome) private var chrome
  let title: String
  var icon: LucideIcon? = nil
  var kind: Kind = .secondary
  var help: String? = nil
  let action: () -> Void

  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      HStack(spacing: 5) {
        if let icon { Icon(icon, size: Metrics.iconSmall) }
        Text(title).font(ChromeFont.ui(11.5, weight: .medium)).lineLimit(1)
      }
      .padding(.horizontal, 9)
      .frame(height: 22)
      .foregroundStyle(foreground)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radius - 1, style: .continuous).fill(background)
      )
      .overlay(
        RoundedRectangle(cornerRadius: Metrics.radius - 1, style: .continuous)
          .strokeBorder(kind == .secondary ? chrome.hairlineStrong : .clear, lineWidth: 1)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .help(help ?? title)
  }

  private var foreground: Color {
    switch kind {
    case .primary: return chrome.textOnAccent
    case .danger: return chrome.danger
    case .secondary, .ghost: return chrome.text
    }
  }

  private var background: Color {
    switch kind {
    case .primary: return hovering ? chrome.accent.opacity(0.88) : chrome.accent
    case .secondary: return hovering ? chrome.raised.opacity(0.9) : chrome.raised
    case .ghost, .danger: return hovering ? chrome.hover : .clear
    }
  }
}

// MARK: - Chip

/// Small pill carrying context (cwd, branch, diff stats). Optional action.
struct ChromeChip<Content: View>: View {
  @Environment(\.chrome) private var chrome
  var help: String? = nil
  var action: (() -> Void)? = nil
  @ViewBuilder let content: () -> Content

  @State private var hovering = false

  var body: some View {
    let label = HStack(spacing: 5) { content() }
      .font(ChromeFont.mono(11))
      .foregroundStyle(chrome.textSecondary)
      .padding(.horizontal, 7)
      .frame(height: 20)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
          .fill(hovering && action != nil ? chrome.pressed : chrome.raised)
      )
      .contentShape(Rectangle())

    if let action {
      Button(action: action) { label }
        .buttonStyle(ChromePressStyle())
        .onHover { hovering = $0 }
        .help(help ?? "")
    } else {
      label.help(help ?? "")
    }
  }
}

// MARK: - Small indicators

/// Inline keyboard hint, e.g. `KeyHint("⌘P")`.
struct KeyHint: View {
  @Environment(\.chrome) private var chrome
  let keys: String

  init(_ keys: String) { self.keys = keys }

  var body: some View {
    Text(keys)
      .font(ChromeFont.ui(10.5, weight: .medium))
      .foregroundStyle(chrome.textTertiary)
      .padding(.horizontal, 4)
      .frame(height: 16)
      .overlay(
        RoundedRectangle(cornerRadius: 3, style: .continuous)
          .strokeBorder(chrome.hairlineStrong, lineWidth: 1)
      )
  }
}

/// Colored status dot.
struct StatusDot: View {
  let color: Color
  var size: CGFloat = 7

  var body: some View {
    Circle().fill(color).frame(width: size, height: size).accessibilityHidden(true)
  }
}

/// Count/label badge.
struct Badge: View {
  @Environment(\.chrome) private var chrome
  let text: String
  var color: Color? = nil

  var body: some View {
    Text(text)
      .font(ChromeFont.ui(10, weight: .bold))
      .foregroundStyle(color == nil ? chrome.textSecondary : chrome.textOnAccent)
      .padding(.horizontal, 5)
      .frame(minWidth: 16, minHeight: 15)
      .background(Capsule().fill(color ?? chrome.raised))
  }
}

/// Determinate ring (0...1) or an indeterminate spinner when `progress` is nil.
struct ProgressRing: View {
  let progress: Double?
  let color: Color
  var size: CGFloat = 12
  var lineWidth: CGFloat = 2

  @State private var spin = false
  @Environment(\.accessibilityReduceMotion) private var reduceMotion

  var body: some View {
    ZStack {
      Circle().stroke(color.opacity(0.25), lineWidth: lineWidth)
      if let progress {
        Circle()
          .trim(from: 0, to: max(0.02, min(1, progress)))
          .stroke(color, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
          .rotationEffect(.degrees(-90))
      } else {
        Circle()
          .trim(from: 0, to: 0.28)
          .stroke(color, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
          .rotationEffect(.degrees(spin ? 360 : 0))
          .animation(
            reduceMotion ? nil : .linear(duration: 0.9).repeatForever(autoreverses: false), value: spin
          )
          .onAppear { spin = !reduceMotion }
      }
    }
    .frame(width: size, height: size)
    .accessibilityLabel(progress.map { "\(Int($0 * 100)) percent" } ?? "In progress")
  }
}

/// `+12 −3` diff counts in git colors.
struct DiffStat: View {
  @Environment(\.chrome) private var chrome
  let added: Int
  let removed: Int

  var body: some View {
    HStack(spacing: 4) {
      if added > 0 || removed == 0 {
        Text("+\(added)").foregroundStyle(chrome.gitAdded)
      }
      if removed > 0 {
        Text("−\(removed)").foregroundStyle(chrome.gitDeleted)
      }
    }
    .font(ChromeFont.mono(11))
    .monospacedDigit()
  }
}

// MARK: - Section header

/// Uppercase panel section header with optional count and trailing actions.
struct SectionHeader<Trailing: View>: View {
  @Environment(\.chrome) private var chrome
  let title: String
  var count: Int? = nil
  var isExpanded: Binding<Bool>? = nil
  @ViewBuilder var trailing: () -> Trailing

  var body: some View {
    HStack(spacing: 6) {
      if let isExpanded {
        Icon(isExpanded.wrappedValue ? .chevronDown : .chevronRight, size: 11)
          .foregroundStyle(chrome.textTertiary)
      }
      Text(title.uppercased())
        .font(ChromeFont.ui(10.5, weight: .semibold))
        .tracking(0.6)
        .foregroundStyle(chrome.textTertiary)
      if let count {
        Text("\(count)")
          .font(ChromeFont.ui(10.5, weight: .semibold))
          .foregroundStyle(chrome.textTertiary)
          .monospacedDigit()
      }
      Spacer(minLength: 4)
      trailing()
    }
    .padding(.horizontal, 12)
    .frame(height: 26)
    .contentShape(Rectangle())
    .onTapGesture {
      if let isExpanded { isExpanded.wrappedValue.toggle() }
    }
  }
}

extension SectionHeader where Trailing == EmptyView {
  init(title: String, count: Int? = nil, isExpanded: Binding<Bool>? = nil) {
    self.init(title: title, count: count, isExpanded: isExpanded) { EmptyView() }
  }
}

// MARK: - Row background

/// Hover/selection background for list rows.
struct RowBackground: ViewModifier {
  @Environment(\.chrome) private var chrome
  let isSelected: Bool
  let isHovered: Bool

  func body(content: Content) -> some View {
    content.background(
      RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
        .fill(isSelected ? chrome.selection : isHovered ? chrome.hover : .clear)
        .padding(.horizontal, 6)
    )
  }
}

extension View {
  func rowBackground(selected: Bool, hovered: Bool) -> some View {
    modifier(RowBackground(isSelected: selected, isHovered: hovered))
  }
}

// MARK: - Hairline

struct Hairline: View {
  @Environment(\.chrome) private var chrome
  var vertical = false
  var strong = false

  var body: some View {
    Rectangle()
      .fill(strong ? chrome.hairlineStrong : chrome.hairline)
      .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
  }
}

// MARK: - Popup menu button

/// One entry in a `ChromeMenuButton` menu.
struct ChromeMenuItem {
  var title: String
  var isEnabled = true
  var isSeparator = false
  var action: () -> Void = {}

  static let separator = ChromeMenuItem(title: "", isSeparator: true)

  init(_ title: String, isEnabled: Bool = true, action: @escaping () -> Void) {
    self.title = title
    self.isEnabled = isEnabled
    self.action = action
  }

  private init(title: String, isSeparator: Bool) {
    self.title = title
    self.isSeparator = isSeparator
  }
}

/// A chrome button (any label) that pops up a native menu below itself.
/// Used instead of SwiftUI `Menu`, whose borderless style rescales icon
/// labels and can't be styled to match the chrome.
struct ChromeMenuButton<Label: View>: View {
  let help: String
  let items: () -> [ChromeMenuItem]
  @ViewBuilder let label: () -> Label

  @State private var hovering = false
  @Environment(\.chrome) private var chrome

  var body: some View {
    label()
      .padding(.horizontal, 4)
      .frame(minWidth: 22, minHeight: 22)
      .background(
        RoundedRectangle(cornerRadius: Metrics.radiusSmall + 1, style: .continuous)
          .fill(hovering ? chrome.hover : .clear)
      )
      .contentShape(Rectangle())
      .overlay(MenuAnchor(items: items))
      .onHover { hovering = $0 }
      .help(help)
      .accessibilityLabel(help)
      .accessibilityAddTraits(.isButton)
  }
}

/// Transparent AppKit view that shows the menu on mouse down, anchored to
/// its own bottom-left corner.
private struct MenuAnchor: NSViewRepresentable {
  let items: () -> [ChromeMenuItem]

  func makeNSView(context: Context) -> AnchorView {
    let view = AnchorView()
    view.items = items
    return view
  }

  func updateNSView(_ nsView: AnchorView, context: Context) {
    nsView.items = items
  }

  final class AnchorView: NSView {
    var items: (() -> [ChromeMenuItem])?

    override func mouseDown(with event: NSEvent) {
      guard let items else { return }
      let menu = NSMenu()
      menu.autoenablesItems = false
      for item in items() {
        if item.isSeparator {
          menu.addItem(.separator())
          continue
        }
        let menuItem = ClosureMenuItem(title: item.title, action: item.action)
        menuItem.isEnabled = item.isEnabled
        menu.addItem(menuItem)
      }
      menu.popUp(positioning: nil, at: NSPoint(x: 0, y: isFlipped ? bounds.height + 4 : -4), in: self)
    }
  }
}

/// NSMenuItem that runs a closure.
final class ClosureMenuItem: NSMenuItem {
  private let handler: () -> Void

  init(title: String, action: @escaping () -> Void) {
    handler = action
    super.init(title: title, action: #selector(run), keyEquivalent: "")
    target = self
  }

  @available(*, unavailable)
  required init(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  @objc private func run() { handler() }
}
