import AppKit

/// A container that supports splitting terminals horizontally or vertically
/// using `NSSplitView`. Starts with a single `TerminalTab` and can split
/// into an arbitrary number of panes.
class TerminalContainer: NSView, NSSplitViewDelegate {

    // MARK: Public Properties

    /// All terminal tabs currently in this container, in display order.
    private(set) var terminals: [TerminalTab] = []

    /// Index of the currently active (focused) terminal within `terminals`.
    private(set) var activeTerminalIndex: Int = 0

    /// The computed active terminal, or nil if the container is empty.
    var activeTerminal: TerminalTab? {
        guard terminals.indices.contains(activeTerminalIndex) else { return nil }
        return terminals[activeTerminalIndex]
    }

    // MARK: Private Properties

    private var splitView: NSSplitView?

    /// Settings and theme for creating new terminals when splitting.
    private var currentSettings: TerminalSettings
    private var currentTheme: TerminalTheme

    // MARK: Initializer

    init(frame frameRect: NSRect, settings: TerminalSettings, theme: TerminalTheme) {
        self.currentSettings = settings
        self.currentTheme = theme
        super.init(frame: frameRect)

        let initialTerminal = createTerminal()
        terminals.append(initialTerminal)

        addSubview(initialTerminal)
        constrainChildToFill(initialTerminal)

        initialTerminal.spawnShell(initialDirectory: settings.lastDirectory.isEmpty ? nil : settings.lastDirectory)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: Splitting

    /// Split the current layout horizontally, placing terminals side by side.
    @discardableResult
    func splitVertically() -> TerminalTab {
        return performSplit(isVertical: true)
    }

    /// Split the current layout vertically, stacking terminals top and bottom.
    @discardableResult
    func splitHorizontally() -> TerminalTab {
        return performSplit(isVertical: false)
    }

    private func performSplit(isVertical: Bool) -> TerminalTab {
        let newTerminal = createTerminal()
        // Capture the active terminal's CWD before we change the active index.
        let inheritedCwd = activeTerminal?.currentWorkingDirectory

        if splitView == nil {
            // Currently showing a single terminal. Replace it with a split view.
            guard let existingTerminal = terminals.first else {
                // Should never happen; add and return the new terminal directly.
                terminals.append(newTerminal)
                addSubview(newTerminal)
                constrainChildToFill(newTerminal)
                newTerminal.spawnShell(initialDirectory: inheritedCwd)
                return newTerminal
            }

            existingTerminal.removeFromSuperview()
            removeAllConstraints()

            let split = NSSplitView()
            split.isVertical = isVertical
            split.dividerStyle = .thin
            split.delegate = self

            split.addArrangedSubview(existingTerminal)
            split.addArrangedSubview(newTerminal)

            addSubview(split)
            constrainChildToFill(split)

            splitView = split
        } else if let split = splitView {
            // Already split. If the orientation matches, add another pane.
            // If the orientation differs, wrap the active pane in a nested split.
            if split.isVertical == isVertical {
                // Same orientation: insert after the active terminal.
                let insertIndex = min(activeTerminalIndex + 1, split.arrangedSubviews.count)
                split.insertArrangedSubview(newTerminal, at: insertIndex)
            } else {
                // Different orientation: wrap the active terminal in a nested split.
                guard let activeView = activeTerminal else {
                    split.addArrangedSubview(newTerminal)
                    terminals.append(newTerminal)
                    newTerminal.spawnShell(initialDirectory: inheritedCwd)
                    return newTerminal
                }

                let nestedSplit = NSSplitView()
                nestedSplit.isVertical = isVertical
                nestedSplit.dividerStyle = .thin
                nestedSplit.delegate = self

                let activeIndex = split.arrangedSubviews.firstIndex(of: activeView)
                activeView.removeFromSuperview()

                nestedSplit.addArrangedSubview(activeView)
                nestedSplit.addArrangedSubview(newTerminal)

                if let idx = activeIndex {
                    split.insertArrangedSubview(nestedSplit, at: idx)
                } else {
                    split.addArrangedSubview(nestedSplit)
                }
            }
        }

        terminals.append(newTerminal)
        activeTerminalIndex = terminals.count - 1
        newTerminal.spawnShell(initialDirectory: inheritedCwd)
        newTerminal.focus()

        return newTerminal
    }

    // MARK: Removing Terminals

    /// Remove the terminal at the given index. If only one terminal remains
    /// after removal, collapse the split view back to a single terminal.
    func removeTerminal(at index: Int) {
        guard terminals.indices.contains(index) else { return }
        let terminal = terminals[index]
        terminals.remove(at: index)

        // Remove the terminal view from whatever parent it sits in.
        terminal.removeFromSuperview()

        if terminals.count <= 1 {
            // Collapse back to single view.
            collapseSplitView()
        } else {
            // Clean up empty nested split views.
            cleanUpEmptySplitViews()
        }

        // Adjust the active index.
        if terminals.isEmpty {
            activeTerminalIndex = 0
        } else {
            activeTerminalIndex = min(index, terminals.count - 1)
            terminals[activeTerminalIndex].focus()
        }
    }

    private func collapseSplitView() {
        splitView?.removeFromSuperview()
        splitView = nil
        removeAllConstraints()

        guard let remaining = terminals.first else { return }
        // The remaining terminal may be inside a nested split; pull it out.
        remaining.removeFromSuperview()
        addSubview(remaining)
        constrainChildToFill(remaining)
    }

    private func cleanUpEmptySplitViews() {
        guard let split = splitView else { return }
        pruneEmptySplits(in: split)
    }

    /// Recursively remove NSSplitViews that have fewer than 2 arranged subviews,
    /// promoting the sole child up to the parent.
    private func pruneEmptySplits(in split: NSSplitView) {
        for subview in split.arrangedSubviews {
            if let nested = subview as? NSSplitView {
                pruneEmptySplits(in: nested)
                if nested.arrangedSubviews.count == 1, let sole = nested.arrangedSubviews.first {
                    let idx = split.arrangedSubviews.firstIndex(of: nested)
                    sole.removeFromSuperview()
                    nested.removeFromSuperview()
                    if let idx = idx {
                        split.insertArrangedSubview(sole, at: idx)
                    } else {
                        split.addArrangedSubview(sole)
                    }
                } else if nested.arrangedSubviews.isEmpty {
                    nested.removeFromSuperview()
                }
            }
        }
    }

    // MARK: Focus Navigation

    /// Move focus to the next terminal in the list, wrapping around.
    func focusNextSplit() {
        guard terminals.count > 1 else { return }
        activeTerminalIndex = (activeTerminalIndex + 1) % terminals.count
        terminals[activeTerminalIndex].focus()
    }

    /// Move focus to the previous terminal in the list, wrapping around.
    func focusPreviousSplit() {
        guard terminals.count > 1 else { return }
        activeTerminalIndex = (activeTerminalIndex - 1 + terminals.count) % terminals.count
        terminals[activeTerminalIndex].focus()
    }

    // MARK: Propagating Settings

    /// Apply a theme to all child terminals.
    func applyTheme(theme: TerminalTheme) {
        currentTheme = theme
        for terminal in terminals {
            terminal.applyTheme(theme: theme)
        }
    }

    /// Apply settings to all child terminals.
    func applySettings(settings: TerminalSettings) {
        currentSettings = settings
        for terminal in terminals {
            terminal.configureTerminal(settings: settings, theme: currentTheme)
        }
    }

    // MARK: NSSplitViewDelegate

    func splitView(
        _ splitView: NSSplitView,
        constrainMinCoordinate proposedMinimumPosition: CGFloat,
        ofSubviewAt dividerIndex: Int
    ) -> CGFloat {
        return max(proposedMinimumPosition, 100)
    }

    func splitView(
        _ splitView: NSSplitView,
        constrainMaxCoordinate proposedMaximumPosition: CGFloat,
        ofSubviewAt dividerIndex: Int
    ) -> CGFloat {
        let dimension = splitView.isVertical ? splitView.bounds.width : splitView.bounds.height
        return min(proposedMaximumPosition, dimension - 100)
    }

    // MARK: Helpers

    private func createTerminal() -> TerminalTab {
        let terminal = TerminalTab(frame: bounds)
        terminal.configureTerminal(settings: currentSettings, theme: currentTheme)
        return terminal
    }

    private func constrainChildToFill(_ child: NSView) {
        child.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            child.topAnchor.constraint(equalTo: topAnchor),
            child.leadingAnchor.constraint(equalTo: leadingAnchor),
            child.trailingAnchor.constraint(equalTo: trailingAnchor),
            child.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    private func removeAllConstraints() {
        for constraint in constraints {
            removeConstraint(constraint)
        }
    }
}
