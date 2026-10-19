// The client side of the control socket: one request line out, one
// response line back, on a connection of its own. `impulse` sends one
// request and exits; `impulse mcp` sends one per tool call, possibly
// several at once, and hangs up on a call the agent cancels (which stops a
// waiting request in the app, as ⌃C does for `impulse tasks wait`).

#if canImport(Darwin)
  import Darwin
#else
  import Glibc
#endif
import Foundation

public final class ControlClient: @unchecked Sendable {
  public enum Failure: Error, Equatable {
    /// The socket couldn't be reached: Impulse quit, or this isn't its
    /// socket.
    case unreachable
    /// The connection closed (or the client hung up) before an answer.
    case noAnswer
  }

  private let socketPath: String
  private let lock = NSLock()
  private var fd: Int32 = -1
  private var cancelled = false

  public init(socketPath: String) {
    self.socketPath = socketPath
  }

  /// Send `request` and wait for its answer (an `edit` or a `tasks wait`
  /// waits here until it completes). Call once per client.
  public func send(_ request: ControlRequest) -> Result<ControlResponse, Failure> {
    let socket = Self.dial(socketPath)
    guard socket >= 0 else { return .failure(.unreachable) }
    lock.lock()
    if cancelled {
      lock.unlock()
      close(socket)
      return .failure(.noAnswer)
    }
    fd = socket
    lock.unlock()
    defer {
      lock.lock()
      fd = -1
      lock.unlock()
      close(socket)
    }
    guard let line = try? ControlProtocol.encodeLine(request) else { return .failure(.noAnswer) }
    var sent = 0
    let ok = line.withUnsafeBytes { bytes -> Bool in
      guard let base = bytes.baseAddress else { return false }
      while sent < bytes.count {
        let written = write(socket, base + sent, bytes.count - sent)
        if written > 0 {
          sent += written
        } else if written < 0, errno == EINTR {
          continue
        } else {
          return false
        }
      }
      return true
    }
    guard ok else { return .failure(.noAnswer) }
    var response = Data()
    var buffer = [UInt8](repeating: 0, count: 4096)
    reading: while true {
      let count = read(socket, &buffer, buffer.count)
      if count < 0, errno == EINTR { continue }
      guard count > 0 else { break }
      for byte in buffer[0..<count] {
        if byte == 0x0A { break reading }
        response.append(byte)
      }
    }
    guard let reply = try? ControlProtocol.decode(ControlResponse.self, from: response) else {
      return .failure(.noAnswer)
    }
    return .success(reply)
  }

  /// Hang up: a request waiting for its answer gets `.noAnswer`, and the
  /// app stops what it was waiting for. Safe from any thread, before or
  /// during `send`.
  public func cancel() {
    lock.lock()
    defer { lock.unlock() }
    cancelled = true
    if fd >= 0 { shutdown(fd, Int32(SHUT_RDWR)) }
  }

  private static func dial(_ path: String) -> Int32 {
    #if canImport(Darwin)
      let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    #else
      let fd = socket(AF_UNIX, Int32(SOCK_STREAM.rawValue), 0)
    #endif
    guard fd >= 0 else { return -1 }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
      close(fd)
      return -1
    }
    withUnsafeMutableBytes(of: &address.sun_path) { buffer in
      for (index, byte) in bytes.enumerated() { buffer[index] = byte }
    }
    let connected = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
      }
    }
    guard connected == 0 else {
      close(fd)
      return -1
    }
    #if canImport(Darwin)
      // A write to a socket the app closed mustn't kill the tool.
      var noSigPipe: Int32 = 1
      setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))
    #endif
    return fd
  }
}
