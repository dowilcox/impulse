#if canImport(Testing)
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct CommandHistoryTests {
    private func database() throws -> CommandHistoryDatabase {
      try #require(CommandHistoryDatabase(path: ":memory:"))
    }

    private func at(_ seconds: TimeInterval) -> Date { Date(timeIntervalSince1970: seconds) }

    @Test func recordsDistinctCommandsNewestFirst() throws {
      let db = try database()
      db.insert([
        HistoryEntry(command: "git status", cwd: "/a", exitCode: 0, startedAt: at(100)),
        HistoryEntry(command: "make", cwd: "/a", exitCode: 2, startedAt: at(200)),
        HistoryEntry(command: "git status", cwd: "/b", exitCode: 0, startedAt: at(300)),
        HistoryEntry(command: " secret --token x", startedAt: at(400)),
        HistoryEntry(command: "   ", startedAt: at(500)),
      ])
      #expect(db.count == 3)
      let hits = db.recent(limit: 10)
      #expect(hits.map(\.entry.command) == ["git status", "make"])
      // The most recent run's details, and how often it ran.
      #expect(hits[0].entry.cwd == "/b")
      #expect(hits[0].uses == 2)
      #expect(hits[1].entry.exitCode == 2)
    }

    @Test func filtersByTextDirectoryFailureAndTime() throws {
      let db = try database()
      db.insert([
        HistoryEntry(command: "cargo test", cwd: "/repo", repo: "/repo", exitCode: 101, startedAt: at(100)),
        HistoryEntry(command: "cargo build", cwd: "/repo/sub", repo: "/repo", exitCode: 0, startedAt: at(200)),
        HistoryEntry(command: "ls -la", cwd: "/tmp", exitCode: 0, startedAt: at(300)),
      ])
      #expect(db.recent(matching: "CARGO", limit: 10).map(\.entry.command) == ["cargo build", "cargo test"])
      #expect(db.recent(filter: HistoryFilter(cwd: "/tmp"), limit: 10).map(\.entry.command) == ["ls -la"])
      #expect(db.recent(filter: HistoryFilter(repo: "/repo"), limit: 10).count == 2)
      #expect(db.recent(filter: HistoryFilter(failedOnly: true), limit: 10).map(\.entry.command) == ["cargo test"])
      #expect(db.recent(filter: HistoryFilter(since: at(250)), limit: 10).map(\.entry.command) == ["ls -la"])
      #expect(db.recent(limit: 1).count == 1)
    }

    @Test func prunesOldestRows() throws {
      let db = try database()
      db.insert((0..<10).map { HistoryEntry(command: "cmd \($0)", startedAt: at(Double($0))) })
      db.prune(keeping: 3)
      #expect(db.count == 3)
      #expect(db.recent(limit: 10).map(\.entry.command) == ["cmd 9", "cmd 8", "cmd 7"])
    }

    @Test func persistsAcrossConnections() throws {
      let path = FileManager.default.temporaryDirectory
        .appendingPathComponent("impulse-history-\(UUID().uuidString).sqlite3").path
      defer {
        for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: path + suffix) }
      }
      do {
        let db = try #require(CommandHistoryDatabase(path: path))
        db.insert([HistoryEntry(command: "echo kept", startedAt: at(1))])
      }
      let reopened = try #require(CommandHistoryDatabase(path: path))
      #expect(reopened.recent(limit: 5).map(\.entry.command) == ["echo kept"])
    }

    @Test func importsZshHistory() {
      var data = Data(": 1700000000:0;git log\n: 1700000100:3;echo one \\\n two\nplain\n".utf8)
      // A metafied "é" (0xC3 0xA9 → 0xC3 0x83 0x89).
      data.append(contentsOf: [0x65, 0x63, 0x68, 0x6F, 0x20, 0xC3, 0x83, 0xA9 ^ 32, 0x0A])
      let entries = ShellHistoryImport.zsh(data)
      #expect(entries.map(\.command) == ["git log", "echo one \n two", "plain", "echo é"])
      #expect(entries[0].startedAt == at(1_700_000_000))
      #expect(entries[2].startedAt == .distantPast)
    }

    @Test func importsBashAndFishHistory() {
      let bash = ShellHistoryImport.bash("#1700000000\nls\n pwd\nmake test\n")
      #expect(bash.map(\.command) == ["ls", "make test"])
      #expect(bash[0].startedAt == at(1_700_000_000))

      let fish = ShellHistoryImport.fish(
        "- cmd: git status\n  when: 1700000000\n- cmd: echo a\\nb \\\\ c\n  when: 1700000005\n  paths:\n    - a\n")
      #expect(fish.map(\.command) == ["git status", "echo a\nb \\ c"])
      #expect(fish[1].startedAt == at(1_700_000_005))
    }
  }
#endif
