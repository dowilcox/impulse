import Foundation

/// Runs a program to completion and collects its output.
///
/// Unlike `Process`, the child gets its own process group, so a timeout
/// stops everything it started (hooks, ssh, credential helpers, LFS) rather
/// than only the program itself, and it inherits no file descriptors besides
/// its standard streams. Both output pipes are drained while it runs, so a
/// chatty program can't fill a pipe and stall.
public enum ChildProcess {
  public struct Output: Sendable {
    /// Exit status, or 128 + the signal number when a signal ended it.
    public let status: Int32
    public let stdout: Data
    public let stderr: Data
    public let timedOut: Bool
  }

  /// - Parameters:
  ///   - environment: the complete environment (nil: the app's own).
  ///   - stdin: written to the program's standard input, which is otherwise
  ///     /dev/null.
  ///   - timeout: wall-clock limit (nil: none). When it passes, the whole
  ///     process group gets SIGTERM, then SIGKILL five seconds later.
  ///   - onStderr: each chunk of standard error as it arrives (on a
  ///     background queue), e.g. for progress lines.
  public static func run(
    _ executable: String, _ arguments: [String], in directory: String? = nil,
    environment: [String: String]? = nil, stdin: Data? = nil, timeout: TimeInterval? = nil,
    onStderr: ((Data) -> Void)? = nil
  ) throws -> Output {
    var outPipe: [Int32] = [-1, -1]
    var errPipe: [Int32] = [-1, -1]
    var inPipe: [Int32] = [-1, -1]
    var opened: [Int32] = []
    func closeAll() { opened.forEach { close($0) } }
    for (index, pipeFDs) in [stdin != nil, true, true].enumerated() where pipeFDs {
      var fds: [Int32] = [-1, -1]
      guard pipe(&fds) == 0 else {
        closeAll()
        throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
      }
      for fd in fds { _ = fcntl(fd, F_SETFD, FD_CLOEXEC) }
      opened += fds
      switch index {
      case 0: inPipe = fds
      case 1: outPipe = fds
      default: errPipe = fds
      }
    }

    var actions: posix_spawn_file_actions_t? = nil
    posix_spawn_file_actions_init(&actions)
    defer { posix_spawn_file_actions_destroy(&actions) }
    if stdin != nil {
      posix_spawn_file_actions_adddup2(&actions, inPipe[0], STDIN_FILENO)
    } else {
      posix_spawn_file_actions_addopen(&actions, STDIN_FILENO, "/dev/null", O_RDONLY, 0)
    }
    posix_spawn_file_actions_adddup2(&actions, outPipe[1], STDOUT_FILENO)
    posix_spawn_file_actions_adddup2(&actions, errPipe[1], STDERR_FILENO)
    if let directory {
      posix_spawn_file_actions_addchdir(&actions, directory)
    }

    var attributes: posix_spawnattr_t? = nil
    posix_spawnattr_init(&attributes)
    defer { posix_spawnattr_destroy(&attributes) }
    // Its own process group; no inherited descriptors; default signal
    // handling and an empty mask, whatever the app has set.
    let flags = POSIX_SPAWN_SETPGROUP | POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF
      | POSIX_SPAWN_SETSIGMASK
    posix_spawnattr_setflags(&attributes, Int16(flags))
    posix_spawnattr_setpgroup(&attributes, 0)
    var signals = sigset_t()
    sigfillset(&signals)
    posix_spawnattr_setsigdefault(&attributes, &signals)
    sigemptyset(&signals)
    posix_spawnattr_setsigmask(&attributes, &signals)

    let argv: [UnsafeMutablePointer<CChar>?] = ([executable] + arguments).map { strdup($0) } + [nil]
    defer { argv.forEach { free($0) } }
    let environmentStrings = (environment ?? ProcessInfo.processInfo.environment).map { "\($0.key)=\($0.value)" }
    let envp: [UnsafeMutablePointer<CChar>?] = environmentStrings.map { strdup($0) } + [nil]
    defer { envp.forEach { free($0) } }

    var pid: pid_t = 0
    let spawned = posix_spawn(&pid, executable, &actions, &attributes, argv, envp)
    guard spawned == 0 else {
      closeAll()
      throw POSIXError(POSIXErrorCode(rawValue: spawned) ?? .EIO)
    }
    // The child's ends now live in the child only.
    close(outPipe[1])
    close(errPipe[1])
    if stdin != nil { close(inPipe[0]) }

    let drained = DispatchGroup()
    let queue = DispatchQueue(label: "impulse.child-process.output")
    let out = Drain(fd: outPipe[0], queue: queue, group: drained, onChunk: nil)
    let err = Drain(fd: errPipe[0], queue: queue, group: drained, onChunk: onStderr)

    if let stdin {
      let fd = inPipe[1]
      // The program may exit without reading it; the write must fail, not
      // raise SIGPIPE in the app.
      _ = fcntl(fd, F_SETNOSIGPIPE, 1)
      DispatchQueue.global(qos: .userInitiated).async {
        stdin.withUnsafeBytes { buffer in
          var offset = 0
          while offset < buffer.count {
            let written = write(fd, buffer.baseAddress! + offset, buffer.count - offset)
            if written > 0 {
              offset += written
            } else if written < 0 && errno == EINTR {
              continue
            } else {
              break
            }
          }
        }
        close(fd)
      }
    }

    let exited = DispatchSemaphore(value: 0)
    let waitStatus = WaitStatus()
    DispatchQueue.global(qos: .userInitiated).async {
      var status: Int32 = 0
      while waitpid(pid, &status, 0) == -1 && errno == EINTR {}
      waitStatus.value = status
      exited.signal()
    }

    var timedOut = false
    if let timeout, exited.wait(timeout: .now() + timeout) == .timedOut {
      timedOut = true
      kill(-pid, SIGTERM)
      if exited.wait(timeout: .now() + 5) == .timedOut {
        kill(-pid, SIGKILL)
        exited.wait()
      }
    } else if timeout == nil {
      exited.wait()
    }

    // Output ends when every holder of the pipes has exited. Something that
    // left the process group (a daemonized ssh master) can keep them open;
    // don't wait for it.
    if drained.wait(timeout: .now() + (timedOut ? 1 : 5)) == .timedOut {
      queue.sync {
        out.stop()
        err.stop()
      }
    }
    let (stdout, stderr) = queue.sync { (out.data, err.data) }
    return Output(status: decode(waitStatus.value), stdout: stdout, stderr: stderr, timedOut: timedOut)
  }

