import AppKit

// MARK: - Toolbar Item Identifiers

private extension NSToolbarItem.Identifier {
    static let editor = NSToolbarItem.Identifier("editor")
    static let terminal = NSToolbarItem.Identifier("terminal")
    static let appearance = NSToolbarItem.Identifier("appearance")
    static let automation = NSToolbarItem.Identifier("automation")
    static let keybindings = NSToolbarItem.Identifier("keybindings")
}

// MARK: - Pane Metadata

private struct PaneInfo {
    let id: NSToolbarItem.Identifier
    let label: String
    let icon: String // SF Symbol name
}

private let allPanes: [PaneInfo] = [
    PaneInfo(id: .editor, label: "Editor", icon: "doc.plaintext"),
    PaneInfo(id: .terminal, label: "Terminal", icon: "terminal"),
    PaneInfo(id: .appearance, label: "Appearance", icon: "paintpalette"),
    PaneInfo(id: .automation, label: "Automation", icon: "gearshape.2"),
    PaneInfo(id: .keybindings, label: "Keybindings", icon: "keyboard"),
]

// MARK: - Settings Window Controller

/// NSToolbar-based preferences window with panes for Editor, Terminal,
/// Appearance, Automation, and Keybindings. Changes save immediately.
final class SettingsWindowController: NSWindowController {

    private var settings: Settings
    private var paneCache: [String: NSView] = [:]
    private var currentPaneId: String = "editor"
    private var saveTimer: Timer?

    /// The singleton preferences window. Only one is shown at a time.
    private static var shared: SettingsWindowController?

    static func show(settings: Settings) {
        if let existing = shared {
            existing.settings = settings
            existing.window?.makeKeyAndOrderFront(nil)
            return
        }
        let controller = SettingsWindowController(settings: settings)
        shared = controller
        controller.showWindow(nil)
    }

