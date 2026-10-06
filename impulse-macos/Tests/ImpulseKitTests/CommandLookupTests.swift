#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct CommandLookupTests {
    private func lookup() -> CommandLookup {
      CommandLookup { directory in
        switch directory {
        case "/bin": return ["ls", "cat"]
        case "/opt/tools": return ["rg"]
        default: return []
        }
      }
    }

    @Test func unsureUntilTheShellReports() {
      let commands = lookup()
      #expect(commands.isKnown("gti", cwd: nil) == nil)
      #expect(commands.isKnown("cd", cwd: nil) == true, "builtins everyone has")
    }

    @Test func namesAndPathExecutables() {
      let commands = lookup()
      commands.setNames(["gst", "myfn"])
      #expect(commands.isKnown("gst", cwd: nil) == true)
      #expect(commands.isKnown("ls", cwd: nil) == nil, "PATH not listed yet")
      commands.setPath("/bin:/opt/tools")
      commands.waitForScan()
      #expect(commands.isKnown("ls", cwd: nil) == true)
      #expect(commands.isKnown("rg", cwd: nil) == true)
      #expect(commands.isKnown("\\ls", cwd: nil) == true, "a backslash only skips aliases")
      #expect(commands.isKnown("gti", cwd: nil) == false)
    }

    @Test func aPathChangeDuringAScanIsPickedUp() {
      let started = DispatchSemaphore(value: 0)
      let release = DispatchSemaphore(value: 0)
      let commands = CommandLookup { directory in
        if directory == "/bin" {
          started.signal()
          release.wait()
          return ["ls"]
        }
        return directory == "/opt/tools" ? ["rg"] : []
      }
      commands.setPath("/bin")
      started.wait()
      commands.setPath("/opt/tools")  // arrives while /bin is being listed
      release.signal()
      commands.waitForScan()
      commands.waitForScan()
      #expect(commands.isKnown("rg", cwd: nil) == true)
      #expect(commands.isKnown("ls", cwd: nil) == false)
    }

    @Test func quotedAndExpandedWordsAreLeftAlone() {
      let commands = lookup()
      commands.setPath("/bin")
      commands.waitForScan()
      #expect(commands.isKnown("$EDITOR", cwd: nil) == nil)
      #expect(commands.isKnown("\"ls\"", cwd: nil) == nil)
      #expect(commands.isKnown("ls*", cwd: nil) == nil)
    }

    @Test func pathsAreCheckedOnDisk() throws {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("lookup-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: dir) }
      let script = dir.appendingPathComponent("run.sh")
      try "#!/bin/sh\n".write(to: script, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
      try "".write(to: dir.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)

      let commands = lookup()
      #expect(commands.isKnown("./run.sh", cwd: dir.path) == true)
      #expect(commands.isKnown(script.path, cwd: nil) == true)
      #expect(commands.isKnown("./notes.txt", cwd: dir.path) == false, "not executable")
      #expect(commands.isKnown("./missing", cwd: dir.path) == false)
      #expect(commands.isKnown("./run.sh", cwd: nil) == nil, "relative without a directory")
    }
  }
#endif
