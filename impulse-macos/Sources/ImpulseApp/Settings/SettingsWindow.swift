import AppKit

// MARK: - Settings Window Controller

/// NSWindow-based preferences window with tabbed sections for Editor, Terminal,
/// Appearance, Automation, and Keybindings. Changes save immediately.
final class SettingsWindowController: NSWindowController {

    private var settings: Settings
    private let tabView = NSTabView()

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
            contentRect: NSRect(x: 0, y: 0, width: 700, height: 560),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "Settings"
        window.center()
        window.isReleasedWhenClosed = false
        window.minSize = NSSize(width: 560, height: 420)

        super.init(window: window)

        buildTabs()
        window.contentView = tabView
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func close() {
        super.close()
        SettingsWindowController.shared = nil
    }

    // MARK: Tab Construction

    private func buildTabs() {
        tabView.tabViewType = .topTabsBezelBorder

        tabView.addTabViewItem(makeEditorTab())
        tabView.addTabViewItem(makeTerminalTab())
        tabView.addTabViewItem(makeAppearanceTab())
        tabView.addTabViewItem(makeAutomationTab())
        tabView.addTabViewItem(makeKeybindingsTab())
    }

    // MARK: - Editor Tab

    private func makeEditorTab() -> NSTabViewItem {
        let item = NSTabViewItem(identifier: "editor")
        item.label = "Editor"

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

        item.view = wrapInScrollView(stack)
        return item
    }

    // MARK: - Terminal Tab

    private func makeTerminalTab() -> NSTabViewItem {
        let item = NSTabViewItem(identifier: "terminal")
        item.label = "Terminal"

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

        item.view = wrapInScrollView(stack)
        return item
    }

    // MARK: - Appearance Tab

