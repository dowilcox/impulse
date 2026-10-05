import AppKit
import ImpulseKit
import SwiftUI

/// Keys the command editor hands to its owner before handling them itself.
enum CommandEditorKey {
  /// ↑ on the first line / ↓ on the last line (history, dropdown).
  case up, down
  case tab
  /// → at the end of the text (accept part of the suggestion).
  case right
  case escape
  case controlC
  case controlR
  /// ⌥⌘↩ in prompt mode (send without pressing Return). Not ⇧⌘↩: that's
  /// Zoom Pane.
  case alternateSubmit
  /// ⌘↑: select the most recent command block.
  case selectBlock
}

/// The terminal input: a multi-line shell command editor. Return runs the
/// command (⇧⏎ / ⌥⏎ add a line), paste keeps newlines, IME and dictation
/// work, the command is colored by the shell parser, and a history
/// suggestion shows as ghost text after the cursor.
struct CommandEditor: NSViewRepresentable {
  @Binding var text: String
  var placeholder: String
  /// Full suggested line; the part after `text` is drawn dimmed.
  var suggestion: String?
  var colors: CommandEditorColors
  var font: NSFont = .monospacedSystemFont(ofSize: 13, weight: .regular)
  /// Color the text as a shell command (off for prose, e.g. agent prompts).
  var shellSyntax = true
  /// Whether the shell can run a command word (nil: can't tell); unknown
  /// commands get a dashed underline.
  var isKnownCommand: ((String) -> Bool?)? = nil
  /// Return submits (commands). Off: Return adds a line and ⌘↩ submits
  /// (prompts).
  var returnSubmits = true
  /// Bump to move keyboard focus here.
  var focusToken: Int
  var onSubmit: () -> Void
  /// Return true when handled (the editor then ignores the key).
  var onKey: (CommandEditorKey) -> Bool
  var onFocusChange: (Bool) -> Void = { _ in }

  func makeNSView(context: Context) -> CommandScrollView {
    let scroll = CommandScrollView()
    let textView = scroll.textView
    textView.delegate = context.coordinator
    textView.coordinator = context.coordinator
    context.coordinator.textView = textView
    apply(to: textView)
    textView.string = text
    textView.highlight()
    return scroll
  }

  func updateNSView(_ scroll: CommandScrollView, context: Context) {
    context.coordinator.parent = self
    let textView = scroll.textView
    apply(to: textView)
    if textView.string != text, !textView.hasMarkedText() {
      textView.string = text
      textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
      textView.highlight()
      scroll.invalidateIntrinsicContentSize()
    }
    if context.coordinator.lastFocusToken != focusToken {
      context.coordinator.lastFocusToken = focusToken
      DispatchQueue.main.async { textView.window?.makeFirstResponder(textView) }
    }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, nsView: CommandScrollView, context: Context) -> CGSize? {
    CGSize(width: proposal.width ?? 300, height: nsView.textView.preferredHeight)
  }

  private func apply(to textView: CommandTextView) {
    textView.isKnownCommand = shellSyntax ? isKnownCommand : nil
    let ghost = suggestion.flatMap { suggestion -> String? in
      guard suggestion.hasPrefix(text), suggestion.count > text.count else { return nil }
      return String(suggestion.dropFirst(text.count))
    }
    if textView.ghost != ghost {
      textView.ghost = ghost
      textView.needsDisplay = true
    }
    if textView.placeholder != placeholder {
      textView.placeholder = placeholder
      textView.needsDisplay = true
    }
    if textView.colors != colors || textView.font != font || textView.shellSyntax != shellSyntax {
      textView.colors = colors
      textView.font = font
      textView.shellSyntax = shellSyntax
      textView.insertionPointColor = colors.caret
      textView.highlight()
    }
  }

  func makeCoordinator() -> Coordinator { Coordinator(self) }

  final class Coordinator: NSObject, NSTextViewDelegate {
    var parent: CommandEditor
    weak var textView: CommandTextView?
    var lastFocusToken: Int

    init(_ parent: CommandEditor) {
      self.parent = parent
      self.lastFocusToken = parent.focusToken
    }

    func textDidChange(_ notification: Notification) {
      guard let textView else { return }
      textView.highlight()
      textView.enclosingScrollView?.invalidateIntrinsicContentSize()
      if parent.text != textView.string { parent.text = textView.string }
    }

