#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct ChildProcessTests {
    private func sh(_ script: String, stdin: Data? = nil, timeout: TimeInterval? = nil) throws -> ChildProcess.Output {
      try ChildProcess.run("/bin/sh", ["-c", script], stdin: stdin, timeout: timeout)
    }

    @Test func collectsOutputAndStatus() throws {
      let output = try sh("echo out; echo err >&2; exit 3")
      #expect(String(decoding: output.stdout, as: UTF8.self) == "out\n")
      #expect(String(decoding: output.stderr, as: UTF8.self) == "err\n")
      #expect(output.status == 3)
      #expect(!output.timedOut)
    }

    @Test func aSignalEndingTheProgramShowsInTheStatus() throws {
      #expect(try sh("kill -TERM $$").status == 128 + SIGTERM)
    }

    @Test func feedsStandardInput() throws {
      let input = Data((0..<200_000).map { UInt8($0 % 251) })
      let output = try ChildProcess.run("/bin/cat", [], stdin: input)
      #expect(output.stdout == input)
    }

    @Test func standardInputIsEmptyOtherwise() throws {
      #expect(try ChildProcess.run("/bin/cat", [], timeout: 5).stdout.isEmpty)
    }

    @Test func lotsOfOutputOnBothStreamsDoesNotStall() throws {
      let output = try sh("head -c 300000 /dev/zero; head -c 300000 /dev/zero >&2", timeout: 20)
      #expect(output.stdout.count == 300_000)
      #expect(output.stderr.count == 300_000)
      #expect(!output.timedOut)
    }

    @Test func runsInTheGivenDirectory() throws {
      let dir = FileManager.default.temporaryDirectory.appendingPathComponent("child-\(UUID().uuidString)")
      try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: dir) }
      let output = try ChildProcess.run("/bin/pwd", ["-P"], in: dir.path)
      let expected = String(cString: realpath(dir.path, nil))
      #expect(String(decoding: output.stdout, as: UTF8.self) == expected + "\n")
    }

    @Test func aTimeoutStopsEverythingTheProgramStarted() throws {
      let started = Date()
      // The background sleep is a grandchild; killing only sh would leave it.
      let output = try sh("sleep 30 & echo $!; wait", timeout: 0.5)
      #expect(output.timedOut)
      #expect(Date().timeIntervalSince(started) < 5)
      let grandchild = pid_t(String(decoding: output.stdout, as: UTF8.self)
        .trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
      #expect(grandchild > 0)
      #expect(kill(grandchild, 0) == -1 && errno == ESRCH, "the grandchild is gone too")
    }

    @Test func aDaemonHoldingThePipeDoesNotHoldUpTheResult() throws {
      let started = Date()
      // perl leaves the process group and keeps stdout open after sh exits.
      let output = try sh("perl -e 'setpgrp(0, 0); sleep 8' & echo started")
      #expect(String(decoding: output.stdout, as: UTF8.self) == "started\n")
      #expect(Date().timeIntervalSince(started) < 7.5)
    }

    @Test func aMissingProgramThrows() {
      #expect(throws: (any Error).self) { try ChildProcess.run("/nonexistent/tool", []) }
    }

    @Test func standardErrorStreamsAsItArrives() throws {
      let chunks = Chunks()
      _ = try ChildProcess.run(
        "/bin/sh", ["-c", "echo one >&2; sleep 0.2; echo two >&2"], timeout: 5,
        onStderr: { chunks.append($0) })
      #expect(String(decoding: chunks.joined, as: UTF8.self) == "one\ntwo\n")
      #expect(chunks.count >= 2)
    }

    private final class Chunks: @unchecked Sendable {
      private let lock = NSLock()
      private var items: [Data] = []
      func append(_ data: Data) { lock.withLock { items.append(data) } }
      var count: Int { lock.withLock { items.count } }
      var joined: Data { lock.withLock { items.reduce(Data(), +) } }
    }
  }
#endif
