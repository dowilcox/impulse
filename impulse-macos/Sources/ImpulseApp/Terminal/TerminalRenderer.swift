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
/// characters. Uses a CVDisplayLink for refresh timing and `KeyEncoder` for
/// keyboard input translation.
class TerminalRenderer: NSView {

    // MARK: Public Properties

    var backend: TerminalBackend?
    private(set) var fontMetrics: TerminalFontMetrics
    let padding: CGFloat = 8

    /// Called when the backend emits a non-wakeup event (title change, bell, exit, etc.).
    var onEvent: ((TerminalBackendEvent) -> Void)?

    /// Called when the user presses Cmd+V in the terminal.
    var onPaste: (() -> Void)?

    /// Called when the user presses Cmd+C in the terminal.
    var onCopy: (() -> Void)?

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

    /// 16-color ANSI palette (RGB triplets). Set from the theme.
    /// Used to substitute bold text colors when `boldIsBright` is true.
    var paletteRgb: [(UInt8, UInt8, UInt8)] = []

    // MARK: Private Properties

    private var displayLink: CADisplayLink?
    private var scrollAccumulator: CGFloat = 0
    private var isScrolledBack: Bool = false
    private var isSelecting = false
    private var needsRedraw = false
    private var cursorBlinkOn: Bool = true
    private var blinkTimer: DispatchSourceTimer?

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

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    override func becomeFirstResponder() -> Bool {
        backend?.setFocus(true)
        return true
    }

