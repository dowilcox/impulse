import Darwin
import Foundation

/// A TCP port some process under a terminal is listening on.
struct ListeningPort: Hashable, Comparable {
  let port: Int
  let pid: pid_t
  /// The listening process's name ("node", "python3.12").
  let process: String

  static func < (a: ListeningPort, b: ListeningPort) -> Bool { a.port < b.port }
}

/// Finds listening sockets of a set of processes with libproc (no lsof,
/// no privileges needed for the user's own processes).
enum PortScanner {
  static func listeningPorts(of pids: [pid_t]) -> [ListeningPort] {
    var found: [Int: ListeningPort] = [:]
    for pid in pids {
      for port in listeningPorts(of: pid) where found[port] == nil {
        found[port] = ListeningPort(port: port, pid: pid, process: processName(pid))
      }
    }
    return found.values.sorted()
  }

  private static func listeningPorts(of pid: pid_t) -> [Int] {
    let size = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, nil, 0)
    guard size > 0 else { return [] }
    let count = Int(size) / MemoryLayout<proc_fdinfo>.stride
    var fds = [proc_fdinfo](repeating: proc_fdinfo(), count: count)
    let filled = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, &fds, size)
    guard filled > 0 else { return [] }
    var ports: [Int] = []
    for fd in fds.prefix(Int(filled) / MemoryLayout<proc_fdinfo>.stride)
    where fd.proc_fdtype == PROX_FDTYPE_SOCKET {
      var info = socket_fdinfo()
      let bytes = proc_pidfdinfo(
        pid, fd.proc_fd, PROC_PIDFDSOCKETINFO, &info, Int32(MemoryLayout<socket_fdinfo>.size))
      guard bytes == Int32(MemoryLayout<socket_fdinfo>.size),
        info.psi.soi_kind == Int32(SOCKINFO_TCP),
        info.psi.soi_proto.pri_tcp.tcpsi_state == TSI_S_LISTEN
      else { continue }
      let raw = info.psi.soi_proto.pri_tcp.tcpsi_ini.insi_lport
      let port = Int(UInt16(bigEndian: UInt16(truncatingIfNeeded: raw)))
      if port > 0 { ports.append(port) }
    }
    return ports
  }

  private static func processName(_ pid: pid_t) -> String {
    var buffer = [CChar](repeating: 0, count: 256)
    let length = proc_name(pid, &buffer, UInt32(buffer.count))
    guard length > 0 else { return "" }
    return String(decoding: buffer.prefix(Int(length)).map { UInt8(bitPattern: $0) }, as: UTF8.self)
  }
}
