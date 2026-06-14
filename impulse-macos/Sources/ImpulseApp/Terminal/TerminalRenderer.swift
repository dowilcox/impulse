import AppKit
import CoreText
import os.log

// MARK: - Font Metrics

/// Computes cell dimensions from a monospace font for terminal grid rendering.
struct TerminalFontMetrics {
    let font: CTFont
    let cellWidth: CGFloat
    let cellHeight: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let leading: CGFloat

    init(fontFamily: String, fontSize: CGFloat) {
        let ctFont: CTFont
        if !fontFamily.isEmpty,
           let descriptor = CTFontDescriptorCreateWithNameAndSize(fontFamily as CFString, fontSize) as CTFontDescriptor? {
            ctFont = CTFontCreateWithFontDescriptor(descriptor, fontSize, nil)
        } else {
            ctFont = CTFontCreateUIFontForLanguage(.userFixedPitch, fontSize, nil)!
        }
        self.font = ctFont
        self.ascent = CTFontGetAscent(ctFont)
        self.descent = CTFontGetDescent(ctFont)
        self.leading = CTFontGetLeading(ctFont)
        self.cellHeight = ceil(ascent + descent + leading)

        // Measure "W" for monospace cell width.
        let attrString = CFAttributedStringCreate(
            nil, "W" as CFString,
            [kCTFontAttributeName: ctFont] as CFDictionary
        )!
        let line = CTLineCreateWithAttributedString(attrString)
        let bounds = CTLineGetBoundsWithOptions(line, [])
        self.cellWidth = ceil(bounds.width)
    }

    /// Compute the grid dimensions that fit in a view of the given size.
    func gridSize(viewWidth: CGFloat, viewHeight: CGFloat, padding: CGFloat) -> (cols: Int, rows: Int) {
        let cols = max(2, Int((viewWidth - padding * 2) / cellWidth))
        let rows = max(1, Int((viewHeight - padding * 2) / cellHeight))
        return (cols, rows)
    }
}

// MARK: - Terminal Renderer

/// NSView subclass that renders a terminal grid using CoreText and CoreGraphics.
///
/// Reads cell data from a `TerminalBackend` via `gridSnapshot()` and renders
/// text, backgrounds, selections, search highlights, cursor, and box-drawing
/// characters. Uses an adaptive main-thread timer for refresh timing and `KeyEncoder` for
/// keyboard input translation.
class TerminalRenderer: NSView {

    // MARK: Public Properties

    var backend: TerminalBackend?
    private(set) var fontMetrics: TerminalFontMetrics
    let padding: CGFloat = 12

    /// Called when the backend emits a non-wakeup event (title change, bell, exit, etc.).
    var onEvent: ((TerminalBackendEvent) -> Void)?

    /// Called when the user presses Cmd+V in the terminal.
    var onPaste: (() -> Void)?

    /// Called when the user presses Cmd+C in the terminal.
    var onCopy: (() -> Void)?

    /// Called when the user chooses command-block actions from the context menu.
    var onCopyLastCommand: (() -> Void)?
    var onCopyLastCommandOutput: (() -> Void)?
    var onRerunLastCommand: (() -> Void)?

    /// Per-block actions (Warp-style: act on the block that was clicked, not
    /// the most recent one), driven by the right-click menu and the hover
    /// toolbar.
    var onCopyBlockCommand: ((UInt64) -> Void)?
    var onCopyBlockOutput: ((UInt64) -> Void)?
    var onCopyBlockCommandAndOutput: ((UInt64) -> Void)?
    var onRerunBlock: ((UInt64) -> Void)?
    private var contextBlockId: UInt64?

    // MARK: Block Hover Toolbar

    private enum BlockToolbarButton: Equatable {
        case copyOutput
        case menu
    }
    private struct BlockToolbarTarget {
        let rect: CGRect  // in view coordinates (includes the bottom-anchor offset)
        let button: BlockToolbarButton
        let blockId: UInt64
    }
    /// Hit targets for the hover toolbar buttons, rebuilt every frame.
    private var hoverToolbarTargets: [BlockToolbarTarget] = []
    /// The toolbar button currently under the pointer (for hover highlight).
    private var hoveredToolbarButton: BlockToolbarButton? = nil {
        didSet { if hoveredToolbarButton != oldValue { needsDisplay = true } }
    }
    var onShowCommandHistory: (() -> Void)?
    var onJumpToPreviousCommandBlock: (() -> Void)?
    var onJumpToNextCommandBlock: (() -> Void)?
    var onJumpToLastFailedCommandBlock: (() -> Void)?

    /// Called when a mouse selection gesture finishes with a real selection.
    var onSelectionFinished: (() -> Void)?

    /// Called when the renderer gains or loses first responder focus.
    var onFocusChanged: ((Bool) -> Void)?

    /// Override cursor shape from user settings (0=Block, 1=Beam, 2=Underline).
    /// Applied instead of the shape reported by the backend grid buffer.
    var cursorShapeOverride: UInt8 = 0

    /// Whether cursor blinking is enabled (from user settings).
    var cursorBlinkEnabled: Bool = true {
        didSet {
            if cursorBlinkEnabled {
                startBlinkTimer()
            } else {
                stopBlinkTimer()
                cursorBlinkOn = true
            }
        }
    }

    /// Cursor color from the active theme.
    var cursorColor: CGColor = CGColor(srgbRed: 0.86, green: 0.84, blue: 0.73, alpha: 1.0)

    /// Default terminal background from the active theme/config.
    var defaultBackgroundColor: CGColor = CGColor(srgbRed: 0.12, green: 0.12, blue: 0.16, alpha: 1.0)
    var defaultBackgroundRgb: (UInt8, UInt8, UInt8) = (31, 31, 40)

    /// Theme-aware selection overlay color.
    var selectionColor: CGColor = CGColor(srgbRed: 0.2, green: 0.4, blue: 0.8, alpha: 0.35)

    /// Whether bold text should use bright palette colors (0-7 → 8-15).
    var boldIsBright: Bool = true

    /// Whether to auto-scroll to the bottom when the terminal produces output.
    var scrollOnOutput: Bool = true

    /// Whether OSC 8 and auto-detected URLs should be interactive.
    var allowHyperlinks: Bool = true

    /// Runtime keybinding overrides from settings.
    var keybindingOverrides: [String: String] = [:]

    /// 16-color ANSI palette (RGB triplets). Set from the theme.
    /// Used to substitute bold text colors when `boldIsBright` is true.
    var paletteRgb: [(UInt8, UInt8, UInt8)] = []

    // MARK: Block Decoration Properties (Warp-style)

    /// Master switch for command-block decorations (driven by settings).
    var blocksEnabled: Bool = true {
        didSet { if blocksEnabled != oldValue { needsDisplay = true } }
    }

    /// Block emphasized by block navigation; drawn with an accent wash.
    var highlightedBlockId: UInt64? = nil {
        didSet { if highlightedBlockId != oldValue { needsDisplay = true } }
    }

