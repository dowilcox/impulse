// What Impulse needs from a Docker Compose file, read line by line (Compose
// files keep these parts simple; there's no YAML library here): each
// service's `container_name`, `image`, published `ports` and `volumes`.
// From those, the project setup screen suggests ports and a database to
// clone, warns about what keeps two checkouts' stacks from running at once,
// and each new task gets an override file that renames the containers and
// moves the fixed ports by its slot, so the project's own file never has to
// change.

import Foundation

public struct ComposeFile: Equatable, Sendable {
  public struct Port: Equatable, Sendable {
    /// As written: `"8000:8000"`, `"${APP_PORT:-8000}:8000"`.
    public let raw: String
    /// A fixed host port (`8000` in `8000:8000` or `127.0.0.1:8000:8000`).
    public let host: Int?
    /// The variable a host port is read from, with its default
    /// (`APP_PORT`, 8000 in `${APP_PORT:-8000}:8000`).
    public let variable: String?
    public let variableDefault: Int?
  }

  public struct Volume: Equatable, Sendable {
    /// As written: `./docker/data/mysql:/var/lib/mysql`, `/app/vendor`.
    public let raw: String
    /// The host side of a bind mount (`./docker/data/mysql`), nil otherwise.
    public let hostPath: String?
    /// The path in the container.
    public let target: String
    /// A container path with no source: an anonymous volume, which hides the
    /// host's folder from the container.
    public let isAnonymous: Bool
  }

  public struct Service: Equatable, Sendable {
    public let name: String
    public var containerName: String?
    public var image: String?
    public var ports: [Port] = []
    public var volumes: [Volume] = []
  }

  public var services: [Service] = []

  /// The names Compose looks for, in its order.
  public static let fileNames = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"]

  /// The Compose file in `folder`, if any: its name and contents.
  public static func find(in folder: String) -> (name: String, text: String)? {
    for name in fileNames {
      if let text = try? String(contentsOfFile: (folder as NSString).appendingPathComponent(name), encoding: .utf8) {
        return (name, text)
      }
    }
    return nil
  }

