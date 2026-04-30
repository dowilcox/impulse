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

    /// Whether any split pane in this terminal tab is requesting attention.
    var needsAttention: Bool {
        terminals.contains { $0.needsAttention }
    }

    // MARK: Private Properties

    private var splitView: ThemedSplitView?

    /// Color used for the split view divider, matching the status bar border.
    private var dividerColor: NSColor?

    /// Settings and theme for creating new terminals when splitting.
    private var currentSettings: TerminalSettings
    private var currentTheme: TerminalTheme

    // MARK: Initializer

    init(frame frameRect: NSRect, settings: TerminalSettings, theme: TerminalTheme, initialCommand: String? = nil) {
        self.currentSettings = settings
        self.currentTheme = theme
        super.init(frame: frameRect)

        let initialTerminal = createTerminal()
        terminals.append(initialTerminal)

        addSubview(initialTerminal)
        constrainChildToFill(initialTerminal)

        // Defer shell spawning until after Auto Layout has resolved the
        // terminal view's frame, ensuring the PTY starts with the correct
        // column/row dimensions. Spawning synchronously here would use the
        // pre-layout frame (missing the 8px padding insets), causing a
        // COLUMNS mismatch that breaks line wrapping and cursor navigation.
        let dir = settings.lastDirectory.isEmpty ? nil : settings.lastDirectory
        DispatchQueue.main.async {
            initialTerminal.spawnShell(initialDirectory: dir, initialCommand: initialCommand)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    // MARK: Splitting

    /// Split with a vertical divider, placing terminals side by side.
    @discardableResult
    func splitVertically() -> TerminalTab {
        return performSplit(isVertical: true)
    }

    /// Split with a horizontal divider, stacking terminals top and bottom.
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
                DispatchQueue.main.async {
                    newTerminal.spawnShell(initialDirectory: inheritedCwd)
                }
                return newTerminal
            }

            existingTerminal.removeFromSuperview()
            removeAllConstraints()

            let split = ThemedSplitView()
            split.isVertical = isVertical
            split.dividerStyle = .thin
            split.delegate = self
            split.customDividerColor = dividerColor

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
                    DispatchQueue.main.async {
                        newTerminal.spawnShell(initialDirectory: inheritedCwd)
                    }
                    return newTerminal
                }

                let nestedSplit = ThemedSplitView()
                nestedSplit.isVertical = isVertical
                nestedSplit.dividerStyle = .thin
                nestedSplit.delegate = self
                nestedSplit.customDividerColor = dividerColor

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
        DispatchQueue.main.async {
            newTerminal.spawnShell(initialDirectory: inheritedCwd)
        }
        newTerminal.focus()

        return newTerminal
    }

    // MARK: Removing Terminals

    /// Terminate all shell processes in this container. Must be called before
    /// the container is removed from the tab list to ensure child processes
    /// are cleaned up.
    func terminateAllProcesses() {
        for terminal in terminals {
            terminal.terminateProcess()
        }
    }

    /// Remove the terminal at the given index. If only one terminal remains
    /// after removal, collapse the split view back to a single terminal.
    func removeTerminal(at index: Int) {
        guard terminals.indices.contains(index) else { return }
        let terminal = terminals[index]
        terminal.terminateProcess()
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
            // Force a redraw on all remaining terminals. When the split
            // view rearranges after pane removal, child views don't
            // automatically get needsDisplay and stay black until resized.
            for t in terminals {
                t.renderer.needsDisplay = true
            }
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
        remaining.renderer.needsDisplay = true
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

    /// Apply a theme to all child terminals and update split divider colors.
    func applyTheme(theme: TerminalTheme, dividerColor: NSColor? = nil) {
        currentTheme = theme
        if let color = dividerColor {
            self.dividerColor = color
        }
        for terminal in terminals {
            terminal.applyTheme(theme: theme)
        }
        // Update divider color on all split views.
        if let color = self.dividerColor {
            applyDividerColor(color, in: self)
        }
    }

    /// Recursively set the divider color on all ThemedSplitViews.
    private func applyDividerColor(_ color: NSColor, in view: NSView) {
        for subview in view.subviews {
            if let split = subview as? ThemedSplitView {
                split.customDividerColor = color
            }
            applyDividerColor(color, in: subview)
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
        terminal.onFocused = { [weak self] focusedTerminal in
            guard let self,
                  let index = self.terminals.firstIndex(where: { $0 === focusedTerminal }) else {
                return
            }
            self.activeTerminalIndex = index
        }
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
