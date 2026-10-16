#if canImport(Testing)
  import AppKit
  @testable import ImpulseApp
  import Testing

  struct TerminalRendererGeometryTests {
    @Test func rowRangeCoversFullBounds() {
      let rect = NSRect(x: 0, y: 0, width: 800, height: 4 + 40 * 17)
      let range = TerminalRenderer.rowRange(
        intersecting: rect, lines: 40, padding: 4, cellHeight: 17)
      #expect(range == 0..<40)
    }

    @Test func rowRangeForSingleRowRect() {
      // Row 10 occupies y in [4 + 170, 4 + 187).
      let rect = NSRect(x: 0, y: 4 + 170, width: 800, height: 17)
      let range = TerminalRenderer.rowRange(
        intersecting: rect, lines: 40, padding: 4, cellHeight: 17)
      #expect(range.contains(10))
      // May include a clipped neighbour at the boundary, but never more.
      #expect(range.lowerBound >= 10 && range.upperBound <= 12)
    }

    @Test func rowRangeClampsAboveAndBelow() {
      let above = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: -100, width: 800, height: 50),
        lines: 40, padding: 4, cellHeight: 17)
      #expect(above.lowerBound == 0)

      let below = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 10_000, width: 800, height: 50),
        lines: 40, padding: 4, cellHeight: 17)
      #expect(below.isEmpty)
    }

    @Test func rowRangeHandlesDegenerateInput() {
      let zeroLines = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 0, width: 10, height: 10),
        lines: 0, padding: 4, cellHeight: 17)
      #expect(zeroLines.isEmpty)

      let zeroCell = TerminalRenderer.rowRange(
        intersecting: NSRect(x: 0, y: 0, width: 10, height: 10),
        lines: 5, padding: 4, cellHeight: 0)
      #expect(zeroCell == 0..<5)
    }

    // MARK: IME marked text

    @Test func markedTextTakesTheCellsTheGridWill() {
      #expect("日本".map(TerminalRenderer.cellWidth(of:)) == [2, 2])
      #expect("ka".map(TerminalRenderer.cellWidth(of:)) == [1, 1])
      #expect(TerminalRenderer.cellWidth(of: "한") == 2)
      #expect(TerminalRenderer.cellWidth(of: "　") == 2, "ideographic space")
      #expect(TerminalRenderer.cellWidth(of: "é") == 1)

      // The candidate window goes under the range the input method asks about.
      let third = TerminalRenderer.markedTextCells("にほんご", range: NSRange(location: 2, length: 1))
      #expect(third.offset == 4 && third.width == 2)
      #expect(third.range == NSRange(location: 2, length: 1))
      let caret = TerminalRenderer.markedTextCells("", range: NSRange(location: 0, length: 0))
      #expect(caret.offset == 0 && caret.width == 0)
      let past = TerminalRenderer.markedTextCells("aか", range: NSRange(location: 1, length: 9))
      #expect(past.offset == 1 && past.width == 2 && past.range == NSRange(location: 1, length: 1))
      let unknown = TerminalRenderer.markedTextCells("aか", range: NSRange(location: NSNotFound, length: 0))
      #expect(unknown.offset == 0)
    }

    // MARK: Mouse reports

    @Test func mouseReportsInSGRAndNormalEncoding() {
      #expect(
        TerminalRenderer.mouseReport(button: 0, col: 0, row: 0, release: false, sgr: true)
          == Array("\u{1B}[<0;1;1M".utf8))
      #expect(
        TerminalRenderer.mouseReport(button: 0, col: 9, row: 4, release: true, sgr: true)
          == Array("\u{1B}[<0;10;5m".utf8))
      // Wheel up at column 5, row 3, without SGR: button 64 + 32.
      #expect(
        TerminalRenderer.mouseReport(button: 64, col: 4, row: 2, release: false, sgr: false)
          == [0x1B, 0x5B, 0x4D, 96, 37, 35])
      // A release is button 3; columns past 223 can't be told apart.
      #expect(
        TerminalRenderer.mouseReport(button: 4, col: 300, row: 0, release: true, sgr: false)
          == [0x1B, 0x5B, 0x4D, 35, 255, 33])
    }

    @MainActor
    @Test func shiftOrOptionSelectsDespiteMouseReporting() {
      func click(_ flags: NSEvent.ModifierFlags) -> NSEvent {
        NSEvent.mouseEvent(
          with: .leftMouseDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: 0,
          context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
      }
      #expect(TerminalRenderer.selectsDespiteMouseReporting(click(.shift)))
      #expect(TerminalRenderer.selectsDespiteMouseReporting(click(.option)))
      #expect(!TerminalRenderer.selectsDespiteMouseReporting(click([])))
      #expect(!TerminalRenderer.selectsDespiteMouseReporting(click(.control)))
    }

    // MARK: File references

    @Test func absolutePathsAreWorkedOutFromTheText() {
      #expect(TerminalRenderer.absolutePath("src/../main.rs", in: "/Users/me/app") == "/Users/me/app/main.rs")
      #expect(TerminalRenderer.absolutePath("./a//b/./c.txt", in: "/tmp") == "/tmp/a/b/c.txt")
      #expect(TerminalRenderer.absolutePath("/etc/../var/log", in: "/x") == "/var/log")
      #expect(TerminalRenderer.absolutePath("../../../..", in: "/a") == "/")
      #expect(TerminalRenderer.absolutePath("~/notes.md", in: "/x") == NSHomeDirectory() + "/notes.md")
    }

    @Test func networkMountsAndVolumesAreNotLookedUpFromHover() {
      for path in [
        "/net/build-host/src/x.c", "/Network/Servers/nas/a", "/Volumes/Backup/file.txt", "/volumes/backup",
        "/Volumes", "/System/Volumes/Data/Volumes/Backup/a",
      ] {
        #expect(TerminalRenderer.isOnNetworkOrVolume(path), "\(path)")
      }
      for path in ["/Users/me/Volumes/a", "/networking/x", "/tmp/net/a", "/System/Volumes/Data/Users/me/a"] {
        #expect(!TerminalRenderer.isOnNetworkOrVolume(path), "\(path)")
      }
    }

    /// Paths on a volume are looked up like local ones once its mount
    /// point is known to be a disk on this Mac.
    @Test func volumesAreLocalDisksOrShares() {
      #expect(TerminalRenderer.volumeMountPoint("/Volumes/Work/app/src/main.rs") == "/Volumes/Work")
      #expect(TerminalRenderer.volumeMountPoint("/volumes/Work") == "/volumes/Work")
      #expect(TerminalRenderer.volumeMountPoint("/Volumes") == nil)
      #expect(TerminalRenderer.volumeMountPoint("/net/host/a") == nil)
      #expect(TerminalRenderer.volumeMountPoint("/Users/me/Volumes/a") == nil)
      #expect(TerminalRenderer.isLocalMount("/"))
      #expect(!TerminalRenderer.isLocalMount("/Volumes/no-such-volume-\(UUID().uuidString)"))
    }
  }
#endif
