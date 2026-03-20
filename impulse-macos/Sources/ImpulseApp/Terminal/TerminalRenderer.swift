import AppKit
import CoreText
import os.log

// MARK: - Font Metrics

/// Computed metrics for a monospace terminal font.
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

        // Measure the width of "W" (a wide character) for monospace.
        let measureString = "W" as CFString
        let attrString = CFAttributedStringCreate(nil, measureString, [
            kCTFontAttributeName: ctFont,
        ] as CFDictionary)!
        let line = CTLineCreateWithAttributedString(attrString)
        let bounds = CTLineGetBoundsWithOptions(line, [])
        self.cellWidth = ceil(bounds.width)
    }

    /// Calculate grid dimensions for a given view size (accounting for padding).
    func gridSize(viewWidth: CGFloat, viewHeight: CGFloat, padding: CGFloat) -> (cols: Int, rows: Int) {
        let contentWidth = viewWidth - padding * 2
        let contentHeight = viewHeight - padding * 2
        let cols = max(2, Int(contentWidth / cellWidth))
        let rows = max(1, Int(contentHeight / cellHeight))
        return (cols, rows)
    }
}

// MARK: - Terminal Renderer

/// NSView subclass that renders the terminal grid from a `TerminalBackend`
/// using CoreGraphics + CoreText.
///
/// This replaces SwiftTerm's built-in rendering. The backend owns all terminal
/// state; this view only draws what `gridSnapshot()` provides and forwards
/// user input to the backend.
class TerminalRenderer: NSView {

    // MARK: Properties

    /// The terminal backend providing grid state.
    var backend: TerminalBackend?

    /// Cached font metrics.
    private(set) var fontMetrics: TerminalFontMetrics

    /// Padding around the terminal content.
    let padding: CGFloat = 8

    /// Cached grid snapshot for drawing.
    private var cachedSnapshot: TerminalGridSnapshot?

    /// Display link for refresh.
    private var refreshTimer: Timer?

    /// Accumulated scroll delta for smooth trackpad scrolling.
    private var scrollAccumulator: CGFloat = 0

    /// Whether the user has scrolled away from the bottom of the terminal.
    /// While true, Wakeup events don't reset the viewport to the bottom.
    private var isScrolledBack: Bool = false

    /// Selection tracking.
    private var isSelecting = false
    private var selectionStartCol: UInt16 = 0
    private var selectionStartRow: UInt16 = 0

    /// Callback for terminal events.
    var onEvent: ((TerminalBackendEvent) -> Void)?

    /// Callback for paste request (Cmd+V).
    var onPaste: (() -> Void)?

    /// Callback for copy request (Cmd+C).
    var onCopy: (() -> Void)?

    // MARK: Initialization

    init(frame: NSRect, fontFamily: String, fontSize: CGFloat) {
        self.fontMetrics = TerminalFontMetrics(fontFamily: fontFamily, fontSize: fontSize)
        super.init(frame: frame)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    deinit {
        stopRefreshLoop()
    }

    // MARK: View Lifecycle

    override var acceptsFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        backend?.setFocus(true)
        return true
    }

    override func resignFirstResponder() -> Bool {
        backend?.setFocus(false)
        return true
    }

