import AppKit
import ImpulseKit
import SwiftUI

/// Every diagnostic the language servers report, by file: filter by
/// severity or text, click to jump, and hand them to an agent.
@Observable
final class ProblemsModel {
  var severities: Set<Problem.Severity> = Set(Problem.Severity.allCases)
  var filter = ""
  var collapsed: Set<String> = []
  var palette: ChromePalette
  /// The window's diagnostics (read live).
  @ObservationIgnored weak var window: WindowModel?
  @ObservationIgnored var root: () -> String? = { nil }
  @ObservationIgnored var onOpen: ((Problem) -> Void)?
  @ObservationIgnored var agents: () -> [AgentSummary] = { [] }
  @ObservationIgnored var onSendToAgent: ((String, UUID) -> Void)?

  init(palette: ChromePalette) {
    self.palette = palette
  }

  var all: [Problem] { window?.problemsByPath.values.flatMap { $0 } ?? [] }
  var visible: [Problem] { Problems.filter(all, severities: severities, text: filter) }

  func relative(_ path: String) -> String {
    let root = root()
    if path == root { return "" }
    guard let root, path.hasPrefix(root + "/") else { return TabManager.abbreviateHomePath(path) }
    return String(path.dropFirst(root.count + 1))
  }

  func toggle(_ severity: Problem.Severity) {
    if severities.contains(severity) { severities.remove(severity) } else { severities.insert(severity) }
  }
}

final class ProblemsSurface: NSView, ToolSurface {
  let model: ProblemsModel

  var toolKind: String { "problems" }
  var toolTitle: String { "Problems" }
  var toolSymbol: String { "exclamationmark.triangle" }