    init(settings: Settings) {
        self.settings = settings

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 680, height: 560),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Settings"
        window.center()
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 560, height: 400)

        super.init(window: window)

        // Toolbar setup
        let toolbar = NSToolbar(identifier: "SettingsToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconAndLabel
        window.toolbarStyle = .preference
        window.toolbar = toolbar

        // Restore last selected pane
        let lastPane = UserDefaults.standard.string(forKey: "settingsSelectedPane") ?? "editor"
        switchToPane(lastPane, animated: false)
        toolbar.selectedItemIdentifier = NSToolbarItem.Identifier(lastPane)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func close() {
        // Flush any pending debounced save before closing.
        if saveTimer?.isValid == true {
            saveTimer?.invalidate()
            saveTimer = nil
            settings.save()
        }
        super.close()
        SettingsWindowController.shared = nil
    }

    // MARK: - Pane Switching

    private func switchToPane(_ identifier: String, animated: Bool = true) {
        currentPaneId = identifier
        UserDefaults.standard.set(identifier, forKey: "settingsSelectedPane")

        let paneView: NSView
        if let cached = paneCache[identifier] {
            paneView = cached
        } else {
            switch identifier {
            case "editor":      paneView = makeEditorPane()
            case "terminal":    paneView = makeTerminalPane()
            case "appearance":  paneView = makeAppearancePane()
            case "automation":  paneView = makeAutomationPane()
            case "keybindings": paneView = makeKeybindingsPane()
            default:            paneView = makeEditorPane()
            }
            paneCache[identifier] = paneView
        }

        guard let window else { return }

        window.contentView = paneView
    }

    @objc private func toolbarPaneSelected(_ sender: NSToolbarItem) {
        switchToPane(sender.itemIdentifier.rawValue)
    }

    // MARK: - Editor Pane

    private func makeEditorPane() -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10

        // -- Font Section --

        let fontFamilyField = NSTextField(string: settings.fontFamily)
        fontFamilyField.target = self
        fontFamilyField.action = #selector(fontFamilyChanged(_:))
        fontFamilyField.tag = 0

        let fontSizeStepper = NSStepper()
        fontSizeStepper.minValue = 6
        fontSizeStepper.maxValue = 72
        fontSizeStepper.integerValue = settings.fontSize
        fontSizeStepper.target = self
        fontSizeStepper.action = #selector(fontSizeStepperChanged(_:))
        let fontSizeField = NSTextField(string: "\(settings.fontSize)")
        fontSizeField.isEditable = false
        fontSizeField.tag = 100
        let fontSizeRow = NSStackView(views: [fontSizeField, fontSizeStepper])
        fontSizeRow.orientation = .horizontal
        fontSizeRow.spacing = 4

        let ligaturesCheck = NSButton(checkboxWithTitle: "Font ligatures",
                                       target: self, action: #selector(fontLigaturesChanged(_:)))
        ligaturesCheck.state = settings.fontLigatures ? .on : .off

        addSection(to: stack, title: "Font", subtitle: "Editor typeface and size", rows: [
            makeRow(label: "Font Family:", control: fontFamilyField),
            makeRow(label: "Font Size:", control: fontSizeRow),
            ligaturesCheck,
        ], addSeparator: false)

        // -- Indentation Section --

        let tabWidthField = NSTextField(string: "\(settings.tabWidth)")
        tabWidthField.target = self
        tabWidthField.action = #selector(tabWidthChanged(_:))
        tabWidthField.tag = 1

        let useSpacesCheck = NSButton(checkboxWithTitle: "Insert spaces instead of tabs",
                                       target: self, action: #selector(useSpacesChanged(_:)))
        useSpacesCheck.state = settings.useSpaces ? .on : .off

        let indentGuidesCheck = NSButton(checkboxWithTitle: "Indent guides",
                                          target: self, action: #selector(indentGuidesChanged(_:)))
        indentGuidesCheck.state = settings.indentGuides ? .on : .off

        addSection(to: stack, title: "Indentation", rows: [
            makeRow(label: "Tab Width:", control: tabWidthField),
            useSpacesCheck,
            indentGuidesCheck,
        ])

        // -- Display Section --

        let lineNumbersCheck = NSButton(checkboxWithTitle: "Show line numbers",
                                         target: self, action: #selector(showLineNumbersChanged(_:)))
        lineNumbersCheck.state = settings.showLineNumbers ? .on : .off

        let wordWrapCheck = NSButton(checkboxWithTitle: "Word wrap",
                                      target: self, action: #selector(wordWrapChanged(_:)))
        wordWrapCheck.state = settings.wordWrap ? .on : .off

        let minimapCheck = NSButton(checkboxWithTitle: "Show minimap",
                                     target: self, action: #selector(minimapChanged(_:)))
        minimapCheck.state = settings.minimapEnabled ? .on : .off

        let highlightLineCheck = NSButton(checkboxWithTitle: "Highlight current line",
                                           target: self, action: #selector(highlightLineChanged(_:)))
        highlightLineCheck.state = settings.highlightCurrentLine ? .on : .off

        let bracketCheck = NSButton(checkboxWithTitle: "Bracket pair colorization",
                                     target: self, action: #selector(bracketColorizationChanged(_:)))
        bracketCheck.state = settings.bracketPairColorization ? .on : .off

        let stickyScrollCheck = NSButton(checkboxWithTitle: "Sticky scroll",
                                          target: self, action: #selector(stickyScrollChanged(_:)))
        stickyScrollCheck.state = settings.stickyScroll ? .on : .off

        let whitespacePopup = NSPopUpButton(title: "", target: self, action: #selector(renderWhitespaceChanged(_:)))
        whitespacePopup.addItems(withTitles: ["none", "boundary", "selection", "trailing", "all"])
        whitespacePopup.selectItem(withTitle: settings.renderWhitespace)

        let rightMarginCheck = NSButton(checkboxWithTitle: "Show right margin",
                                         target: self, action: #selector(showRightMarginChanged(_:)))
        rightMarginCheck.state = settings.showRightMargin ? .on : .off

        let marginStepper = NSStepper()
        marginStepper.minValue = 1
        marginStepper.maxValue = 500
        marginStepper.integerValue = settings.rightMarginPosition
        marginStepper.target = self
        marginStepper.action = #selector(rightMarginPositionStepperChanged(_:))
        let marginField = NSTextField(string: "\(settings.rightMarginPosition)")
        marginField.isEditable = false
        marginField.tag = 101
        let marginRow = NSStackView(views: [marginField, marginStepper])
        marginRow.orientation = .horizontal
        marginRow.spacing = 4

        let lineHeightStepper = NSStepper()
        lineHeightStepper.minValue = 0
        lineHeightStepper.maxValue = 50
        lineHeightStepper.integerValue = settings.editorLineHeight
        lineHeightStepper.target = self
        lineHeightStepper.action = #selector(lineHeightStepperChanged(_:))
        let lineHeightField = NSTextField(string: settings.editorLineHeight > 0 ? "\(settings.editorLineHeight)" : "Default")
        lineHeightField.isEditable = false
        lineHeightField.tag = 102
        let lineHeightRow = NSStackView(views: [lineHeightField, lineHeightStepper])
        lineHeightRow.orientation = .horizontal
        lineHeightRow.spacing = 4

        addSection(to: stack, title: "Display", subtitle: "Visual appearance of the editor", rows: [
            lineNumbersCheck,
            wordWrapCheck,
            minimapCheck,
            highlightLineCheck,
            bracketCheck,
            stickyScrollCheck,
            makeRow(label: "Render Whitespace:", control: whitespacePopup),
            rightMarginCheck,
            makeRow(label: "Right Margin Column:", control: marginRow),
            makeRow(label: "Line Height:", control: lineHeightRow),
        ])

        // -- Cursor Section --

        let cursorPopup = NSPopUpButton(title: "", target: self, action: #selector(cursorStyleChanged(_:)))
        cursorPopup.addItems(withTitles: ["line", "block", "underline", "line-thin", "block-outline", "underline-thin"])
        cursorPopup.selectItem(withTitle: settings.editorCursorStyle)

        let blinkPopup = NSPopUpButton(title: "", target: self, action: #selector(cursorBlinkingChanged(_:)))
        blinkPopup.addItems(withTitles: ["blink", "smooth", "phase", "expand", "solid"])
        blinkPopup.selectItem(withTitle: settings.editorCursorBlinking)

        addSection(to: stack, title: "Cursor", rows: [
            makeRow(label: "Cursor Style:", control: cursorPopup),
            makeRow(label: "Cursor Blinking:", control: blinkPopup),
        ])

        // -- Behavior Section --

        let autoClosePopup = NSPopUpButton(title: "", target: self, action: #selector(autoClosingBracketsChanged(_:)))
        autoClosePopup.addItems(withTitles: ["always", "languageDefined", "beforeWhitespace", "never"])
        autoClosePopup.selectItem(withTitle: settings.editorAutoClosingBrackets)

        let foldingCheck = NSButton(checkboxWithTitle: "Code folding",
                                     target: self, action: #selector(foldingChanged(_:)))
        foldingCheck.state = settings.folding ? .on : .off

        let autoSaveCheck = NSButton(checkboxWithTitle: "Auto save on focus loss",
                                      target: self, action: #selector(autoSaveChanged(_:)))
        autoSaveCheck.state = settings.autoSave ? .on : .off

        let scrollBeyondCheck = NSButton(checkboxWithTitle: "Scroll beyond last line",
                                          target: self, action: #selector(scrollBeyondLastLineChanged(_:)))
        scrollBeyondCheck.state = settings.scrollBeyondLastLine ? .on : .off

        let smoothScrollCheck = NSButton(checkboxWithTitle: "Smooth scrolling",
                                          target: self, action: #selector(smoothScrollingChanged(_:)))
        smoothScrollCheck.state = settings.smoothScrolling ? .on : .off

        addSection(to: stack, title: "Behavior", rows: [
            makeRow(label: "Auto-Close Brackets:", control: autoClosePopup),
            foldingCheck,
            autoSaveCheck,
            scrollBeyondCheck,
            smoothScrollCheck,
        ])

        return wrapInScrollView(stack)
    }

    // MARK: - Terminal Pane

    private func makeTerminalPane() -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10

        // -- Font Section --

        let fontFamilyField = NSTextField(string: settings.terminalFontFamily)
        fontFamilyField.target = self
        fontFamilyField.action = #selector(termFontFamilyChanged(_:))

        let fontSizeStepper = NSStepper()
        fontSizeStepper.minValue = 6
        fontSizeStepper.maxValue = 72
        fontSizeStepper.integerValue = settings.terminalFontSize
        fontSizeStepper.target = self
        fontSizeStepper.action = #selector(termFontSizeStepperChanged(_:))
        let fontSizeLabel = NSTextField(string: "\(settings.terminalFontSize)")
        fontSizeLabel.isEditable = false
        fontSizeLabel.tag = 200
        let fontSizeRow = NSStackView(views: [fontSizeLabel, fontSizeStepper])
        fontSizeRow.orientation = .horizontal
        fontSizeRow.spacing = 4

        addSection(to: stack, title: "Font", subtitle: "Terminal typeface and size", rows: [
            makeRow(label: "Font Family:", control: fontFamilyField),
            makeRow(label: "Font Size:", control: fontSizeRow),
        ], addSeparator: false)

        // -- Cursor Section --

        let cursorPopup = NSPopUpButton(title: "", target: self, action: #selector(termCursorShapeChanged(_:)))
        cursorPopup.addItems(withTitles: ["block", "underline", "bar"])
        cursorPopup.selectItem(withTitle: settings.terminalCursorShape)

        let cursorBlinkCheck = NSButton(checkboxWithTitle: "Cursor blink",
                                         target: self, action: #selector(termCursorBlinkChanged(_:)))
        cursorBlinkCheck.state = settings.terminalCursorBlink ? .on : .off

        addSection(to: stack, title: "Cursor", rows: [
            makeRow(label: "Cursor Shape:", control: cursorPopup),
            cursorBlinkCheck,
        ])

        // -- Behavior Section --

        let copyOnSelectCheck = NSButton(checkboxWithTitle: "Copy on select",
                                          target: self, action: #selector(termCopyOnSelectChanged(_:)))
        copyOnSelectCheck.state = settings.terminalCopyOnSelect ? .on : .off

        let bellCheck = NSButton(checkboxWithTitle: "Audible bell",
                                  target: self, action: #selector(termBellChanged(_:)))
        bellCheck.state = settings.terminalBell ? .on : .off

        let scrollOutputCheck = NSButton(checkboxWithTitle: "Scroll on output",
                                          target: self, action: #selector(termScrollOnOutputChanged(_:)))
        scrollOutputCheck.state = settings.terminalScrollOnOutput ? .on : .off

        let hyperlinkCheck = NSButton(checkboxWithTitle: "Allow hyperlinks",
                                       target: self, action: #selector(termHyperlinkChanged(_:)))
        hyperlinkCheck.state = settings.terminalAllowHyperlink ? .on : .off

        let boldBrightCheck = NSButton(checkboxWithTitle: "Bold is bright",
                                        target: self, action: #selector(termBoldIsBrightChanged(_:)))
        boldBrightCheck.state = settings.terminalBoldIsBright ? .on : .off

        addSection(to: stack, title: "Behavior", rows: [
            copyOnSelectCheck,
            bellCheck,
            scrollOutputCheck,
            hyperlinkCheck,
            boldBrightCheck,
        ])

        // -- Scrollback Section --

        let scrollbackField = NSTextField(string: "\(settings.terminalScrollback)")
        scrollbackField.target = self
        scrollbackField.action = #selector(scrollbackChanged(_:))

        addSection(to: stack, title: "Scrollback", subtitle: "Number of lines kept in the scroll buffer", rows: [
            makeRow(label: "Scrollback Lines:", control: scrollbackField),
        ])

        return wrapInScrollView(stack)
    }

    // MARK: - Appearance Pane

    private func makeAppearancePane() -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10

        // -- Color Scheme Section --

        let schemePopup = NSPopUpButton(title: "", target: self, action: #selector(colorSchemeChanged(_:)))
        schemePopup.addItems(withTitles: ThemeManager.availableThemes())
        schemePopup.selectItem(withTitle: settings.colorScheme)

        // Color preview
        let previewBox = NSBox()
        previewBox.boxType = .custom
        previewBox.cornerRadius = 8
        previewBox.borderWidth = 1
        previewBox.borderColor = .separatorColor
        previewBox.translatesAutoresizingMaskIntoConstraints = false
        previewBox.identifier = NSUserInterfaceItemIdentifier("colorPreviewBox")
        // Also tag for full-width stretching in addSection
        previewBox.setContentHuggingPriority(.defaultLow, for: .horizontal)
        previewBox.heightAnchor.constraint(equalToConstant: 200).isActive = true

        let theme = ThemeManager.theme(forName: settings.colorScheme)
        previewBox.fillColor = theme.bg
        previewBox.contentView = buildColorPreview(theme: theme)

        addSection(to: stack, title: "Color Scheme", subtitle: "Theme applied to editor, terminal, and UI", rows: [
            makeRow(label: "Color Scheme:", control: schemePopup),
            previewBox,
        ], addSeparator: false)

        // -- Sidebar Section --

        let hiddenCheck = NSButton(checkboxWithTitle: "Show hidden files in sidebar",
                                    target: self, action: #selector(sidebarShowHiddenChanged(_:)))
        hiddenCheck.state = settings.sidebarShowHidden ? .on : .off

        addSection(to: stack, title: "Sidebar", rows: [
            hiddenCheck,
        ])

        return wrapInScrollView(stack)
    }

    private func buildColorPreview(theme: Theme) -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.spacing = 6
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)

        let colors: [(String, NSColor)] = [
            ("Background", theme.bg),
            ("Background Dark", theme.bgDark),
            ("Foreground", theme.fg),
            ("Cyan", theme.cyan),
            ("Blue", theme.blue),
            ("Green", theme.green),
            ("Magenta", theme.magenta),
            ("Red", theme.red),
            ("Yellow", theme.yellow),
            ("Orange", theme.orange),
            ("Comment", theme.comment),
        ]

        // Show color swatches in rows
        let row1 = NSStackView()
        row1.orientation = .horizontal
        row1.spacing = 8
        let row2 = NSStackView()
        row2.orientation = .horizontal
        row2.spacing = 8

        for (index, (name, color)) in colors.enumerated() {
            let swatch = NSView()
            swatch.wantsLayer = true
            swatch.layer?.backgroundColor = color.cgColor
            swatch.layer?.cornerRadius = 4
            swatch.translatesAutoresizingMaskIntoConstraints = false
            swatch.widthAnchor.constraint(equalToConstant: 32).isActive = true
            swatch.heightAnchor.constraint(equalToConstant: 32).isActive = true
            swatch.toolTip = name

            if index < 6 {
                row1.addArrangedSubview(swatch)
            } else {
                row2.addArrangedSubview(swatch)
            }
        }

        stack.addArrangedSubview(row1)
        stack.addArrangedSubview(row2)

        // Terminal palette row
        let paletteLabel = NSTextField(labelWithString: "Terminal Palette:")
        paletteLabel.textColor = theme.fg
        paletteLabel.font = NSFont.systemFont(ofSize: 11)
        stack.addArrangedSubview(paletteLabel)

        let paletteRow = NSStackView()
        paletteRow.orientation = .horizontal
        paletteRow.spacing = 2
        for color in theme.terminalPalette {
            let swatch = NSView()
            swatch.wantsLayer = true
            swatch.layer?.backgroundColor = color.cgColor
            swatch.layer?.cornerRadius = 2
            swatch.translatesAutoresizingMaskIntoConstraints = false
            swatch.widthAnchor.constraint(equalToConstant: 20).isActive = true
            swatch.heightAnchor.constraint(equalToConstant: 20).isActive = true
            paletteRow.addArrangedSubview(swatch)
        }
        stack.addArrangedSubview(paletteRow)

        return stack
    }

    // MARK: - Automation Pane

    private func makeAutomationPane() -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10

        // -- Commands on Save Section --

        let cmdScrollView = NSScrollView()
        cmdScrollView.translatesAutoresizingMaskIntoConstraints = false
        cmdScrollView.hasVerticalScroller = true
        cmdScrollView.borderType = .bezelBorder

        let tableView = NSTableView()
        tableView.tag = 400
        tableView.headerView = NSTableHeaderView()
        tableView.usesAlternatingRowBackgroundColors = true

        let nameCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("name"))
        nameCol.title = "Name"
        nameCol.width = 120
        tableView.addTableColumn(nameCol)

        let commandCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("command"))
        commandCol.title = "Command"
        commandCol.width = 150
        tableView.addTableColumn(commandCol)

        let patternCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("pattern"))
        patternCol.title = "File Pattern"
        patternCol.width = 100
        tableView.addTableColumn(patternCol)

        let reloadCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("reload"))
        reloadCol.title = "Reload"
        reloadCol.width = 50
        tableView.addTableColumn(reloadCol)

        tableView.delegate = self
        tableView.dataSource = self
        tableView.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        cmdScrollView.documentView = tableView

        cmdScrollView.heightAnchor.constraint(equalToConstant: 120).isActive = true

        let addCmdButton = NSButton(title: "Add", target: self, action: #selector(addCommandOnSave(_:)))
        let removeCmdButton = NSButton(title: "Remove", target: self, action: #selector(removeCommandOnSave(_:)))
        removeCmdButton.tag = 401
        let cmdButtonRow = NSStackView(views: [addCmdButton, removeCmdButton])
        cmdButtonRow.orientation = .horizontal
        cmdButtonRow.spacing = 8

        addSection(to: stack, title: "Commands on Save",
                   subtitle: "Commands that run automatically when a file matching the pattern is saved.",
                   rows: [cmdScrollView, cmdButtonRow], addSeparator: false)

        // -- File Type Overrides Section --

        let ftoScrollView = NSScrollView()
        ftoScrollView.translatesAutoresizingMaskIntoConstraints = false
        ftoScrollView.hasVerticalScroller = true
        ftoScrollView.borderType = .bezelBorder

        let ftoTable = NSTableView()
        ftoTable.tag = 600
        ftoTable.headerView = NSTableHeaderView()
        ftoTable.usesAlternatingRowBackgroundColors = true
        ftoTable.doubleAction = #selector(fileTypeOverrideDoubleClicked(_:))
        ftoTable.target = self

        let ftoPatternCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("fto_pattern"))
        ftoPatternCol.title = "Pattern"
        ftoPatternCol.width = 100
        ftoTable.addTableColumn(ftoPatternCol)

        let ftoTabCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("fto_tab_width"))
        ftoTabCol.title = "Tab Width"
        ftoTabCol.width = 70
        ftoTable.addTableColumn(ftoTabCol)

        let ftoSpacesCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("fto_use_spaces"))
        ftoSpacesCol.title = "Spaces"
        ftoSpacesCol.width = 60
        ftoTable.addTableColumn(ftoSpacesCol)

        let ftoFmtCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("fto_formatter"))
        ftoFmtCol.title = "Formatter"
        ftoFmtCol.width = 150
        ftoTable.addTableColumn(ftoFmtCol)

        ftoTable.delegate = self
        ftoTable.dataSource = self
        ftoTable.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        ftoScrollView.documentView = ftoTable

        ftoScrollView.heightAnchor.constraint(equalToConstant: 120).isActive = true

        let addFtoButton = NSButton(title: "Add", target: self, action: #selector(addFileTypeOverride(_:)))
        let removeFtoButton = NSButton(title: "Remove", target: self, action: #selector(removeFileTypeOverride(_:)))
        let ftoButtonRow = NSStackView(views: [addFtoButton, removeFtoButton])
        ftoButtonRow.orientation = .horizontal
        ftoButtonRow.spacing = 8

        addSection(to: stack, title: "File Type Overrides",
                   subtitle: "Per-file-type overrides for tab width, spaces vs tabs, and format-on-save.",
                   rows: [ftoScrollView, ftoButtonRow])

        return wrapInScrollView(stack)
    }

    // MARK: - Keybindings Pane

    private func makeKeybindingsPane() -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10

        // -- Built-in Keybindings --

        let scrollView = NSScrollView()
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder

        let tableView = NSTableView()
        tableView.tag = 500
        tableView.headerView = NSTableHeaderView()
        tableView.usesAlternatingRowBackgroundColors = true
        tableView.doubleAction = #selector(keybindingDoubleClicked(_:))
        tableView.target = self

        let descCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("kb_description"))
        descCol.title = "Action"
        descCol.width = 180
        tableView.addTableColumn(descCol)

        let categoryCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("kb_category"))
        categoryCol.title = "Category"
        categoryCol.width = 90
        tableView.addTableColumn(categoryCol)

        let shortcutCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("kb_shortcut"))
        shortcutCol.title = "Shortcut"
        shortcutCol.width = 150
        tableView.addTableColumn(shortcutCol)

        tableView.delegate = self
        tableView.dataSource = self
        tableView.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        scrollView.documentView = tableView

        scrollView.heightAnchor.constraint(equalToConstant: 200).isActive = true

        let resetButton = NSButton(title: "Reset All to Defaults", target: self,
                                    action: #selector(resetKeybindings(_:)))

        addSection(to: stack, title: "Built-in Keybindings",
                   subtitle: "Double-click a shortcut to edit it. Overrides are saved to settings.",
                   rows: [scrollView, resetButton], addSeparator: false)

        // -- Custom Keybindings --

        let customScrollView = NSScrollView()
        customScrollView.translatesAutoresizingMaskIntoConstraints = false
        customScrollView.hasVerticalScroller = true
        customScrollView.borderType = .bezelBorder

        let customTable = NSTableView()
        customTable.tag = 501
        customTable.headerView = NSTableHeaderView()
        customTable.usesAlternatingRowBackgroundColors = true
        customTable.doubleAction = #selector(customKeybindingDoubleClicked(_:))
        customTable.target = self

        let ckNameCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("ck_name"))
        ckNameCol.title = "Name"
        ckNameCol.width = 120
        customTable.addTableColumn(ckNameCol)

        let ckKeyCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("ck_key"))
        ckKeyCol.title = "Key"
        ckKeyCol.width = 120
        customTable.addTableColumn(ckKeyCol)

        let ckCommandCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("ck_command"))
        ckCommandCol.title = "Command"
        ckCommandCol.width = 120
        customTable.addTableColumn(ckCommandCol)

        let ckArgsCol = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("ck_args"))
        ckArgsCol.title = "Args"
        ckArgsCol.width = 120
        customTable.addTableColumn(ckArgsCol)

        customTable.delegate = self
        customTable.dataSource = self
        customTable.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        customScrollView.documentView = customTable

        customScrollView.heightAnchor.constraint(equalToConstant: 140).isActive = true

        let addCustomButton = NSButton(title: "Add", target: self, action: #selector(addCustomKeybinding(_:)))
        let removeCustomButton = NSButton(title: "Remove", target: self, action: #selector(removeCustomKeybinding(_:)))
        let customButtonRow = NSStackView(views: [addCustomButton, removeCustomButton])
        customButtonRow.orientation = .horizontal
        customButtonRow.spacing = 8

        addSection(to: stack, title: "Custom Keybindings",
                   subtitle: "User-defined shortcuts that open a new terminal and run a command. Double-click to edit.",
                   rows: [customScrollView, customButtonRow])

        return wrapInScrollView(stack)
    }

    // MARK: - Layout Helpers

    private func makeLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13)
        label.alignment = .right
        return label
    }

    private func makeSectionHeader(title: String, subtitle: String? = nil) -> NSView {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 2
        stack.translatesAutoresizingMaskIntoConstraints = false

        let titleLabel = NSTextField(labelWithString: title)
        titleLabel.font = NSFont.boldSystemFont(ofSize: 13)
        stack.addArrangedSubview(titleLabel)

        if let subtitle = subtitle {
            let subtitleLabel = NSTextField(labelWithString: subtitle)
            subtitleLabel.font = NSFont.systemFont(ofSize: 11)
            subtitleLabel.textColor = .secondaryLabelColor
            stack.addArrangedSubview(subtitleLabel)
        }

        return stack
    }

    private func makeSeparator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    private static let labelFieldRowId = NSUserInterfaceItemIdentifier("settingsLabelFieldRow")

    private func makeRow(label: String, control: NSView) -> NSStackView {
        let labelView = NSTextField(labelWithString: label)
        labelView.font = NSFont.systemFont(ofSize: 13)
        labelView.alignment = .right
        labelView.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        labelView.translatesAutoresizingMaskIntoConstraints = false
        labelView.widthAnchor.constraint(greaterThanOrEqualToConstant: 130).isActive = true

        // Allow the control to stretch horizontally
        control.setContentHuggingPriority(.defaultLow, for: .horizontal)
        control.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let row = NSStackView(views: [labelView, control])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 12
        row.translatesAutoresizingMaskIntoConstraints = false
        row.identifier = Self.labelFieldRowId
        return row
    }

    private func addSection(to stack: NSStackView, title: String, subtitle: String? = nil,
                            rows: [NSView], addSeparator: Bool = true) {
        if addSeparator && !stack.arrangedSubviews.isEmpty {
            stack.addArrangedSubview(makeSeparator())
        }
        stack.addArrangedSubview(makeSectionHeader(title: title, subtitle: subtitle))
        for row in rows {
            stack.addArrangedSubview(row)
            // Stretch label+field rows, tables, and boxes to fill the available width
            let shouldStretch = row is NSScrollView
                || row.identifier == Self.labelFieldRowId
                || (row is NSBox && (row as! NSBox).boxType == .custom)
            if shouldStretch {
                row.trailingAnchor.constraint(equalTo: stack.trailingAnchor).isActive = true
            }
        }
    }

    private final class FlippedView: NSView {
        override var isFlipped: Bool { true }
    }

    private func wrapInScrollView(_ stack: NSStackView) -> NSView {
        stack.translatesAutoresizingMaskIntoConstraints = false

        let docView = FlippedView()
        docView.translatesAutoresizingMaskIntoConstraints = false
        docView.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: docView.topAnchor),
            stack.leadingAnchor.constraint(equalTo: docView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: docView.trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: docView.bottomAnchor),
        ])

        let scrollView = NSScrollView()
        scrollView.documentView = docView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false

        // Pin document view width to the scroll view's clip view so content fills horizontally
        if let clipView = scrollView.contentView as? NSClipView {
            docView.widthAnchor.constraint(equalTo: clipView.widthAnchor).isActive = true
        }

        let container = NSView()
        container.addSubview(scrollView)
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor, constant: 20),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -20),
        ])

        return container
    }

    private func persistSettings() {
        // Propagate in-memory changes immediately so the UI stays responsive.
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings = settings
        }
        NotificationCenter.default.post(name: .impulseSettingsDidChange, object: settings)

        // Debounce the disk write: coalesce rapid changes (e.g. stepper clicks)
        // into a single save 0.3 s after the last change.
        saveTimer?.invalidate()
        saveTimer = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: false) { [weak self] _ in
            self?.settings.save()
        }
    }

    // MARK: - Editor Actions

    @objc private func fontFamilyChanged(_ sender: NSTextField) {
        settings.fontFamily = sender.stringValue
        persistSettings()
    }

    @objc private func fontSizeStepperChanged(_ sender: NSStepper) {
        settings.fontSize = sender.integerValue
        if let label = sender.superview?.subviews.compactMap({ $0 as? NSTextField }).first(where: { $0.tag == 100 }) {
            label.stringValue = "\(settings.fontSize)"
        }
        persistSettings()
    }

    @objc private func tabWidthChanged(_ sender: NSTextField) {
        settings.tabWidth = max(1, sender.integerValue)
        persistSettings()
    }

    @objc private func useSpacesChanged(_ sender: NSButton) {
        settings.useSpaces = sender.state == .on
        persistSettings()
    }

    @objc private func showLineNumbersChanged(_ sender: NSButton) {
        settings.showLineNumbers = sender.state == .on
        persistSettings()
    }

    @objc private func wordWrapChanged(_ sender: NSButton) {
        settings.wordWrap = sender.state == .on
        persistSettings()
    }

    @objc private func minimapChanged(_ sender: NSButton) {
        settings.minimapEnabled = sender.state == .on
        persistSettings()
    }

    @objc private func highlightLineChanged(_ sender: NSButton) {
        settings.highlightCurrentLine = sender.state == .on
        persistSettings()
    }

    @objc private func bracketColorizationChanged(_ sender: NSButton) {
        settings.bracketPairColorization = sender.state == .on
        persistSettings()
    }

    @objc private func fontLigaturesChanged(_ sender: NSButton) {
        settings.fontLigatures = sender.state == .on
        persistSettings()
    }

    @objc private func indentGuidesChanged(_ sender: NSButton) {
        settings.indentGuides = sender.state == .on
        persistSettings()
    }

    @objc private func stickyScrollChanged(_ sender: NSButton) {
        settings.stickyScroll = sender.state == .on
        persistSettings()
    }

    @objc private func autoSaveChanged(_ sender: NSButton) {
        settings.autoSave = sender.state == .on
        persistSettings()
    }

    @objc private func renderWhitespaceChanged(_ sender: NSPopUpButton) {
        settings.renderWhitespace = sender.titleOfSelectedItem ?? "selection"
        persistSettings()
    }

    @objc private func cursorStyleChanged(_ sender: NSPopUpButton) {
        settings.editorCursorStyle = sender.titleOfSelectedItem ?? "line"
        persistSettings()
    }

    @objc private func cursorBlinkingChanged(_ sender: NSPopUpButton) {
        settings.editorCursorBlinking = sender.titleOfSelectedItem ?? "smooth"
        persistSettings()
    }

    @objc private func autoClosingBracketsChanged(_ sender: NSPopUpButton) {
        settings.editorAutoClosingBrackets = sender.titleOfSelectedItem ?? "languageDefined"
        persistSettings()
    }

    @objc private func showRightMarginChanged(_ sender: NSButton) {
        settings.showRightMargin = sender.state == .on
        persistSettings()
    }

    @objc private func rightMarginPositionStepperChanged(_ sender: NSStepper) {
        settings.rightMarginPosition = sender.integerValue
        if let label = sender.superview?.subviews.compactMap({ $0 as? NSTextField }).first(where: { $0.tag == 101 }) {
            label.stringValue = "\(settings.rightMarginPosition)"
        }
        persistSettings()
    }

    @objc private func scrollBeyondLastLineChanged(_ sender: NSButton) {
        settings.scrollBeyondLastLine = sender.state == .on
        persistSettings()
    }

    @objc private func smoothScrollingChanged(_ sender: NSButton) {
        settings.smoothScrolling = sender.state == .on
        persistSettings()
    }

    @objc private func foldingChanged(_ sender: NSButton) {
        settings.folding = sender.state == .on
        persistSettings()
    }

    @objc private func lineHeightStepperChanged(_ sender: NSStepper) {
        settings.editorLineHeight = sender.integerValue
        if let label = sender.superview?.subviews.compactMap({ $0 as? NSTextField }).first(where: { $0.tag == 102 }) {
            label.stringValue = settings.editorLineHeight > 0 ? "\(settings.editorLineHeight)" : "Default"
        }
        persistSettings()
    }

    // MARK: - Terminal Actions

    @objc private func termFontFamilyChanged(_ sender: NSTextField) {
        settings.terminalFontFamily = sender.stringValue
        persistSettings()
    }

    @objc private func termFontSizeStepperChanged(_ sender: NSStepper) {
        settings.terminalFontSize = sender.integerValue
        if let label = sender.superview?.subviews.compactMap({ $0 as? NSTextField }).first(where: { $0.tag == 200 }) {
            label.stringValue = "\(settings.terminalFontSize)"
        }
        persistSettings()
    }

    @objc private func scrollbackChanged(_ sender: NSTextField) {
        settings.terminalScrollback = max(100, sender.integerValue)
        persistSettings()
    }

    @objc private func termCursorShapeChanged(_ sender: NSPopUpButton) {
        settings.terminalCursorShape = sender.titleOfSelectedItem ?? "block"
        persistSettings()
    }

    @objc private func termCursorBlinkChanged(_ sender: NSButton) {
        settings.terminalCursorBlink = sender.state == .on
        persistSettings()
    }

    @objc private func termCopyOnSelectChanged(_ sender: NSButton) {
        settings.terminalCopyOnSelect = sender.state == .on
        persistSettings()
    }

    @objc private func termBellChanged(_ sender: NSButton) {
        settings.terminalBell = sender.state == .on
        persistSettings()
    }

    @objc private func termScrollOnOutputChanged(_ sender: NSButton) {
        settings.terminalScrollOnOutput = sender.state == .on
        persistSettings()
    }

    @objc private func termHyperlinkChanged(_ sender: NSButton) {
        settings.terminalAllowHyperlink = sender.state == .on
        persistSettings()
    }

    @objc private func termBoldIsBrightChanged(_ sender: NSButton) {
        settings.terminalBoldIsBright = sender.state == .on
        persistSettings()
    }

    // MARK: - Appearance Actions

    @objc private func colorSchemeChanged(_ sender: NSPopUpButton) {
        guard let name = sender.titleOfSelectedItem else { return }
        settings.colorScheme = name
        persistSettings()

        let theme = ThemeManager.theme(forName: name)
        if let previewBox = findView(withIdentifier: "colorPreviewBox", in: window?.contentView) as? NSBox {
            previewBox.fillColor = theme.bg
            previewBox.contentView = buildColorPreview(theme: theme)
        }

        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.applyTheme(named: name)
        }
        NotificationCenter.default.post(name: .impulseThemeDidChange, object: theme)
    }

    private func findView(withIdentifier id: String, in view: NSView?) -> NSView? {
        guard let view else { return nil }
        if view.identifier?.rawValue == id { return view }
        for sub in view.subviews {
            if let found = findView(withIdentifier: id, in: sub) { return found }
        }
        return nil
    }

    @objc private func sidebarShowHiddenChanged(_ sender: NSButton) {
        settings.sidebarShowHidden = sender.state == .on
        persistSettings()
    }

    // MARK: - Automation Actions

    @objc private func addCommandOnSave(_ sender: Any?) {
        settings.commandsOnSave.append(CommandOnSave(
            name: "New Command",
            command: "",
            args: [],
            filePattern: "*",
            reloadFile: false
        ))
        persistSettings()
        reloadAutomationTable()
    }

    @objc private func removeCommandOnSave(_ sender: Any?) {
        guard let tableView = findTableView(withTag: 400) else { return }
        let row = tableView.selectedRow
        guard row >= 0 && row < settings.commandsOnSave.count else { return }
        settings.commandsOnSave.remove(at: row)
        persistSettings()
        reloadAutomationTable()
    }

    private func reloadAutomationTable() {
        findTableView(withTag: 400)?.reloadData()
    }

    // MARK: - File Type Override Actions

    @objc private func addFileTypeOverride(_ sender: Any?) {
        settings.fileTypeOverrides.append(FileTypeOverride(pattern: "*.ext"))
        persistSettings()
        findTableView(withTag: 600)?.reloadData()
    }

    @objc private func removeFileTypeOverride(_ sender: Any?) {
        guard let tableView = findTableView(withTag: 600) else { return }
        let row = tableView.selectedRow
        guard row >= 0 && row < settings.fileTypeOverrides.count else { return }
        settings.fileTypeOverrides.remove(at: row)
        persistSettings()
        tableView.reloadData()
    }

    @objc private func fileTypeOverrideDoubleClicked(_ sender: NSTableView) {
        let row = sender.clickedRow
        guard row >= 0 && row < settings.fileTypeOverrides.count else { return }
        let override_ = settings.fileTypeOverrides[row]
        guard let parentWindow = window else { return }

        let indentationValue: String = {
            if let useSpaces = override_.useSpaces {
                return useSpaces ? "Spaces" : "Tabs"
            }
            return "Default"
        }()

        SettingsFormSheet.present(
            on: parentWindow,
            title: "Edit File Type Override",
            fields: [
                FormField(label: "Pattern:", key: "pattern",
                          type: .text(placeholder: "e.g. *.rs", value: override_.pattern)),
                FormField(label: "Tab Width:", key: "tabWidth",
                          type: .text(placeholder: "Default", value: override_.tabWidth.map { "\($0)" } ?? "")),
                FormField(label: "Indentation:", key: "indentation",
                          type: .popup(options: ["Default", "Spaces", "Tabs"], selected: indentationValue)),
                FormField(label: "Format Command:", key: "fmtCommand",
                          type: .text(placeholder: "e.g. rustfmt", value: override_.formatOnSave?.command ?? "")),
                FormField(label: "Format Args:", key: "fmtArgs",
                          type: .text(placeholder: "e.g. --edition 2021",
                                      value: (override_.formatOnSave?.args ?? []).joined(separator: " "))),
            ]
        ) { [weak self] values in
            guard let self else { return }
            let pattern = values["pattern"]?.trimmingCharacters(in: .whitespaces) ?? ""
            guard !pattern.isEmpty else { return }

            let tabWidth = Int(values["tabWidth"] ?? "")
            let useSpaces: Bool? = {
                switch values["indentation"] {
                case "Spaces": return true
                case "Tabs":   return false
                default:       return nil
                }
            }()

            let fmtCmd = (values["fmtCommand"] ?? "").trimmingCharacters(in: .whitespaces)
            let fmtArgs = (values["fmtArgs"] ?? "")
                .split(separator: " ")
                .map(String.init)
            let formatOnSave: FormatOnSave? = fmtCmd.isEmpty ? nil : FormatOnSave(command: fmtCmd, args: fmtArgs)

            self.settings.fileTypeOverrides[row] = FileTypeOverride(
                pattern: pattern,
                tabWidth: tabWidth,
                useSpaces: useSpaces,
                formatOnSave: formatOnSave
            )
            self.persistSettings()
            sender.reloadData()
        }
    }

    // MARK: - Keybinding Actions

    @objc private func keybindingDoubleClicked(_ sender: NSTableView) {
        let row = sender.clickedRow
        guard row >= 0 && row < Keybindings.builtins.count else { return }
        let binding = Keybindings.builtins[row]
        guard let parentWindow = window else { return }

        let currentShortcut = settings.keybindingOverrides[binding.id] ?? binding.defaultShortcut

        SettingsFormSheet.present(
            on: parentWindow,
            title: "Edit Shortcut for \"\(binding.description)\"",
            fields: [
                FormField(label: "Shortcut:", key: "shortcut",
                          type: .text(placeholder: "e.g. Cmd+Shift+B", value: currentShortcut)),
            ]
        ) { [weak self] values in
            guard let self else { return }
            let newShortcut = (values["shortcut"] ?? "").trimmingCharacters(in: .whitespaces)
            guard !newShortcut.isEmpty else { return }

            if newShortcut == binding.defaultShortcut {
                self.settings.keybindingOverrides.removeValue(forKey: binding.id)
            } else {
                self.settings.keybindingOverrides[binding.id] = newShortcut
            }
            self.persistSettings()
            sender.reloadData()
        }
    }

    @objc private func resetKeybindings(_ sender: Any?) {
        settings.keybindingOverrides.removeAll()
        persistSettings()
        findTableView(withTag: 500)?.reloadData()
    }

    // MARK: - Custom Keybinding Actions

    @objc private func addCustomKeybinding(_ sender: Any?) {
        settings.customKeybindings.append(CustomKeybinding(
            name: "New Keybinding",
            key: "",
            command: "",
            args: []
        ))
        persistSettings()
        findTableView(withTag: 501)?.reloadData()
    }

    @objc private func removeCustomKeybinding(_ sender: Any?) {
        guard let tableView = findTableView(withTag: 501) else { return }
        let row = tableView.selectedRow
        guard row >= 0 && row < settings.customKeybindings.count else { return }
        settings.customKeybindings.remove(at: row)
        persistSettings()
        tableView.reloadData()
    }

    @objc private func customKeybindingDoubleClicked(_ sender: NSTableView) {
        let row = sender.clickedRow
        guard row >= 0 && row < settings.customKeybindings.count else { return }
        let kb = settings.customKeybindings[row]
        guard let parentWindow = window else { return }

        SettingsFormSheet.present(
            on: parentWindow,
            title: "Edit Custom Keybinding",
            fields: [
                FormField(label: "Name:", key: "name",
                          type: .text(placeholder: "e.g. Lazygit", value: kb.name)),
                FormField(label: "Key:", key: "key",
                          type: .text(placeholder: "e.g. Cmd+Shift+G", value: kb.key)),
                FormField(label: "Command:", key: "command",
                          type: .text(placeholder: "e.g. lazygit", value: kb.command)),
                FormField(label: "Args:", key: "args",
                          type: .text(placeholder: "e.g. --use-config-dir", value: kb.args.joined(separator: " "))),
            ]
        ) { [weak self] values in
            guard let self else { return }
            let name = (values["name"] ?? "").trimmingCharacters(in: .whitespaces)
            let key = (values["key"] ?? "").trimmingCharacters(in: .whitespaces)
            let command = (values["command"] ?? "").trimmingCharacters(in: .whitespaces)
            let args = (values["args"] ?? "")
                .split(separator: " ")
                .map(String.init)

            self.settings.customKeybindings[row] = CustomKeybinding(
                name: name.isEmpty ? "Untitled" : name,
                key: key,
                command: command,
                args: args
            )
            self.persistSettings()
            sender.reloadData()
        }
    }

    // MARK: - Table View Helpers

    private func findTableView(withTag tag: Int) -> NSTableView? {
        func find(in view: NSView) -> NSTableView? {
            if let tv = view as? NSTableView, tv.tag == tag { return tv }
            for sub in view.subviews {
                if let found = find(in: sub) { return found }
            }
            return nil
        }
        guard let content = window?.contentView else { return nil }
        return find(in: content)
    }
}

