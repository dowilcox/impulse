#if canImport(Testing)
  import Darwin
  import Foundation
  import ImpulseProtocol
  import Testing

  @testable import ImpulseApp

  /// The control socket with real clients: a silent one doesn't hold up the
  /// rest, a waiting one that hangs up is noticed, and the caller's pid is
  /// known.
  @MainActor
  struct ControlServerTests {
    @Test func clientsAreHandledOnTheirOwn() async throws {
      // A socket in the temporary folder, not the app's own.
      let persistence = AppState.persistenceEnabled
      AppState.persistenceEnabled = false
      let server = ControlServer.shared
      var callers: [ControlCaller] = []
      var hungUp = false
      server.handler = { request, caller, reply in
        callers.append(caller)
        if request.command == "wait" {
          caller.onHangUp = { hungUp = true }
        } else {
          reply(ControlResponse(ok: true, message: request.command))
        }
      }
      // Long enough that only a server held up by the silent client
      // could answer it first, however busy the test run is.
      let deadline = ControlConnection.requestDeadline
      ControlConnection.requestDeadline = 60
      server.start()
      AppState.persistenceEnabled = persistence
      defer {
        server.stop()
        server.handler = nil
        ControlConnection.requestDeadline = deadline
      }
      let path = try #require(server.socketPath)

      // Connected, and saying nothing.
      let silent = try #require(Self.connect(path))
      defer { close(silent) }

      let answer = try await Self.ask(path, ControlRequest(command: "ping"))
      #expect(answer?.message == "ping")
      // Answered while the silent client is still waiting on its deadline.
      var pending = pollfd(fd: silent, events: Int16(POLLIN), revents: 0)
      #expect(poll(&pending, 1, 0) == 0, "not after the silent client times out")
      #expect(callers.first?.pid == getpid())
      #expect(callers.first?.descends(from: getpid()) == true)
      #expect(callers.first?.descends(from: 999_999) == false)

      // A waiting request whose client goes away.
      let waiting = try #require(Self.connect(path))
      let line = try ControlProtocol.encodeLine(ControlRequest(command: "wait"))
      _ = line.withUnsafeBytes { write(waiting, $0.baseAddress, line.count) }
      for _ in 0..<100 where callers.count < 2 { try await Task.sleep(for: .milliseconds(20)) }
      #expect(callers.count == 2)
      close(waiting)
      for _ in 0..<100 where !hungUp { try await Task.sleep(for: .milliseconds(20)) }
      #expect(hungUp)
    }

    private nonisolated static func connect(_ path: String) -> Int32? {
      let fd = socket(AF_UNIX, SOCK_STREAM, 0)
      guard fd >= 0 else { return nil }
      var address = sockaddr_un()
      address.sun_family = sa_family_t(AF_UNIX)
      withUnsafeMutableBytes(of: &address.sun_path) { buffer in
        for (index, byte) in path.utf8.enumerated() { buffer[index] = byte }
      }
      let connected = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
          Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
      }
      guard connected == 0 else {
        close(fd)
        return nil
      }
      var timeout = timeval(tv_sec: 3, tv_usec: 0)
      setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
      return fd
    }

    /// Send a request and read the answer, off the main thread (which
    /// answers it).
    private static func ask(_ path: String, _ request: ControlRequest) async throws -> ControlResponse? {
      let line = try ControlProtocol.encodeLine(request)
      return await Task.detached {
        guard let fd = connect(path) else { return nil }
        defer { close(fd) }
        _ = line.withUnsafeBytes { write(fd, $0.baseAddress, line.count) }
        var response = Data()
        var byte: UInt8 = 0
        while read(fd, &byte, 1) == 1, byte != 0x0A { response.append(byte) }
        return try? ControlProtocol.decode(ControlResponse.self, from: response)
      }.value
    }
  }
#endif
