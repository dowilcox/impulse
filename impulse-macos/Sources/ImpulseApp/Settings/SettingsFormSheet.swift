import AppKit

// MARK: - Form Field Definition

struct FormField {
    enum FieldType {
        case text(placeholder: String, value: String)
        case popup(options: [String], selected: String)
    }
    let label: String
    let key: String
    let type: FieldType
}

// MARK: - Settings Form Sheet

/// A reusable sheet panel for editing multi-field settings items.
/// Replaces NSAlert+accessoryView dialogs with a proper form layout.
enum SettingsFormSheet {

    static func present(
        on parent: NSWindow,
        title: String,
        fields: [FormField],
        onSave: @escaping ([String: String]) -> Void
    ) {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 0),
            styleMask: [.titled],
            backing: .buffered,
            defer: true
        )
        panel.isReleasedWhenClosed = false

        // -- Title label --
        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = NSFont.boldSystemFont(ofSize: 15)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        // -- Grid of label/field pairs --
        let grid = NSGridView(numberOfColumns: 2, rows: 0)
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.rowSpacing = 10
        grid.columnSpacing = 12
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .fill

        var controls: [(String, NSView)] = []

        for field in fields {
            let label = NSTextField(labelWithString: field.label)
            label.font = NSFont.systemFont(ofSize: 13)
            label.alignment = .right
            label.setContentHuggingPriority(.defaultHigh, for: .horizontal)

            switch field.type {
            case .text(let placeholder, let value):
                let textField = NSTextField(string: value)
                textField.placeholderString = placeholder
                textField.translatesAutoresizingMaskIntoConstraints = false
                grid.addRow(with: [label, textField])
                controls.append((field.key, textField))

            case .popup(let options, let selected):
                let popup = NSPopUpButton()
                popup.addItems(withTitles: options)
                popup.selectItem(withTitle: selected)
                popup.translatesAutoresizingMaskIntoConstraints = false
                grid.addRow(with: [label, popup])
                controls.append((field.key, popup))
            }
        }

        // Handler must be created before buttons so we can wire targets
        let handler = SheetHandler(panel: panel, parent: parent, controls: controls, onSave: onSave)
        objc_setAssociatedObject(panel, "sheetHandler", handler, .OBJC_ASSOCIATION_RETAIN)

        // -- Buttons --
        let cancelButton = NSButton(title: "Cancel", target: handler, action: #selector(SheetHandler.cancelClicked(_:)))
        cancelButton.keyEquivalent = "\u{1b}" // Escape
        let saveButton = NSButton(title: "Save", target: handler, action: #selector(SheetHandler.saveClicked(_:)))
        saveButton.keyEquivalent = "\r" // Return — default button
        saveButton.bezelStyle = .rounded

        let buttonStack = NSStackView(views: [cancelButton, saveButton])
        buttonStack.orientation = .horizontal
        buttonStack.spacing = 8
        buttonStack.translatesAutoresizingMaskIntoConstraints = false

        // -- Content layout --
        let contentView = NSView()
        contentView.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(titleLabel)
        contentView.addSubview(grid)
        contentView.addSubview(buttonStack)

        NSLayoutConstraint.activate([
            titleLabel.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 20),
            titleLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            titleLabel.trailingAnchor.constraint(lessThanOrEqualTo: contentView.trailingAnchor, constant: -20),

            grid.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 16),
            grid.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 20),
            grid.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),

            buttonStack.topAnchor.constraint(equalTo: grid.bottomAnchor, constant: 20),
            buttonStack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -20),
            buttonStack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -16),
        ])

        panel.contentView = contentView

        // Chain nextKeyView so Tab moves between fields
        let textFields = controls.compactMap { $0.1 as? NSTextField }
        for i in 0 ..< textFields.count - 1 {
            textFields[i].nextKeyView = textFields[i + 1]
        }
        if let last = textFields.last {
            last.nextKeyView = saveButton
        }
        if let first = textFields.first {
            panel.initialFirstResponder = first
        }

        parent.beginSheet(panel)
    }
}

// MARK: - Sheet Handler

/// Mediates between the sheet's controls and the onSave callback.
private class SheetHandler: NSObject {
    let panel: NSPanel
    weak var parent: NSWindow?
    let controls: [(String, NSView)]
    let onSave: ([String: String]) -> Void

    init(panel: NSPanel, parent: NSWindow, controls: [(String, NSView)],
         onSave: @escaping ([String: String]) -> Void) {
        self.panel = panel
        self.parent = parent
        self.controls = controls
        self.onSave = onSave
    }

    @objc func saveClicked(_ sender: Any?) {
        var values: [String: String] = [:]
        for (key, control) in controls {
            if let textField = control as? NSTextField {
                values[key] = textField.stringValue
            } else if let popup = control as? NSPopUpButton {
                values[key] = popup.titleOfSelectedItem ?? ""
            }
        }
        parent?.endSheet(panel, returnCode: .OK)
        onSave(values)
    }

    @objc func cancelClicked(_ sender: Any?) {
        parent?.endSheet(panel, returnCode: .cancel)
    }
}