    private func makeAppearanceTab() -> NSTabViewItem {
        let item = NSTabViewItem(identifier: "appearance")
        item.label = "Appearance"

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

        item.view = wrapInScrollView(stack)
        return item
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

    // MARK: - Automation Tab

    private func makeAutomationTab() -> NSTabViewItem {
        let item = NSTabViewItem(identifier: "automation")
        item.label = "Automation"

        let outerScroll = NSScrollView()
        outerScroll.hasVerticalScroller = true
        outerScroll.drawsBackground = false

        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        // -- Commands on Save Section --

        let headerLabel = NSTextField(labelWithString: "Commands on Save")
        headerLabel.font = NSFont.boldSystemFont(ofSize: 13)
        headerLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(headerLabel)

        let descLabel = NSTextField(wrappingLabelWithString:
            "Commands that run automatically when a file matching the pattern is saved.")
        descLabel.font = NSFont.systemFont(ofSize: 11)
        descLabel.textColor = .secondaryLabelColor
        descLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(descLabel)

        // Table view for commands
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
        cmdScrollView.documentView = tableView

        container.addSubview(cmdScrollView)

        // Add/Remove buttons for commands
        let addCmdButton = NSButton(title: "Add", target: self, action: #selector(addCommandOnSave(_:)))
        addCmdButton.translatesAutoresizingMaskIntoConstraints = false
        let removeCmdButton = NSButton(title: "Remove", target: self, action: #selector(removeCommandOnSave(_:)))
        removeCmdButton.translatesAutoresizingMaskIntoConstraints = false
        removeCmdButton.tag = 401

        container.addSubview(addCmdButton)
        container.addSubview(removeCmdButton)

        // -- File Type Overrides Section --

        let ftoHeader = NSTextField(labelWithString: "File Type Overrides")
        ftoHeader.font = NSFont.boldSystemFont(ofSize: 13)
        ftoHeader.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(ftoHeader)

        let ftoDesc = NSTextField(wrappingLabelWithString:
            "Per-file-type overrides for tab width, spaces vs tabs, and format-on-save.")
        ftoDesc.font = NSFont.systemFont(ofSize: 11)
        ftoDesc.textColor = .secondaryLabelColor
        ftoDesc.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(ftoDesc)

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
        ftoScrollView.documentView = ftoTable

        container.addSubview(ftoScrollView)

        let addFtoButton = NSButton(title: "Add", target: self, action: #selector(addFileTypeOverride(_:)))
        addFtoButton.translatesAutoresizingMaskIntoConstraints = false
        let removeFtoButton = NSButton(title: "Remove", target: self, action: #selector(removeFileTypeOverride(_:)))
        removeFtoButton.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(addFtoButton)
        container.addSubview(removeFtoButton)

        NSLayoutConstraint.activate([
            // Commands on Save
            headerLabel.topAnchor.constraint(equalTo: container.topAnchor, constant: 16),
            headerLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),

            descLabel.topAnchor.constraint(equalTo: headerLabel.bottomAnchor, constant: 4),
            descLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            descLabel.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),

            cmdScrollView.topAnchor.constraint(equalTo: descLabel.bottomAnchor, constant: 12),
            cmdScrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            cmdScrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
            cmdScrollView.heightAnchor.constraint(equalToConstant: 120),

            addCmdButton.topAnchor.constraint(equalTo: cmdScrollView.bottomAnchor, constant: 8),
            addCmdButton.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),

            removeCmdButton.topAnchor.constraint(equalTo: cmdScrollView.bottomAnchor, constant: 8),
            removeCmdButton.leadingAnchor.constraint(equalTo: addCmdButton.trailingAnchor, constant: 8),

            // File Type Overrides
            ftoHeader.topAnchor.constraint(equalTo: addCmdButton.bottomAnchor, constant: 24),
            ftoHeader.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),

            ftoDesc.topAnchor.constraint(equalTo: ftoHeader.bottomAnchor, constant: 4),
            ftoDesc.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            ftoDesc.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),

            ftoScrollView.topAnchor.constraint(equalTo: ftoDesc.bottomAnchor, constant: 12),
            ftoScrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            ftoScrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
            ftoScrollView.heightAnchor.constraint(equalToConstant: 120),

            addFtoButton.topAnchor.constraint(equalTo: ftoScrollView.bottomAnchor, constant: 8),
            addFtoButton.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),

            removeFtoButton.topAnchor.constraint(equalTo: ftoScrollView.bottomAnchor, constant: 8),
            removeFtoButton.leadingAnchor.constraint(equalTo: addFtoButton.trailingAnchor, constant: 8),

            removeFtoButton.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -16),

            container.widthAnchor.constraint(greaterThanOrEqualToConstant: 500),
        ])

        outerScroll.documentView = container
        item.view = outerScroll
        return item
    }

    // MARK: - Keybindings Tab

    private func makeKeybindingsTab() -> NSTabViewItem {
        let item = NSTabViewItem(identifier: "keybindings")
        item.label = "Keybindings"

        let container = NSView()

        let descLabel = NSTextField(wrappingLabelWithString:
            "Double-click a shortcut to edit it. Overrides are saved to settings.")
        descLabel.font = NSFont.systemFont(ofSize: 11)
        descLabel.textColor = .secondaryLabelColor
        descLabel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(descLabel)

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
        scrollView.documentView = tableView

        container.addSubview(scrollView)

        // Reset button
        let resetButton = NSButton(title: "Reset All to Defaults", target: self,
                                    action: #selector(resetKeybindings(_:)))
        resetButton.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(resetButton)

        NSLayoutConstraint.activate([
            descLabel.topAnchor.constraint(equalTo: container.topAnchor, constant: 16),
            descLabel.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            descLabel.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),

            scrollView.topAnchor.constraint(equalTo: descLabel.bottomAnchor, constant: 8),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
            scrollView.bottomAnchor.constraint(equalTo: resetButton.topAnchor, constant: -8),

            resetButton.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
            resetButton.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -16),
        ])

        item.view = container
        return item
    }

    // MARK: - Helpers

    private func makeLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.systemFont(ofSize: 13)
        label.alignment = .right
        return label
    }

    /// Creates a section header with a bold title and optional subtitle.
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

    /// Creates a horizontal separator line.
    private func makeSeparator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }

    /// Creates a labeled row with a control on the right side.
    private func makeRow(label: String, control: NSView) -> NSStackView {
        let labelView = NSTextField(labelWithString: label)
        labelView.font = NSFont.systemFont(ofSize: 13)
        labelView.alignment = .right
        labelView.setContentHuggingPriority(.defaultHigh, for: .horizontal)
        labelView.translatesAutoresizingMaskIntoConstraints = false
        labelView.widthAnchor.constraint(greaterThanOrEqualToConstant: 130).isActive = true

        let row = NSStackView(views: [labelView, control])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 12
        row.translatesAutoresizingMaskIntoConstraints = false
        return row
    }

    /// Wraps a section header + items into a vertical group within the outer stack.
    private func addSection(to stack: NSStackView, title: String, subtitle: String? = nil,
                            rows: [NSView], addSeparator: Bool = true) {
        if addSeparator && !stack.arrangedSubviews.isEmpty {
            stack.addArrangedSubview(makeSeparator())
        }
        stack.addArrangedSubview(makeSectionHeader(title: title, subtitle: subtitle))
        for row in rows {
            stack.addArrangedSubview(row)
        }
    }

    /// Wraps a stack view in a scroll view with consistent padding.
    /// A flipped NSView so the document view pins to the top of the scroll area.
    private final class FlippedView: NSView {
        override var isFlipped: Bool { true }
    }

    private func wrapInScrollView(_ stack: NSStackView) -> NSView {
        stack.translatesAutoresizingMaskIntoConstraints = false

        // Wrap the stack in a flipped container so content aligns to the top.
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
        settings.save()
        // Propagate to AppDelegate so runtime state stays in sync.
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.settings = settings
        }
        // Notify all windows so they can apply the new settings immediately.
        NotificationCenter.default.post(name: .impulseSettingsDidChange, object: settings)
    }

    // MARK: - Editor Actions

    @objc private func fontFamilyChanged(_ sender: NSTextField) {
        settings.fontFamily = sender.stringValue
        persistSettings()
    }

    @objc private func fontSizeStepperChanged(_ sender: NSStepper) {
        settings.fontSize = sender.integerValue
        // Update the label next to the stepper
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

        // Update the preview box by searching the entire window content view.
        let theme = ThemeManager.theme(forName: name)
        if let previewBox = findView(withIdentifier: "colorPreviewBox", in: window?.contentView) as? NSBox {
            previewBox.fillColor = theme.bg
            previewBox.contentView = buildColorPreview(theme: theme)
        }

        // Post theme change notification
        if let delegate = NSApp.delegate as? AppDelegate {
            delegate.applyTheme(named: name)
        }
        NotificationCenter.default.post(name: .impulseThemeDidChange, object: theme)
    }

    /// Recursively searches for a view with the given identifier.
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

        let alert = NSAlert()
        alert.messageText = "Edit File Type Override"
        alert.informativeText = "Configure per-file-type settings."
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")

        let grid = NSGridView(numberOfColumns: 2, rows: 0)
        grid.rowSpacing = 6
        grid.columnSpacing = 8
        grid.column(at: 0).xPlacement = .trailing

        let patternField = NSTextField(string: override_.pattern)
        grid.addRow(with: [makeLabel("Pattern:"), patternField])

        let tabWidthField = NSTextField(string: override_.tabWidth.map { "\($0)" } ?? "")
        tabWidthField.placeholderString = "Default"
        grid.addRow(with: [makeLabel("Tab Width:"), tabWidthField])

        let spacesPopup = NSPopUpButton()
        spacesPopup.addItems(withTitles: ["Default", "Spaces", "Tabs"])
        if let useSpaces = override_.useSpaces {
            spacesPopup.selectItem(withTitle: useSpaces ? "Spaces" : "Tabs")
        } else {
            spacesPopup.selectItem(withTitle: "Default")
        }
        grid.addRow(with: [makeLabel("Indentation:"), spacesPopup])

        let fmtCommandField = NSTextField(string: override_.formatOnSave?.command ?? "")
        fmtCommandField.placeholderString = "e.g. rustfmt"
        grid.addRow(with: [makeLabel("Format Command:"), fmtCommandField])

        let fmtArgsField = NSTextField(string: (override_.formatOnSave?.args ?? []).joined(separator: " "))
        fmtArgsField.placeholderString = "e.g. --edition 2021"
        grid.addRow(with: [makeLabel("Format Args:"), fmtArgsField])

        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.widthAnchor.constraint(greaterThanOrEqualToConstant: 320).isActive = true
        alert.accessoryView = grid
        alert.window.initialFirstResponder = patternField

        let response = alert.runModal()
        guard response == .alertFirstButtonReturn else { return }

        let pattern = patternField.stringValue.trimmingCharacters(in: .whitespaces)
        guard !pattern.isEmpty else { return }

        let tabWidth = Int(tabWidthField.stringValue)
        let useSpaces: Bool? = {
            switch spacesPopup.titleOfSelectedItem {
            case "Spaces": return true
            case "Tabs":   return false
            default:       return nil
            }
        }()

        let fmtCmd = fmtCommandField.stringValue.trimmingCharacters(in: .whitespaces)
        let fmtArgs = fmtArgsField.stringValue
            .split(separator: " ")
            .map(String.init)
        let formatOnSave: FormatOnSave? = fmtCmd.isEmpty ? nil : FormatOnSave(command: fmtCmd, args: fmtArgs)

        settings.fileTypeOverrides[row] = FileTypeOverride(
            pattern: pattern,
            tabWidth: tabWidth,
            useSpaces: useSpaces,
            formatOnSave: formatOnSave
        )
        persistSettings()
        sender.reloadData()
    }

    // MARK: - Keybinding Actions

    @objc private func keybindingDoubleClicked(_ sender: NSTableView) {
        let row = sender.clickedRow
        guard row >= 0 && row < Keybindings.builtins.count else { return }
        let binding = Keybindings.builtins[row]

        let currentShortcut = settings.keybindingOverrides[binding.id] ?? binding.defaultShortcut

        let alert = NSAlert()
        alert.messageText = "Edit Shortcut for \"\(binding.description)\""
        alert.informativeText = "Enter the new shortcut (e.g. Cmd+Shift+B):"
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        alert.addButton(withTitle: "Reset to Default")

        let input = NSTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        input.stringValue = currentShortcut
        alert.accessoryView = input

        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            let newShortcut = input.stringValue.trimmingCharacters(in: .whitespaces)
            if !newShortcut.isEmpty {
                if newShortcut == binding.defaultShortcut {
                    settings.keybindingOverrides.removeValue(forKey: binding.id)
                } else {
                    settings.keybindingOverrides[binding.id] = newShortcut
                }
                persistSettings()
                sender.reloadData()
            }
        } else if response == .alertThirdButtonReturn {
            settings.keybindingOverrides.removeValue(forKey: binding.id)
            persistSettings()
            sender.reloadData()
        }
    }

    @objc private func resetKeybindings(_ sender: Any?) {
        settings.keybindingOverrides.removeAll()
        persistSettings()
        findTableView(withTag: 500)?.reloadData()
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

// MARK: - NSTableViewDataSource & Delegate

extension SettingsWindowController: NSTableViewDataSource, NSTableViewDelegate {

    func numberOfRows(in tableView: NSTableView) -> Int {
        switch tableView.tag {
        case 400: return settings.commandsOnSave.count
        case 500: return Keybindings.builtins.count
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