    func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
      guard let view = textView as? CommandTextView else { return false }
      switch selector {
      case #selector(NSResponder.insertNewline(_:)):
        let flags = NSApp.currentEvent?.modifierFlags ?? []
        if !parent.returnSubmits {
          if flags.contains(.command) {
            if flags.contains(.option) { _ = parent.onKey(.alternateSubmit) } else { parent.onSubmit() }
          } else {
            textView.insertText("\n", replacementRange: textView.selectedRange())
          }
          return true
        }
        if flags.contains(.shift) || flags.contains(.option) {
          textView.insertText("\n", replacementRange: textView.selectedRange())
          return true
        }
        parent.onSubmit()
        return true
      case #selector(NSResponder.insertNewlineIgnoringFieldEditor(_:)):
        textView.insertText("\n", replacementRange: textView.selectedRange())
        return true
      case #selector(NSResponder.moveUp(_:)):
        return view.isOnFirstLine ? parent.onKey(.up) : false
      case #selector(NSResponder.moveDown(_:)):
        return view.isOnLastLine ? parent.onKey(.down) : false
      case #selector(NSResponder.insertTab(_:)):
        _ = parent.onKey(.tab)
        return true  // never move focus out
      case #selector(NSResponder.moveRight(_:)):
        return view.isAtEnd ? parent.onKey(.right) : false
      case #selector(NSResponder.cancelOperation(_:)):
        return parent.onKey(.escape)
      default:
        return false
      }
    }
  }
}

struct CommandEditorColors: Equatable {
  var text: NSColor
  var ghost: NSColor
  var caret: NSColor
  var command: NSColor
  var flag: NSColor
  var string: NSColor
  var variable: NSColor
  var operatorColor: NSColor
  var error: NSColor

  init(theme: Theme) {
    text = theme.fgColor
    ghost = theme.fgCommentColor
    caret = theme.accentColor
    command = theme.blueColor
    flag = theme.yellowColor
    string = theme.greenColor
    variable = theme.magentaColor
    operatorColor = theme.cyanColor
    error = theme.redColor
  }
}

/// Scroll container that sizes to its text (up to a cap, then scrolls).
final class CommandScrollView: NSScrollView {
  let textView = CommandTextView()

  init() {
    super.init(frame: .zero)
    drawsBackground = false
    hasVerticalScroller = true
    autohidesScrollers = true
    borderType = .noBorder
    textView.minSize = .zero
    textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: .greatestFiniteMagnitude)
    textView.isVerticallyResizable = true
    textView.isHorizontallyResizable = false
    textView.autoresizingMask = [.width]
    textView.textContainer?.widthTracksTextView = true
    documentView = textView
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override var intrinsicContentSize: NSSize {
    NSSize(width: NSView.noIntrinsicMetric, height: textView.preferredHeight)
  }
}

final class CommandTextView: NSTextView {
  weak var coordinator: CommandEditor.Coordinator?
  var ghost: String?
  var placeholder = ""
  var colors = CommandEditorColors(theme: ThemeManager.theme(forName: "nord"))
  var shellSyntax = true
  var isKnownCommand: ((String) -> Bool?)?
  /// Lines shown before scrolling.
  static let maxVisibleLines: CGFloat = 8

  init() {
    let storage = NSTextStorage()
    let layout = NSLayoutManager()
    storage.addLayoutManager(layout)
    let container = NSTextContainer(size: NSSize(width: 300, height: CGFloat.greatestFiniteMagnitude))
    container.lineFragmentPadding = 0
    layout.addTextContainer(container)
    super.init(frame: .zero, textContainer: container)
    drawsBackground = false
    isRichText = false
    importsGraphics = false
    allowsUndo = true
    isAutomaticQuoteSubstitutionEnabled = false
    isAutomaticDashSubstitutionEnabled = false
    isAutomaticTextReplacementEnabled = false
    isAutomaticSpellingCorrectionEnabled = false
    isContinuousSpellCheckingEnabled = false
    isGrammarCheckingEnabled = false
    isAutomaticLinkDetectionEnabled = false
    smartInsertDeleteEnabled = false
    textContainerInset = .zero
    setAccessibilityLabel("Command input")
  }