// MARK: - NSToolbarDelegate

extension SettingsWindowController: NSToolbarDelegate {

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        allPanes.map(\.id)
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        allPanes.map(\.id)
    }

    func toolbarSelectableItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        allPanes.map(\.id)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier itemIdentifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        guard let pane = allPanes.first(where: { $0.id == itemIdentifier }) else { return nil }

        let item = NSToolbarItem(itemIdentifier: itemIdentifier)
        item.label = pane.label
        item.image = NSImage(systemSymbolName: pane.icon, accessibilityDescription: pane.label)
        item.target = self
        item.action = #selector(toolbarPaneSelected(_:))
        return item
    }
}

// MARK: - NSTableViewDataSource & Delegate

extension SettingsWindowController: NSTableViewDataSource, NSTableViewDelegate {

    func numberOfRows(in tableView: NSTableView) -> Int {
        switch tableView.tag {
        case 400: return settings.commandsOnSave.count
        case 500: return Keybindings.builtins.count
        case 501: return settings.customKeybindings.count
        case 600: return settings.fileTypeOverrides.count
        default:  return 0
        }
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let identifier = tableColumn?.identifier ?? NSUserInterfaceItemIdentifier("")
        let cellId = NSUserInterfaceItemIdentifier("Cell_\(identifier.rawValue)")

        let cell: NSTextField
        if let existing = tableView.makeView(withIdentifier: cellId, owner: self) as? NSTextField {
            cell = existing
        } else {
            cell = NSTextField()
            cell.identifier = cellId
            cell.isBordered = false
            cell.drawsBackground = false
            cell.isEditable = false
            cell.lineBreakMode = .byTruncatingTail
        }

        switch tableView.tag {
        case 400:
            guard row < settings.commandsOnSave.count else { break }
            let cmd = settings.commandsOnSave[row]
            switch identifier.rawValue {
            case "name":    cell.stringValue = cmd.name
            case "command": cell.stringValue = cmd.command
            case "pattern": cell.stringValue = cmd.filePattern
            case "reload":  cell.stringValue = cmd.reloadFile ? "Yes" : "No"
            default: break
            }

        case 500:
            guard row < Keybindings.builtins.count else { break }
            let binding = Keybindings.builtins[row]
            switch identifier.rawValue {
            case "kb_description": cell.stringValue = binding.description
            case "kb_category":    cell.stringValue = binding.category
            case "kb_shortcut":
                let shortcut = settings.keybindingOverrides[binding.id] ?? binding.defaultShortcut
                cell.stringValue = shortcut
                if settings.keybindingOverrides[binding.id] != nil {
                    cell.textColor = .systemBlue
                } else {
                    cell.textColor = .labelColor
                }
            default: break
            }

        case 501:
            guard row < settings.customKeybindings.count else { break }
            let kb = settings.customKeybindings[row]
            switch identifier.rawValue {
            case "ck_name":    cell.stringValue = kb.name
            case "ck_key":     cell.stringValue = kb.key
            case "ck_command": cell.stringValue = kb.command
            case "ck_args":    cell.stringValue = kb.args.joined(separator: " ")
            default: break
            }

        case 600:
            guard row < settings.fileTypeOverrides.count else { break }
            let fto = settings.fileTypeOverrides[row]
            switch identifier.rawValue {
            case "fto_pattern":    cell.stringValue = fto.pattern
            case "fto_tab_width":  cell.stringValue = fto.tabWidth.map { "\($0)" } ?? "-"
            case "fto_use_spaces":
                if let s = fto.useSpaces { cell.stringValue = s ? "Yes" : "No" }
                else { cell.stringValue = "-" }
            case "fto_formatter":  cell.stringValue = fto.formatOnSave?.command ?? "-"
            default: break
            }

        default:
            break
        }

        return cell
    }
}
