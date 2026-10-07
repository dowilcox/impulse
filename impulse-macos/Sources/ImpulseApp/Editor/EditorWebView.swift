import AppKit
import WebKit

/// The web view Monaco runs in. Monaco binds many ⌘ keys of its own (⌘D add
/// next occurrence, ⌘G and ⇧⌘G find next/previous, ⇧⌘↩ insert line above,
/// …), and a key the page handles never reaches the menus, so Impulse's
/// command on the same keys wouldn't run while an editor has focus.
///
/// Impulse's shortcuts win: a key bound to one of its commands goes to the
/// menus first. Monaco keeps the keys of commands that would do nothing
/// here or that the editor carries out itself: copy and paste, Find (⌘F)
/// and Save (⌘S), and the terminal's commands (blocks, hints, the agent
/// composer), so ⇧⌘K still deletes a line and ⇧⌘Space shows parameter
/// hints. A command the user unbinds or moves gives its old keys back to
/// Monaco.
final class EditorWebView: WKWebView {
  override func performKeyEquivalent(with event: NSEvent) -> Bool {
    if hasKeyboardFocus, Self.appCommandWins(event), NSApp.mainMenu?.performKeyEquivalent(with: event) == true {
      return true
    }
    return super.performKeyEquivalent(with: event)
  }

  /// Key equivalents are offered to every view in the window; only claim
  /// them while this editor has focus.
  private var hasKeyboardFocus: Bool {
    guard let responder = window?.firstResponder as? NSView else { return false }
    return responder === self || responder.isDescendant(of: self)
  }

  /// Commands the editor does itself (or that only act on a terminal).
  private static let editorKeeps: Set<String> = ["copy", "paste", "find", "save"]
  private static let terminalCategories: Set<String> = ["Terminal", "Blocks"]

  static func appCommandWins(_ event: NSEvent) -> Bool {
    let overrides = SettingsStore.shared.settings.keybindingOverrides
    guard let binding = Keybindings.matchingKeybinding(for: event, overrides: overrides),
      !binding.keyEquivalent.isEmpty
    else { return false }
    return !editorKeeps.contains(binding.id) && !terminalCategories.contains(binding.category)
  }
}
