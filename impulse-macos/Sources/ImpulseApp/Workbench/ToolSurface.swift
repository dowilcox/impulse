import AppKit

/// An app tool shown as a tab (Settings, Keybindings, …): one per window,
/// found again by `toolKind` when reopened. Not saved with the session.
protocol ToolSurface: NSView {
  /// Identifies the tool ("settings", "keybindings").
  var toolKind: String { get }
  var toolTitle: String { get }
  /// SF Symbol for the tab.
  var toolSymbol: String { get }
  func focusTool()
  func applyToolTheme(_ theme: Theme)
  func cleanupTool()
}

extension ToolSurface {
  func focusTool() { window?.makeFirstResponder(self) }
  func cleanupTool() {}
}
