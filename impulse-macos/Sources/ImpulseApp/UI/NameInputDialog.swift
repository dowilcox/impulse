import AppKit

/// Modal text-input dialog used for new file / new folder / rename actions.
enum NameInputDialog {

    /// Show a modal alert with a text field for entering a name. Calls
    /// `completion` with the trimmed text only when the user confirms.
    static func show(
        title: String,
        message: String,
        placeholder: String,
        defaultValue: String,
        completion: @escaping (String) -> Void
    ) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")

        let textField = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        textField.placeholderString = placeholder
        textField.stringValue = defaultValue
        alert.accessoryView = textField

        // Make the text field the first responder so it receives focus.
        alert.window.initialFirstResponder = textField

        // If there is an extension, select just the stem so the user can
        // easily type a new name without losing the extension.
        if !defaultValue.isEmpty, let dotRange = defaultValue.range(of: ".", options: .backwards) {
            let stemLength = defaultValue.distance(
                from: defaultValue.startIndex, to: dotRange.lowerBound)
            textField.currentEditor()?.selectedRange = NSRange(location: 0, length: stemLength)
        }

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            completion(textField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }
}
