#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  struct PasteTests {
    @Test func bracketedPasteCannotBeEndedEarly() {
      // Removing only the end marker would rejoin these into ESC[201~.
      let poisoned = "\u{1b}[2\u{1b}[200~01~\nrm -rf ~\n"
      let body = TerminalTab.bracketedPasteBody(poisoned)
      #expect(!body.contains("\u{1b}"))
      #expect(body == "[2[200~01~\nrm -rf ~\n")
      #expect(TerminalTab.bracketedPasteBody("a\tb\r\nc\u{9b}201~\u{7}") == "a\tb\r\nc201~")
      #expect(TerminalTab.bracketedPasteBody("naïve 日本 🚀") == "naïve 日本 🚀")
    }

    @Test func aSentLineEndsEachLineWithOneReturn() {
      #expect(TerminalTab.typedLine("") == "\r")
      #expect(TerminalTab.typedLine(" secret ") == " secret \r")
      #expect(TerminalTab.typedLine("a\nb") == "a\rb\r")
      #expect(TerminalTab.typedLine("a\r\nb\r\n") == "a\rb\r\r")
    }

    @Test func pastedImagesBecomePNGFiles() throws {
      let pasteboard = NSPasteboard(name: NSPasteboard.Name("impulse-test-\(UUID().uuidString)"))
      defer { pasteboard.releaseGlobally() }
      let image = NSImage(size: NSSize(width: 4, height: 4))
      image.lockFocus()
      NSColor.red.setFill()
      NSRect(x: 0, y: 0, width: 4, height: 4).fill()
      image.unlockFocus()
      pasteboard.clearContents()
      #expect(pasteboard.writeObjects([image]))

      let path = try #require(CommandTextView.savePastedImage(from: pasteboard))
      defer { try? FileManager.default.removeItem(atPath: path) }
      #expect(path.hasSuffix(".png"))
      let data = try #require(FileManager.default.contents(atPath: path))
      #expect(data.prefix(4) == Data([0x89, 0x50, 0x4E, 0x47]), "PNG signature")

      pasteboard.clearContents()
      pasteboard.setString("just text", forType: .string)
      #expect(CommandTextView.savePastedImage(from: pasteboard) == nil)
    }
  }
#endif