  public static func parse(_ text: String) -> ComposeFile {
    var file = ComposeFile()
    var inServices = false
    var serviceIndent: Int?
    var keyIndent: Int?
    var listKey: String?
    var current: Service?

    func finish() {
      if let current { file.services.append(current) }
      current = nil
      keyIndent = nil
      listKey = nil
    }

    for rawLine in text.components(separatedBy: "\n") {
      let line = stripComment(rawLine)
      guard !line.trimmingCharacters(in: .whitespaces).isEmpty else { continue }
      let indent = line.prefix(while: { $0 == " " }).count
      let content = line.trimmingCharacters(in: .whitespaces)
      if indent == 0 {
        finish()
        inServices = content == "services:"
        serviceIndent = nil
        continue
      }
      guard inServices else { continue }
      if serviceIndent == nil { serviceIndent = indent }
      if indent == serviceIndent, content.hasSuffix(":") {
        finish()
        current = Service(name: unquote(String(content.dropLast())))
        continue
      }
      guard current != nil, let serviceIndent, indent > serviceIndent else { continue }
      if keyIndent == nil { keyIndent = indent }
      if indent == keyIndent {
        listKey = nil
        guard let colon = content.firstIndex(of: ":") else { continue }
        let key = String(content[..<colon])
        let value = content[content.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        switch key {
        case "container_name": current?.containerName = unquote(value)
        case "image": current?.image = unquote(value)
        case "ports", "volumes":
          if value.hasPrefix("[") {
            for item in flowItems(value) { add(item, to: key, of: &current) }
          } else if value.isEmpty {
            listKey = key
          }
        default: break
        }
      } else if let listKey, content.hasPrefix("- ") {
        add(String(content.dropFirst(2)), to: listKey, of: &current)
      }
    }
    finish()
    return file
  }

  private static func add(_ item: String, to key: String, of service: inout Service?) {
    let value = unquote(item.trimmingCharacters(in: .whitespaces))
    // The long syntax (`- target: 80`) isn't read; it's left alone.
    guard !value.isEmpty, !value.contains(": ") else { return }
    if key == "ports" {
      service?.ports.append(port(value))
    } else {
      service?.volumes.append(volume(value))
    }
  }

  static func port(_ raw: String) -> Port {
    // [ip:]host:container[/protocol]; the host part may be a variable.
    if let match = raw.range(of: #"^\$\{([A-Za-z_][A-Za-z0-9_]*)(:?-([0-9]+))?\}:"#, options: .regularExpression) {
      let head = String(raw[match])
      let name = head.dropFirst(2).prefix(while: { $0.isLetter || $0.isNumber || $0 == "_" })
      let digits = head.contains("-") ? head.split(separator: "-").last.map { $0.filter(\.isNumber) } : nil
      return Port(raw: raw, host: nil, variable: String(name), variableDefault: digits.flatMap { Int($0) })
    }
    let parts = raw.split(separator: "/").first.map { $0.split(separator: ":").map(String.init) } ?? []
    let host = parts.count >= 2 ? Int(parts[parts.count - 2]) : nil
    return Port(raw: raw, host: host, variable: nil, variableDefault: nil)
  }

  static func volume(_ raw: String) -> Volume {
    let parts = raw.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
    if parts.count == 1 { return Volume(raw: raw, hostPath: nil, target: raw, isAnonymous: true) }
    let source = parts[0]
    let isBind = source.hasPrefix(".") || source.hasPrefix("/") || source.hasPrefix("~") || source.hasPrefix("$")
    return Volume(raw: raw, hostPath: isBind ? source : nil, target: parts[1], isAnonymous: false)
  }

  /// A line without its `# comment` (a `#` inside quotes stays).
  private static func stripComment(_ line: String) -> String {
    var quote: Character?
    var previous: Character = " "
    for (offset, character) in line.enumerated() {
      if let open = quote {
        if character == open { quote = nil }
      } else if character == "\"" || character == "'" {
        quote = character
      } else if character == "#", previous == " " || offset == 0 {
        return String(line.prefix(offset))
      }
      previous = character
    }
    return line
  }

  private static func unquote(_ text: String) -> String {
    let trimmed = text.trimmingCharacters(in: .whitespaces)
    guard trimmed.count >= 2, let first = trimmed.first, first == trimmed.last, first == "\"" || first == "'" else {
      return trimmed
    }
    return String(trimmed.dropFirst().dropLast())
  }

  private static func flowItems(_ value: String) -> [String] {
    value.trimmingCharacters(in: CharacterSet(charactersIn: "[] ")).split(separator: ",").map(String.init)
  }

  // MARK: - A task's override

  /// The override file that lets a task's stack run beside the main
  /// checkout's: containers renamed (`app` → `app-fix-elevation`) and fixed
  /// host ports moved by `slot` × `offset`. Ports read from variables are
  /// kept: the task's env file moves them. Nil when nothing needs
  /// overriding. Needs Compose 2.24 or later (`!override`).
  public func override(task: String, slot: Int, offset: Int) -> String? {
    var lines: [String] = []
    for service in services {
      var body: [String] = []
      if let name = service.containerName { body.append("    container_name: \(name)-\(task)") }
      if service.ports.contains(where: { $0.host != nil }) {
        body.append("    ports: !override")
        for port in service.ports {
          body.append("      - \"\(port.host.map { Self.moving(port.raw, from: $0, by: slot * offset) } ?? port.raw)\"")
        }
      }
      guard !body.isEmpty else { continue }
      lines.append("  \(service.name):")
      lines += body
    }
    guard !lines.isEmpty else { return nil }
    return """
      # Written by Impulse for the task \(task) (slot \(slot)), so its stack runs
      # beside the main checkout's. Deleted when the task is archived.
      services:
      \(lines.joined(separator: "\n"))

      """
  }

  /// `8000:8000` → `8100:8000`, `127.0.0.1:8000:8000` → `127.0.0.1:8100:8000`.
  private static func moving(_ raw: String, from host: Int, by amount: Int) -> String {
    var parts = raw.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
    let index = parts.count - 2
    guard index >= 0 else { return raw }
    parts[index] = String(host + amount)
    return parts.joined(separator: ":")
  }
}