  override init(frame frameRect: NSRect, textContainer container: NSTextContainer?) {
    super.init(frame: frameRect, textContainer: container)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  private var lineHeight: CGFloat {
    guard let font else { return 17 }
    return ceil(layoutManager?.defaultLineHeight(for: font) ?? font.pointSize * 1.3)
  }

  /// Height for the current text, one line minimum, capped.
  var preferredHeight: CGFloat {
    guard let layoutManager, let textContainer else { return lineHeight }
    layoutManager.ensureLayout(for: textContainer)
    let used = layoutManager.usedRect(for: textContainer).height
    let lines = max(1, (used / lineHeight).rounded())
    return min(lines, Self.maxVisibleLines) * lineHeight
  }

  var isAtEnd: Bool { selectedRange().location >= (string as NSString).length && selectedRange().length == 0 }

  var isOnFirstLine: Bool {
    let location = selectedRange().location
    return !(string as NSString).substring(to: min(location, (string as NSString).length)).contains("\n")
  }

  var isOnLastLine: Bool {
    let ns = string as NSString
    let location = min(selectedRange().location + selectedRange().length, ns.length)
    return !ns.substring(from: location).contains("\n")
  }

  // MARK: Focus

  override func becomeFirstResponder() -> Bool {
    let became = super.becomeFirstResponder()
    if became { coordinator?.parent.onFocusChange(true) }
    return became
  }

  override func resignFirstResponder() -> Bool {
    let resigned = super.resignFirstResponder()
    if resigned { coordinator?.parent.onFocusChange(false) }
    return resigned
  }

  // MARK: Keys

  override func keyDown(with event: NSEvent) {
    let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    // ⌘↩ doesn't arrive as insertNewline; route it the same way.
    if flags.contains(.command), event.keyCode == 36 || event.keyCode == 76 {
      _ = coordinator?.textView(self, doCommandBy: #selector(NSResponder.insertNewline(_:)))
      return
    }
    if flags.subtracting(.numericPad).subtracting(.function) == .command, event.keyCode == 126,
      coordinator?.parent.onKey(.selectBlock) == true
    {
      return
    }
    if flags == .control, let characters = event.charactersIgnoringModifiers?.lowercased() {
      if characters == "c", coordinator?.parent.onKey(.controlC) == true { return }
      if characters == "r", coordinator?.parent.onKey(.controlR) == true { return }
    }
    super.keyDown(with: event)
  }

  // Copied files paste as their paths and an image as the path of a PNG
  // saved for it (agents read both); anything else as plain text, newlines
  // kept.
  override func paste(_ sender: Any?) {
    let pasteboard = NSPasteboard.general
    let shell = coordinator?.parent.shellSyntax == true
    if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true])
      as? [URL], !urls.isEmpty
    {
      insertText(urls.map { shell ? $0.path.shellEscaped : $0.path }.joined(separator: " "), replacementRange: selectedRange())
      return
    }
    if pasteboard.string(forType: .string) == nil, let path = Self.savePastedImage(from: pasteboard) {
      insertText(shell ? path.shellEscaped : path, replacementRange: selectedRange())
      return
    }
    pasteAsPlainText(sender)
  }

  /// Write a pasted image to a PNG in the caches folder; its path.
  static func savePastedImage(from pasteboard: NSPasteboard) -> String? {
    guard let image = NSImage(pasteboard: pasteboard), let tiff = image.tiffRepresentation,
      let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
    else { return nil }
    let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
      ?? FileManager.default.temporaryDirectory
    let folder = caches.appendingPathComponent("Impulse/Pasted Images", isDirectory: true)
    try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    let formatter = DateFormatter()
    formatter.dateFormat = "yyyy-MM-dd 'at' HH.mm.ss"
    let url = folder.appendingPathComponent("Pasted image \(formatter.string(from: Date())).png")
    return (try? png.write(to: url)) != nil ? url.path : nil
  }

  // MARK: Highlighting

