import Foundation
import ImpulseKit

// The close-risk, command-palette, search-result, shell, glob, and update
// logic lives in ImpulseKit (ported from impulse-core). These typealiases
// keep the app-wide names working without a per-file import.
typealias CloseRiskAction = ImpulseKit.CloseRiskAction
typealias CloseRiskCommand = ImpulseKit.RunningCommandRisk
typealias CloseRiskInput = ImpulseKit.CloseRiskInput
typealias CloseRiskSummary = ImpulseKit.CloseRiskSummary
typealias SearchResult = ImpulseKit.SearchResult
typealias CommandPaletteItem = ImpulseKit.CommandPaletteItem
typealias RecentCommandItem = ImpulseKit.RecentCommandItem
typealias RecentCommandStore = ImpulseKit.RecentCommandStore
typealias CommandPalette = ImpulseKit.CommandPalette
typealias Glob = ImpulseKit.Glob
typealias LoginShell = ImpulseKit.LoginShell
typealias ShellIntegration = ImpulseKit.ShellIntegration
typealias UpdateChecker = ImpulseKit.UpdateChecker
typealias MarkdownPreview = ImpulseKit.MarkdownPreview
typealias SVGPreview = ImpulseKit.SVGPreview
typealias MarkdownThemeColors = ImpulseKit.MarkdownThemeColors
typealias TextSpan = ImpulseKit.TextSpan
typealias CompletionCandidate = ImpulseKit.CompletionCandidate
typealias CompletionResult = ImpulseKit.CompletionResult
typealias InputCompletion = ImpulseKit.InputCompletion

extension ImpulseKit.SearchResult {
  /// Stable identity for SwiftUI ForEach diffing. Combines path, line, and
  /// column to uniquely identify each result without relying on array offset.
  var stableId: String {
    "\(path):\(lineNumber ?? 0):\(columnStart ?? 0)"
  }
}

/// Shell integration script for a shell name, or nil when unavailable —
/// optional-returning shim matching the old FFI wrapper shape.
func shellIntegrationScript(forShell shell: String) -> String? {
  let script = ShellIntegration.script(forShell: shell)
  return script.isEmpty ? nil : script
}

enum AppVersion {
  /// The app version from Info.plist (stamped by build.sh). Falls back to
  /// "0.0.0" when running outside a bundle (e.g. bare `swift build` output).
  static let current: String =
    Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.0.0"
}

func currentUnixTimeMs() -> UInt64 {
  UInt64((Date().timeIntervalSince1970 * 1000).rounded())
}
