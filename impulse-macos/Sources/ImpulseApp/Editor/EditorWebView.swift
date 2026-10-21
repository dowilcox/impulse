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
/// hints. It also keeps the editing keys everyone knows from VS Code,
/// whatever they're bound to: ⌘D adds the next occurrence, ⌥⌘↑/↓ add a
/// cursor, ⇧⌘↩ inserts a line above. A command the user unbinds or moves
/// gives its old keys back to Monaco.
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
  /// Monaco's editing keys that Impulse's commands also use (Split Right,
  /// Focus Pane Above/Below, Zoom Pane by default).
  private static let monacoKeys = ["Cmd+D", "Alt+Cmd+Up", "Alt+Cmd+Down", "Shift+Cmd+Return"].map(
    Keybindings.parseShortcut)

  static func appCommandWins(_ event: NSEvent) -> Bool {
    let overrides = SettingsStore.shared.settings.keybindingOverrides
    guard let binding = Keybindings.matchingKeybinding(for: event, overrides: overrides),
      !binding.keyEquivalent.isEmpty
    else { return false }
    if monacoKeys.contains(where: {
      Keybindings.eventMatchesShortcut(event, keyEquivalent: $0.keyEquivalent, modifierFlags: $0.modifierFlags)
    }) {
      return false
    }
    return !editorKeeps.contains(binding.id) && !terminalCategories.contains(binding.category)
  }

  // MARK: Dropped files

  // Files dropped on the editor (from the file tree or Finder) open in
  // editor tabs. The page never sees them: WebKit's own handling of a file
  // drop is to load the file in place of the editor.

  private static func droppedFiles(_ sender: NSDraggingInfo) -> [URL] {
    sender.draggingPasteboard.readObjects(
      forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
  }

  override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
    Self.droppedFiles(sender).isEmpty ? super.draggingEntered(sender) : .copy
  }

  override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
    Self.droppedFiles(sender).isEmpty ? super.draggingUpdated(sender) : .copy
  }

  override func draggingExited(_ sender: NSDraggingInfo?) {
    if let sender, !Self.droppedFiles(sender).isEmpty { return }
    super.draggingExited(sender)
  }

  override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool {
    Self.droppedFiles(sender).isEmpty ? super.prepareForDragOperation(sender) : true
  }

  override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
    let files = Self.droppedFiles(sender)
    guard !files.isEmpty else { return super.performDragOperation(sender) }
    guard let controller = window?.windowController as? MainWindowController else { return false }
    for file in files { controller.openFile(path: file.path) }
    return true
  }

  override func concludeDragOperation(_ sender: NSDraggingInfo?) {
    if let sender, !Self.droppedFiles(sender).isEmpty { return }
    super.concludeDragOperation(sender)
  }
}