  /// Color the command from the shell parser's tokens.
  func highlight() {
    guard let storage = textStorage else { return }
    let text = storage.string
    let whole = NSRange(location: 0, length: (text as NSString).length)
    storage.beginEditing()
    storage.setAttributes(
      [.font: font ?? NSFont.monospacedSystemFont(ofSize: 13, weight: .regular), .foregroundColor: colors.text],
      range: whole)
    if shellSyntax, !text.isEmpty {
      let parsed = parseShellInput(text, cursor: text.utf8.count)
      let offsets = Self.utf16Offsets(text)
      func range(_ span: TextSpan) -> NSRange? {
        guard span.start >= 0, span.end <= offsets.count - 1, span.start < span.end else { return nil }
        let start = offsets[span.start]
        return NSRange(location: start, length: offsets[span.end] - start)
      }
      let cursor = selectedRange().location
      for token in parsed.tokens + parsed.assignments {
        guard let tokenRange = range(token.span) else { continue }
        // A command the shell can't run, once you've moved past it.
        if token.role == .command, !token.quoted,
          cursor < tokenRange.location || cursor > NSMaxRange(tokenRange),
          isKnownCommand?(token.text) == false
        {
          storage.addAttributes(
            [
              .underlineStyle: NSUnderlineStyle.single.rawValue | NSUnderlineStyle.patternDash.rawValue,
              .underlineColor: colors.error,
              .toolTip: "\(token.text): command not found",
            ], range: tokenRange)
        }
        let color: NSColor?
        switch token.role {
        case .command: color = colors.command
        case .assignment: color = colors.variable
        case .pipelineSeparator, .controlOperator, .redirectionOperator: color = colors.operatorColor
        case .redirectionTarget, .argument:
          if token.quoted {
            color = colors.string
          } else if token.text.hasPrefix("-") {
            color = colors.flag
          } else if token.text.hasPrefix("$") {
            color = colors.variable
          } else {
            color = nil
          }
        }
        if let color { storage.addAttribute(.foregroundColor, value: color, range: tokenRange) }
      }
      for redirect in parsed.redirects {
        if let operatorRange = range(redirect.operatorSpan) {
          storage.addAttribute(.foregroundColor, value: colors.operatorColor, range: operatorRange)
        }
      }
    }
    storage.endEditing()
    typingAttributes = [
      .font: font ?? NSFont.monospacedSystemFont(ofSize: 13, weight: .regular), .foregroundColor: colors.text,
    ]
  }

  /// UTF-8 byte offset → UTF-16 offset (the parser's spans are bytes).
  static func utf16Offsets(_ text: String) -> [Int] {
    var offsets: [Int] = []
    offsets.reserveCapacity(text.utf8.count + 1)
    var utf16 = 0
    for scalar in text.unicodeScalars {
      let bytes = String(scalar).utf8.count
      for _ in 0..<bytes { offsets.append(utf16) }
      utf16 += scalar.utf16.count
    }
    offsets.append(utf16)
    return offsets
  }

  // MARK: Ghost text and placeholder

  override func draw(_ dirtyRect: NSRect) {
    super.draw(dirtyRect)
    let attributes: [NSAttributedString.Key: Any] = [
      .font: font ?? NSFont.monospacedSystemFont(ofSize: 13, weight: .regular),
      .foregroundColor: colors.ghost,
    ]
    if string.isEmpty {
      guard !placeholder.isEmpty, !hasMarkedText() else { return }
      NSAttributedString(string: placeholder, attributes: attributes).draw(at: .zero)
      return
    }
    // The suggestion continues where the text ends, when the caret is there.
    guard let ghost, !ghost.isEmpty, isAtEnd, !hasMarkedText(), let layoutManager, let textContainer
    else { return }
    let length = (string as NSString).length
    let glyphIndex = layoutManager.glyphIndexForCharacter(at: length - 1)
    var lineRect = layoutManager.lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: nil)
    let glyphRect = layoutManager.boundingRect(
      forGlyphRange: NSRange(location: glyphIndex, length: 1), in: textContainer)
    if string.hasSuffix("\n") {
      lineRect = layoutManager.extraLineFragmentUsedRect
    }
    let origin = NSPoint(
      x: string.hasSuffix("\n") ? lineRect.minX : glyphRect.maxX, y: lineRect.minY)
    let firstLine = ghost.split(separator: "\n", omittingEmptySubsequences: false).first.map(String.init) ?? ghost
    NSAttributedString(string: firstLine, attributes: attributes).draw(at: origin)
  }

  override func didChangeText() {
    super.didChangeText()
    needsDisplay = true
  }

  override func setSelectedRanges(
    _ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool
  ) {
    super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
    needsDisplay = true
  }
}
