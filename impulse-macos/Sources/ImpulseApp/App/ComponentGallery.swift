import AppKit
import ImpulseKit
import SwiftUI

/// Dev builds only: the chrome's components in every built-in theme, with
/// and without Increase Contrast, to check a theme (or a component change)
/// at a glance. "Component Gallery" in the command palette.
final class ComponentGalleryWindowController: NSWindowController {
  private static var shared: ComponentGalleryWindowController?

  static func show(light: Bool? = nil, increaseContrast: Bool = false) {
    let controller = shared ?? ComponentGalleryWindowController()
    shared = controller
    controller.window?.contentView = NSHostingView(
      rootView: ComponentGalleryView(light: light, increaseContrast: increaseContrast))
    controller.showWindow(nil)
    controller.window?.makeKeyAndOrderFront(nil)
  }

  private init() {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1180, height: 820),
      styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
    window.title = "Component Gallery"
    window.isReleasedWhenClosed = false
    window.contentView = NSHostingView(rootView: ComponentGalleryView())
    window.center()
    super.init(window: window)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}

struct ComponentGalleryView: View {
  /// nil: all themes; true / false: light or dark ones.
  @State var light: Bool?
  @State var increaseContrast = false

  private var themes: [String] {
    ThemeStore.builtinThemeNames().filter { name in
      light.map { ThemeManager.theme(forName: name).isLight == $0 } ?? true
    }
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text("\(themes.count) built-in themes").font(.headline)
        Picker("", selection: $light) {
          Text("All").tag(Bool?.none)
          Text("Dark").tag(Bool?.some(false))
          Text("Light").tag(Bool?.some(true))
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .frame(width: 200)
        Spacer()
        Toggle("Increase Contrast", isOn: $increaseContrast)
      }
      .padding(12)
      Divider()
      ScrollView {
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)], spacing: 12) {
          ForEach(themes, id: \.self) { name in
            ThemeSampleCard(name: name, increaseContrast: increaseContrast)
          }
        }
        .padding(12)
      }
    }
  }
}

/// One theme's chrome: surfaces, controls, list rows, text, status and git
/// colors, and a toast.
private struct ThemeSampleCard: View {
  let name: String
  let increaseContrast: Bool

  var body: some View {
    let theme = ThemeManager.theme(forName: name)
    let chrome = ChromePalette(theme: theme, increaseContrast: increaseContrast)
    VStack(alignment: .leading, spacing: 0) {
      // Titlebar-like strip.
      HStack(spacing: 8) {
        Text(ThemeManager.displayName(for: name))
          .font(ChromeFont.ui(12, weight: .semibold))
          .foregroundStyle(chrome.text)
        Text(theme.isLight ? "light" : "dark")
          .font(ChromeFont.ui(10))
          .foregroundStyle(chrome.textTertiary)
        Spacer()
        ChromeChip {
          Icon(.gitBranch, size: 11)
          Text("main")
        }
        ChromeChip { DiffStat(added: 12, removed: 3) }
        ChromeIconButton(icon: .search, help: "Search") {}
        ChromeIconButton(icon: .panelRight, help: "Panel", isActive: true) {}
      }
      .padding(.horizontal, 10)
      .frame(height: 36)
      .background(chrome.chrome)
      Hairline()

      HStack(alignment: .top, spacing: 0) {
        // Panel: section header and rows.
        VStack(alignment: .leading, spacing: 2) {
          SectionHeader(title: "Changes", count: 3)
          sampleRow("cache.swift", status: "M", color: chrome.gitModified, selected: false, hovered: false)
          sampleRow("new.txt", status: "U", color: chrome.gitUntracked, selected: false, hovered: true)
          sampleRow("README.md", status: "A", color: chrome.gitAdded, selected: true, hovered: false)
          sampleRow("old.swift", status: "D", color: chrome.gitDeleted, selected: true, hovered: false, focused: true)
          sampleRow("moved.swift", status: "R", color: chrome.gitRenamed, selected: false, hovered: false)
          sampleRow("conflict.swift", status: "C", color: chrome.gitConflict, selected: false, hovered: false)
        }
        .padding(.vertical, 6)
        .frame(width: 220, alignment: .top)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(chrome.panel)
        Hairline(vertical: true)

        // Content: text levels, controls, status colors.
        VStack(alignment: .leading, spacing: 10) {
          VStack(alignment: .leading, spacing: 3) {
            Text("Primary text").foregroundStyle(chrome.text)
            Text("Secondary text").foregroundStyle(chrome.textSecondary)
            Text("Tertiary text").foregroundStyle(chrome.textTertiary)
          }
          .font(ChromeFont.ui(12))
          HStack(spacing: 6) {
            ChromeButton(title: "Commit", icon: .check, kind: .primary) {}
            ChromeButton(title: "Stage", kind: .secondary) {}
            ChromeButton(title: "Later", kind: .ghost) {}
            ChromeButton(title: "Discard", kind: .danger) {}
          }
          HStack(spacing: 10) {
            ForEach(
              [("ok", chrome.success), ("warn", chrome.warning), ("err", chrome.danger), ("info", chrome.info),
               ("busy", chrome.working), ("you", chrome.attention)],
              id: \.0
            ) { label, color in
              HStack(spacing: 4) {
                StatusDot(color: color)
                Text(label).font(ChromeFont.ui(11)).foregroundStyle(chrome.textSecondary)
              }
            }
          }
          HStack(spacing: 8) {
            Badge(text: "3")
            Badge(text: "12", color: chrome.accent)
            KeyHint("⌘⇧P")
            ProgressRing(progress: 0.6, color: chrome.working, size: 14, lineWidth: 2)
            ProgressRing(progress: nil, color: chrome.textSecondary, size: 14, lineWidth: 2)
            Text("accent").font(ChromeFont.ui(11, weight: .medium)).foregroundStyle(chrome.accent)
              .padding(.horizontal, 6).padding(.vertical, 2)
              .background(Capsule().fill(chrome.accentSoft))
          }
          ToastView(
            toast: Toast(kind: .success, message: "Committed 3 files", detail: "on main", actionTitle: "Undo") {},
            palette: chrome, dismiss: {})
        }
        .padding(10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(chrome.content)
      }
      .fixedSize(horizontal: false, vertical: true)
      Hairline()
      // Status bar.
      HStack(spacing: 10) {
        Label("main", systemImage: "arrow.triangle.branch")
        Text("3 changes")
        Spacer()
        Text("Ln 12, Col 4")
      }
      .font(ChromeFont.ui(11))
      .foregroundStyle(chrome.textSecondary)
      .padding(.horizontal, 10)
      .frame(height: 24)
      .background(chrome.chrome)
    }
    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(chrome.hairlineStrong))
    .environment(\.chrome, chrome)
  }

  private func sampleRow(
    _ name: String, status: String, color: Color, selected: Bool, hovered: Bool, focused: Bool = false
  ) -> some View {
    GalleryRow(name: name, status: status, color: color)
      .rowBackground(selected: selected, hovered: hovered, focused: focused)
  }
}

private struct GalleryRow: View {
  @Environment(\.chrome) private var chrome
  let name: String
  let status: String
  let color: Color

  var body: some View {
    HStack(spacing: 6) {
      Icon(.file, size: 12).foregroundStyle(chrome.textTertiary)
      Text(name).font(ChromeFont.ui(12)).foregroundStyle(chrome.text)
      Spacer()
      Text(status).font(ChromeFont.mono(11, weight: .bold)).foregroundStyle(color)
    }
    .padding(.horizontal, 14)
    .frame(height: 24)
  }
}