  /// Exit code, or 128 + signal for a program a signal ended.
  static func decode(_ status: Int32) -> Int32 {
    let signal = status & 0x7f
    return signal == 0 ? (status >> 8) & 0xff : 128 + signal
  }

  private final class WaitStatus: @unchecked Sendable {
    var value: Int32 = 0
  }

  /// Reads a pipe until it closes, on `queue`.
  private final class Drain {
    private(set) var data = Data()
    private let source: DispatchSourceRead
    private let fd: Int32

    init(fd: Int32, queue: DispatchQueue, group: DispatchGroup, onChunk: ((Data) -> Void)?) {
      self.fd = fd
      _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
      source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
      group.enter()
      source.setEventHandler { [weak self] in
        guard let self else { return }
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
          let count = read(fd, &buffer, buffer.count)
          if count > 0 {
            let chunk = Data(buffer[0..<count])
            self.data.append(chunk)
            onChunk?(chunk)
          } else if count < 0 && errno == EINTR {
            continue
          } else if count < 0 && errno == EAGAIN {
            return
          } else {
            self.source.cancel()  // end of file, or an error
            return
          }
        }
      }
      source.setCancelHandler {
        close(fd)
        group.leave()
      }
      source.resume()
    }

    /// Stop reading (call on the drain's queue).
    func stop() {
      if !source.isCancelled { source.cancel() }
    }
  }
}