  init(palette: ChromePalette) {
    model = ProblemsModel(palette: palette)
    super.init(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
    let hosting = WorkbenchHosting.make(ProblemsView(model: model))
    hosting.translatesAutoresizingMaskIntoConstraints = false
    addSubview(hosting)
    NSLayoutConstraint.activate([
      hosting.topAnchor.constraint(equalTo: topAnchor),
      hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
      hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
      hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
    ])
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  func applyToolTheme(_ theme: Theme) {
    model.palette = ChromePalette(theme: theme)
  }
}

struct ProblemsView: View {
  var model: ProblemsModel

  var body: some View {
    let chrome = model.palette
    let visible = model.visible
    let counts = Problems.counts(model.all)
    VStack(spacing: 0) {
      HStack(spacing: 8) {
        Text("Problems").font(ChromeFont.ui(14, weight: .semibold)).foregroundStyle(chrome.text)
        severityToggle(.error, count: counts.errors, icon: .circleX, color: chrome.danger)
        severityToggle(.warning, count: counts.warnings, icon: .triangleAlert, color: chrome.warning)
        severityToggle(.info, count: counts.others, icon: .info, color: chrome.info)
        Spacer()
        HStack(spacing: 6) {
          Icon(.listFilter, size: 12).foregroundStyle(chrome.textTertiary)
          TextField("Filter", text: Binding(get: { model.filter }, set: { model.filter = $0 }))
            .textFieldStyle(.plain)
            .font(ChromeFont.ui(12))
            .frame(width: 160)
        }
        .padding(.horizontal, 8)
        .frame(height: 26)
        .background(RoundedRectangle(cornerRadius: Metrics.radius).fill(chrome.raised))
        ChromeMenuButton(help: "Send to an agent or copy as a prompt") {
          let prompt = Problems.prompt(visible, root: model.root())
          var items = model.agents().map { agent in
            ChromeMenuItem("Send to \(agent.agentName) · \(agent.tabTitle)", isEnabled: !visible.isEmpty) {
              model.onSendToAgent?(prompt, agent.id)
            }
          }
          if !items.isEmpty { items.append(.separator) }
          items.append(
            ChromeMenuItem("Copy as Prompt", isEnabled: !visible.isEmpty) {
              NSPasteboard.general.clearContents()
              NSPasteboard.general.setString(prompt, forType: .string)
            })
          return items
        } label: {
          HStack(spacing: 4) {
            Icon(.bot, size: 12)
            Text("Fix with Agent").font(ChromeFont.ui(11.5, weight: .medium))
          }
          .foregroundStyle(chrome.textSecondary)
          .padding(.horizontal, 6)
          .frame(height: 24)
        }
      }
      .padding(.horizontal, 14)
      .frame(height: 44)
      .background(chrome.panel)
      Rectangle().fill(chrome.hairline).frame(height: 1)

      if visible.isEmpty {
        VStack(spacing: 6) {
          Icon(.circleCheck, size: 22).foregroundStyle(chrome.success)
          Text(model.all.isEmpty ? "No problems reported." : "Nothing matches the filter.")
            .font(ChromeFont.ui(12.5)).foregroundStyle(chrome.textSecondary)
          if model.all.isEmpty {
            Text("Language servers report problems for the files they've checked.")
              .font(ChromeFont.ui(11.5)).foregroundStyle(chrome.textTertiary)
          }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 0) {
            ForEach(Problems.grouped(visible), id: \.path) { group in
              fileHeader(group.path, count: group.problems.count)
              if !model.collapsed.contains(group.path) {
                ForEach(group.problems, id: \.self) { problem in
                  ProblemRow(model: model, problem: problem)
                }
              }
            }
          }
          .padding(.vertical, 6)
        }
      }
    }
    .background(chrome.content)
    .environment(\.chrome, chrome)
  }

  private func severityToggle(_ severity: Problem.Severity, count: Int, icon: LucideIcon, color: Color) -> some View {
    let chrome = model.palette
    let on = model.severities.contains(severity)
    return Button {
      model.toggle(severity)
      if severity == .info { if on { model.severities.remove(.hint) } else { model.severities.insert(.hint) } }
    } label: {
      HStack(spacing: 4) {
        Icon(icon, size: 11).foregroundStyle(on ? color : chrome.textTertiary)
        Text(verbatim: "\(count)").font(ChromeFont.mono(11)).foregroundStyle(on ? chrome.text : chrome.textTertiary)
      }
      .padding(.horizontal, 6)
      .frame(height: 22)
      .background(RoundedRectangle(cornerRadius: 4).fill(on ? chrome.raised : .clear))
    }
    .buttonStyle(.plain)
    .help(on ? "Hide \(severity.label)s" : "Show \(severity.label)s")
  }

  private func fileHeader(_ path: String, count: Int) -> some View {
    let chrome = model.palette
    let collapsed = model.collapsed.contains(path)
    return Button {
      if collapsed { model.collapsed.remove(path) } else { model.collapsed.insert(path) }
    } label: {
      HStack(spacing: 6) {
        Icon(collapsed ? .chevronRight : .chevronDown, size: 11).foregroundStyle(chrome.textTertiary)
        Icon(.file, size: 12).foregroundStyle(chrome.textSecondary)
        Text((path as NSString).lastPathComponent).font(ChromeFont.ui(12, weight: .semibold))
          .foregroundStyle(chrome.text)
        Text(model.relative((path as NSString).deletingLastPathComponent))
          .font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).lineLimit(1).truncationMode(.head)
        Text(verbatim: "\(count)").font(ChromeFont.mono(10)).foregroundStyle(chrome.textTertiary)
        Spacer()
      }
      .padding(.horizontal, 14)
      .frame(height: 26)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }
}

private struct ProblemRow: View {
  @Environment(\.chrome) private var chrome
  var model: ProblemsModel
  let problem: Problem
  @State private var hovering = false

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 8) {
      icon
      Text(problem.message)
        .font(ChromeFont.ui(12))
        .foregroundStyle(chrome.text)
        .lineLimit(2)
        .textSelection(.enabled)
      if let source = problem.source {
        Text([source, problem.code].compactMap { $0 }.joined(separator: " "))
          .font(ChromeFont.ui(11)).foregroundStyle(chrome.textTertiary).lineLimit(1)
      }
      Spacer(minLength: 8)
      Text(verbatim: "\(problem.line):\(problem.column)")
        .font(ChromeFont.mono(10.5)).foregroundStyle(chrome.textTertiary)
    }
    .padding(.leading, 44)
    .padding(.trailing, 14)
    .padding(.vertical, 4)
    .background(hovering ? chrome.hover : .clear)
    .contentShape(Rectangle())
    .onHover { hovering = $0 }
    .onTapGesture { model.onOpen?(problem) }
    .accessibilityElement(children: .combine)
    .accessibilityAddTraits(.isButton)
  }

  @ViewBuilder private var icon: some View {
    switch problem.severity {
    case .error: Icon(.circleX, size: 12).foregroundStyle(chrome.danger)
    case .warning: Icon(.triangleAlert, size: 12).foregroundStyle(chrome.warning)
    case .info, .hint: Icon(.info, size: 12).foregroundStyle(chrome.info)
    }
  }
}