    override var isFlipped: Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            // Force a fresh snapshot when re-added to the view hierarchy.
            cachedSnapshot = backend?.gridSnapshot()
            needsDisplay = true
        }
    }

    // MARK: Drawing

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }

        // Always get a fresh snapshot when drawing (cached may be stale after tab switch).
        let snapshot: TerminalGridSnapshot
        if let cached = cachedSnapshot {
            snapshot = cached
        } else if let fresh = backend?.gridSnapshot() {
            snapshot = fresh
        } else {
            // No backend or snapshot — fill with background color.
            context.setFillColor(red: 31/255, green: 31/255, blue: 40/255, alpha: 1)
            context.fill(bounds)
            return
        }

        let fm = fontMetrics
        let cols = snapshot.cols
        let lines = snapshot.lines

        // 1. Fill background.
        if let firstRow = snapshot.cells.first, let firstCell = firstRow.first {
            let bg = firstCell.bg
            context.setFillColor(red: CGFloat(bg.r)/255, green: CGFloat(bg.g)/255, blue: CGFloat(bg.b)/255, alpha: 1)
        } else {
            context.setFillColor(red: 31/255, green: 31/255, blue: 40/255, alpha: 1)
        }
        context.fill(bounds)

        // 2. Draw cell backgrounds (only non-default).
        let defaultBg = snapshot.cells.first?.first?.bg ?? TerminalRgb(r: 31, g: 31, b: 40)
        for (rowIdx, row) in snapshot.cells.enumerated() {
            for (colIdx, cell) in row.enumerated() {
                if cell.bg.r != defaultBg.r || cell.bg.g != defaultBg.g || cell.bg.b != defaultBg.b {
                    let x = padding + CGFloat(colIdx) * fm.cellWidth
                    let y = padding + CGFloat(rowIdx) * fm.cellHeight
                    context.setFillColor(
                        red: CGFloat(cell.bg.r)/255,
                        green: CGFloat(cell.bg.g)/255,
                        blue: CGFloat(cell.bg.b)/255,
                        alpha: 1
                    )
                    context.fill(CGRect(x: x, y: y, width: fm.cellWidth, height: fm.cellHeight))
                }
            }
        }

        // 3. Draw selection highlight.
        if snapshot.hasSelection {
            context.setFillColor(red: 0.3, green: 0.5, blue: 0.8, alpha: 0.3)
            for range in snapshot.selectionRanges {
                guard range.count == 3 else { continue }
                let row = range[0]
                let startCol = range[1]
                let endCol = range[2]
                let x = padding + CGFloat(startCol) * fm.cellWidth
                let y = padding + CGFloat(row) * fm.cellHeight
                let w = CGFloat(endCol - startCol + 1) * fm.cellWidth
                context.fill(CGRect(x: x, y: y, width: w, height: fm.cellHeight))
            }
        }

        // 4. Draw text cell by cell.
        // Each character is positioned at its exact cell origin to ensure
        // box-drawing characters (U+2500–U+259F) connect without gaps.
        // In a flipped NSView, CoreGraphics text still renders Y-up,
        // so we set the text matrix to flip text rendering right-side up.
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)

        for (rowIdx, row) in snapshot.cells.enumerated() {
            let baseY = padding + CGFloat(rowIdx) * fm.cellHeight

            for (colIdx, cell) in row.enumerated() {
                let ch = cell.character
                // Skip spaces and null characters.
                guard ch != " " && ch != "\0" else { continue }

                let scalar = ch.unicodeScalars.first?.value ?? 0
                let isBoxDrawing = scalar >= 0x2500 && scalar <= 0x259F
                let x = padding + CGFloat(colIdx) * fm.cellWidth

                if isBoxDrawing {
                    // Draw box-drawing characters programmatically for pixel-perfect connections.
                    let fgColor = NSColor(
                        red: CGFloat(cell.fg.r) / 255,
                        green: CGFloat(cell.fg.g) / 255,
                        blue: CGFloat(cell.fg.b) / 255,
                        alpha: 1
                    )
                    drawBoxDrawingChar(
                        context: context,
                        scalar: scalar,
                        x: x, y: baseY,
                        cellWidth: fm.cellWidth,
                        cellHeight: fm.cellHeight,
                        color: fgColor
                    )
                } else {
                    // Regular character — draw with CoreText.
                    let fgColor = NSColor(
                        red: CGFloat(cell.fg.r) / 255,
                        green: CGFloat(cell.fg.g) / 255,
                        blue: CGFloat(cell.fg.b) / 255,
                        alpha: 1
                    )
                    let attrs: [NSAttributedString.Key: Any] = [
                        .font: fm.font as Any,
                        .foregroundColor: fgColor,
                    ]
                    let str = String(ch)
                    let attrStr = NSAttributedString(string: str, attributes: attrs)
                    let line = CTLineCreateWithAttributedString(attrStr)
                    context.textPosition = CGPoint(x: x, y: baseY + fm.ascent)
                    CTLineDraw(line, context)
                }
            }
        }

        // 5. Draw cursor.
        if snapshot.cursor.visible {
            let cursorX = padding + CGFloat(snapshot.cursor.col) * fm.cellWidth
            let cursorY = padding + CGFloat(snapshot.cursor.row) * fm.cellHeight

            context.setFillColor(red: 0.87, green: 0.84, blue: 0.73, alpha: 1) // Light cursor color

            switch snapshot.cursor.shape {
            case "Block":
                context.setFillColor(red: 0.87, green: 0.84, blue: 0.73, alpha: 0.5)
                context.fill(CGRect(x: cursorX, y: cursorY, width: fm.cellWidth, height: fm.cellHeight))
            case "Beam":
                context.fill(CGRect(x: cursorX, y: cursorY, width: 2, height: fm.cellHeight))
            case "Underline":
                context.fill(CGRect(x: cursorX, y: cursorY + fm.cellHeight - 2, width: fm.cellWidth, height: 2))
            case "HollowBlock":
                context.stroke(CGRect(x: cursorX, y: cursorY, width: fm.cellWidth, height: fm.cellHeight))
            default:
                break
            }
        }
    }

    // MARK: Refresh Loop

    func startRefreshLoop() {
        guard refreshTimer == nil else { return }
        // ~60fps polling for terminal events + redraw.
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 1.0/60.0, repeats: true) { [weak self] _ in
            self?.tick()
        }
    }

    func stopRefreshLoop() {
        refreshTimer?.invalidate()
        refreshTimer = nil
    }

    private func tick() {
        guard let backend, !backend.isShutdown else { return }

        let events = backend.pollEvents()
        var needsRedraw = false

        for event in events {
            switch event {
            case .wakeup:
                needsRedraw = true
            default:
                onEvent?(event)
            }
        }

        if needsRedraw && !isScrolledBack {
            cachedSnapshot = backend.gridSnapshot()
            needsDisplay = true
        }
    }

    // MARK: Keyboard Input

    override func keyDown(with event: NSEvent) {
        guard let backend else { return }

        // Handle Cmd shortcuts before passing to the terminal.
        if event.modifierFlags.contains(.command) {
            let key = event.charactersIgnoringModifiers?.lowercased() ?? ""
            if key == "v" {
                onPaste?()
                return
            }
            if key == "c" {
                onCopy?()
                return
            }
            // Let other Cmd+ shortcuts propagate to the menu/responder chain.
            super.keyDown(with: event)
            return
        }

        let bytes = translateKeyEvent(event)
        if !bytes.isEmpty {
            // Any keyboard input snaps back to the bottom of the terminal.
            if isScrolledBack {
                isScrolledBack = false
                // Large negative delta gets clamped to display_offset=0 (bottom).
                backend.scroll(delta: -999999)
            }
            backend.write(bytes: bytes)
        }
    }

    /// Translate an NSEvent key event into terminal escape sequences.
    private func translateKeyEvent(_ event: NSEvent) -> [UInt8] {
        let flags = event.modifierFlags
        let hasCmd = flags.contains(.command)
        let hasCtrl = flags.contains(.control)
        let hasShift = flags.contains(.shift)
        let hasOption = flags.contains(.option)

        // Don't handle Cmd+ shortcuts (those are menu items).
        if hasCmd { return [] }

        let keyCode = event.keyCode

        // Shift+Enter → CSI u
        if (keyCode == 36 || keyCode == 76) && hasShift && !hasCtrl {
            return [0x1B, 0x5B, 0x31, 0x33, 0x3B, 0x32, 0x75]
        }

        // Check app cursor mode for arrow keys.
        let appCursor = backend?.mode()?.appCursor ?? false

        // Special keys.
        switch keyCode {
        case 36, 76: return [0x0D] // Return / Enter
        case 51: return [0x7F]     // Backspace
        case 48: return [0x09]     // Tab
        case 53: return [0x1B]     // Escape
        case 126: return appCursor ? [0x1B, 0x4F, 0x41] : [0x1B, 0x5B, 0x41] // Up
        case 125: return appCursor ? [0x1B, 0x4F, 0x42] : [0x1B, 0x5B, 0x42] // Down
        case 124: return appCursor ? [0x1B, 0x4F, 0x43] : [0x1B, 0x5B, 0x43] // Right
        case 123: return appCursor ? [0x1B, 0x4F, 0x44] : [0x1B, 0x5B, 0x44] // Left
        case 115: return [0x1B, 0x5B, 0x48]    // Home
        case 119: return [0x1B, 0x5B, 0x46]    // End
        case 116: return [0x1B, 0x5B, 0x35, 0x7E] // Page Up
        case 121: return [0x1B, 0x5B, 0x36, 0x7E] // Page Down
        case 117: return [0x1B, 0x5B, 0x33, 0x7E] // Delete (forward)
        // F1-F12
        case 122: return [0x1B, 0x4F, 0x50]         // F1
        case 120: return [0x1B, 0x4F, 0x51]         // F2
        case 99:  return [0x1B, 0x4F, 0x52]         // F3
        case 118: return [0x1B, 0x4F, 0x53]         // F4
        case 96:  return [0x1B, 0x5B, 0x31, 0x35, 0x7E] // F5
        case 97:  return [0x1B, 0x5B, 0x31, 0x37, 0x7E] // F6
        case 98:  return [0x1B, 0x5B, 0x31, 0x38, 0x7E] // F7
        case 100: return [0x1B, 0x5B, 0x31, 0x39, 0x7E] // F8
        case 101: return [0x1B, 0x5B, 0x32, 0x30, 0x7E] // F9
        case 109: return [0x1B, 0x5B, 0x32, 0x31, 0x7E] // F10
        case 103: return [0x1B, 0x5B, 0x32, 0x33, 0x7E] // F11
        case 111: return [0x1B, 0x5B, 0x32, 0x34, 0x7E] // F12
        default:
            break
        }

        // Ctrl+letter → control codes.
        if hasCtrl, let chars = event.charactersIgnoringModifiers?.lowercased(), chars.count == 1 {
            let c = chars.unicodeScalars.first!.value
            if c >= 0x61 && c <= 0x7A { // a-z
                return [UInt8(c - 0x60)]
            }
            // Ctrl+[ → ESC, Ctrl+] → GS, Ctrl+\ → FS
            switch c {
            case 0x5B: return [0x1B]
            case 0x5D: return [0x1D]
            case 0x5C: return [0x1C]
            default: break
            }
        }

        // Option+key → send ESC prefix (meta encoding).
        if hasOption, let chars = event.charactersIgnoringModifiers, chars.count == 1 {
            return [0x1B] + Array(chars.utf8)
        }

        // Regular character input.
        if let chars = event.characters, !chars.isEmpty {
            return Array(chars.utf8)
        }

        return []
    }

    // MARK: Mouse Input

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let (col, row) = gridPoint(from: event)
        backend?.clearSelection()
        isSelecting = true
        selectionStartCol = col
        selectionStartRow = row
        backend?.startSelection(col: col, row: row, kind: "simple")
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard isSelecting else { return }
        let (col, row) = gridPoint(from: event)
        backend?.updateSelection(col: col, row: row)
        cachedSnapshot = backend?.gridSnapshot()
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        isSelecting = false
    }

    override func scrollWheel(with event: NSEvent) {
        guard let backend else { return }

        // Check if the terminal has mouse reporting enabled.
        if let mode = backend.mode(), mode.mouseReportClick || mode.mouseMotion || mode.mouseDrag {
            // Forward scroll events as mouse button 4/5.
            let (col, row) = gridPoint(from: event)
            let button = event.deltaY > 0 ? 4 : 5
            // For SGR mouse encoding: \e[<button;col;row M/m
            if mode.mouseSgr {
                let seq = "\u{1B}[<\(64 + button);\(col + 1);\(row + 1)M"
                backend.write(seq)
            }
            return
        }

        // Normal scrolling: accumulate deltas and scroll in whole-line increments.
        // macOS: positive deltaY = scroll content down (see history/up).
        // alacritty: positive Scroll::Delta = scroll towards history (up).
        // So the signs match — pass deltaY directly.
        let rawDelta: CGFloat
        if event.hasPreciseScrollingDeltas {
            // Trackpad: scrollingDeltaY is in pixels, convert to lines.
            rawDelta = event.scrollingDeltaY / fontMetrics.cellHeight
        } else {
            // Mouse wheel: deltaY is already in lines.
            rawDelta = event.deltaY * 3
        }

        scrollAccumulator += rawDelta
        let lines = Int32(scrollAccumulator)
        if lines != 0 {
            scrollAccumulator -= CGFloat(lines)
            backend.scroll(delta: lines)
            isScrolledBack = true
            cachedSnapshot = backend.gridSnapshot()
            needsDisplay = true
        }
    }

    /// Convert mouse event coordinates to terminal grid position.
    private func gridPoint(from event: NSEvent) -> (col: UInt16, row: UInt16) {
        let point = convert(event.locationInWindow, from: nil)
        let col = max(0, Int((point.x - padding) / fontMetrics.cellWidth))
        let row = max(0, Int((point.y - padding) / fontMetrics.cellHeight))
        return (UInt16(col), UInt16(row))
    }

    // MARK: Box Drawing

    /// Draw a box-drawing character programmatically for pixel-perfect connections.
    private func drawBoxDrawingChar(
        context: CGContext,
        scalar: UInt32,
        x: CGFloat, y: CGFloat,
        cellWidth: CGFloat, cellHeight: CGFloat,
        color: NSColor
    ) {
        let midX = x + cellWidth / 2
        let midY = y + cellHeight / 2
        let lineWidth: CGFloat = 1.0

        context.setStrokeColor(color.cgColor)
        context.setFillColor(color.cgColor)
        context.setLineWidth(lineWidth)
        context.setLineCap(.square)

        // Determine which segments to draw based on the Unicode code point.
        // Box-drawing chars U+2500-U+257F encode which directions have lines:
        //   right, left, down, up — and light vs heavy weight.
        let right = x + cellWidth
        let bottom = y + cellHeight

        switch scalar {
        // Light lines
        case 0x2500: // ─ horizontal
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2501: // ━ heavy horizontal
            context.setLineWidth(2)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2502: // │ vertical
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x2503: // ┃ heavy vertical
            context.setLineWidth(2)
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x250C: // ┌ down-right
            context.move(to: CGPoint(x: midX, y: bottom))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x250E, 0x250F: // ┎┏ heavy variants of down-right
            context.setLineWidth(2)
            context.move(to: CGPoint(x: midX, y: bottom))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2510: // ┐ down-left
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x2512, 0x2513: // ┒┓ heavy variants of down-left
            context.setLineWidth(2)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x2514: // └ up-right
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2516, 0x2517: // ┖┗ heavy variants of up-right
            context.setLineWidth(2)
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2518: // ┘ up-left
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: y))
        case 0x251A, 0x251B: // ┚┛ heavy variants of up-left
            context.setLineWidth(2)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: y))
        case 0x251C, 0x251D, 0x251E, 0x251F, 0x2520, 0x2521, 0x2522, 0x2523: // ├ variants (vertical + right)
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
            context.strokePath()
            context.move(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2524, 0x2525, 0x2526, 0x2527, 0x2528, 0x2529, 0x252A, 0x252B: // ┤ variants (vertical + left)
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
            context.strokePath()
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX, y: midY))
        case 0x252C, 0x252D, 0x252E, 0x252F, 0x2530, 0x2531, 0x2532, 0x2533: // ┬ variants (horizontal + down)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
            context.strokePath()
            context.move(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x2534, 0x2535, 0x2536, 0x2537, 0x2538, 0x2539, 0x253A, 0x253B: // ┴ variants (horizontal + up)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
            context.strokePath()
            context.move(to: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: y))
        case 0x253C, 0x253D, 0x253E, 0x253F, 0x2540, 0x2541, 0x2542, 0x2543,
             0x2544, 0x2545, 0x2546, 0x2547, 0x2548, 0x2549, 0x254A, 0x254B: // ┼ variants (cross)
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
            context.strokePath()
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        // Dashed/dotted lines
        case 0x2504, 0x2505, 0x2508, 0x2509: // ┄┅┈┉ dashed horizontal
            context.setLineDash(phase: 0, lengths: [3, 2])
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x2506, 0x2507, 0x250A, 0x250B: // ┆┇┊┋ dashed vertical
            context.setLineDash(phase: 0, lengths: [3, 2])
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        // Double lines (╔╗╚╝═║ etc.)
        case 0x2550: // ═ double horizontal
            let offset: CGFloat = 1.5
            context.move(to: CGPoint(x: x, y: midY - offset))
            context.addLine(to: CGPoint(x: right, y: midY - offset))
            context.strokePath()
            context.move(to: CGPoint(x: x, y: midY + offset))
            context.addLine(to: CGPoint(x: right, y: midY + offset))
        case 0x2551: // ║ double vertical
            let offset: CGFloat = 1.5
            context.move(to: CGPoint(x: midX - offset, y: y))
            context.addLine(to: CGPoint(x: midX - offset, y: bottom))
            context.strokePath()
            context.move(to: CGPoint(x: midX + offset, y: y))
            context.addLine(to: CGPoint(x: midX + offset, y: bottom))
        // Rounded corners
        case 0x256D: // ╭ rounded down-right
            context.move(to: CGPoint(x: midX, y: bottom))
            context.addLine(to: CGPoint(x: midX, y: midY + 2))
            context.addQuadCurve(to: CGPoint(x: midX + 2, y: midY), control: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        case 0x256E: // ╮ rounded down-left
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX - 2, y: midY))
            context.addQuadCurve(to: CGPoint(x: midX, y: midY + 2), control: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: bottom))
        case 0x256F: // ╯ rounded up-left
            context.move(to: CGPoint(x: x, y: midY))
            context.addLine(to: CGPoint(x: midX - 2, y: midY))
            context.addQuadCurve(to: CGPoint(x: midX, y: midY - 2), control: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: midX, y: y))
        case 0x2570: // ╰ rounded up-right
            context.move(to: CGPoint(x: midX, y: y))
            context.addLine(to: CGPoint(x: midX, y: midY - 2))
            context.addQuadCurve(to: CGPoint(x: midX + 2, y: midY), control: CGPoint(x: midX, y: midY))
            context.addLine(to: CGPoint(x: right, y: midY))
        // Block elements
        case 0x2580: // ▀ upper half block
            context.fill(CGRect(x: x, y: y, width: cellWidth, height: cellHeight / 2))
            return // already filled, don't stroke
        case 0x2584: // ▄ lower half block
            context.fill(CGRect(x: x, y: y + cellHeight / 2, width: cellWidth, height: cellHeight / 2))
            return
        case 0x2588: // █ full block
            context.fill(CGRect(x: x, y: y, width: cellWidth, height: cellHeight))
            return
        case 0x258C: // ▌ left half block
            context.fill(CGRect(x: x, y: y, width: cellWidth / 2, height: cellHeight))
            return
        case 0x2590: // ▐ right half block
            context.fill(CGRect(x: x + cellWidth / 2, y: y, width: cellWidth / 2, height: cellHeight))
            return
        default:
            // For unhandled box-drawing chars, fall back to font rendering.
            let fgColor = color
            let attrs: [NSAttributedString.Key: Any] = [
                .font: fontMetrics.font as Any,
                .foregroundColor: fgColor,
            ]
            let str = String(Unicode.Scalar(scalar)!)
            let attrStr = NSAttributedString(string: str, attributes: attrs)
            let line = CTLineCreateWithAttributedString(attrStr)
            context.textPosition = CGPoint(x: x, y: y + fontMetrics.ascent)
            CTLineDraw(line, context)
            return
        }

        context.strokePath()
        context.setLineDash(phase: 0, lengths: []) // Reset dash pattern
    }

    // MARK: Font Updates

    /// Update the font metrics and trigger a resize.
    func updateFont(family: String, size: CGFloat) {
        fontMetrics = TerminalFontMetrics(fontFamily: family, fontSize: size)
        resizeToFit()
    }

    /// Recalculate grid dimensions based on current view size and notify the backend.
    func resizeToFit() {
        let (cols, rows) = fontMetrics.gridSize(
            viewWidth: bounds.width,
            viewHeight: bounds.height,
            padding: padding
        )
        backend?.resize(
            cols: UInt16(cols),
            rows: UInt16(rows),
            cellWidth: UInt16(fontMetrics.cellWidth),
            cellHeight: UInt16(fontMetrics.cellHeight)
        )
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        resizeToFit()
    }
}
