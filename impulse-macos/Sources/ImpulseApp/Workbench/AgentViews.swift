import ImpulseKit
import SwiftUI

/// A coding agent's state at a glance: a spinner while it works, a bot
/// with a colored dot when it wants the user, a quiet bot otherwise.
struct AgentStatusGlyph: View {
  @Environment(\.chrome) private var chrome
  let state: AgentState
  var size: CGFloat = 13

  var body: some View {
    switch state {
    case .working:
      ProgressRing(progress: nil, color: chrome.working, size: size - 1, lineWidth: 1.6)
        .accessibilityLabel("Agent working")
    case .needsInput, .done:
      Icon(.bot, size: size)
        .foregroundStyle(chrome.textSecondary)
        .overlay(alignment: .topTrailing) {
          StatusDot(color: state == .needsInput ? chrome.attention : chrome.success, size: size * 0.45)
            .offset(x: size * 0.18, y: -size * 0.12)
        }
        .accessibilityLabel(state == .needsInput ? "Agent needs input" : "Agent finished")
    case .idle, .exited:
      Icon(.bot, size: size)
        .foregroundStyle(chrome.textTertiary)
        .accessibilityLabel("Agent idle")
    }
  }
}

extension AgentState {
  var label: String {
    switch self {
    case .idle: return "Idle"
    case .working: return "Working"
    case .needsInput: return "Needs your input"
    case .done: return "Finished"
    case .exited: return "Exited"
    }
  }
}

/// Every agent in the window, most urgent first; choosing one shows its
/// pane.
struct AgentInboxView: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let dismiss: () -> Void

  private static let relative: RelativeDateTimeFormatter = {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .abbreviated
    return formatter
  }()

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack {
        Text("Agents")
          .font(ChromeFont.ui(12, weight: .semibold))
          .foregroundStyle(chrome.text)
        Spacer()
        KeyHint("⌘⇧U").help("Go to the next agent that needs you")
      }
      .padding(.horizontal, 12)
      .frame(height: 30)
      Hairline()
      if model.agents.isEmpty {
        Text("No agents running. Start Claude Code, Codex, Gemini or another agent in any terminal.")
          .font(ChromeFont.ui(11.5))
          .foregroundStyle(chrome.textTertiary)
          .fixedSize(horizontal: false, vertical: true)
          .padding(12)
      } else {
        ScrollView {
          VStack(spacing: 1) {
            ForEach(model.agents) { agent in
              AgentInboxRow(
                agent: agent, relative: Self.relative,
                review: {
                  model.onReviewAgentTurn?(agent.id)
                  dismiss()
                }
              ) {
                model.onRevealTerminal?(agent.id)
                dismiss()
              }
            }
          }
          .padding(.vertical, 4)
        }
        .frame(maxHeight: 320)
      }
      Hairline()
      Button {
        model.onShowAgentHooks?()
        dismiss()
      } label: {
        HStack(spacing: 5) {
          Icon(.plug, size: 11)
          Text("Agent hooks give exact status…")
            .font(ChromeFont.ui(11))
          Spacer(minLength: 0)
        }
        .foregroundStyle(chrome.textTertiary)
        .padding(.horizontal, 12)
        .frame(height: 28)
        .contentShape(Rectangle())
      }
      .buttonStyle(ChromePressStyle())
    }
    .frame(width: 340)
    .background(chrome.overlay)
    .environment(\.chrome, model.palette)
  }
}

private struct AgentInboxRow: View {
  @Environment(\.chrome) private var chrome
  let agent: AgentSummary
  let relative: RelativeDateTimeFormatter
  let review: () -> Void
  let action: () -> Void
  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      HStack(spacing: 8) {
        AgentStatusGlyph(state: agent.state, size: 14)
          .frame(width: 18)
        VStack(alignment: .leading, spacing: 1) {
          Text("\(agent.agentName) · \(agent.tabTitle)")
            .font(ChromeFont.ui(12, weight: agent.state.wantsUser ? .semibold : .regular))
            .foregroundStyle(chrome.text)
            .lineLimit(1)
            .truncationMode(.middle)
          Text(
            [agent.workspaceName, agent.state.label, relative.localizedString(for: agent.since, relativeTo: Date())]
              .filter { !$0.isEmpty }.joined(separator: " · ")
          )
          .font(ChromeFont.ui(10.5))
          .foregroundStyle(agent.state == .needsInput ? chrome.attention : chrome.textTertiary)
          .lineLimit(1)
        }
        Spacer(minLength: 0)
        if agent.hasTurns && (hovering || agent.state == .done) {
          ChromeButton(title: "Review", icon: .fileDiff, kind: .secondary, help: "Review the last turn's changes")
          {
            review()
          }
        }
      }
      .padding(.horizontal, 12)
      .frame(height: 40)
      .rowBackground(selected: false, hovered: hovering)
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .onHover { hovering = $0 }
    .accessibilityLabel("\(agent.agentName), \(agent.tabTitle), \(agent.state.label)")
  }
}

