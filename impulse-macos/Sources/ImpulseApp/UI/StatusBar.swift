import AppKit

// MARK: - Status Bar

/// Bottom status bar showing CWD, git branch, shell name, cursor position,
/// language, encoding, and indentation info.
///
/// This mirrors the functionality of the GTK `StatusBar` in the Linux frontend.
/// Label visibility is context-dependent: terminal tabs show shell name and CWD;
/// editor tabs additionally show cursor position, language, encoding, and
/// indentation info.
final class StatusBar: NSView {

    // MARK: - Properties

    private let cwdLabel = NSTextField(labelWithString: "")
    private let gitBranchLabel = NSTextField(labelWithString: "")
    private let shellNameLabel = NSTextField(labelWithString: "")
    private let blameLabel = NSTextField(labelWithString: "")
    private let cursorPositionLabel = NSTextField(labelWithString: "")
    private let languageLabel = NSTextField(labelWithString: "")
    private let encodingLabel = NSTextField(labelWithString: "UTF-8")
    private let indentInfoLabel = NSTextField(labelWithString: "")

    private let topBorder = NSView()

    /// The fixed status bar height.
    static let barHeight: CGFloat = 24

    // MARK: - Initialization

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setupViews()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setupViews()
    }

    private func setupViews() {
        wantsLayer = true

        // Top border
        topBorder.wantsLayer = true
        topBorder.translatesAutoresizingMaskIntoConstraints = false
        addSubview(topBorder)

        // Configure all labels
        let allLabels = [cwdLabel, gitBranchLabel, shellNameLabel,
                         blameLabel, cursorPositionLabel, languageLabel,
                         encodingLabel, indentInfoLabel]
        for label in allLabels {
            label.font = NSFont.systemFont(ofSize: 12)
            label.drawsBackground = false
            label.isBezeled = false
            label.isEditable = false
            label.isSelectable = false
            label.lineBreakMode = .byTruncatingTail
        }

        // CWD takes remaining space
        cwdLabel.lineBreakMode = .byTruncatingMiddle
        cwdLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)
        cwdLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        // Right-side labels should not compress
        for label in [cursorPositionLabel, languageLabel, encodingLabel, indentInfoLabel] {
            label.setContentHuggingPriority(.defaultHigh, for: .horizontal)
            label.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        }

        // Shell name label (visible for terminal tabs only)
        shellNameLabel.isHidden = true

        // Blame label (hidden by default, shown in editor context)
        blameLabel.isHidden = true
        blameLabel.lineBreakMode = .byTruncatingTail
        blameLabel.setContentHuggingPriority(.defaultLow - 1, for: .horizontal)
        blameLabel.setContentCompressionResistancePriority(.defaultLow - 1, for: .horizontal)

        // Editor-specific labels (hidden by default)
        cursorPositionLabel.isHidden = true
        languageLabel.isHidden = true
        encodingLabel.isHidden = true
        indentInfoLabel.isHidden = true

        // Git branch label (hidden until a branch is detected)
        gitBranchLabel.isHidden = true

        // Build the horizontal stack
        let stackView = NSStackView(views: [
            shellNameLabel,
            gitBranchLabel,
            cwdLabel,
            blameLabel,
            indentInfoLabel,
            encodingLabel,
            languageLabel,
            cursorPositionLabel,
        ])
        stackView.orientation = .horizontal
        stackView.spacing = 12
        stackView.alignment = .centerY
        stackView.distribution = .fill
        stackView.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12)
        stackView.translatesAutoresizingMaskIntoConstraints = false

        addSubview(stackView)

        NSLayoutConstraint.activate([
            // Fixed height
            heightAnchor.constraint(equalToConstant: Self.barHeight),

            // Top border
            topBorder.topAnchor.constraint(equalTo: topAnchor),
            topBorder.leadingAnchor.constraint(equalTo: leadingAnchor),
            topBorder.trailingAnchor.constraint(equalTo: trailingAnchor),
            topBorder.heightAnchor.constraint(equalToConstant: 1),

            // Stack view fills the bar below the border
            stackView.topAnchor.constraint(equalTo: topBorder.bottomAnchor),
            stackView.leadingAnchor.constraint(equalTo: leadingAnchor),
            stackView.trailingAnchor.constraint(equalTo: trailingAnchor),
            stackView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    // MARK: - Update Methods

    /// Updates the status bar for a terminal tab context.
    ///
    /// Shows CWD, git branch (if available), and shell name. Hides
    /// editor-specific labels.
    func updateForTerminal(cwd: String, gitBranch: String?, shellName: String) {
        let displayPath = shortenHomePath(cwd)
        cwdLabel.stringValue = displayPath

        if let branch = gitBranch, !branch.isEmpty {
            gitBranchLabel.stringValue = "\u{E0A0} \(branch)"  // Branch icon
            gitBranchLabel.isHidden = false
        } else {
            gitBranchLabel.isHidden = true
        }

        shellNameLabel.stringValue = shellName
        shellNameLabel.isHidden = false

        // Hide editor-specific labels
        cursorPositionLabel.isHidden = true
        languageLabel.isHidden = true
        encodingLabel.isHidden = true
        indentInfoLabel.isHidden = true
        blameLabel.isHidden = true
    }

    /// Updates the status bar for an editor tab context.
    ///
    /// Shows CWD (derived from file path), git branch (if available), cursor
    /// position, language, encoding, and indentation info. Hides shell name.
    func updateForEditor(filePath: String, gitBranch: String? = nil,
                         cursorLine: Int, cursorCol: Int,
                         language: String, tabWidth: Int, useSpaces: Bool) {
        // Derive CWD from file path
        let dir = (filePath as NSString).deletingLastPathComponent
        let displayPath = shortenHomePath(dir)
        cwdLabel.stringValue = displayPath

        if let branch = gitBranch, !branch.isEmpty {
            gitBranchLabel.stringValue = "\u{E0A0} \(branch)"
            gitBranchLabel.isHidden = false
        } else {
            gitBranchLabel.isHidden = true
        }

        shellNameLabel.isHidden = true

        cursorPositionLabel.stringValue = "Ln \(cursorLine), Col \(cursorCol)"
        cursorPositionLabel.isHidden = false

        languageLabel.stringValue = language
        languageLabel.isHidden = false

        encodingLabel.stringValue = "UTF-8"
        encodingLabel.isHidden = false

        let indentType = useSpaces ? "Spaces" : "Tabs"
        indentInfoLabel.stringValue = "\(indentType): \(tabWidth)"
        indentInfoLabel.isHidden = false
    }

    /// Updates the blame label with the given info string.
    func updateBlame(_ info: String) {
        blameLabel.stringValue = info
        blameLabel.isHidden = false
    }

    /// Hides the blame label.
    func clearBlame() {
        blameLabel.isHidden = true
    }

    // MARK: - Theme

    /// Applies the given theme colors to the status bar and all labels.
    func applyTheme(_ theme: Theme) {
        layer?.backgroundColor = theme.bgDark.cgColor
        topBorder.layer?.backgroundColor = theme.bgHighlight.cgColor

        cwdLabel.textColor = theme.fg
        gitBranchLabel.textColor = theme.magenta
        shellNameLabel.textColor = theme.cyan
        cursorPositionLabel.textColor = theme.fgDark
        languageLabel.textColor = theme.blue
        encodingLabel.textColor = theme.fgDark
        indentInfoLabel.textColor = theme.fgDark
        blameLabel.textColor = theme.fgDark
    }

    // MARK: - Helpers

    /// Shortens a path by replacing the home directory prefix with `~`.
    private func shortenHomePath(_ path: String) -> String {
        let home = NSHomeDirectory()
        if path.hasPrefix(home) {
            return "~" + String(path.dropFirst(home.count))
        }
        return path
    }
}
