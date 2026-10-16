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
  var handler: ((ControlRequest, ControlCaller, @escaping (ControlResponse) -> Void) -> Void)?

  private var listenSource: DispatchSourceRead?
  private let queue = DispatchQueue(label: "impulse.control")
  fileprivate static let log = OSLog(subsystem: "dev.impulse.Impulse", category: "Control")

  /// Start listening. Snapshot runs use a private temporary socket so they
  /// never take over a running app's.
  func start() {
    guard listenSource == nil else { return }
    var path =
      AppState.persistenceEnabled
      ? AppPaths.dataDirectory.appendingPathComponent("impulse.sock").path
      : NSTemporaryDirectory() + "impulse-\(getpid()).sock"
    if Self.isServing(path) {
      // Another copy of the app answers there: leave its socket alone, and
      // give this one's terminals a socket of their own.
      os_log(.info, log: Self.log, "Impulse is already listening on %{public}@", path)
      path = NSTemporaryDirectory() + "impulse-\(getpid()).sock"
    }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return }
    // Shells started later mustn't inherit the listener (or clients).
    _ = fcntl(fd, F_SETFD, FD_CLOEXEC)
    guard var address = Self.address(path) else {
      os_log(.error, log: Self.log, "Socket path too long: %{public}@", path)
      close(fd)
      return
    }
    // A leftover socket from a previous run would make bind fail (one
    // that still answers was ruled out above).
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
    var info = stat()
    let identity = stat(path, &info) == 0 ? (info.st_dev, info.st_ino) : nil
    _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
    socketPath = path

    let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
    source.setEventHandler { [weak self] in self?.acceptConnections(fd) }
    source.setCancelHandler {
      close(fd)
      // Only if it's still ours: another instance may have taken the path
      // since.
      var now = stat()
      if let identity, stat(path, &now) == 0, now.st_dev == identity.0, now.st_ino == identity.1 {
        unlink(path)
      }
    }
    listenSource = source
    source.resume()
  }

  func stop() {
    listenSource?.cancel()
    listenSource = nil
    socketPath = nil
  }

  private static func address(_ path: String) -> sockaddr_un? {
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else { return nil }
    withUnsafeMutableBytes(of: &address.sun_path) { buffer in
      for (index, byte) in bytes.enumerated() { buffer[index] = byte }
    }
    return address
  }

  /// Whether a server answers at `path` (a stale socket refuses).
  private static func isServing(_ path: String) -> Bool {
    guard var address = address(path) else { return false }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { close(fd) }
    let connected = withUnsafePointer(to: &address) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
      }
    }
    return connected == 0
  }

  private func acceptConnections(_ listener: Int32) {
    while true {
      let client = accept(listener, nil, nil)
      guard client >= 0 else { return }
      _ = fcntl(client, F_SETFD, FD_CLOEXEC)
      // Only the same user can connect (the socket is 0600), but check the
      // peer anyway.
      var uid: uid_t = 0
      var gid: gid_t = 0
      if getpeereid(client, &uid, &gid) != 0 || uid != getuid() {
        close(client)
        continue
      }
      var noSigPipe: Int32 = 1
      setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))
      // A client that stops reading can hold up only its own reply.
      var timeout = timeval(tv_sec: 5, tv_usec: 0)
      setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
      var pid: pid_t = 0
      var length = socklen_t(MemoryLayout<pid_t>.size)
      let peer = getsockopt(client, SOL_LOCAL, LOCAL_PEERPID, &pid, &length) == 0 && pid > 0 ? pid : nil
      ControlConnection(fd: client, caller: ControlCaller(pid: peer)).start()
    }
  }
}

/// Who sent a request, as far as the socket can tell.
final class ControlCaller {
  /// The connected process (LOCAL_PEERPID), when the kernel said.
  let pid: pid_t?
  /// Runs on the main queue when the client hangs up before it's answered
  /// (a waiting `impulse edit` or `tasks wait` stopped with ⌃C). Main
  /// thread only.
  var onHangUp: (() -> Void)?

  init(pid: pid_t?) {
    self.pid = pid
  }

