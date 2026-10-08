// A task's own environment: the values that have to differ between the main
// checkout and each task so their dev stacks don't collide (ports, database
// names), worked out from the project settings and the task's slot, and
// written into the task's copy of its dotenv file. The file is the source of
// truth: the app in its container, `docker compose` from any terminal and
// Vite all read it.

import Darwin
import Foundation

public enum TaskEnvironment {
  /// One value to write: `APP_PORT=8100`.
  public struct Value: Equatable, Sendable {
    public let key: String
    public let value: String

    public init(_ key: String, _ value: String) {
      self.key = key
      self.value = value
    }
  }

  /// The ports for `slot`: each of the main checkout's ports plus
  /// `slot` × `offset`. Slot 0 is the main checkout.
  public static func ports(_ base: [String: Int], slot: Int, offset: Int) -> [String: Int] {
    base.mapValues { $0 + slot * offset }
  }

  /// `template` with `{task}` (the task's folder name), `{task_}` (the same
  /// with dashes as underscores, for database names), `{slot}` and each
  /// port's `{NAME}` filled in. Unknown placeholders are left as they are.
  public static func expand(_ template: String, task: String, slot: Int, ports: [String: Int]) -> String {
    var result = template
    result = result.replacingOccurrences(of: "{task_}", with: underscored(task))
    result = result.replacingOccurrences(of: "{task}", with: task)
    result = result.replacingOccurrences(of: "{slot}", with: String(slot))
    for (name, port) in ports {
      result = result.replacingOccurrences(of: "{\(name)}", with: String(port))
    }
    return result
  }

  /// A task's values: its ports, then the `[worktrees.env]` values, each
  /// sorted by name.
  public static func values(config: ProjectConfig, task: String, slot: Int) -> [Value] {
    let ports = ports(config.ports, slot: slot, offset: config.portOffset)
    return ports.keys.sorted().map { Value($0, String(ports[$0]!)) }
      + config.worktreeEnv.keys.sorted().map {
        Value($0, expand(config.worktreeEnv[$0]!, task: task, slot: slot, ports: ports))
      }
  }

  /// `text` (a dotenv file) with `values` set: a key already in it is
  /// changed in place (every line that sets it), a missing one is added at
  /// the end under `comment`. Everything else stays as it is.
  public static func applying(_ values: [Value], to text: String, comment: String) -> String {
    var lines = text.isEmpty ? [] : text.components(separatedBy: "\n")
    if lines.last == "" { lines.removeLast() }
    var missing: [Value] = []
    for value in values {
      var found = false
      for index in lines.indices where key(of: lines[index]) == value.key {
        let export = lines[index].trimmingCharacters(in: .whitespaces).hasPrefix("export ") ? "export " : ""
        lines[index] = "\(export)\(value.key)=\(quoted(value.value))"
        found = true
      }
      if !found { missing.append(value) }
    }
    if !missing.isEmpty {
      if let last = lines.last, !last.trimmingCharacters(in: .whitespaces).isEmpty { lines.append("") }
      lines.append("# \(comment)")
      lines += missing.map { "\($0.key)=\(quoted($0.value))" }
    }
    return lines.joined(separator: "\n") + "\n"
  }

  /// The key a dotenv line sets (`KEY=…` or `export KEY=…`), nil for
  /// comments and blank lines.
  static func key(of line: String) -> String? {
    var text = line.trimmingCharacters(in: .whitespaces)
    guard !text.isEmpty, !text.hasPrefix("#") else { return nil }
    if text.hasPrefix("export ") { text = String(text.dropFirst(7)).trimmingCharacters(in: .whitespaces) }
    guard let equals = text.firstIndex(of: "=") else { return nil }
    let key = text[..<equals].trimmingCharacters(in: .whitespaces)
    return key.isEmpty ? nil : key
  }

  /// A value as dotenv needs it: bare when it's simple, double-quoted when
  /// it has spaces, `#`, quotes or `$`.
  static func quoted(_ value: String) -> String {
    guard value.contains(where: { " \t#\"'$\\".contains($0) }) else { return value }
    let escaped = value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
    return "\"\(escaped)\""
  }

  /// `interia-upgrade` → `interia_upgrade`; anything but letters, digits
  /// and underscores becomes an underscore.
  static func underscored(_ name: String) -> String {
    String(name.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") ? $0 : "_" })
  }
}

/// Whether a TCP port is free on this Mac: nothing can be bound to it on
/// any address, as a dev server or a container's published port would be.
public enum PortProbe {
  public static func isFree(_ port: Int) -> Bool {
    guard (1...65535).contains(port) else { return false }
    return canBind(port, address: INADDR_ANY) && canBind(port, address: INADDR_LOOPBACK)
  }

  private static func canBind(_ port: Int, address: UInt32) -> Bool {
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { close(fd) }
    var addr = sockaddr_in()
    addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = in_port_t(UInt16(port).bigEndian)
    addr.sin_addr = in_addr(s_addr: address.bigEndian)
    return withUnsafePointer(to: &addr) {
      $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0
      }
    }
  }
}
