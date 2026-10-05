import Darwin
import Foundation
import ImpulseProtocol
import os.log

/// Listens on a Unix socket for the `impulse` command-line tool. Each
/// connection carries one request line and gets one response line; requests
/// that wait (`impulse edit`) keep their connection open until the action
/// completes. Requests are handled on the main queue by `handler`.
final class ControlServer {
  static let shared = ControlServer()

  /// Where the socket lives (nil until started).
  private(set) var socketPath: String?
  /// Answers a request; call `reply` exactly once (possibly later).
  var handler: ((ControlRequest, @escaping (ControlResponse) -> Void) -> Void)?

  private var listenSource: DispatchSourceRead?
  private let queue = DispatchQueue(label: "impulse.control")
  private static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "Control")

  /// Start listening. Snapshot runs use a private temporary socket so they
  /// never take over a running app's.
  func start() {
    guard listenSource == nil else { return }
    let path =
      AppState.persistenceEnabled
      ? AppPaths.dataDirectory.appendingPathComponent("impulse.sock").path
      : NSTemporaryDirectory() + "impulse-\(getpid()).sock"
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
      os_log(.error, log: Self.log, "Socket path too long: %{public}@", path)
      close(fd)
      return
    }
    withUnsafeMutableBytes(of: &address.sun_path) { buffer in
      for (index, byte) in bytes.enumerated() { buffer[index] = byte }
    }
    // A leftover socket from a previous run would make bind fail.
    unlink(path)
    let bound = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
      }
    }
    guard bound == 0, listen(fd, 16) == 0 else {
      os_log(.error, log: Self.log, "Couldn't listen on %{public}@: %{public}d", path, errno)
      close(fd)
      return
    }
    chmod(path, 0o600)
    _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
    socketPath = path

    let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
    source.setEventHandler { [weak self] in self?.acceptConnections(fd) }
    source.setCancelHandler {
      close(fd)
      unlink(path)
    }
    listenSource = source
    source.resume()
  }

  func stop() {
    listenSource?.cancel()
    listenSource = nil
    socketPath = nil
  }

  private func acceptConnections(_ listener: Int32) {
    while true {
      let client = accept(listener, nil, nil)
      guard client >= 0 else { return }
      // Only the same user can connect (the socket is 0600), but check the
      // peer anyway.
      var uid: uid_t = 0
      var gid: gid_t = 0
      if getpeereid(client, &uid, &gid) != 0 || uid != getuid() {
        close(client)
        continue
      }
      // Blocking reads with a timeout, so a silent client can't stall us.
      _ = fcntl(client, F_SETFL, fcntl(client, F_GETFL) & ~O_NONBLOCK)
      var timeout = timeval(tv_sec: 5, tv_usec: 0)
      setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
      var noSigPipe: Int32 = 1
      setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))
      readRequest(from: client)
    }
  }

  private func readRequest(from client: Int32) {
    var data = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    // Requests are small; read until the newline (or a size cap).
    while data.count < 1_048_576 {
      let count = read(client, &buffer, buffer.count)
      if count <= 0 { break }
      data.append(contentsOf: buffer[0..<count])
      if buffer[0..<count].contains(0x0A) { break }
    }
    guard let newline = data.firstIndex(of: 0x0A),
      let request = try? ControlProtocol.decode(ControlRequest.self, from: data[..<newline])
    else {
      respond(client, ControlResponse(ok: false, message: "impulse: malformed request"))
      return
    }
    DispatchQueue.main.async { [weak self] in
      guard let self, let handler = self.handler else {
        self?.respond(client, ControlResponse(ok: false, message: "Impulse isn't ready yet."))
        return
      }
      var answered = false
      handler(request) { [weak self] response in
        guard !answered else { return }
        answered = true
        self?.queue.async { self?.respond(client, response) }
      }
    }
  }

  private func respond(_ client: Int32, _ response: ControlResponse) {
    if let line = try? ControlProtocol.encodeLine(response) {
      _ = line.withUnsafeBytes { write(client, $0.baseAddress, line.count) }
    }
    close(client)
  }
}