/// Titlebar / status-bar entry to the inbox: shows how many agents want the
/// user and how many are working.
struct AgentInboxButton: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  var compact = false
  @State private var showing = false

  var body: some View {
    let waiting = model.agents.filter { $0.state.wantsUser }.count
    let working = model.agents.filter { $0.state == .working }.count
    Button {
      showing.toggle()
    } label: {
      HStack(spacing: 5) {
        Icon(waiting > 0 ? .bellDot : .bot, size: compact ? 11.5 : 13)
          .foregroundStyle(waiting > 0 ? chrome.attention : chrome.textSecondary)
        if working > 0 {
          ProgressRing(progress: nil, color: chrome.working, size: 10, lineWidth: 1.4)
          Text("\(working)").font(ChromeFont.mono(10.5)).foregroundStyle(chrome.textSecondary)
        }
        if waiting > 0 {
          Text("\(waiting) waiting")
            .font(ChromeFont.ui(compact ? 11 : 11.5, weight: .medium))
            .foregroundStyle(chrome.attention)
        }
      }
      .padding(.horizontal, compact ? 6 : 8)
      .frame(height: compact ? 20 : 24)
      .contentShape(Rectangle())
    }
    .buttonStyle(ChromePressStyle())
    .help("Agents (⌘⇧U jumps to the next one that needs you)")
    .accessibilityLabel("Agents: \(working) working, \(waiting) waiting")
    .popover(isPresented: $showing, arrowEdge: compact ? .top : .bottom) {
      AgentInboxView(model: model) { showing = false }
    }
  }
}

/// Under an agent's terminal while its TUI runs: what it's doing, and the
/// things you do around it — write to it, review its last turn, step back
/// through its turns.
struct AgentToolbelt: View {
  @Environment(\.chrome) private var chrome
  var model: WindowModel
  let agent: AgentSummary

  private static let time: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .none
    formatter.timeStyle = .short
    return formatter
  }()

  var body: some View {
    HStack(spacing: 8) {
      AgentStatusGlyph(state: agent.state, size: 12)
      Text(agent.agentName).font(ChromeFont.ui(11.5, weight: .semibold)).foregroundStyle(chrome.text)
      TimelineView(.periodic(from: .now, by: 1)) { context in
        Text(statusText(now: context.date)).font(ChromeFont.ui(11)).foregroundStyle(chrome.textSecondary)
          .monospacedDigit()
      }
      Spacer(minLength: 8)
      button("Compose", icon: .messageSquarePlus, hint: "⌘I") { model.onOpenComposer?() }
      button("Review Turn", icon: .fileDiff, hint: nil) { model.onReviewAgentTurn?(agent.id) }
        .disabled(!agent.hasTurns)
        .opacity(agent.hasTurns ? 1 : 0.5)
      ChromeMenuButton(help: "Turns this agent took: review one, or restore the files to before it") {
        let turns = model.agentTurns?(agent.id) ?? []
        guard !turns.isEmpty else { return [ChromeMenuItem("No turns yet", isEnabled: false) {}] }
        var items: [ChromeMenuItem] = []
        for turn in turns.prefix(12) {
          let label = "Turn \(turn.id + 1) · \(Self.time.string(from: turn.started))\(turn.finished ? "" : " (running)")"
          items.append(ChromeMenuItem("Review \(label)") { model.onReviewAgentTurnAt?(agent.id, turn.id) })
          items.append(ChromeMenuItem("Restore Files to Before \(label)…") { model.onRestoreAgentTurn?(agent.id, turn.id) })
          items.append(.separator)
        }
        return Array(items.dropLast())
      } label: {
        HStack(spacing: 4) {
          Icon(.history, size: 11)
          Text("Turns").font(ChromeFont.ui(11))
        }
        .foregroundStyle(chrome.textSecondary)
        .padding(.horizontal, 6)
        .frame(height: 22)
      }
    }
    .padding(.horizontal, 12)
    .frame(height: 32)
    .background(model.theme.colorBgDark)
    .overlay(alignment: .top) { Rectangle().fill(model.theme.colorBorder).frame(height: 1) }
  }

  private func statusText(now: Date) -> String {
    let seconds = max(0, Int(now.timeIntervalSince(agent.since)))
    let elapsed = seconds < 60 ? "\(seconds)s" : seconds < 3600 ? "\(seconds / 60)m" : "\(seconds / 3600)h \(seconds / 60 % 60)m"
    switch agent.state {
    case .working: return "Working · \(elapsed)"
    case .needsInput: return "Needs your input"
    case .done: return "Finished \(elapsed) ago"
    case .idle: return "Idle"
    case .exited: return "Exited"
    }
  }

  private func button(_ title: String, icon: LucideIcon, hint: String?, action: @escaping () -> Void) -> some View {
    Button(action: action) {
      HStack(spacing: 4) {
        Icon(icon, size: 11)
        Text(title).font(ChromeFont.ui(11))
        if let hint { Text(hint).font(ChromeFont.ui(10)).foregroundStyle(chrome.textTertiary) }
      }
      .foregroundStyle(chrome.textSecondary)
      .padding(.horizontal, 6)
      .frame(height: 22)
      .background(RoundedRectangle(cornerRadius: 5).fill(chrome.raised))
    }
    .buttonStyle(.plain)
  }
}