    override func resignFirstResponder() -> Bool {
        backend?.setFocus(false)
        return true
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            needsDisplay = true
        }
    }

    // MARK: Display Link Refresh Loop

    func startRefreshLoop() {
        guard displayLink == nil else { return }
        // NSView.displayLink (macOS 15+) replaces the deprecated CVDisplayLink.
        // Fires on the main thread at the display's refresh rate.
        let link = self.displayLink(target: self, selector: #selector(displayLinkTick))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    func stopRefreshLoop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func displayLinkTick() {
        tick()
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
            self.needsDisplay = true
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

    private func tick() {
        guard let backend, !backend.isShutdown else { return }
        let events = backend.pollEvents()
        var wakeup = false
        for event in events {
            switch event {
            case .wakeup:
                wakeup = true
            default:
                DispatchQueue.main.async { [weak self] in
                    self?.onEvent?(event)
                }
            }
        }
        if wakeup && !isScrolledBack {
            // Auto-scroll to bottom on output when enabled and the user
            // hasn't manually scrolled back.
            if scrollOnOutput {
                backend.scrollToBottom()
            }
            DispatchQueue.main.async { [weak self] in
                self?.needsDisplay = true
            }
        }
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
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

        // 1. Fill background with the configured terminal background color.
        // Do not infer it from the top-left cell: TUIs like Codex/Claude can
        // place a colored block in the first visible cell, which would make
        // the entire viewport inherit that accent color.
        context.setFillColor(defaultBackgroundColor)
        context.fill(bounds)

        // 2. Draw non-default cell backgrounds.
        for row in 0..<lines {
            let rowY = padding + CGFloat(row) * ch
            for col in 0..<cols {
                let cell = grid.cell(row: row, col: col)
                let flags = cell.flags

                // Skip wide char spacer cells.
                if flags & GridBufferReader.flagWideCharSpacer != 0 { continue }

                var bgR = cell.bgR, bgG = cell.bgG, bgB = cell.bgB
                var fgR = cell.fgR, fgG = cell.fgG, fgB = cell.fgB

                // Handle inverse attribute.
                if flags & GridBufferReader.flagInverse != 0 {
                    swap(&bgR, &fgR)
                    swap(&bgG, &fgG)
                    swap(&bgB, &fgB)
                }

                // Only draw if background differs from the default.
                if bgR != defaultBackgroundRgb.0
                    || bgG != defaultBackgroundRgb.1
                    || bgB != defaultBackgroundRgb.2 {
                    let cellWidth = (flags & GridBufferReader.flagWideChar != 0) ? cw * 2 : cw
                    let rect = CGRect(x: padding + CGFloat(col) * cw, y: rowY, width: cellWidth, height: ch)
                    context.setFillColor(CGColor(
                        srgbRed: CGFloat(bgR) / 255.0,
                        green: CGFloat(bgG) / 255.0,
                        blue: CGFloat(bgB) / 255.0,
                        alpha: 1.0
                    ))
                    context.fill(rect)
                }
            }
        }

        // 3. Draw selection highlights using the active theme color.
        for i in 0..<grid.selectionRangeCount {
            let range = grid.selectionRange(at: i)
            let rowY = padding + CGFloat(range.row) * ch
            let startX = padding + CGFloat(range.startCol) * cw
            let endX = padding + CGFloat(range.endCol) * cw
            let rect = CGRect(x: startX, y: rowY, width: endX - startX, height: ch)
            context.setFillColor(selectionColor)
            context.fill(rect)
        }

        // 4. Draw search match highlights (amber semi-transparent).
        let searchColor = CGColor(srgbRed: 0.9, green: 0.7, blue: 0.1, alpha: 0.35)
        for i in 0..<grid.searchMatchRangeCount {
            let range = grid.searchMatchRange(at: i)
            let rowY = padding + CGFloat(range.row) * ch
            let startX = padding + CGFloat(range.startCol) * cw
            let endX = padding + CGFloat(range.endCol) * cw
            let rect = CGRect(x: startX, y: rowY, width: endX - startX, height: ch)
            context.setFillColor(searchColor)
            context.fill(rect)
        }

        // 5. Draw text using run-based rendering.
        // In a flipped NSView, CoreGraphics text still renders Y-up.
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)

        for row in 0..<lines {
            let rowY = padding + CGFloat(row) * ch

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
                    context.setStrokeColor(CGColor(
                        srgbRed: CGFloat(fgR) / 255.0,
                        green: CGFloat(fgG) / 255.0,
                        blue: CGFloat(fgB) / 255.0,
                        alpha: alpha
                    ))
                    context.setLineWidth(1)
                    let underlineY = rowY + fontMetrics.ascent + fontMetrics.descent - 1
                    context.move(to: CGPoint(x: cellX, y: underlineY))
                    context.addLine(to: CGPoint(x: cellX + cellWidth, y: underlineY))
                    context.strokePath()
                }

                if flags & GridBufferReader.flagStrikethrough != 0 {
                    context.setStrokeColor(CGColor(
                        srgbRed: CGFloat(fgR) / 255.0,
                        green: CGFloat(fgG) / 255.0,
                        blue: CGFloat(fgB) / 255.0,
                        alpha: alpha
                    ))
                    context.setLineWidth(1)
                    let strikeY = rowY + ch / 2
                    context.move(to: CGPoint(x: cellX, y: strikeY))
                    context.addLine(to: CGPoint(x: cellX + cellWidth, y: strikeY))
                    context.strokePath()
                }
            }
        }

        // 7. Draw cursor (respects blink phase and shape override from settings).
        if grid.cursorVisible && cursorBlinkOn {
            let cursorRow = grid.cursorRow
            let cursorCol = grid.cursorCol
            if cursorRow < lines && cursorCol < cols {
                let cursorX = padding + CGFloat(cursorCol) * cw
                let cursorY = padding + CGFloat(cursorRow) * ch
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
            let yBase = padding + CGFloat(hoverRow) * ch + ch - 1
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
            let startY = padding + CGFloat(cursorRow) * ch

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
        let color = CGColor(
            srgbRed: CGFloat(fgR) / 255.0,
            green: CGFloat(fgG) / 255.0,
            blue: CGFloat(fgB) / 255.0,
            alpha: alpha
        )

        let font = fontForStyle(bold: bold, italic: italic)
        let cw = fontMetrics.cellWidth
        let baseX = padding + CGFloat(col) * cw
        let textY = rowY + fontMetrics.ascent

        // Draw each character at its exact cell position to prevent drift.
        // CoreText's natural advances may differ slightly from cellWidth,
        // causing characters to misalign with the grid over long runs.
        var charCol = 0
        for ch in text.unicodeScalars {
            let str = String(ch) as CFString
            let attrs: [CFString: Any] = [
                kCTFontAttributeName: font,
                kCTForegroundColorAttributeName: color,
            ]
            let attrStr = CFAttributedStringCreate(nil, str, attrs as CFDictionary)!
            let line = CTLineCreateWithAttributedString(attrStr)
            context.textPosition = CGPoint(x: baseX + CGFloat(charCol) * cw, y: textY)
            CTLineDraw(line, context)
            charCol += 1
        }
    }

    // MARK: Font Variant Cache

    private func cacheFontVariants() {
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
        let color = CGColor(
            srgbRed: CGFloat(fgR) / 255.0,
            green: CGFloat(fgG) / 255.0,
            blue: CGFloat(fgB) / 255.0,
            alpha: alpha
        )

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

    override func keyDown(with event: NSEvent) {
        // Cmd shortcuts.
        if event.modifierFlags.contains(.command) {
            let key = event.charactersIgnoringModifiers?.lowercased() ?? ""
            if key == "v" { onPaste?(); return }
            if key == "c" { onCopy?(); return }
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
        if event.modifierFlags.contains(.command) {
            return super.performKeyEquivalent(with: event)
        }
        return false
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

    override func mouseExited(with event: NSEvent) {
        if hoverIsLink {
            hoverIsLink = false
            hoverLinkUri = nil
            needsDisplay = true
            window?.invalidateCursorRects(for: self)
        }
        hoverCol = -1
        hoverRow = -1
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
    }

    // MARK: Mouse Input

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)

        // Cmd+Click on a hyperlink opens the URL. We first try the OSC 8
        // link at the cell, then fall back to an auto-detected URL in the row.
        if event.modifierFlags.contains(.command) {
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
        isSelecting = true
        backend?.startSelection(col: col, row: row, kind: kind)
        if kind != 0 {
            backend?.updateSelection(col: col, row: row)
        }
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        if reportMouseEvent(event, button: 0, motion: true, release: false) {
            return
        }
        guard isSelecting else { return }
        let (col, row) = gridPoint(from: event)
        backend?.updateSelection(col: col, row: row)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        if reportMouseEvent(event, button: 0, motion: false, release: true) {
            return
        }
        isSelecting = false
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
        let menu = NSMenu()
        menu.autoenablesItems = false

        let copyItem = NSMenuItem(
            title: "Copy",
            action: #selector(contextCopy(_:)),
            keyEquivalent: ""
        )
        copyItem.target = self
        copyItem.isEnabled = backend?.selectedText() != nil
        menu.addItem(copyItem)

        let pasteItem = NSMenuItem(
            title: "Paste",
            action: #selector(contextPaste(_:)),
            keyEquivalent: ""
        )
        pasteItem.target = self
        pasteItem.isEnabled = NSPasteboard.general.string(forType: .string) != nil
        menu.addItem(pasteItem)

        menu.addItem(NSMenuItem.separator())

        let selectAllItem = NSMenuItem(
            title: "Select All",
            action: #selector(contextSelectAll(_:)),
            keyEquivalent: ""
        )
        selectAllItem.target = self
        menu.addItem(selectAllItem)

        let clearItem = NSMenuItem(
            title: "Clear",
            action: #selector(contextClear(_:)),
            keyEquivalent: ""
        )
        clearItem.target = self
        menu.addItem(clearItem)

        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }

    @objc private func contextCopy(_ sender: Any?) {
        onCopy?()
    }

    @objc private func contextPaste(_ sender: Any?) {
        onPaste?()
    }

    @objc private func contextSelectAll(_ sender: Any?) {
        guard let grid = backend?.gridSnapshot() else { return }
        let lastCol = max(0, grid.cols - 1)
        let lastRow = max(0, grid.lines - 1)
        backend?.startSelection(col: 0, row: 0, kind: 1)
        backend?.updateSelection(col: UInt16(lastCol), row: UInt16(lastRow))
        needsDisplay = true
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

        let lines = Int(scrollAccumulator)
        guard lines != 0 else { return }
        scrollAccumulator -= CGFloat(lines)
        backend.scroll(delta: Int32(lines))
        isScrolledBack = true
        needsDisplay = true
    }

    private func gridPoint(from event: NSEvent) -> (col: UInt16, row: UInt16) {
        let point = convert(event.locationInWindow, from: nil)
        var col = max(0, Int((point.x - padding) / fontMetrics.cellWidth))
        var row = max(0, Int((point.y - padding) / fontMetrics.cellHeight))
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
        backend.write(text)
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