  /// Whether the caller is `ancestor` or was started from it, directly or
  /// through other processes. A process that left the tree (a tmux server,
  /// a daemon) isn't.
  func descends(from ancestor: pid_t) -> Bool {
    guard var current = pid, ancestor > 1 else { return false }
    // Real trees are far shallower; this only bounds the walk.
    for _ in 0..<64 {
      if current == ancestor { return true }
      guard current > 1, let parent = Self.parent(of: current), parent != current else { return false }
      current = parent
    }
    return false
  }

  private static func parent(of pid: pid_t) -> pid_t? {
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
    guard sysctl(&mib, u_int(mib.count), &info, &size, nil, 0) == 0, size > 0 else { return nil }
    return info.kp_eproc.e_ppid
  }
}

/// One client, on its own queue so a slow or silent one holds up no one
/// else: reads the request line, hands it to the main queue, answers once,
/// and notices when the client goes away while it waits.
final class ControlConnection {
  /// Seconds a client has to send its request line (tests lengthen it).
  nonisolated(unsafe) static var requestDeadline: TimeInterval = 5

  private let fd: Int32
  private let caller: ControlCaller
  private let queue = DispatchQueue(label: "impulse.control.client")
  private var source: DispatchSourceRead?
  private var received = Data()
  private var requested = false
  private var finished = false

  init(fd: Int32, caller: ControlCaller) {
    self.fd = fd
    self.caller = caller
  }

  /// Keeps itself alive (through its read source) until it's finished.
  func start() {
    _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
    let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
    source.setEventHandler { [self] in readAvailable() }
    source.setCancelHandler { [fd] in Darwin.close(fd) }
    self.source = source
    source.resume()
    // Requests are one short line: a client that hasn't sent one by now
    // isn't going to.
    queue.asyncAfter(deadline: .now() + Self.requestDeadline) { [self] in
      if !requested { send(ControlResponse(ok: false, message: "impulse: malformed request")) }
    }
  }

  private func readAvailable() {
    var buffer = [UInt8](repeating: 0, count: 4096)
    let count = read(fd, &buffer, buffer.count)
    if count < 0, errno == EAGAIN || errno == EINTR { return }
    guard count > 0 else {
      // The client hung up (or the socket failed).
      if requested { hangUp() } else { finish() }
      return
    }
    // After the request, nothing else is expected.
    guard !requested else { return }
    received.append(contentsOf: buffer[0..<count])
    guard let newline = received.firstIndex(of: 0x0A) else {
      if received.count >= 1_048_576 { send(ControlResponse(ok: false, message: "impulse: malformed request")) }
      return
    }
    requested = true
    guard let request = try? ControlProtocol.decode(ControlRequest.self, from: received[..<newline]) else {
      return send(ControlResponse(ok: false, message: "impulse: malformed request"))
    }
    received = Data()
    let caller = self.caller
    DispatchQueue.main.async { [self] in
      guard let handler = ControlServer.shared.handler else {
        return reply(ControlResponse(ok: false, message: "Impulse isn't ready yet."))
      }
      handler(request, caller) { [self] response in reply(response) }
    }
  }

  /// Answer from any thread; only the first answer is sent.
  private func reply(_ response: ControlResponse) {
    queue.async { [self] in send(response) }
  }

  private func send(_ response: ControlResponse) {
    guard !finished else { return }
    if let line = try? ControlProtocol.encodeLine(response) {
      // Blocking, so SO_SNDTIMEO bounds a reply the client is slow to read.
      _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) & ~O_NONBLOCK)
      line.withUnsafeBytes { bytes in
        guard let base = bytes.baseAddress else { return }
        var offset = 0
        while offset < bytes.count {
          let written = write(fd, base + offset, bytes.count - offset)
          if written > 0 {
            offset += written
          } else if written < 0, errno == EINTR {
            continue
          } else {
            break
          }
        }
      }
    }
    finish()
    // Answered: what was waiting on a hang-up (and holding on to this
    // connection) can go.
    let caller = self.caller
    DispatchQueue.main.async { caller.onHangUp = nil }
  }

  private func hangUp() {
    guard !finished else { return }
    finish()
    let caller = self.caller
    DispatchQueue.main.async {
      let hangUp = caller.onHangUp
      caller.onHangUp = nil
      hangUp?()
    }
  }

  /// Done with the client: the read source's cancel handler closes it.
  private func finish() {
    guard !finished else { return }
    finished = true
    source?.cancel()
    source = nil
  }
}