    /// Hairline drawn between command blocks.
    var blockSeparatorColor = CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.10)
    /// Status chip text for successful commands.
    var blockMutedTextColor = CGColor(srgbRed: 0.62, green: 0.64, blue: 0.70, alpha: 1.0)
    /// Failure stripe, wash, and chip text (theme red).
    var blockFailedColor = CGColor(srgbRed: 0.75, green: 0.38, blue: 0.42, alpha: 1.0)
    /// Running-command stripe and navigation highlight (theme accent).
    var blockAccentColor = CGColor(srgbRed: 0.51, green: 0.63, blue: 0.76, alpha: 1.0)
    /// Subtle wash behind the live input-prompt region.
    var blockPromptFillColor = CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.045)

    /// Smaller font for block status chips, derived from the terminal font.
    private var chipFont: CTFont?

    /// Block currently under the pointer; washed like Warp's hover highlight.
    private var hoveredBlockId: UInt64? = nil {
        didSet {
            if hoveredBlockId != oldValue {
                needsDisplay = true
                window?.invalidateCursorRects(for: self)
            }
        }
    }

    /// Fired (on the main queue) when the alternate screen toggles, so the
    /// input bar can hide while TUIs own the grid.
    var onAltScreenChanged: ((Bool) -> Void)?
    private var lastAltScreen = false

    /// Warp model: the grid only takes keyboard input while a full-screen TUI
    /// owns the alternate screen — otherwise all typing flows through the
    /// pinned input bar, so the scrollback reads as immutable command blocks.
    var keyboardInteractive: Bool { lastAltScreen }

    /// Asked to move keyboard focus to the input bar (e.g. the user clicked
    /// the read-only grid at a prompt).
    var onRequestInputFocus: (() -> Void)?

    /// Warp model: the input bar replaces the shell's in-grid prompt, so the
    /// live prompt region is not rendered at all and the last output line is
    /// anchored directly above the bar. Driven by the context-bar setting;
    /// ignored on the alternate screen (TUIs own the full grid).
    var suppressLivePrompt: Bool = true {
        didSet { if suppressLivePrompt != oldValue { needsDisplay = true } }
    }

    /// Warp-style bottom anchoring: while the session is shorter than the
    /// viewport, content is pushed to the bottom edge so the input bar sits
    /// directly under the last block. 0 once the screen fills.
    private(set) var contentYOffset: CGFloat = 0

    /// Guards the async "tuck" scroll that hides the in-grid prompt below a
    /// full screen, so it isn't dispatched repeatedly before it lands.
    private var pendingPromptTuck = false

    /// Set when a command finishes so the prompt is tucked off-screen once, on
    /// the next idle frame. Not re-armed by manual scrolling, so scrolling back
    /// to the true bottom reveals the prompt instead of bouncing.
    private var shouldTuckPrompt = false

    /// True once a tuck scroll has actually been issued for the armed command.
    /// Lets us tell "prompt scrolled off-screen" (done) apart from "prompt not
    /// drawn yet" (both report no visible prompt) so we disarm at the right time.
    private var tuckScrolled = false

    /// Display offset captured the moment the prompt finished tucking off the
    /// bottom. Manual scroll-down is clamped so the offset can't drop below it,
    /// which keeps the prompt hidden — the last command output stays the true
    /// scroll bottom. `nil` while a command runs or before any tuck lands.
    private var scrollFloorOffset: Int?

    /// Latest display offset reported by the overlay; used by the scroll clamp.
    private var currentDisplayOffset: Int32 = 0

    /// Vertical gap (a "tad" of padding) each collapsed blank prompt-padding row
    /// reserves between command blocks, in points. A normal row is `cellHeight`.
    private let blockPadPixels: CGFloat = 6

    /// Per-frame row layout, written during `draw()` so methods running outside
    /// it (`rectForRow`, blink, `gridPoint`) map rows consistently.
    /// `frameRowTops[r]` = the content-space top Y of grid row r (collapsed rows
    /// shrink to `blockPadPixels`); length `lines + 1`.
    private var frameCollapsedRows: Set<Int> = []
    private var frameRowTops: [CGFloat] = []

    /// Content-space top Y of grid row `r` (before the bottom-anchor translate).
    /// Falls back to the uniform layout before the per-frame map is built.
    private func rowTopY(_ r: Int) -> CGFloat {
        guard !frameRowTops.isEmpty else { return padding + CGFloat(r) * fontMetrics.cellHeight }
        return frameRowTops[min(max(0, r), frameRowTops.count - 1)]
    }

    /// Y of the hairline border drawn at the top of a block. Centered in the
    /// collapsed prompt-padding gap above the block (when present) so the block
    /// reads with symmetric top/bottom breathing room; otherwise the row's top.
    private func blockBorderY(_ row: Int) -> CGFloat {
        if row > 0, frameCollapsedRows.contains(row - 1) {
            return rowTopY(row) - blockPadPixels / 2
        }
        return rowTopY(row)
    }

    /// Inverse of `rowTopY`: the grid row drawn at content-space Y `py`. A point
    /// in a collapsed gap resolves to the block-start row just below it — the
    /// desired hover/selection target.
    private func gridRowForContentY(_ py: CGFloat) -> Int {
        guard frameRowTops.count > 1 else {
            return max(0, Int((py - padding) / fontMetrics.cellHeight))
        }
        let lines = frameRowTops.count - 1
        // Largest row whose top is <= py.
        var lo = 0, hi = lines - 1, found = 0
        while lo <= hi {
            let mid = (lo + hi) / 2
            if frameRowTops[mid] <= py { found = mid; lo = mid + 1 } else { hi = mid - 1 }
        }
        if frameCollapsedRows.contains(found), found + 1 < lines { return found + 1 }
        return found
    }

    /// Arm a one-shot prompt tuck (called when a command completes).
    func scheduleTuck() {
        shouldTuckPrompt = true
        tuckScrolled = false
        scrollFloorOffset = nil
    }

    // MARK: Private Properties

    private var refreshTimer: DispatchSourceTimer?
    private var scrollAccumulator: CGFloat = 0
    private var isScrolledBack: Bool = false
    private var isSelecting = false
    private var pendingSelectionAnchor: (col: UInt16, row: UInt16)?
    private var shouldCopySelectionOnMouseUp = false
    private var needsRedraw = false
    private var cursorBlinkOn: Bool = true
    private var blinkTimer: DispatchSourceTimer?
    /// Viewport row the cursor occupied at the last draw; used to invalidate
    /// only that row when the blink phase toggles. -1 when unknown/hidden.
    private var lastCursorRow: Int = -1
    private var colorCache: [UInt32: CGColor] = [:]
    private var textLineCache: [UInt64: CTLine] = [:]
    private let activeRefreshInterval: TimeInterval = 1.0 / 60.0
    private let idleRefreshInterval: TimeInterval = 1.0 / 30.0

    // IME composition state. When non-empty, an IME is composing text that
    // has not yet been committed to the PTY.
    private var markedText: String = ""
    private var markedSelection = NSRange(location: 0, length: 0)
    private var currentKeyEvent: NSEvent?

    // Hyperlink hover state.
    private var hoverCol: Int = -1
    private var hoverRow: Int = -1
    private var hoverIsLink: Bool = false
    /// When hovering a link, the cell-range [startCol, endCol) on hoverRow
    /// that forms the link. Used to draw the hover underline and to resolve
    /// the URI on Cmd+Click (for auto-detected URLs).
    private var hoverLinkStartCol: Int = 0
    private var hoverLinkEndCol: Int = 0
    private var hoverLinkUri: String?
    private var trackingArea: NSTrackingArea?

    /// Regex for auto-detecting plain URLs in terminal output.
    /// Matches http/https URLs up to the first whitespace or common
    /// trailing-punctuation boundary.
    private static let urlRegex: NSRegularExpression? = {
        try? NSRegularExpression(
            pattern: #"https?://[^\s<>"'`]+"#,
            options: []
        )
    }()

    // Cached font variants for bold/italic.
    private var boldFont: CTFont?
    private var italicFont: CTFont?
    private var boldItalicFont: CTFont?

    // MARK: Initialization

    init(frame: NSRect, fontFamily: String, fontSize: CGFloat) {
        self.fontMetrics = TerminalFontMetrics(fontFamily: fontFamily, fontSize: fontSize)
        super.init(frame: frame)
        cacheFontVariants()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        stopRefreshLoop()
        stopBlinkTimer()
    }

    // MARK: View Properties

    override var acceptsFirstResponder: Bool { keyboardInteractive }
    override var isFlipped: Bool { true }

    override func becomeFirstResponder() -> Bool {
        backend?.setFocus(true)
        onFocusChanged?(true)
        return true
    }

    override func resignFirstResponder() -> Bool {
        backend?.setFocus(false)
        onFocusChanged?(false)
        return true
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            startRefreshLoop()
            needsDisplay = true
        } else {
            stopRefreshLoop()
        }
    }

    // MARK: Adaptive Refresh Loop

    func startRefreshLoop() {
        guard refreshTimer == nil else { return }
        scheduleRefresh(after: 0)
    }

    func stopRefreshLoop() {
        refreshTimer?.cancel()
        refreshTimer = nil
    }

    private func scheduleRefresh(after interval: TimeInterval) {
        guard refreshTimer == nil else { return }
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + interval)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.refreshTimer = nil
            guard self.window != nil else { return }
            let hadActivity = self.tick()
            self.scheduleRefresh(after: hadActivity ? self.activeRefreshInterval : self.idleRefreshInterval)
        }
        refreshTimer = timer
        timer.resume()
    }

    // MARK: Cursor Blink Timer

    private func startBlinkTimer() {
        stopBlinkTimer()
        cursorBlinkOn = true
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 0.5, repeating: 0.5)
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.cursorBlinkOn.toggle()
            if self.lastCursorRow >= 0 {
                self.setNeedsDisplay(self.rectForRow(self.lastCursorRow))
            } else {
                self.needsDisplay = true
            }
        }
        blinkTimer = timer
        timer.resume()
    }

    private func stopBlinkTimer() {
        blinkTimer?.cancel()
        blinkTimer = nil
    }

    /// Reset blink phase to visible. Call on keyboard input so the cursor
    /// stays solid while the user is typing.
    func resetBlink() {
        guard cursorBlinkEnabled else { return }
        cursorBlinkOn = true
        startBlinkTimer()
    }

    @discardableResult
    private func tick() -> Bool {
        guard let backend, !backend.isShutdown else { return false }
        let events = backend.pollEvents()
        var wakeup = false
        for event in events {
            switch event {
            case .wakeup:
                wakeup = true
            default:
                onEvent?(event)
            }
        }
        if wakeup && !isScrolledBack {
            // Auto-scroll to bottom on output when enabled and the user
            // hasn't manually scrolled back.
            if scrollOnOutput {
                backend.scrollToBottom()
            }
            // Invalidate only the damaged rows. Each row rect is expanded by
            // one cell height on both sides so glyphs that overshoot their
            // cell (emoji, tall scripts) repaint cleanly in neighbours.
            switch backend.takeDamage() {
            case .full:
                needsDisplay = true
            case .rows(let rows):
                if contentYOffset > 0 {
                    // Bottom-anchored: every new line shifts all rows.
                    needsRedraw = false
                    setNeedsDisplay(bounds)
                    wakeup = true
                    break
                }
                let ch = fontMetrics.cellHeight
                for row in rows {
                    // Clamp to bounds: top/bottom rows would otherwise damage
                    // the shared canvas outside the view (see draw(_:)).
                    setNeedsDisplay(
                        rectForRow(row).insetBy(dx: 0, dy: -ch).intersection(bounds))
                }
            }
        }
        return !events.isEmpty || wakeup
    }

    // MARK: Dirty-Row Geometry

    /// The full-width rect covering one viewport row (in display coordinates,
    /// so blank-row collapse keeps damage invalidation aligned with drawing).
    func rectForRow(_ row: Int) -> NSRect {
        NSRect(
            x: 0,
            y: contentYOffset + rowTopY(row),
            width: bounds.width,
            height: fontMetrics.cellHeight
        )
    }

    /// The viewport rows whose cells intersect `rect` (clamped to 0..<lines).
    /// Pure geometry so it stays unit-testable.
    static func rowRange(
        intersecting rect: NSRect, lines: Int, padding: CGFloat, cellHeight: CGFloat
    ) -> Range<Int> {
        guard lines > 0, cellHeight > 0 else { return 0..<max(0, lines) }
        let first = Int(((rect.minY - padding) / cellHeight).rounded(.down))
        let last = Int(((rect.maxY - padding) / cellHeight).rounded(.up))
        let lower = min(max(0, first), lines)
        let upper = min(max(lower, last), lines)
        return lower..<upper
    }

    /// True when viewport `row` has no visible glyph (only spaces/nulls).
    static func isBlankRow(grid: GridBufferReader, row: Int, cols: Int) -> Bool {
        guard row >= 0, row < grid.lines else { return true }
        for col in 0..<cols {
            let cell = grid.cell(row: row, col: col)
            if cell.flags & GridBufferReader.flagWideCharSpacer != 0 { continue }
            let value = cell.character.value
            if value != 0 && value != 32 { return false }
        }
        return true
    }

    /// Walk up from `upTo` and return the index of the last viewport row that
    /// contains a non-blank glyph, or -1 if every row at/below it is empty.
    /// Used to strip the blank padding shells print before a prompt so output
    /// sits flush against the input bar.
    static func lastNonBlankRow(grid: GridBufferReader, upTo: Int, cols: Int) -> Int {
        var row = min(upTo, grid.lines - 1)
        while row >= 0 {
            if !isBlankRow(grid: grid, row: row, cols: cols) { return row }
            row -= 1
        }
        return -1
    }

    /// Blank prompt-padding grid rows to collapse so command blocks tile with
    /// no empty row between them. Mirrors `contiguousStartRows`' gap rule
    /// exactly (`gap > 1 && gap <= maxPaddingRows + 1`) so a row is never both
    /// absorbed into the block above's wash AND removed. Only rows at or above
    /// `lastContentRow` are eligible — the suppressed/tucked prompt region below
    /// it is never collapsed.
    static func collapsedRows(
        blocks: [TerminalBlockOverlayRegion],
        grid: GridBufferReader, cols: Int,
        lastContentRow: Int, maxPaddingRows: Int = 3
    ) -> Set<Int> {
        guard lastContentRow >= 0 else { return [] }
        var collapsed = Set<Int>()
        var prevEnd: Int?
        for block in blocks {
            let rawStart = Int(block.startRow)
            if let prevEnd, case let gap = rawStart - prevEnd, gap > 1, gap <= maxPaddingRows + 1 {
                for r in (prevEnd + 1)..<rawStart where r <= lastContentRow {
                    if isBlankRow(grid: grid, row: r, cols: cols) { collapsed.insert(r) }
                }
            }
            prevEnd = Int(block.endRow)
        }
        return collapsed
    }


    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        // AppKit can pass a dirty rect that extends beyond our bounds: inside
        // an NSHostingView hierarchy the view draws into a canvas layer shared
        // with SwiftUI content, and a relayout (e.g. the tab bar appearing)
        // damages the whole canvas. Painting outside our bounds would stamp
        // terminal background over sibling SwiftUI chrome like the tab bar,
        // so clip everything to our own bounds.
        context.clip(to: bounds)
        let dirtyRect = dirtyRect.intersection(bounds)
        guard !dirtyRect.isEmpty else { return }
        guard let backend, !backend.isShutdown else {
            // Draw default background when no backend is available.
            context.setFillColor(defaultBackgroundColor)
            context.fill(bounds)
            return
        }

        guard let grid = backend.gridSnapshot() else {
            context.setFillColor(defaultBackgroundColor)
            context.fill(bounds)
            return
        }

        let cols = grid.cols
        let lines = grid.lines
        let cw = fontMetrics.cellWidth
        let ch = fontMetrics.cellHeight

        // Surface alternate-screen flips to the input bar without mutating
        // observable state mid-render.
        let altScreenNow = grid.altScreen
        if altScreenNow != lastAltScreen {
            lastAltScreen = altScreenNow
            if let onAltScreenChanged {
                DispatchQueue.main.async { onAltScreenChanged(altScreenNow) }
            }
        }

        // Command-block decorations: viewport-mapped block regions from the
        // backend, skipped on the alternate screen (TUIs own the grid there).
        let blockOverlay: TerminalBlockOverlay? = {
            guard blocksEnabled, let overlay = backend.blockOverlay(), !overlay.altScreen else {
                return nil
            }
            return overlay
        }()
        // Rebuilt while drawing the hovered block's toolbar below.
        hoverToolbarTargets = []

        // Warp model: the input bar IS the prompt, so the shell's own in-grid
        // prompt is redundant. When the bar is active (not a TUI) and the shell
        // is idle at a prompt, render only up to the last command's output and
        // bottom-anchor it against the input bar — no empty gap, no redundant
        // prompt. We key off the last block's output-end row (precise) rather
        // than the OSC 133;A row, because shells print the cwd line *before*
        // that mark, so the mark sits mid-prompt.
        let suppressPrompt = suppressLivePrompt && !altScreenNow
        // -1 means "draw nothing" (a fresh shell with only an empty prompt).
        var lastContentRow = lines - 1
        // Push content to the bottom edge (to meet the input bar) only for a
        // short session that fits in the viewport. A screenful of scrollback
        // is already full — pushing it would only open a blank gap at the top.
        var allowAnchor = false
        if let overlay = blockOverlay {
            let contentFillsViewport = overlay.blocks.contains { $0.startRow < 0 }
            let atIdlePrompt = overlay.promptRow != nil
            if suppressPrompt && atIdlePrompt {
                // Always hide the redundant in-grid prompt (the input bar is
                // the prompt), whether or not the screen is full.
                if let lastBlock = overlay.blocks.last {
                    lastContentRow = Int(lastBlock.endRow)
                } else if overlay.hasBlocks {
                    lastContentRow = overlay.promptRow.map { Int($0) - 1 } ?? (lines - 1)
                } else {
                    lastContentRow = -1
                }
                // The block's end row runs down to where the next prompt begins,
                // so it includes the blank padding line(s) shells print before a
                // prompt. Trim those trailing blanks so the last *visible* text —
                // not empty rows — anchors against the input bar; otherwise the
                // suppressed prompt's padding shows up as a gap.
                lastContentRow = Self.lastNonBlankRow(
                    grid: grid, upTo: min(lastContentRow, lines - 1), cols: cols)
                allowAnchor = !contentFillsViewport
                // A full screen can't be pushed (it would gap at the top), so
                // instead scroll the grid up to tuck the prompt off the bottom —
                // the last output then sits flush against the input bar, with
                // scrollback filling the rows above. The shell renders its prompt
                // over several frames, so a single scroll can land short; keep
                // nudging once per command until the last content is flush, then
                // disarm. Disarming on flush is what stops the scroll-to-bottom
                // bounce: manual scrolling never re-arms it.
                if contentFillsViewport && shouldTuckPrompt {
                    let hidden = (lines - 1) - lastContentRow
                    if hidden <= 0 {
                        shouldTuckPrompt = false
                    } else if !pendingPromptTuck {
                        pendingPromptTuck = true
                        tuckScrolled = true
                        let backend = self.backend
                        DispatchQueue.main.async { [weak self] in
                            backend?.scroll(delta: Int32(hidden))
                            self?.pendingPromptTuck = false
                            self?.needsDisplay = true
                        }
                    }
                }
            } else if let cursorRow = overlay.cursorRow {
                // Running command (or prompt not suppressed): the live output
                // anchors to the bottom while it fits. Drop the scroll floor so
                // output isn't clamped while it streams in.
                lastContentRow = Int(cursorRow)
                allowAnchor = !contentFillsViewport
                scrollFloorOffset = nil
            }
            // Once a tuck has scrolled the prompt off the bottom, the prompt is
            // no longer visible (promptRow == nil) — that is the signal that
            // convergence is done. Disarm so a later manual scroll-to-bottom
            // can't re-fire the tuck and bounce. We require tuckScrolled so a
            // not-yet-drawn prompt (also nil) doesn't disarm us prematurely. The
            // offset at this moment becomes the scroll floor: scrolling back down
            // can't pass it, so the prompt stays tucked and the last output stays
            // the bottom.
            if shouldTuckPrompt && tuckScrolled && !pendingPromptTuck && !atIdlePrompt {
                shouldTuckPrompt = false
                scrollFloorOffset = Int(overlay.displayOffset)
            }
            currentDisplayOffset = overlay.displayOffset
        }
        lastContentRow = max(-1, min(lastContentRow, lines - 1))

        // Collapse blank prompt-padding rows to a small pixel gap (draw-only).
        // Runs AFTER the tuck / anchor / scroll-floor decisions are final —
        // those stay in grid space; this is a pure pixel relayout on top.
        let collapsed: Set<Int> = blockOverlay.map {
            Self.collapsedRows(blocks: $0.blocks, grid: grid, cols: cols, lastContentRow: lastContentRow)
        } ?? []
        frameCollapsedRows = collapsed
        // Cumulative row tops: a collapsed row shrinks to `blockPadPixels`, every
        // other row is `ch`. frameRowTops[lines] is the content's bottom edge.
        var tops = [CGFloat](repeating: padding, count: lines + 1)
        if lines > 0 {
            var y = padding
            for r in 0..<lines {
                tops[r] = y
                y += collapsed.contains(r) ? blockPadPixels : ch
            }
            tops[lines] = y
        }
        frameRowTops = tops
        let collapseActive = !collapsed.isEmpty

        // Anchor so the last content row sits flush against the input bar.
        // Fires when anchoring a short session OR whenever collapse shrank the
        // on-screen content (the freed rows surface at the top, bottom stays
        // flush). Derive the offset from the actual pixel bottom of the last
        // content row so it is correct under both collapse and uniform layout.
        var yOffset: CGFloat = 0
        if lastContentRow >= 0 && (allowAnchor || collapseActive) {
            let contentBottom = rowTopY(lastContentRow) + ch
            yOffset = max(0, padding + CGFloat(lines) * ch - contentBottom)
        }
        let offsetChanged = yOffset != contentYOffset
        contentYOffset = yOffset
        if offsetChanged {
            // Row positions moved; this pass may be clipped to a stale dirty
            // region, so schedule a full repaint for the next frame.
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.setNeedsDisplay(self.bounds)
            }
        }

        // Only repaint rows that intersect the invalidated region. tick()
        // invalidates per damaged row; full invalidations (theme, resize,
        // scroll, …) arrive as the whole bounds and select every row. Rows
        // past lastContentRow (the suppressed prompt region) are never drawn.
        let rowCeiling = max(0, lastContentRow + 1)
        // `rowRange` maps the dirty rect to *display* lines, but `drawRows` is
        // consumed as *grid* rows. When collapse is active the two diverge, so
        // repaint every row (collapse only happens at an idle prompt — the cost
        // is negligible). Same when the anchor offset moved.
        let rawDrawRows = (offsetChanged || collapseActive)
            ? 0..<lines
            : Self.rowRange(
                intersecting: dirtyRect.offsetBy(dx: 0, dy: -yOffset),
                lines: lines, padding: padding, cellHeight: ch
            )
        let drawRows = min(rawDrawRows.lowerBound, rowCeiling)..<min(rawDrawRows.upperBound, rowCeiling)

        // 1. Fill background with the configured terminal background color.
        // Do not infer it from the top-left cell: TUIs like Codex/Claude can
        // place a colored block in the first visible cell, which would make
        // the entire viewport inherit that accent color.
        context.setFillColor(defaultBackgroundColor)
        context.fill(dirtyRect)

        // All content below draws in bottom-anchored coordinates.
        context.translateBy(x: 0, y: yOffset)

        // 1b. Washes under the text: failed/highlighted block tints and the
        // live input-prompt region. Per-row fills so partial repaints stay
        // consistent with the row-based damage model.
        if let blockOverlay {
            drawBlockWashes(
                context: context, overlay: blockOverlay, drawRows: drawRows, lines: lines
            )
        }

        // 2. Draw non-default cell backgrounds.
        drawCellBackgrounds(
            context: context,
            grid: grid,
            cols: cols,
            rows: drawRows,
            cellWidth: cw,
            cellHeight: ch
        )

        // 3. Draw selection highlights using the active theme color.
        for i in 0..<grid.selectionRangeCount {
            let range = grid.selectionRange(at: i)
            guard drawRows.contains(Int(range.row)) else { continue }
            let rowY = rowTopY(Int(range.row))
            let startX = padding + CGFloat(range.startCol) * cw
            let endX = padding + CGFloat(min(range.endCol + 1, cols)) * cw
            let rect = CGRect(x: startX, y: rowY, width: endX - startX, height: ch)
            context.setFillColor(selectionColor)
            context.fill(rect)
        }

        // 4. Draw search match highlights (amber semi-transparent).
        let searchColor = CGColor(srgbRed: 0.9, green: 0.7, blue: 0.1, alpha: 0.35)
        for i in 0..<grid.searchMatchRangeCount {
            let range = grid.searchMatchRange(at: i)
            guard drawRows.contains(Int(range.row)) else { continue }
            let rowY = rowTopY(Int(range.row))
            let startX = padding + CGFloat(range.startCol) * cw
            let endX = padding + CGFloat(min(range.endCol + 1, cols)) * cw
            let rect = CGRect(x: startX, y: rowY, width: endX - startX, height: ch)
            context.setFillColor(searchColor)
            context.fill(rect)
        }

        // 5. Draw text using run-based rendering.
        // In a flipped NSView, CoreGraphics text still renders Y-up.
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)

        for row in drawRows {
            if frameCollapsedRows.contains(row) { continue }
            let rowY = rowTopY(row)

            // Run accumulation state.
            var runString = ""
            var runStartCol = 0
            var runFgR: UInt8 = 0
            var runFgG: UInt8 = 0
            var runFgB: UInt8 = 0
            var runBold = false
            var runItalic = false
            var runDim = false
            var hasRun = false

            for col in 0..<cols {
                let cell = grid.cell(row: row, col: col)
                let flags = cell.flags

                // Skip wide char spacer cells.
                if flags & GridBufferReader.flagWideCharSpacer != 0 { continue }

                // Skip hidden cells.
                if flags & GridBufferReader.flagHidden != 0 { continue }

                let scalar = cell.character
                let codepoint = scalar.value

                // Determine effective fg color (respecting inverse).
                var fgR = cell.fgR, fgG = cell.fgG, fgB = cell.fgB
                var bgR = cell.bgR, bgG = cell.bgG, bgB = cell.bgB
                if flags & GridBufferReader.flagInverse != 0 {
                    swap(&fgR, &bgR)
                    swap(&fgG, &bgG)
                    swap(&fgB, &bgB)
                }

                let isBold = flags & GridBufferReader.flagBold != 0
                let isItalic = flags & GridBufferReader.flagItalic != 0
                let isDim = flags & GridBufferReader.flagDim != 0
                // Keep custom rendering limited to line/box glyphs. Block and
                // shade glyphs (0x2580...) are common in TUIs for fills and
                // scroll indicators; the font renderer handles those more
                // reliably than our snapped-rect path.
                let isBoxDrawing = codepoint >= 0x2500 && codepoint <= 0x257F
                let isWideChar = flags & GridBufferReader.flagWideChar != 0

                // Bold-is-bright: if bold and the foreground matches one of
                // the 8 normal ANSI palette colors, substitute with the bright
                // variant (palette index + 8).
                if isBold && boldIsBright && paletteRgb.count >= 16 {
                    for i in 0..<8 {
                        let (pr, pg, pb) = paletteRgb[i]
                        if fgR == pr && fgG == pg && fgB == pb {
                            let (br, bg, bb) = paletteRgb[i + 8]
                            fgR = br; fgG = bg; fgB = bb
                            break
                        }
                    }
                }

                // Check if we need to flush the current run.
                let styleChanged = hasRun && (fgR != runFgR || fgG != runFgG || fgB != runFgB
                    || isBold != runBold || isItalic != runItalic || isDim != runDim)

                if (styleChanged || isBoxDrawing || isWideChar) && hasRun && !runString.isEmpty {
                    drawTextRun(
                        context: context, text: runString, col: runStartCol, rowY: rowY,
                        fgR: runFgR, fgG: runFgG, fgB: runFgB,
                        bold: runBold, italic: runItalic, dim: runDim
                    )
                    runString = ""
                    hasRun = false
                }

                if isWideChar {
                    // Wide characters (East Asian fullwidth) occupy 2 cells.
                    // Draw alone so subsequent runs restart at col + 2 (the
                    // spacer cell at col + 1 is skipped by flagWideCharSpacer).
                    drawTextRun(
                        context: context, text: String(scalar),
                        col: col, rowY: rowY,
                        fgR: fgR, fgG: fgG, fgB: fgB,
                        bold: isBold, italic: isItalic, dim: isDim
                    )
                    continue
                }

                if isBoxDrawing {
                    // Draw box-drawing character programmatically.
                    let cellX = padding + CGFloat(col) * cw
                    drawBoxDrawing(
                        context: context, codepoint: codepoint,
                        x: cellX, y: rowY, width: cw, height: ch,
                        fgR: fgR, fgG: fgG, fgB: fgB, dim: isDim
                    )
                } else {
                    // Skip spaces (empty cells) that use the default background.
                    // We still need to advance the column though.
                    if codepoint == 0x20 || codepoint == 0 {
                        if hasRun && !runString.isEmpty {
                            // Append a space to the run to preserve positioning.
                            runString.append(" ")
                        }
                        // If no run, just skip.
                        continue
                    }

                    if !hasRun {
                        runStartCol = col
                        runFgR = fgR
                        runFgG = fgG
                        runFgB = fgB
                        runBold = isBold
                        runItalic = isItalic
                        runDim = isDim
                        hasRun = true
                    }
                    runString.append(String(scalar))
                }
            }

            // Flush remaining run for this row.
            if hasRun && !runString.isEmpty {
                drawTextRun(
                    context: context, text: runString, col: runStartCol, rowY: rowY,
                    fgR: runFgR, fgG: runFgG, fgB: runFgB,
                    bold: runBold, italic: runItalic, dim: runDim
                )
            }

            // 6. Draw underline and strikethrough decorations.
            for col in 0..<cols {
                let cell = grid.cell(row: row, col: col)
                let flags = cell.flags
                if flags & GridBufferReader.flagWideCharSpacer != 0 { continue }

                var fgR = cell.fgR, fgG = cell.fgG, fgB = cell.fgB
                if flags & GridBufferReader.flagInverse != 0 {
                    fgR = cell.bgR; fgG = cell.bgG; fgB = cell.bgB
                }
                let alpha: CGFloat = (flags & GridBufferReader.flagDim != 0) ? 0.5 : 1.0
                let cellX = padding + CGFloat(col) * cw
                let cellWidth = (flags & GridBufferReader.flagWideChar != 0) ? cw * 2 : cw

                if flags & GridBufferReader.flagUnderline != 0 {
                    context.setStrokeColor(cachedColor(red: fgR, green: fgG, blue: fgB, alpha: alpha))
                    context.setLineWidth(1)
                    let underlineY = rowY + fontMetrics.ascent + fontMetrics.descent - 1
                    context.move(to: CGPoint(x: cellX, y: underlineY))
                    context.addLine(to: CGPoint(x: cellX + cellWidth, y: underlineY))
                    context.strokePath()
                }

                if flags & GridBufferReader.flagStrikethrough != 0 {
                    context.setStrokeColor(cachedColor(red: fgR, green: fgG, blue: fgB, alpha: alpha))
                    context.setLineWidth(1)
                    let strikeY = rowY + ch / 2
                    context.move(to: CGPoint(x: cellX, y: strikeY))
                    context.addLine(to: CGPoint(x: cellX + cellWidth, y: strikeY))
                    context.strokePath()
                }
            }
        }

        // 6b. Block separators, stripes, and status chips above the text.
        if let blockOverlay {
            drawBlockDecorations(
                context: context, overlay: blockOverlay, drawRows: drawRows, lines: lines
            )
        }

        // 7. Draw cursor (respects blink phase and shape override from
        // settings). When the input bar is the prompt (not a TUI), the input
        // bar owns the cursor — never blink a redundant grid cursor, even when
        // the in-grid prompt is shown (a screenful of scrollback).
        lastCursorRow = grid.cursorVisible ? Int(grid.cursorRow) : -1
        if grid.cursorVisible && cursorBlinkOn && !suppressPrompt {
            let cursorRow = grid.cursorRow
            let cursorCol = grid.cursorCol
            if cursorRow < lines && cursorCol < cols && drawRows.contains(Int(cursorRow)) {
                let cursorX = padding + CGFloat(cursorCol) * cw
                let cursorY = rowTopY(Int(cursorRow))
                let cursorRect = CGRect(x: cursorX, y: cursorY, width: cw, height: ch)

                // Use the theme-derived cursor color.
                let cursorColor = self.cursorColor

                switch cursorShapeOverride {
                case 0: // Block
                    context.setFillColor(cursorColor)
                    context.fill(cursorRect)
                    // Redraw the character under the cursor with inverted color.
                    let cell = grid.cell(row: cursorRow, col: cursorCol)
                    if cell.character.value != 0x20 && cell.character.value != 0 {
                        let bgR = cell.bgR, bgG = cell.bgG, bgB = cell.bgB
                        drawTextRun(
                            context: context, text: String(cell.character),
                            col: cursorCol, rowY: cursorY,
                            fgR: bgR, fgG: bgG, fgB: bgB,
                            bold: cell.flags & GridBufferReader.flagBold != 0,
                            italic: cell.flags & GridBufferReader.flagItalic != 0,
                            dim: false
                        )
                    }
                case 1: // Beam
                    context.setFillColor(cursorColor)
                    context.fill(CGRect(x: cursorX, y: cursorY, width: 2, height: ch))
                case 2: // Underline
                    context.setFillColor(cursorColor)
                    context.fill(CGRect(x: cursorX, y: cursorY + ch - 2, width: cw, height: 2))
                case 3: // Hollow block
                    context.setStrokeColor(cursorColor)
                    context.setLineWidth(1)
                    context.stroke(cursorRect.insetBy(dx: 0.5, dy: 0.5))
                default: // Hidden or unknown
                    break
                }
            }
        }

        // 8. Draw hover underline across the hovered link (OSC 8 or detected URL).
        if hoverIsLink && hoverRow >= 0 && hoverRow < lines
            && hoverLinkEndCol > hoverLinkStartCol {
            let yBase = rowTopY(hoverRow) + ch - 1
            let x1 = padding + CGFloat(hoverLinkStartCol) * cw
            let x2 = padding + CGFloat(hoverLinkEndCol) * cw
            context.setStrokeColor(cursorColor)
            context.setLineWidth(1)
            context.move(to: CGPoint(x: x1, y: yBase))
            context.addLine(to: CGPoint(x: x2, y: yBase))
            context.strokePath()
        }

        // 9. Draw IME marked text overlay at the cursor position.
        if !markedText.isEmpty {
            let cursorRow = grid.cursorRow
            let cursorCol = grid.cursorCol
            let startX = padding + CGFloat(cursorCol) * cw
            let startY = rowTopY(Int(cursorRow))

            // Background for the marked text run.
            let markedWidth = cw * CGFloat(max(1, markedText.count))
            context.setFillColor(CGColor(srgbRed: 0.15, green: 0.15, blue: 0.2, alpha: 1.0))
            context.fill(CGRect(x: startX, y: startY, width: markedWidth, height: ch))

            // Draw the marked text using the current cursor color as fg.
            var runText = ""
            for char in markedText {
                runText.append(char)
            }
            drawTextRun(
                context: context, text: runText,
                col: cursorCol, rowY: startY,
                fgR: 255, fgG: 255, fgB: 255,
                bold: false, italic: false, dim: false
            )

            // Underline the marked text to indicate active composition.
            context.setStrokeColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.8))
            context.setLineWidth(1)
            let underlineY = startY + ch - 1
            context.move(to: CGPoint(x: startX, y: underlineY))
            context.addLine(to: CGPoint(x: startX + markedWidth, y: underlineY))
            context.strokePath()
        }
    }

    // MARK: Color and Background Caches

    private func cachedColor(
        red: UInt8,
        green: UInt8,
        blue: UInt8,
        alpha: CGFloat = 1.0
    ) -> CGColor {
        let alphaByte = UInt8(clamping: Int((alpha * 255).rounded()))
        let key = colorKey(red: red, green: green, blue: blue, alphaByte: alphaByte)

        if let cached = colorCache[key] {
            return cached
        }

        if colorCache.count > 4096 {
            // Evict half rather than clearing — a full clear forces every
            // color on screen to be re-allocated on the next frame.
            for staleKey in Array(colorCache.keys.prefix(colorCache.count / 2)) {
                colorCache.removeValue(forKey: staleKey)
            }
        }

        let color = CGColor(
            srgbRed: CGFloat(red) / 255.0,
            green: CGFloat(green) / 255.0,
            blue: CGFloat(blue) / 255.0,
            alpha: CGFloat(alphaByte) / 255.0
        )
        colorCache[key] = color
        return color
    }

    private func colorKey(
        red: UInt8,
        green: UInt8,
        blue: UInt8,
        alphaByte: UInt8
    ) -> UInt32 {
        (UInt32(alphaByte) << 24)
            | (UInt32(red) << 16)
            | (UInt32(green) << 8)
            | UInt32(blue)
    }

    /// Draw contiguous same-color background cells as row runs instead of one
    /// fill per cell. This preserves the existing coordinate system and only
    /// reduces fill calls for full-screen TUIs with large color blocks.
    private func drawCellBackgrounds(
        context: CGContext,
        grid: GridBufferReader,
        cols: Int,
        rows: Range<Int>,
        cellWidth: CGFloat,
        cellHeight: CGFloat
    ) {
        for row in rows {
            if frameCollapsedRows.contains(row) { continue }
            let rowY = rowTopY(row)
            var runStartCol: Int?
            var runEndCol = 0
            var runR: UInt8 = 0
            var runG: UInt8 = 0
            var runB: UInt8 = 0

            func flushRun() {
                guard let startCol = runStartCol, runEndCol > startCol else { return }
                let rect = CGRect(
                    x: padding + CGFloat(startCol) * cellWidth,
                    y: rowY,
                    width: CGFloat(runEndCol - startCol) * cellWidth,
                    height: cellHeight
                )
                context.setFillColor(cachedColor(red: runR, green: runG, blue: runB))
                context.fill(rect)
                runStartCol = nil
            }

            var col = 0
            while col < cols {
                let cell = grid.cell(row: row, col: col)
                let flags = cell.flags

                if flags & GridBufferReader.flagWideCharSpacer != 0 {
                    flushRun()
                    col += 1
                    continue
                }

                var bgR = cell.bgR
                var bgG = cell.bgG
                var bgB = cell.bgB
                var fgR = cell.fgR
                var fgG = cell.fgG
                var fgB = cell.fgB

                if flags & GridBufferReader.flagInverse != 0 {
                    swap(&bgR, &fgR)
                    swap(&bgG, &fgG)
                    swap(&bgB, &fgB)
                }

                let span = (flags & GridBufferReader.flagWideChar != 0) ? 2 : 1
                let endCol = min(cols, col + span)
                let drawsBackground = bgR != defaultBackgroundRgb.0
                    || bgG != defaultBackgroundRgb.1
                    || bgB != defaultBackgroundRgb.2

                guard drawsBackground else {
                    flushRun()
                    col = endCol
                    continue
                }

                if runStartCol != nil,
                   runEndCol == col,
                   runR == bgR,
                   runG == bgG,
                   runB == bgB {
                    runEndCol = endCol
                } else {
                    flushRun()
                    runStartCol = col
                    runEndCol = endCol
                    runR = bgR
                    runG = bgG
                    runB = bgB
                }

                col = endCol
            }

            flushRun()
        }
    }

    // MARK: Block Decoration Drawing

    /// Translucent row washes under the text: failure tint, navigation
    /// highlight, and the live input-prompt region. Restricted to damaged
    /// rows so partial repaints never double-tint a row.
    private func drawBlockWashes(
        context: CGContext, overlay: TerminalBlockOverlay, drawRows: Range<Int>, lines: Int
    ) {
        let ch = fontMetrics.cellHeight
        let fullWidth = bounds.width

        func fillRows(_ start: Int, _ end: Int, color: CGColor) {
            let clampedStart = max(start, 0)
            let clampedEnd = min(end, lines - 1)
            guard clampedEnd >= clampedStart else { return }
            context.setFillColor(color)
            for row in clampedStart...clampedEnd
            where drawRows.contains(row) && !frameCollapsedRows.contains(row) {
                context.fill(
                    CGRect(x: 0, y: rowTopY(row), width: fullWidth, height: ch)
                )
            }
        }

        for block in overlay.blocks {
            // Blank prompt padding is collapsed away, so the real prompt row
            // already abuts the previous block — wash from there.
            let start = Int(block.startRow)
            let isHighlighted = block.id == highlightedBlockId
            let isHovered = block.id == hoveredBlockId
            if isHovered, !isHighlighted,
               let hover = blockPromptFillColor.copy(alpha: 0.05) {
                fillRows(start, Int(block.endRow), color: hover)
            }
            guard block.failed || isHighlighted else { continue }
            let base = isHighlighted ? blockAccentColor : blockFailedColor
            guard let wash = base.copy(alpha: isHighlighted ? 0.09 : 0.07) else { continue }
            fillRows(start, Int(block.endRow), color: wash)
        }
    }

    /// Hairline separators between blocks, left-edge status stripes
    /// (Warp's "flag pole"), and right-aligned exit/duration chips.
    private func drawBlockDecorations(
        context: CGContext, overlay: TerminalBlockOverlay, drawRows: Range<Int>, lines: Int
    ) {
        let ch = fontMetrics.cellHeight
        let fullWidth = bounds.width

        for block in overlay.blocks {
            // The blank prompt padding is collapsed away, so the block's real
            // prompt row already sits directly below the previous block — draw
            // the separator there as the border between the two blocks.
            let startRow = Int(block.startRow)
            let endRow = min(Int(block.endRow), lines - 1)

            // Separator along the block's top edge (skip the viewport edge).
            if startRow > 0, startRow < lines, drawRows.contains(startRow),
               !frameCollapsedRows.contains(startRow) {
                let y = blockBorderY(startRow).rounded() - 0.5
                context.setStrokeColor(blockSeparatorColor)
                context.setLineWidth(1)
                context.move(to: CGPoint(x: padding, y: y))
                context.addLine(to: CGPoint(x: fullWidth - padding, y: y))
                context.strokePath()
            }

            // Left-edge stripe for failed, running, and highlighted blocks,
            // drawn in the padding gutter so it never covers glyphs.
            if block.failed || block.isRunning || block.id == highlightedBlockId {
                let color = block.failed ? blockFailedColor : blockAccentColor
                context.setFillColor(color)
                let visibleStart = max(startRow, 0)
                if endRow >= visibleStart {
                    for row in visibleStart...endRow
                    where drawRows.contains(row) && !frameCollapsedRows.contains(row) {
                        context.fill(
                            CGRect(x: 3, y: rowTopY(row), width: 3, height: ch)
                        )
                    }
                }
            }

        }

        // Warp-style sticky header: when a block extends above the viewport,
        // pin its command along the top edge so you always know whose output
        // you're reading.
        if drawRows.contains(0),
           let pinned = overlay.blocks.first(where: { $0.startRow < 0 && $0.endRow >= 1 }),
           let command = pinned.command, !command.isEmpty {
            drawStickyBlockHeader(context: context, block: pinned, command: command)
        }

        // Hairline above the live prompt region, separating it from the last
        // block's output.
        if let promptRow = overlay.promptRow.map(Int.init),
           promptRow > 0, promptRow < lines, drawRows.contains(promptRow),
           !frameCollapsedRows.contains(promptRow) {
            let y = blockBorderY(promptRow).rounded() - 0.5
            context.setStrokeColor(blockSeparatorColor)
            context.setLineWidth(1)
            context.move(to: CGPoint(x: padding, y: y))
            context.addLine(to: CGPoint(x: fullWidth - padding, y: y))
            context.strokePath()
        }

        // Warp-style hover toolbar at the hovered block's top-right.
        if let hoveredBlockId,
           let block = overlay.blocks.first(where: { $0.id == hoveredBlockId }) {
            drawBlockToolbar(context: context, block: block, lines: lines)
        }
    }

    /// Floating action toolbar at a hovered block's top-right: a quick
    /// copy-output button and a "⋯" options menu (Warp-style).
    private func drawBlockToolbar(
        context: CGContext, block: TerminalBlockOverlayRegion, lines: Int
    ) {
        let ch = fontMetrics.cellHeight
        // Pin to the block's first visible row (top edge when scrolled past).
        let row = max(Int(block.startRow), 0)
        guard row < lines else { return }

        let buttonW: CGFloat = 26
        let inset: CGFloat = 4
        let buttons: [BlockToolbarButton] = [.copyOutput, .menu]
        let toolbarH = ch
        let toolbarW = inset * 2 + buttonW * CGFloat(buttons.count)
        let x = bounds.width - padding - toolbarW
        // Nudge the toolbar down from the block's top edge so it floats clear
        // of the separator and reads as part of the command row.
        let y = rowTopY(row) + ch * 0.55
        let toolbarRect = CGRect(x: x, y: y, width: toolbarW, height: toolbarH)
        let radius = min(toolbarH / 2, 7)

        // Pill background + border.
        let pill = CGPath(
            roundedRect: toolbarRect, cornerWidth: radius, cornerHeight: radius, transform: nil)
        if let fill = defaultBackgroundColor.copy(alpha: 0.96) {
            context.setFillColor(fill)
            context.addPath(pill)
            context.fillPath()
        }
        context.setStrokeColor(blockSeparatorColor)
        context.setLineWidth(1)
        context.addPath(pill)
        context.strokePath()

        for (index, button) in buttons.enumerated() {
            let bx = x + inset + CGFloat(index) * buttonW
            let buttonRect = CGRect(x: bx, y: y, width: buttonW, height: toolbarH)

            if hoveredToolbarButton == button {
                let hl = CGRect(x: bx + 1, y: y + 2, width: buttonW - 2, height: toolbarH - 4)
                context.setFillColor(blockMutedTextColor.copy(alpha: 0.15) ?? blockMutedTextColor)
                context.addPath(
                    CGPath(roundedRect: hl, cornerWidth: 4, cornerHeight: 4, transform: nil))
                context.fillPath()
            }

            let iconColor =
                hoveredToolbarButton == button
                ? (blockMutedTextColor.copy(alpha: 1.0) ?? blockMutedTextColor)
                : (blockMutedTextColor.copy(alpha: 0.8) ?? blockMutedTextColor)
            let iconBox = CGRect(x: bx, y: y, width: buttonW, height: toolbarH)
                .insetBy(dx: buttonW / 2 - 6, dy: toolbarH / 2 - 6)
            switch button {
            case .copyOutput:
                drawCopyGlyph(context: context, in: iconBox, color: iconColor)
            case .menu:
                drawKebabGlyph(context: context, in: iconBox, color: iconColor)
            }

            hoverToolbarTargets.append(
                BlockToolbarTarget(
                    rect: buttonRect.offsetBy(dx: 0, dy: contentYOffset),
                    button: button,
                    blockId: block.id))
        }
    }

    /// Two overlapping rounded rectangles — the universal "copy" glyph.
    private func drawCopyGlyph(context: CGContext, in box: CGRect, color: CGColor) {
        let w = box.width * 0.66
        let h = box.height * 0.78
        let front = CGRect(x: box.maxX - w, y: box.maxY - h, width: w, height: h)
        let back = CGRect(x: box.minX, y: box.minY, width: w, height: h)
        context.setStrokeColor(color)
        context.setLineWidth(1.3)
        // Back sheet.
        context.addPath(CGPath(roundedRect: back, cornerWidth: 2, cornerHeight: 2, transform: nil))
        context.strokePath()
        // Front sheet, filled with the background so it visually overlaps.
        context.setFillColor(defaultBackgroundColor)
        let frontPath = CGPath(roundedRect: front, cornerWidth: 2, cornerHeight: 2, transform: nil)
        context.addPath(frontPath)
        context.fillPath()
        context.setStrokeColor(color)
        context.addPath(frontPath)
        context.strokePath()
    }

    /// Three vertical dots — the "more options" kebab glyph.
    private func drawKebabGlyph(context: CGContext, in box: CGRect, color: CGColor) {
        let dotSize: CGFloat = 2.2
        let cx = box.midX - dotSize / 2
        let spacing = (box.height - dotSize) / 2
        context.setFillColor(color)
        for i in 0..<3 {
            let dot = CGRect(
                x: cx, y: box.minY + CGFloat(i) * spacing, width: dotSize, height: dotSize)
            context.fillEllipse(in: dot)
        }
    }

    /// Build the full terminal context menu — the same menu shown by
    /// right-click and by the hover toolbar's "⋯" button. When `blockId` is
    /// set, the per-block actions (copy command/output, re-run) are included
    /// at the top, scoped to that block.
    private func buildTerminalContextMenu(blockId: UInt64?) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false

        if let blockId,
           let block = backend?.blockOverlay()?.blocks.first(where: { $0.id == blockId }) {
            let hasCommand = !(block.command?.isEmpty ?? true)

            let copyCommand = NSMenuItem(
                title: "Copy Command", action: #selector(contextCopyBlockCommand(_:)),
                keyEquivalent: "")
            copyCommand.target = self
            copyCommand.isEnabled = hasCommand
            menu.addItem(copyCommand)

            let copyOutput = NSMenuItem(
                title: "Copy Output", action: #selector(contextCopyBlockOutput(_:)),
                keyEquivalent: "")
            copyOutput.target = self
            menu.addItem(copyOutput)

            let copyBoth = NSMenuItem(
                title: "Copy Command & Output", action: #selector(contextCopyBlockBoth(_:)),
                keyEquivalent: "")
            copyBoth.target = self
            menu.addItem(copyBoth)

            let rerun = NSMenuItem(
                title: "Re-run Command", action: #selector(contextRerunBlock(_:)), keyEquivalent: "")
            rerun.target = self
            rerun.isEnabled = hasCommand && !block.isRunning
            menu.addItem(rerun)

            menu.addItem(.separator())
        }

        let commandFlags = backend?.commandBlockFlags()
            ?? TerminalCommandBlockFlags(hasCommand: false, hasOutput: false, hasFailed: false)
        let hasCommand = commandFlags.hasCommand
        let hasOutput = commandFlags.hasOutput
        let hasBlock = hasCommand || hasOutput
        let hasFailedBlock = commandFlags.hasFailed
        let hasHistory = !(backend?.commandHistorySearch(text: "", cwd: nil, limit: 1).isEmpty ?? true)

        let copyItem = NSMenuItem(
            title: "Copy", action: #selector(contextCopy(_:)), keyEquivalent: "")
        copyItem.target = self
        copyItem.isEnabled = backend?.selectedText() != nil
        menu.addItem(copyItem)

        let pasteItem = NSMenuItem(
            title: "Paste", action: #selector(contextPaste(_:)), keyEquivalent: "")
        pasteItem.target = self
        let pasteboard = NSPasteboard.general
        pasteItem.isEnabled = pasteboard.string(forType: .string) != nil
            || pasteboard.canReadObject(forClasses: [NSImage.self], options: nil)
        menu.addItem(pasteItem)

        menu.addItem(.separator())

        let copyCommandItem = NSMenuItem(
            title: "Copy Last Command", action: #selector(contextCopyLastCommand(_:)),
            keyEquivalent: "")
        copyCommandItem.target = self
        copyCommandItem.isEnabled = hasCommand
        menu.addItem(copyCommandItem)

        let copyOutputItem = NSMenuItem(
            title: "Copy Last Command Output", action: #selector(contextCopyLastCommandOutput(_:)),
            keyEquivalent: "")
        copyOutputItem.target = self
        copyOutputItem.isEnabled = hasOutput
        menu.addItem(copyOutputItem)

        let rerunItem = NSMenuItem(
            title: "Rerun Last Command", action: #selector(contextRerunLastCommand(_:)),
            keyEquivalent: "")
        rerunItem.target = self
        rerunItem.isEnabled = hasCommand
        menu.addItem(rerunItem)

        let historyItem = NSMenuItem(
            title: "Command History...", action: #selector(contextCommandHistory(_:)),
            keyEquivalent: "")
        historyItem.target = self
        historyItem.isEnabled = hasHistory
        menu.addItem(historyItem)

        menu.addItem(.separator())

        let previousBlockItem = NSMenuItem(
            title: "Previous Command Block", action: #selector(contextPreviousCommandBlock(_:)),
            keyEquivalent: "")
        previousBlockItem.target = self
        previousBlockItem.isEnabled = hasBlock
        menu.addItem(previousBlockItem)

        let nextBlockItem = NSMenuItem(
            title: "Next Command Block", action: #selector(contextNextCommandBlock(_:)),
            keyEquivalent: "")
        nextBlockItem.target = self
        nextBlockItem.isEnabled = hasBlock
        menu.addItem(nextBlockItem)

        let failedBlockItem = NSMenuItem(
            title: "Last Failed Command", action: #selector(contextLastFailedCommandBlock(_:)),
            keyEquivalent: "")
        failedBlockItem.target = self
        failedBlockItem.isEnabled = hasFailedBlock
        menu.addItem(failedBlockItem)

        menu.addItem(.separator())

        let selectAllItem = NSMenuItem(
            title: "Select All", action: #selector(contextSelectAll(_:)), keyEquivalent: "")
        selectAllItem.target = self
        menu.addItem(selectAllItem)

        let clearItem = NSMenuItem(
            title: "Clear", action: #selector(contextClear(_:)), keyEquivalent: "")
        clearItem.target = self
        menu.addItem(clearItem)

        return menu
    }

    /// "✓ · 1.2s" / "✗ 1 · 3.4s" summary for a completed block.
    static func blockChipText(exitCode: Int32?, durationMs: UInt64?) -> String? {
        guard let exitCode else { return nil }
        let mark = exitCode == 0 ? "✓" : "✗ \(exitCode)"
        guard let durationMs else { return mark }
        return "\(mark) · \(formatBlockDuration(durationMs))"
    }

    static func formatBlockDuration(_ ms: UInt64) -> String {
        if ms < 1000 { return "\(ms)ms" }
        let seconds = Double(ms) / 1000.0
        if seconds < 60 { return String(format: "%.1fs", seconds) }
        let minutes = Int(seconds) / 60
        if minutes < 60 { return "\(minutes)m \(Int(seconds) % 60)s" }
        return "\(minutes / 60)h \(minutes % 60)m"
    }


    /// Pinned command bar along the top edge for the block being scrolled.
    private func drawStickyBlockHeader(
        context: CGContext, block: TerminalBlockOverlayRegion, command: String
    ) {
        let ch = fontMetrics.cellHeight
        let barHeight = ch + 6
        let barRect = CGRect(x: 0, y: 0, width: bounds.width, height: barHeight)

        context.setFillColor(defaultBackgroundColor)
        context.fill(barRect)
        context.setStrokeColor(blockSeparatorColor)
        context.setLineWidth(1)
        context.move(to: CGPoint(x: 0, y: barHeight - 0.5))
        context.addLine(to: CGPoint(x: bounds.width, y: barHeight - 0.5))
        context.strokePath()

        if block.failed || block.isRunning {
            context.setFillColor(block.failed ? blockFailedColor : blockAccentColor)
            context.fill(CGRect(x: 3, y: 0, width: 3, height: barHeight))
        }

        // Right-aligned status text.
        var statusWidth: CGFloat = 0
        if let status = Self.blockChipText(exitCode: block.exitCode, durationMs: block.durationMs) {
            let color = block.failed ? blockFailedColor : blockMutedTextColor
            let attrs: [CFString: Any] = [
                kCTFontAttributeName: chipFont ?? fontMetrics.font,
                kCTForegroundColorAttributeName: color,
            ]
            if let attrStr = CFAttributedStringCreate(nil, status as CFString, attrs as CFDictionary) {
                let line = CTLineCreateWithAttributedString(attrStr)
                statusWidth = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
                context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
                context.textPosition = CGPoint(
                    x: bounds.width - padding - statusWidth, y: 3 + fontMetrics.ascent
                )
                CTLineDraw(line, context)
            }
        }

        // "❯ command", truncated to the space left of the status text.
        let attrs: [CFString: Any] = [
            kCTFontAttributeName: fontMetrics.font,
            kCTForegroundColorAttributeName: blockMutedTextColor,
        ]
        guard
            let attrStr = CFAttributedStringCreate(
                nil, "❯ \(command)" as CFString, attrs as CFDictionary)
        else { return }
        let line = CTLineCreateWithAttributedString(attrStr)
        let maxWidth = Double(bounds.width - padding * 2 - statusWidth - 16)
        let drawn = CTLineCreateTruncatedLine(line, maxWidth, .end, nil) ?? line
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.textPosition = CGPoint(x: padding, y: 3 + fontMetrics.ascent)
        CTLineDraw(drawn, context)
    }

    // MARK: Text Run Drawing

    /// Draw a run of text at the given grid position using CoreText.
    /// Each glyph is forced to advance by exactly cellWidth so text aligns
    /// perfectly with the grid (cursor, selection, background fills).
    private func drawTextRun(
        context: CGContext, text: String, col: Int, rowY: CGFloat,
        fgR: UInt8, fgG: UInt8, fgB: UInt8,
        bold: Bool, italic: Bool, dim: Bool
    ) {
        let alpha: CGFloat = dim ? 0.5 : 1.0
        let alphaByte = UInt8(clamping: Int((alpha * 255).rounded()))
        let colorKey = colorKey(red: fgR, green: fgG, blue: fgB, alphaByte: alphaByte)
        let color = cachedColor(red: fgR, green: fgG, blue: fgB, alpha: alpha)

        let font = fontForStyle(bold: bold, italic: italic)
        let styleKey = fontStyleKey(bold: bold, italic: italic)
        let cw = fontMetrics.cellWidth
        let baseX = padding + CGFloat(col) * cw
        let textY = rowY + fontMetrics.ascent

        // Draw each character at its exact cell position to prevent drift.
        // CoreText's natural advances may differ slightly from cellWidth,
        // causing characters to misalign with the grid over long runs.
        var charCol = 0
        for ch in text.unicodeScalars {
            let line = cachedTextLine(
                scalar: ch,
                font: font,
                color: color,
                styleKey: styleKey,
                colorKey: colorKey
            )
            context.textPosition = CGPoint(x: baseX + CGFloat(charCol) * cw, y: textY)
            CTLineDraw(line, context)
            charCol += 1
        }
    }

    private func cachedTextLine(
        scalar: UnicodeScalar,
        font: CTFont,
        color: CGColor,
        styleKey: UInt64,
        colorKey: UInt32
    ) -> CTLine {
        let lineKey = (UInt64(colorKey) << 32)
            | (styleKey << 24)
            | UInt64(scalar.value)

        if let line = textLineCache[lineKey] {
            return line
        }

        if textLineCache.count > 8192 {
            // Evict half rather than clearing — a full clear forces a CoreText
            // re-layout of every visible glyph on the next frame.
            for staleKey in Array(textLineCache.keys.prefix(textLineCache.count / 2)) {
                textLineCache.removeValue(forKey: staleKey)
            }
        }

        let attrs: [CFString: Any] = [
            kCTFontAttributeName: font,
            kCTForegroundColorAttributeName: color,
        ]
        let attrStr = CFAttributedStringCreate(nil, String(scalar) as CFString, attrs as CFDictionary)!
        let line = CTLineCreateWithAttributedString(attrStr)
        textLineCache[lineKey] = line
        return line
    }

    private func fontStyleKey(bold: Bool, italic: Bool) -> UInt64 {
        switch (bold, italic) {
        case (false, false): return 0
        case (true, false): return 1
        case (false, true): return 2
        case (true, true): return 3
        }
    }

    // MARK: Font Variant Cache

    private func cacheFontVariants() {
        textLineCache.removeAll(keepingCapacity: true)

        let base = fontMetrics.font
        let size = CTFontGetSize(base)

        boldFont = CTFontCreateCopyWithSymbolicTraits(
            base, size, nil, .boldTrait, .boldTrait
        ) ?? base

        italicFont = CTFontCreateCopyWithSymbolicTraits(
            base, size, nil, .italicTrait, .italicTrait
        ) ?? base

        boldItalicFont = CTFontCreateCopyWithSymbolicTraits(
            base, size, nil, [.boldTrait, .italicTrait], [.boldTrait, .italicTrait]
        ) ?? base

        chipFont = CTFontCreateCopyWithAttributes(base, (size * 0.85).rounded(), nil, nil)
    }

    private func fontForStyle(bold: Bool, italic: Bool) -> CTFont {
        switch (bold, italic) {
        case (true, true): return boldItalicFont ?? fontMetrics.font
        case (true, false): return boldFont ?? fontMetrics.font
        case (false, true): return italicFont ?? fontMetrics.font
        case (false, false): return fontMetrics.font
        }
    }

    // MARK: Box Drawing

    /// Draw a box-drawing or block element character using CGContext paths.
    private func drawBoxDrawing(
        context: CGContext, codepoint: UInt32,
        x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat,
        fgR: UInt8, fgG: UInt8, fgB: UInt8, dim: Bool
    ) {
        let alpha: CGFloat = dim ? 0.5 : 1.0
        let color = cachedColor(red: fgR, green: fgG, blue: fgB, alpha: alpha)

        // Codex/Claude TUIs use box-drawing heavily. Snap all geometry to the
        // backing pixel grid and disable antialiasing so separators do not
        // smear color into adjacent rows/columns on macOS.
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2.0
        func snap(_ value: CGFloat) -> CGFloat {
            (value * scale).rounded() / scale
        }
        func snappedRect(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) -> CGRect {
            let minX = snap(x)
            let minY = snap(y)
            let maxX = snap(x + width)
            let maxY = snap(y + height)
            return CGRect(x: minX, y: minY, width: max(0, maxX - minX), height: max(0, maxY - minY))
        }

        let midX = snap(x + width / 2)
        let midY = snap(y + height / 2)
        let thinWidth: CGFloat = 1.0
        let thickWidth: CGFloat = 2.0

        // Fills below are pixel-snapped, so disabling antialiasing keeps
        // straight separators from bleeding into adjacent rows/columns. Arc
        // cases re-enable it locally so rounded corners stay smooth.
        context.saveGState()
        context.setShouldAntialias(false)
        defer { context.restoreGState() }

        switch codepoint {
        // Horizontal lines.
        case 0x2500: // ─ light horizontal
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: width, height: thinWidth))

        case 0x2501: // ━ heavy horizontal
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - thickWidth / 2, width: width, height: thickWidth))

        // Vertical lines.
        case 0x2502: // │ light vertical
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: height))

        case 0x2503: // ┃ heavy vertical
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thickWidth / 2, y: y, width: thickWidth, height: height))

        // Light corners.
        case 0x250C: // ┌ down and right
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: midY, width: thinWidth, height: height - (midY - y)))
            context.fill(snappedRect(x: midX, y: midY - thinWidth / 2, width: width - (midX - x), height: thinWidth))

        case 0x2510: // ┐ down and left
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: midY, width: thinWidth, height: height - (midY - y)))
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: midX - x, height: thinWidth))

        case 0x2514: // └ up and right
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: midY - y))
            context.fill(snappedRect(x: midX, y: midY - thinWidth / 2, width: width - (midX - x), height: thinWidth))

        case 0x2518: // ┘ up and left
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: midY - y))
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: midX - x, height: thinWidth))

        // T-junctions.
        case 0x251C: // ├ vertical and right
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: height))
            context.fill(snappedRect(x: midX, y: midY - thinWidth / 2, width: width - (midX - x), height: thinWidth))

        case 0x2524: // ┤ vertical and left
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: height))
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: midX - x, height: thinWidth))

        case 0x252C: // ┬ down and horizontal
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: width, height: thinWidth))
            context.fill(snappedRect(x: midX - thinWidth / 2, y: midY, width: thinWidth, height: height - (midY - y)))

        case 0x2534: // ┴ up and horizontal
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: width, height: thinWidth))
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: midY - y))

        // Cross.
        case 0x253C: // ┼ vertical and horizontal
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - thinWidth / 2, width: width, height: thinWidth))
            context.fill(snappedRect(x: midX - thinWidth / 2, y: y, width: thinWidth, height: height))

        // Double lines.
        case 0x2550: // ═ double horizontal
            let gap: CGFloat = 2
            context.setFillColor(color)
            context.fill(snappedRect(x: x, y: midY - gap - thinWidth / 2, width: width, height: thinWidth))
            context.fill(snappedRect(x: x, y: midY + gap - thinWidth / 2, width: width, height: thinWidth))

        case 0x2551: // ║ double vertical
            let gap: CGFloat = 2
            context.setFillColor(color)
            context.fill(snappedRect(x: midX - gap - thinWidth / 2, y: y, width: thinWidth, height: height))
            context.fill(snappedRect(x: midX + gap - thinWidth / 2, y: y, width: thinWidth, height: height))

        // Rounded corners. Arcs need antialiasing to stay smooth.
        case 0x256D: // ╭ arc down and right
            context.setShouldAntialias(true)
            context.setStrokeColor(color)
            context.setLineWidth(thinWidth)
            let radius = min(width, height) / 2
            context.move(to: CGPoint(x: midX, y: y + height))
            context.addLine(to: CGPoint(x: midX, y: midY + radius))
            context.addArc(center: CGPoint(x: midX + radius, y: midY + radius),
                          radius: radius, startAngle: .pi, endAngle: .pi * 1.5, clockwise: false)
            context.addLine(to: CGPoint(x: x + width, y: midY))
            context.strokePath()

        case 0x256E: // ╮ arc down and left
            context.setShouldAntialias(true)
            context.setStrokeColor(color)
            context.setLineWidth(thinWidth)
            let radius = min(width, height) / 2
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX - radius, y: midY))
            context.addArc(center: CGPoint(x: midX - radius, y: midY + radius),
                          radius: radius, startAngle: .pi * 1.5, endAngle: 0, clockwise: false)
            context.addLine(to: CGPoint(x: midX, y: y + height))
            context.strokePath()

        case 0x256F: // ╯ arc up and left
            context.setShouldAntialias(true)
            context.setStrokeColor(color)
            context.setLineWidth(thinWidth)
            let radius = min(width, height) / 2
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: midY - radius))
            context.addArc(center: CGPoint(x: midX - radius, y: midY - radius),
                          radius: radius, startAngle: 0, endAngle: .pi * 0.5, clockwise: false)
            context.addLine(to: CGPoint(x: x, y: midY))
            context.strokePath()

        case 0x2570: // ╰ arc up and right
            context.setShouldAntialias(true)
            context.setStrokeColor(color)
            context.setLineWidth(thinWidth)
            let radius = min(width, height) / 2
            context.move(to: CGPoint(x: x + width, y: midY))
            context.addLine(to: CGPoint(x: midX + radius, y: midY))
            context.addArc(center: CGPoint(x: midX + radius, y: midY - radius),
                          radius: radius, startAngle: .pi * 0.5, endAngle: .pi, clockwise: false)
            context.addLine(to: CGPoint(x: midX, y: y))
            context.strokePath()

        default:
            // Fall back to font glyph for unrecognized box-drawing characters.
            // drawTextRun positions each glyph at an exact cell column so
            // alignment with the grid is preserved regardless of the glyph's
            // natural advance width.
            drawTextRun(
                context: context, text: String(UnicodeScalar(codepoint)!),
                col: Int((x - padding) / width), rowY: y,
                fgR: fgR, fgG: fgG, fgB: fgB,
                bold: false, italic: false, dim: dim
            )
        }
    }

    // MARK: Keyboard Input

    private func eventMatchesKeybinding(_ event: NSEvent, id: String) -> Bool {
        guard let binding = Keybindings.getKeybinding(id: id, overrides: keybindingOverrides) else {
            return false
        }
        return Keybindings.eventMatches(event, keybinding: binding)
    }

    override func keyDown(with event: NSEvent) {
        if eventMatchesKeybinding(event, id: "paste") {
            paste(event)
            return
        }
        if eventMatchesKeybinding(event, id: "copy") {
            copy(event)
            return
        }

        if event.modifierFlags.contains(.command) {
            super.keyDown(with: event)
            return
        }

        // Route through the input manager so dead keys and IME composition
        // work. interpretKeyEvents will call:
        //   - insertText(_:) for committed text (including composed chars)
        //   - setMarkedText(_:...) during composition
        //   - doCommand(by:) for special keys (arrows, enter, tab, etc.)
        // For the doCommand path we need to access the originating event to
        // run it through KeyEncoder, so we stash it temporarily.
        currentKeyEvent = event
        interpretKeyEvents([event])
        currentKeyEvent = nil
    }

    override func doCommand(by selector: Selector) {
        // Special keys fall back to KeyEncoder using the current keyDown event.
        guard let event = currentKeyEvent, let backend else { return }
        let mode = backend.mode()
        let bytes = KeyEncoder.encode(
            event: event,
            appCursor: mode?.appCursor ?? false,
            appKeypad: mode?.appKeypad ?? false
        )
        if !bytes.isEmpty {
            if isScrolledBack {
                isScrolledBack = false
                backend.scrollToBottom()
            }
            backend.write(bytes: bytes)
            resetBlink()
        }
    }

    // Prevent the system beep for unhandled key events.
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // performKeyEquivalent is offered to every view in the key window, not
        // just the first responder. Only claim terminal copy/paste when this
        // grid actually has focus — otherwise we'd steal Cmd+V/Cmd+C from a
        // focused text field (e.g. the command input bar).
        if window?.firstResponder === self {
            if eventMatchesKeybinding(event, id: "paste") {
                paste(event)
                return true
            }
            if eventMatchesKeybinding(event, id: "copy") {
                copy(event)
                return true
            }
        }

        if event.modifierFlags.contains(.command) {
            return super.performKeyEquivalent(with: event)
        }
        return false
    }

    @objc func paste(_ sender: Any?) {
        window?.makeFirstResponder(self)
        onPaste?()
    }

    @objc func copy(_ sender: Any?) {
        onCopy?()
    }

    override func selectAll(_ sender: Any?) {
        selectVisibleGrid()
        needsDisplay = true
    }

    private func selectVisibleGrid() {
        guard let grid = backend?.gridSnapshot() else { return }
        let lastCol = max(0, grid.cols - 1)
        let lastRow = max(0, grid.lines - 1)
        backend?.startSelection(col: 0, row: 0, kind: 1)
        backend?.updateSelection(col: UInt16(lastCol), row: UInt16(lastRow))
    }

    // MARK: Mouse Tracking (hover + hyperlinks)

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea {
            removeTrackingArea(existing)
        }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseMoved(with event: NSEvent) {
        updateHoveredBlock(with: event)

        // Highlight the hover-toolbar button under the pointer.
        let toolbarPoint = convert(event.locationInWindow, from: nil)
        hoveredToolbarButton = hoverToolbarTargets.first {
            $0.rect.contains(toolbarPoint)
        }?.button

        guard allowHyperlinks else {
            clearHoverLink()
            return
        }

        let (col, row) = gridPoint(from: event)
        let colI = Int(col)
        let rowI = Int(row)
        if colI == hoverCol && rowI == hoverRow { return }
        hoverCol = colI
        hoverRow = rowI

        // Check if the cell under the cursor is a hyperlink.
        let wasLink = hoverIsLink
        hoverIsLink = false
        hoverLinkUri = nil
        hoverLinkStartCol = 0
        hoverLinkEndCol = 0

        if let grid = backend?.gridSnapshot(),
           rowI < grid.lines && colI < grid.cols {
            let cell = grid.cell(row: rowI, col: colI)

            // 1. OSC 8 hyperlink takes priority.
            if cell.flags & GridBufferReader.flagHyperlink != 0 {
                hoverIsLink = true
                // Expand to full contiguous hyperlink run on this row.
                var s = colI
                while s > 0 {
                    let c = grid.cell(row: rowI, col: s - 1)
                    if c.flags & GridBufferReader.flagHyperlink == 0 { break }
                    s -= 1
                }
                var e = colI
                while e < grid.cols - 1 {
                    let c = grid.cell(row: rowI, col: e + 1)
                    if c.flags & GridBufferReader.flagHyperlink == 0 { break }
                    e += 1
                }
                hoverLinkStartCol = s
                hoverLinkEndCol = e + 1
                hoverLinkUri = backend?.hyperlinkAt(col: colI, row: rowI)
            } else if let (uri, s, e) = detectUrlAt(col: colI, row: rowI, grid: grid) {
                // 2. Auto-detected plain URL in the row text.
                hoverIsLink = true
                hoverLinkUri = uri
                hoverLinkStartCol = s
                hoverLinkEndCol = e
            }
        }

        if hoverIsLink != wasLink {
            needsDisplay = true
            window?.invalidateCursorRects(for: self)
        }
    }

    /// Track which command block the pointer is over (Warp-style hover wash).
    private func updateHoveredBlock(with event: NSEvent) {
        guard blocksEnabled else {
            hoveredBlockId = nil
            return
        }
        let (_, row) = gridPoint(from: event)
        let rowI = Int32(row)
        guard let overlay = backend?.blockOverlay(), !overlay.altScreen else {
            hoveredBlockId = nil
            return
        }
        // The live prompt region is not a block; don't highlight it.
        if let promptRow = overlay.promptRow, rowI >= promptRow {
            hoveredBlockId = nil
            return
        }
        hoveredBlockId = overlay.blocks.first { block in
            rowI >= block.startRow && rowI <= block.endRow
        }?.id
    }

    override func mouseExited(with event: NSEvent) {
        hoveredBlockId = nil
        hoveredToolbarButton = nil
        clearHoverLink()
    }

    private func clearHoverLink() {
        if hoverIsLink {
            hoverIsLink = false
            hoverLinkUri = nil
            needsDisplay = true
            window?.invalidateCursorRects(for: self)
        }
        hoverCol = -1
        hoverRow = -1
        hoverLinkStartCol = 0
        hoverLinkEndCol = 0
    }

    /// Scan the row's text for a URL pattern and return the match that
    /// contains the given column, along with the start/end column range.
    private func detectUrlAt(
        col: Int, row: Int, grid: GridBufferReader
    ) -> (uri: String, startCol: Int, endCol: Int)? {
        guard let regex = Self.urlRegex else { return nil }

        // Build the row's text, tracking column for each character.
        // Wide-char spacers are skipped so indices map 1:1 with cell columns.
        var text = ""
        var colForIndex: [Int] = []  // index in `text` → grid column
        for c in 0..<grid.cols {
            let cell = grid.cell(row: row, col: c)
            if cell.flags & GridBufferReader.flagWideCharSpacer != 0 { continue }
            let ch = cell.character.value
            // Treat NUL as space for the scan.
            let scalar = (ch == 0) ? UnicodeScalar(0x20)! : cell.character
            colForIndex.append(c)
            text.append(Character(scalar))
        }

        let nsText = text as NSString
        let matches = regex.matches(
            in: text,
            options: [],
            range: NSRange(location: 0, length: nsText.length)
        )
        for match in matches {
            var range = match.range
            // Trim common trailing punctuation.
            while range.length > 0 {
                let lastIdx = range.location + range.length - 1
                let ch = nsText.character(at: lastIdx)
                if ch == 0x2E || ch == 0x2C || ch == 0x3B || ch == 0x3A
                    || ch == 0x21 || ch == 0x3F || ch == 0x29 || ch == 0x5D {
                    range.length -= 1
                } else {
                    break
                }
            }
            if range.length == 0 { continue }
            // Does this match contain the hovered column?
            let lastIdx = range.location + range.length - 1
            guard range.location < colForIndex.count, lastIdx < colForIndex.count else { continue }
            let startCol = colForIndex[range.location]
            let endCol = colForIndex[lastIdx] + 1
            if col >= startCol && col < endCol {
                let uri = nsText.substring(with: range)
                return (uri, startCol, endCol)
            }
        }
        return nil
    }

    override func resetCursorRects() {
        if hoverIsLink {
            // Whole view uses pointing-hand cursor while hovering a link.
            addCursorRect(bounds, cursor: .pointingHand)
        }
        for target in hoverToolbarTargets {
            addCursorRect(target.rect, cursor: .pointingHand)
        }
    }

    // MARK: Mouse Input

    override func mouseDown(with event: NSEvent) {
        // Hover-toolbar buttons take priority over selection.
        let point = convert(event.locationInWindow, from: nil)
        if let target = hoverToolbarTargets.first(where: { $0.rect.contains(point) }) {
            contextBlockId = target.blockId
            switch target.button {
            case .copyOutput:
                onCopyBlockOutput?(target.blockId)
            case .menu:
                highlightedBlockId = target.blockId
                let menu = buildTerminalContextMenu(blockId: target.blockId)
                menu.popUp(
                    positioning: nil,
                    at: NSPoint(x: target.rect.minX, y: target.rect.maxY),
                    in: self)
                highlightedBlockId = nil
            }
            return
        }

        if keyboardInteractive {
            window?.makeFirstResponder(self)
        } else {
            // Read-only grid: selection still works through mouse events, but
            // keyboard focus belongs to the input bar.
            onRequestInputFocus?()
        }

        // Cmd+Click on a hyperlink opens the URL. We first try the OSC 8
        // link at the cell, then fall back to an auto-detected URL in the row.
        if allowHyperlinks && event.modifierFlags.contains(.command) {
            let (col, row) = gridPoint(from: event)
            let colI = Int(col)
            let rowI = Int(row)
            var uri: String? = backend?.hyperlinkAt(col: colI, row: rowI)
            if uri == nil, let grid = backend?.gridSnapshot(),
               let detected = detectUrlAt(col: colI, row: rowI, grid: grid) {
                uri = detected.uri
            }
            if let uri, let url = URL(string: uri) {
                NSWorkspace.shared.open(url)
                return
            }
        }

        if reportMouseEvent(event, button: 0, motion: false, release: false) {
            return
        }
        let (col, row) = gridPoint(from: event)
        let kind: UInt8
        switch event.clickCount {
        case 3...:
            kind = 3
        case 2:
            kind = 2
        default:
            kind = 0
        }
        backend?.clearSelection()
        pendingSelectionAnchor = nil
        shouldCopySelectionOnMouseUp = false

        if kind == 0 {
            isSelecting = false
            pendingSelectionAnchor = (col, row)
        } else {
            isSelecting = true
            shouldCopySelectionOnMouseUp = true
            backend?.startSelection(col: col, row: row, kind: kind)
            backend?.updateSelection(col: col, row: row)
        }
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        if reportMouseEvent(event, button: 0, motion: true, release: false) {
            return
        }
        let (col, row) = gridPoint(from: event)

        if !isSelecting {
            guard let anchor = pendingSelectionAnchor else { return }
            guard anchor.col != col || anchor.row != row else { return }
            isSelecting = true
            pendingSelectionAnchor = nil
            shouldCopySelectionOnMouseUp = true
            backend?.startSelection(col: anchor.col, row: anchor.row, kind: 0)
        }

        backend?.updateSelection(col: col, row: row)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        if reportMouseEvent(event, button: 0, motion: false, release: true) {
            return
        }
        let shouldCopy = shouldCopySelectionOnMouseUp
        isSelecting = false
        pendingSelectionAnchor = nil
        shouldCopySelectionOnMouseUp = false
        if shouldCopy {
            onSelectionFinished?()
        }
    }

    override func otherMouseDown(with event: NSEvent) {
        _ = reportMouseEvent(event, button: 1, motion: false, release: false)
    }

    override func otherMouseDragged(with event: NSEvent) {
        _ = reportMouseEvent(event, button: 1, motion: true, release: false)
    }

    override func otherMouseUp(with event: NSEvent) {
        _ = reportMouseEvent(event, button: 1, motion: false, release: true)
    }

    // MARK: Mouse Reporting

    /// If the terminal has mouse reporting enabled for the given event type,
    /// encode and send the event to the PTY. Returns true if the event was
    /// consumed (so the caller should not also run selection handling).
    ///
    /// `button` is 0=left, 1=middle, 2=right. `motion` is true for drag events.
    /// `release` is true for button-up events.
    private func reportMouseEvent(
        _ event: NSEvent,
        button: Int,
        motion: Bool,
        release: Bool
    ) -> Bool {
        guard let backend, let mode = backend.mode() else { return false }

        // Determine if reporting is enabled for this event type.
        // mouseReportClick: click events only (no motion/drag)
        // mouseDrag: click + drag events
        // mouseMotion: click + all motion events
        let reportsPress = mode.mouseReportClick || mode.mouseDrag || mode.mouseMotion
        guard reportsPress else { return false }
        if motion && !mode.mouseDrag && !mode.mouseMotion {
            return false
        }

        let (col, row) = gridPoint(from: event)

        // Encode modifier flags.
        var cb = button
        if motion { cb += 32 }
        if event.modifierFlags.contains(.shift) { cb += 4 }
        if event.modifierFlags.contains(.option) { cb += 8 }
        if event.modifierFlags.contains(.control) { cb += 16 }

        if mode.mouseSgr {
            // SGR format: CSI < Cb ; Cx ; Cy ; (M|m)
            let suffix = release ? "m" : "M"
            let seq = "\u{1B}[<\(cb);\(Int(col) + 1);\(Int(row) + 1)\(suffix)"
            backend.write(seq)
        } else {
            // Legacy X10 format: CSI M Cb Cx Cy (each byte = value + 32)
            // Release events use button code 3 in X10.
            let x10Button = release ? 3 : cb
            let cbByte = UInt8(clamping: x10Button + 32)
            let cxByte = UInt8(clamping: Int(col) + 1 + 32)
            let cyByte = UInt8(clamping: Int(row) + 1 + 32)
            backend.write(bytes: [0x1B, 0x5B, 0x4D, cbByte, cxByte, cyByte])
        }
        return true
    }

    override func rightMouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)

        // Block under the pointer scopes the per-block actions.
        contextBlockId = nil
        if blocksEnabled, let overlay = backend?.blockOverlay(), !overlay.altScreen {
            let (_, row) = gridPoint(from: event)
            let rowI = Int32(row)
            let inPromptRegion = overlay.promptRow.map { rowI >= $0 } ?? false
            if !inPromptRegion,
               let block = overlay.blocks.first(where: { rowI >= $0.startRow && rowI <= $0.endRow }) {
                contextBlockId = block.id
                highlightedBlockId = block.id
            }
        }

        let menu = buildTerminalContextMenu(blockId: contextBlockId)
        NSMenu.popUpContextMenu(menu, with: event, for: self)
        // popUpContextMenu blocks until dismissal; drop the emphasis wash.
        if contextBlockId != nil {
            highlightedBlockId = nil
        }
    }

    @objc private func contextCopy(_ sender: Any?) {
        onCopy?()
    }

    @objc private func contextPaste(_ sender: Any?) {
        onPaste?()
    }

    @objc private func contextCopyLastCommand(_ sender: Any?) {
        onCopyLastCommand?()
    }

    @objc private func contextCopyBlockCommand(_ sender: Any?) {
        if let contextBlockId { onCopyBlockCommand?(contextBlockId) }
    }

    @objc private func contextCopyBlockOutput(_ sender: Any?) {
        if let contextBlockId { onCopyBlockOutput?(contextBlockId) }
    }

    @objc private func contextCopyBlockBoth(_ sender: Any?) {
        if let contextBlockId { onCopyBlockCommandAndOutput?(contextBlockId) }
    }

    @objc private func contextRerunBlock(_ sender: Any?) {
        if let contextBlockId { onRerunBlock?(contextBlockId) }
    }

    @objc private func contextCopyLastCommandOutput(_ sender: Any?) {
        onCopyLastCommandOutput?()
    }

    @objc private func contextRerunLastCommand(_ sender: Any?) {
        onRerunLastCommand?()
    }

    @objc private func contextCommandHistory(_ sender: Any?) {
        onShowCommandHistory?()
    }

    @objc private func contextPreviousCommandBlock(_ sender: Any?) {
        onJumpToPreviousCommandBlock?()
    }

    @objc private func contextNextCommandBlock(_ sender: Any?) {
        onJumpToNextCommandBlock?()
    }

    @objc private func contextLastFailedCommandBlock(_ sender: Any?) {
        onJumpToLastFailedCommandBlock?()
    }

    @objc private func contextSelectAll(_ sender: Any?) {
        selectAll(sender)
    }

    @objc private func contextClear(_ sender: Any?) {
        // Send Ctrl+L to the shell to clear the screen.
        backend?.write(bytes: [0x0C])
    }

    override func scrollWheel(with event: NSEvent) {
        guard let backend else { return }
        let mode = backend.mode()

        // Direction from precise delta when available; fall back to the
        // coarse delta for legacy mouse wheels that only populate deltaY.
        let direction: CGFloat = event.hasPreciseScrollingDeltas
            ? event.scrollingDeltaY
            : event.deltaY
        let scrollingUp = direction > 0

        let rawDelta: CGFloat
        if event.hasPreciseScrollingDeltas {
            rawDelta = event.scrollingDeltaY / fontMetrics.cellHeight
        } else {
            rawDelta = event.deltaY * 3
        }

        // Mouse reporting: TUIs that opt in get wheel events as SGR mouse
        // codes. Emit one mouse-button event per line of scroll so fast
        // swipes feel proportional.
        if let mode, mode.mouseReportClick || mode.mouseMotion || mode.mouseDrag {
            guard mode.mouseSgr else { return }
            scrollAccumulator += rawDelta
            let lines = Int(scrollAccumulator)
            guard lines != 0 else { return }
            scrollAccumulator -= CGFloat(lines)
            let (col, row) = gridPoint(from: event)
            let button = scrollingUp ? 64 : 65
            let oneEvent = "\u{1B}[<\(button);\(col + 1);\(row + 1)M"
            let count = min(abs(lines), 20)
            backend.write(String(repeating: oneEvent, count: count))
            return
        }

        scrollAccumulator += rawDelta

        // Alt-screen TUIs without mouse reporting: translate to PageUp/
        // PageDown. Arrow keys get treated as input-history navigation by
        // apps like Claude Code, so they hide the transcript instead of
        // scrolling. One page key per ~4 lines of scroll.
        if let mode, mode.altScreen {
            let pageUnit: CGFloat = 4
            let pages = Int(scrollAccumulator / pageUnit)
            guard pages != 0 else { return }
            scrollAccumulator -= CGFloat(pages) * pageUnit
            let seq = pages > 0 ? "\u{1B}[5~" : "\u{1B}[6~"
            let count = min(abs(pages), 10)
            backend.write(String(repeating: seq, count: count))
            return
        }

        var lines = Int(scrollAccumulator)
        guard lines != 0 else { return }
        scrollAccumulator -= CGFloat(lines)
        // Clamp scroll-down (negative) so the offset can't drop below the floor
        // captured when the prompt was tucked away — the last command output is
        // the true bottom, and the redundant prompt below it stays hidden.
        if lines < 0, let floor = scrollFloorOffset {
            let available = Int(currentDisplayOffset) - floor
            if available <= 0 { return }
            lines = max(lines, -available)
        }
        backend.scroll(delta: Int32(lines))
        isScrolledBack = true
        needsDisplay = true
    }

    /// View point -> grid cell, accounting for the bottom-anchor offset and the
    /// blank-row collapse. The pixel Y maps to a *display* line, which the
    /// collapse inverse turns back into the true grid row every consumer expects.
    private func gridPoint(from event: NSEvent) -> (col: UInt16, row: UInt16) {
        let point = convert(event.locationInWindow, from: nil)
        var col = max(0, Int((point.x - padding) / fontMetrics.cellWidth))
        var row = gridRowForContentY(point.y - contentYOffset)
        if let grid = backend?.gridSnapshot() {
            col = min(col, max(0, grid.cols - 1))
            row = min(row, max(0, grid.lines - 1))
        }
        return (UInt16(col), UInt16(row))
    }

    // MARK: Font Update and Resize

    /// Update the font and recompute cell metrics.
    func updateFont(family: String, size: CGFloat) {
        fontMetrics = TerminalFontMetrics(fontFamily: family, fontSize: size)
        cacheFontVariants()
        resizeToFit()
    }

    /// Resize the backend grid to match the current view bounds.
    func resizeToFit() {
        let (cols, rows) = fontMetrics.gridSize(
            viewWidth: bounds.width, viewHeight: bounds.height, padding: padding
        )
        backend?.resize(
            cols: UInt16(cols), rows: UInt16(rows),
            cellWidth: UInt16(fontMetrics.cellWidth), cellHeight: UInt16(fontMetrics.cellHeight)
        )
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        resizeToFit()
        needsDisplay = true
    }
}

