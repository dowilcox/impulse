import Foundation
import CImpulseFFI

enum CloseRiskAction: String, Codable {
  case quit
  case closeWindow = "close_window"
  case closeTab = "close_tab"
}

struct CloseRiskCommand: Codable {
  var command: String?
  var cwd: String?
  var startedAtMs: UInt64

  enum CodingKeys: String, CodingKey {
    case command
    case cwd
    case startedAtMs = "started_at_ms"
  }
}

struct CloseRiskInput: Codable {
  var action: CloseRiskAction
  var unsavedEditorCount: Int
  var runningTerminalProcessCount: Int
  var runningCommands: [CloseRiskCommand]
  var nowMs: UInt64
  var longCommandThresholdSeconds: UInt64

  enum CodingKeys: String, CodingKey {
    case action
    case unsavedEditorCount = "unsaved_editor_count"
    case runningTerminalProcessCount = "running_terminal_process_count"
    case runningCommands = "running_commands"
    case nowMs = "now_ms"
    case longCommandThresholdSeconds = "long_command_threshold_seconds"
  }
}

struct CloseRiskSummary: Codable {
  var hasRisk: Bool
  var title: String
  var informativeText: String
  var detailLines: [String]
  var destructiveActionTitle: String
  var cancelTitle: String

  enum CodingKeys: String, CodingKey {
    case hasRisk = "has_risk"
    case title
    case informativeText = "informative_text"
    case detailLines = "detail_lines"
    case destructiveActionTitle = "destructive_action_title"
    case cancelTitle = "cancel_title"
  }
}

extension ImpulseCore {
  static func closeRiskSummary(input: CloseRiskInput) -> CloseRiskSummary? {
    let encoder = JSONEncoder()
    guard let data = try? encoder.encode(input),
      let json = String(data: data, encoding: .utf8)
    else { return nil }

    guard let raw = json.withCString({ CImpulseFFI.impulse_close_risk_summary($0) }) else {
      return nil
    }
    defer { CImpulseFFI.impulse_free_string(raw) }

    let response = String(cString: raw)
    guard let responseData = response.data(using: .utf8) else { return nil }
    return try? JSONDecoder().decode(CloseRiskSummary.self, from: responseData)
  }
}

func currentUnixTimeMs() -> UInt64 {
  UInt64((Date().timeIntervalSince1970 * 1000).rounded())
}
