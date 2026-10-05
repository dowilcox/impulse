import Darwin
import Foundation

/// Executable path and arguments of a running process.
struct ProcessDetails: Equatable {
  let pid: pid_t
  let executablePath: String
  /// argv, including argv[0].
  let arguments: [String]
}

enum ProcessInspector {
  static func details(of pid: pid_t) -> ProcessDetails? {
    guard pid > 0 else { return nil }
    var pathBuffer = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
    let length = proc_pidpath(pid, &pathBuffer, UInt32(pathBuffer.count))
    let path =
      length > 0
      ? String(decoding: pathBuffer.prefix(Int(length)).map { UInt8(bitPattern: $0) }, as: UTF8.self)
      : ""
    let arguments = Self.arguments(of: pid) ?? []
    guard !path.isEmpty || !arguments.isEmpty else { return nil }
    return ProcessDetails(pid: pid, executablePath: path, arguments: arguments)
  }

  /// argv from `KERN_PROCARGS2`: an argc int, the exec path, NUL padding,
  /// then argc NUL-terminated strings (environment follows).
  static func arguments(of pid: pid_t) -> [String]? {
    var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
    var size = 0
    guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > MemoryLayout<Int32>.size else {
      return nil
    }
    var buffer = [UInt8](repeating: 0, count: size)
    guard sysctl(&mib, 3, &buffer, &size, nil, 0) == 0 else { return nil }
    return parseProcArgs(Array(buffer.prefix(size)))
  }

  static func parseProcArgs(_ buffer: [UInt8]) -> [String]? {
    let intSize = MemoryLayout<Int32>.size
    guard buffer.count > intSize else { return nil }
    let argc = buffer.withUnsafeBytes { Int($0.load(as: Int32.self)) }
    guard argc > 0, argc < 4096 else { return nil }
    var index = intSize
    // Skip the exec path and the padding after it.
    while index < buffer.count, buffer[index] != 0 { index += 1 }
    while index < buffer.count, buffer[index] == 0 { index += 1 }
    var arguments: [String] = []
    while arguments.count < argc, index < buffer.count {
      let start = index
      while index < buffer.count, buffer[index] != 0 { index += 1 }
      arguments.append(String(decoding: buffer[start..<index], as: UTF8.self))
      index += 1
    }
    return arguments
  }
}