// MARK: - NSTextInputClient

extension TerminalRenderer: NSTextInputClient {

    // MARK: Inserting Text

    func insertText(_ string: Any, replacementRange: NSRange) {
        let text: String
        if let s = string as? String {
            text = s
        } else if let attr = string as? NSAttributedString {
            text = attr.string
        } else {
            return
        }
        guard !text.isEmpty, let backend else { return }
        if isScrolledBack {
            isScrolledBack = false
            backend.scrollToBottom()
        }
        if let metaData = KeyEncoder.encodeMetaForInsertText(text: text, event: currentKeyEvent) {
            backend.write(metaData)
        } else {
            backend.write(text)
        }
        // Clear any composition state.
        markedText = ""
        markedSelection = NSRange(location: 0, length: 0)
        resetBlink()
        needsDisplay = true
    }

    // MARK: Marked Text (composition)

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        let text: String
        if let s = string as? String {
            text = s
        } else if let attr = string as? NSAttributedString {
            text = attr.string
        } else {
            text = ""
        }
        markedText = text
        markedSelection = selectedRange
        needsDisplay = true
    }

    func unmarkText() {
        markedText = ""
        markedSelection = NSRange(location: 0, length: 0)
        needsDisplay = true
    }

    func selectedRange() -> NSRange {
        return NSRange(location: NSNotFound, length: 0)
    }

    func markedRange() -> NSRange {
        if markedText.isEmpty {
            return NSRange(location: NSNotFound, length: 0)
        }
        return NSRange(location: 0, length: (markedText as NSString).length)
    }

    func hasMarkedText() -> Bool {
        return !markedText.isEmpty
    }

    // MARK: Query Methods

    func attributedSubstring(
        forProposedRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? {
        return nil
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        return []
    }

    /// Returns the screen rectangle for the given character range. The IME
    /// uses this to position its candidate window. We return the cursor
    /// position since marked text is drawn at the cursor.
    func firstRect(
        forCharacterRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSRect {
        guard let window, let grid = backend?.gridSnapshot() else {
            return NSRect.zero
        }
        let cw = fontMetrics.cellWidth
        let ch = fontMetrics.cellHeight
        let cursorX = padding + CGFloat(grid.cursorCol) * cw
        // View coordinates are flipped; convert to AppKit bottom-up for window conversion.
        let cursorYFromTop = padding + CGFloat(grid.cursorRow) * ch
        let cursorYFromBottom = bounds.height - cursorYFromTop - ch
        let viewRect = NSRect(x: cursorX, y: cursorYFromBottom, width: cw, height: ch)
        let windowRect = self.convert(viewRect, to: nil)
        return window.convertToScreen(windowRect)
    }

    func characterIndex(for point: NSPoint) -> Int {
        return NSNotFound
    }
}
