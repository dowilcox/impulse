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
  /// Whether the tool holds changes that closing it would lose (its tab
  /// title gets a `*`, and closing asks first).
  var toolIsModified: Bool { get }
  /// Save the tool's changes (⌘S, or Save when closing asks); `completion`
  /// says whether they were saved.
  func saveTool(completion: @escaping (Bool) -> Void)
}

extension ToolSurface {
  func focusTool() { window?.makeFirstResponder(self) }
  func cleanupTool() {}
  var toolIsModified: Bool { false }
  func saveTool(completion: @escaping (Bool) -> Void) { completion(true) }
}
