#if canImport(Testing)
  import Darwin
  import Foundation
  import Testing

  @testable import ImpulseKit

  struct TaskEnvironmentTests {
    private let settings = """
      [worktrees]
      port_offset = 100

      [worktrees.ports]
      APP_PORT = 8000
      VITE_PORT = 5173

      [worktrees.env]
      DB_DATABASE = "pulseboard_{task_}"
      APP_URL = "http://localhost:{APP_PORT}"
      COMPOSE_PROJECT_NAME = "pulseboard-{task}"
      """

    @Test func theSettingsAreRead() throws {
      let config = try ProjectConfig.parse(settings).get()
      #expect(config.portOffset == 100)
      #expect(config.ports == ["APP_PORT": 8000, "VITE_PORT": 5173])
      #expect(config.worktreeEnv["DB_DATABASE"] == "pulseboard_{task_}")
      #expect(config.envFile == ".env", "the default")
      #expect(config.hasTaskValues)
      #expect(config.taskValueNames == ["APP_PORT", "VITE_PORT", "APP_URL", "COMPOSE_PROJECT_NAME", "DB_DATABASE"])
      #expect(!(try ProjectConfig.parse("").get()).hasTaskValues)
    }

    @Test func aLocalFileMergesPortsAndValuesKeyByKey() throws {
      let local = """
        [worktrees]
        env_file = ".env.local"

        [worktrees.ports]
        VITE_PORT = 5200
        REDIS_PORT = 6379
        """
      let config = ProjectConfig.resolve([
        try ProjectConfig.parseLayer(settings).get(), try ProjectConfig.parseLayer(local).get(),
      ])
      #expect(config.ports == ["APP_PORT": 8000, "VITE_PORT": 5200, "REDIS_PORT": 6379])
      #expect(config.worktreeEnv.count == 3)
      #expect(config.envFile == ".env.local")
    }

    @Test func aTasksValuesComeFromItsSlot() throws {
      let config = try ProjectConfig.parse(settings).get()
      let values = TaskEnvironment.values(config: config, task: "interia-upgrade", slot: 1)
      #expect(
        values == [
          .init("APP_PORT", "8100"), .init("VITE_PORT", "5273"),
          .init("APP_URL", "http://localhost:8100"), .init("COMPOSE_PROJECT_NAME", "pulseboard-interia-upgrade"),
          .init("DB_DATABASE", "pulseboard_interia_upgrade"),
        ])
      #expect(TaskEnvironment.ports(config.ports, slot: 0, offset: 100) == config.ports, "slot 0 is the main checkout")
      #expect(TaskEnvironment.expand("db_{slot}_{UNKNOWN}", task: "x", slot: 3, ports: [:]) == "db_3_{UNKNOWN}")
    }

    @Test func valuesAreChangedInPlaceOrAddedAtTheEnd() {
      let text = """
        APP_NAME=PulseBoard
        APP_KEY=base64:secret
        APP_PORT=8000
        # the database
        DB_DATABASE=pulseboard
        export VITE_PORT=5173
        """
      let result = TaskEnvironment.applying(
        [.init("APP_PORT", "8100"), .init("VITE_PORT", "5273"), .init("DB_DATABASE", "pulseboard_x"),
         .init("APP_URL", "http://localhost:8100")],
        to: text + "\n", comment: "Impulse task x")
      #expect(
        result == """
          APP_NAME=PulseBoard
          APP_KEY=base64:secret
          APP_PORT=8100
          # the database
          DB_DATABASE=pulseboard_x
          export VITE_PORT=5273

          # Impulse task x
          APP_URL=http://localhost:8100

          """)
    }

    @Test func aMissingFileGetsOnlyTheValues() {
      #expect(
        TaskEnvironment.applying([.init("APP_PORT", "8100")], to: "", comment: "Impulse task x")
          == "# Impulse task x\nAPP_PORT=8100\n")
    }

    @Test func oddValuesAreQuoted() {
      #expect(TaskEnvironment.quoted("plain_value-1") == "plain_value-1")
      #expect(TaskEnvironment.quoted("two words") == "\"two words\"")
      #expect(TaskEnvironment.quoted("a\"b") == "\"a\\\"b\"")
      #expect(TaskEnvironment.key(of: "  export  KEY = value") == "KEY")
      #expect(TaskEnvironment.key(of: "# KEY=value") == nil)
      #expect(TaskEnvironment.underscored("feat-search.ui") == "feat_search_ui")
    }

    @Test func aBoundPortIsNotFree() throws {
      let fd = socket(AF_INET, SOCK_STREAM, 0)
      defer { close(fd) }
      var addr = sockaddr_in()
      addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
      addr.sin_family = sa_family_t(AF_INET)
      addr.sin_port = 0
      addr.sin_addr = in_addr(s_addr: INADDR_LOOPBACK.bigEndian)
      let bound = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
      }
      #expect(bound == 0)
      #expect(listen(fd, 1) == 0)
      var length = socklen_t(MemoryLayout<sockaddr_in>.size)
      _ = withUnsafeMutablePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) }
      }
      let port = Int(UInt16(bigEndian: addr.sin_port))
      #expect(!PortProbe.isFree(port))
      #expect(!PortProbe.isFree(0))
    }
  }
#endif
