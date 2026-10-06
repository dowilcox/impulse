#if canImport(Testing)
  import Foundation
  @testable import ImpulseApp
  import Testing

  struct TextFileTests {
    private func scratchDirectory() throws -> URL {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("impulse-textfile-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      return dir
    }

    @Test func refusesTextThatIsNotUTF8() throws {
      let dir = try scratchDirectory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let latin1 = dir.appendingPathComponent("latin1.txt")
      try Data([0x63, 0x61, 0x66, 0xE9, 0x0A]).write(to: latin1)  // "café\n" in Latin-1
      #expect(TextFile.read(latin1.path) == nil)
      #expect(TextFile.read(dir.appendingPathComponent("missing.txt").path) == nil)
    }

    @Test func keepsTheByteOrderMark() throws {
      let dir = try scratchDirectory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let file = dir.appendingPathComponent("bom.txt")
      try Data([0xEF, 0xBB, 0xBF] + Array("hello\n".utf8)).write(to: file)
      let read = try #require(TextFile.read(file.path))
      #expect(read == TextFile.Contents(text: "hello\n", bom: true))
      try TextFile.write("bye\n", bom: read.bom, to: file.path)
      #expect(try Data(contentsOf: file) == Data([0xEF, 0xBB, 0xBF] + Array("bye\n".utf8)))
    }

    @Test func writesThroughSymlinksAndKeepsPermissions() throws {
      let dir = try scratchDirectory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let target = dir.appendingPathComponent("AGENTS.md")
      let link = dir.appendingPathComponent("CLAUDE.md")
      try "old\n".write(to: target, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: target.path)
      try FileManager.default.createSymbolicLink(atPath: link.path, withDestinationPath: "AGENTS.md")

      try TextFile.write("new\n", bom: false, to: link.path)
      #expect(try FileManager.default.destinationOfSymbolicLink(atPath: link.path) == "AGENTS.md")
      #expect(try String(contentsOf: target, encoding: .utf8) == "new\n")
      let mode = try FileManager.default.attributesOfItem(atPath: target.path)[.posixPermissions] as? NSNumber
      #expect(mode?.intValue == 0o755)
      #expect(TextFile.stamp(link.path) == TextFile.stamp(target.path))
    }
  }
#endif
