// What the project setup screen proposes for a repository, worked out from
// what's in it: the ignored files and folders a fresh checkout lacks, the
// ports and per-task values in its Compose and env files, scripts from its
// lock files and package scripts, and a database whose data folder can be
// cloned. Everything here is a suggestion for the user to accept or edit.

import Foundation

public struct ProjectSuggestions: Equatable, Sendable {
  public struct Entry: Equatable, Sendable {
    public let path: String
    /// Ticked when the screen opens.
    public let suggested: Bool
    /// Why it isn't ticked, when it isn't.
    public let note: String?

    public init(_ path: String, suggested: Bool, note: String? = nil) {
      self.path = path
      self.suggested = suggested
      self.note = note
    }
  }

  /// A host port the Compose file writes as a plain number (`"8080:80"`).
  public struct FixedPort: Equatable, Sendable {
    public let service: String
    public let port: ComposeFile.Port
  }

  /// A service's `container_name`.
  public struct ContainerName: Equatable, Sendable {
    public let service: String
    public let name: String
  }

  /// Ignored files to copy into tasks.
  public var copies: [Entry] = []
  /// Ignored folders to clone into tasks.
  public var clones: [Entry] = []
  /// Ports by name with the main checkout's value.
  public var ports: [String: Int] = [:]
  /// Values that have to differ per task.
  public var values: [String: String] = [:]
  /// What keeps two checkouts' stacks from running at once, which a task's
  /// Compose override moves and renames: fixed host ports and
  /// `container_name`.
  public var fixedPorts: [FixedPort] = []
  public var containerNames: [ContainerName] = []
  public var setup: String?
  public var archive: String?
  public var check: String?
  public var actions: [ProjectConfig.Action] = []
  /// What to run when files change in a pull, merge or checkout.
  public var onChange: [String: String] = [:]
  /// A database whose data is a folder of the project.
  public var databaseFolder: String?
  public var databaseService: String?
  /// The Compose file, when there is one.
  public var composeFileName: String?
}

public enum ProjectDetector {
  /// Folders worth cloning when they're ignored.
  static let cloneCandidates = [
    "vendor", "node_modules", "public/build", "dist", "build", ".venv", "venv", "target", "Pods", "storage/app",
  ]

  /// Lock files and the install command each implies.
  public static let installCommands: [(lockFile: String, command: String)] = [
    ("package-lock.json", "npm ci"),
    ("pnpm-lock.yaml", "pnpm install --frozen-lockfile"),
    ("yarn.lock", "yarn install --immutable"),
    ("bun.lock", "bun install --frozen-lockfile"),
    ("bun.lockb", "bun install --frozen-lockfile"),
    ("composer.lock", "composer install"),
    ("Gemfile.lock", "bundle install"),
    ("uv.lock", "uv sync"),
    ("poetry.lock", "poetry install"),
  ]

  /// Database images and where they keep their data.
  static let databaseDataPaths: [(image: String, target: String)] = [
    ("mysql", "/var/lib/mysql"), ("mariadb", "/var/lib/mysql"), ("postgres", "/var/lib/postgresql/data"),
    ("postgis", "/var/lib/postgresql/data"), ("mongo", "/data/db"),
  ]

  /// Names that usually hold a database name in an env file.
  static let databaseNameKeys = ["DB_DATABASE", "DB_NAME", "DATABASE_NAME", "POSTGRES_DB", "MYSQL_DATABASE"]

  /// Suggestions for the repository at `root`; `ignored` is what git
  /// ignores there (folders end in "/", `GitOperations.ignoredEntries`).
  public static func suggest(root: String, ignored: [String]) -> ProjectSuggestions {
    var suggestions = ProjectSuggestions()
    let fm = FileManager.default
    func exists(_ relative: String) -> Bool { fm.fileExists(atPath: (root as NSString).appendingPathComponent(relative)) }
    func read(_ relative: String) -> String? {
      try? String(contentsOfFile: (root as NSString).appendingPathComponent(relative), encoding: .utf8)
    }

    let compose = ComposeFile.find(in: root).map { ($0.name, ComposeFile.parse($0.text)) }
    suggestions.composeFileName = compose?.0
    let services = compose?.1.services ?? []

    // Folders the containers can't see from the host: a bind mount of the
    // project plus an anonymous volume under it (`.:/var/www` and
    // `/var/www/vendor`).
    var hiddenFromContainers = Set<String>()
    for service in services {
      for bind in service.volumes where bind.hostPath == "." || bind.hostPath == "./" {
        for anonymous in service.volumes where anonymous.isAnonymous && anonymous.target.hasPrefix(bind.target + "/") {
          hiddenFromContainers.insert(String(anonymous.target.dropFirst(bind.target.count + 1)))
        }
      }
    }

    // Files and folders a fresh checkout lacks.
    for entry in ignored.prefix(500) {
      if entry.hasSuffix("/") {
        let folder = String(entry.dropLast())
        guard cloneCandidates.contains(folder) else { continue }
        if hiddenFromContainers.contains(folder) {
          suggestions.clones.append(.init(folder, suggested: false, note: "Compose hides it from the containers (an anonymous volume)"))
        } else if folder == "node_modules", exists("package-lock.json") {
          suggestions.clones.append(.init(folder, suggested: false, note: "npm ci deletes it first; use npm install to keep a clone"))
        } else {
          suggestions.clones.append(.init(folder, suggested: true))
        }
      } else if entry.split(separator: "/").count <= 2 {
        let name = (entry as NSString).lastPathComponent
        let isEnv = name == ".env" || name.hasPrefix(".env.")
        suggestions.copies.append(.init(entry, suggested: isEnv && !name.hasSuffix(".example") && name != ".env.testing"))
      }
    }

    // Ports: variables in the Compose file, then `_PORT` values in .env
    // that are ports on this Mac. Beside `DB_HOST=mysql`, `DB_PORT` is the
    // port inside the Compose network (or on another machine), which a
    // task's stack keeps.
    for service in services {
      for port in service.ports {
        if let variable = port.variable, let value = port.variableDefault {
          suggestions.ports[variable] = value
        } else if port.host != nil {
          suggestions.fixedPorts.append(.init(service: service.name, port: port))
        }
      }
      if let name = service.containerName {
        suggestions.containerNames.append(.init(service: service.name, name: name))
      }
    }
    let env = read(".env").map(parseEnv) ?? [:]
    for (key, value) in env where key.hasSuffix("_PORT") && suggestions.ports[key] == nil {
      let host = env[String(key.dropLast("_PORT".count)) + "_HOST"]
      guard host.map(isThisMac) ?? true, let port = Int(value), (1...65535).contains(port) else { continue }
      suggestions.ports[key] = port
    }

    // A database whose data is a folder of the project.
    for service in services {
      guard let image = service.image?.lowercased(),
        let known = databaseDataPaths.first(where: { image.hasPrefix($0.image) || image.contains("/\($0.image)") }),
        let volume = service.volumes.first(where: { $0.target == known.target && $0.hostPath != nil }),
        let host = volume.hostPath, host.hasPrefix("./"), host.count > 2
      else { continue }
      suggestions.databaseFolder = String(host.dropFirst(2))
      suggestions.databaseService = service.name
      break
    }

    // Per-task values: a database name, unless each task runs its own
    // database (Compose has one), and an app URL on a task port.
    if suggestions.databaseService == nil, !services.contains(where: { isDatabase($0) }) {
      for key in databaseNameKeys {
        if let value = env[key], !value.isEmpty { suggestions.values[key] = "\(value)_{task_}" }
      }
    }
    if let urlKey = ["APP_URL", "BASE_URL", "SITE_URL"].first(where: { env[$0] != nil }),
      let portKey = ["APP_PORT", "PORT", "WEB_PORT", "HTTP_PORT"].first(where: { suggestions.ports[$0] != nil })
    {
      suggestions.values[urlKey] = "http://localhost:{\(portKey)}"
    }

    // Scripts.
    let installs = installCommands.filter { exists($0.lockFile) }.map(\.command)
    var setup = compose == nil ? [] : ["docker compose up -d"]
    for install in installs where !setup.contains(install) { setup.append(install) }
    suggestions.setup = setup.isEmpty ? nil : setup.joined(separator: " && ")
    suggestions.archive = compose == nil ? nil : "docker compose down -v"
    for (lockFile, command) in installCommands where exists(lockFile) { suggestions.onChange[lockFile] = command }
    if compose != nil, exists("Dockerfile") {
      // Anonymous volumes survive a rebuild: renew them, or the old
      // dependencies stay.
      suggestions.onChange["Dockerfile"] = "docker compose build && docker compose up -d --renew-anon-volumes"
    }

    let npmScripts = read("package.json").flatMap(scripts) ?? []
    let composerScripts = read("composer.json").flatMap(scripts) ?? []
    var check: [String] = []
    for name in ["typecheck", "type-check", "check-types", "lint"] where npmScripts.contains(name) {
      check.append("npm run \(name)")
    }
    if npmScripts.contains("test") { check.append("npm test") }
    if composerScripts.contains("test") { check.append("composer test") }
    suggestions.check = check.isEmpty ? nil : check.joined(separator: " && ")

    suggestions.actions =
      npmScripts.prefix(12).map { ProjectConfig.Action(name: $0, command: "npm run \($0)") }
      + composerScripts.prefix(6).filter { !$0.hasPrefix("pre-") && !$0.hasPrefix("post-") }
      .map { ProjectConfig.Action(name: "composer \($0)", command: "composer \($0)") }
      + (read("Makefile").map(makeTargets) ?? []).prefix(8).map {
        ProjectConfig.Action(name: "make \($0)", command: "make \($0)")
      }
    return suggestions
  }

  /// For a new task's setup script: wait until its own database (Compose
  /// `service`) is up, then load a dump of the main checkout's into it.
  /// The task's terminal has `IMPULSE_REPO_ROOT`; credentials come from the
  /// image's own variables, inside the containers. Nil for other databases.
  public static func dumpAndLoad(service: String, image: String) -> String? {
    let image = image.lowercased()
    let exec = "docker compose exec -T \(service)"
    if image.hasPrefix("mysql") || image.hasPrefix("mariadb") || image.contains("/mysql") || image.contains("/mariadb") {
      let auth = #"-uroot -p"$MYSQL_ROOT_PASSWORD""#
      return "until \(exec) sh -c 'mysqladmin ping \(auth) --silent'; do sleep 1; done"
        + " && (cd \"$IMPULSE_REPO_ROOT\" && \(exec) sh -c 'mysqldump \(auth) --all-databases --single-transaction')"
        + " | \(exec) sh -c 'mysql \(auth)'"
    }
    if image.hasPrefix("postgres") || image.hasPrefix("postgis") || image.contains("/postgres") {
      return #"until \#(exec) sh -c 'pg_isready -U "$POSTGRES_USER"'; do sleep 1; done"#
        + #" && (cd "$IMPULSE_REPO_ROOT" && \#(exec) sh -c 'pg_dumpall -U "$POSTGRES_USER"')"#
        + #" | \#(exec) sh -c 'psql -U "$POSTGRES_USER"'"#
    }
    return nil
  }

  /// For a new task's setup script: an empty database with the project's
  /// migrations and seed data, when the framework is one Impulse knows.
  public static func freshDatabase(root: String) -> String? {
    let fm = FileManager.default
    if fm.fileExists(atPath: (root as NSString).appendingPathComponent("artisan")) { return "php artisan migrate --seed" }
    if fm.fileExists(atPath: (root as NSString).appendingPathComponent("bin/rails")) { return "bin/rails db:setup" }
    if fm.fileExists(atPath: (root as NSString).appendingPathComponent("manage.py")) { return "python manage.py migrate" }
    return nil
  }

  /// The Compose service running a database, with its image.
  public static func databaseService(in services: [ComposeFile.Service]) -> ComposeFile.Service? {
    services.first(where: isDatabase)
  }

  static func isDatabase(_ service: ComposeFile.Service) -> Bool {
    guard let image = service.image?.lowercased() else { return false }
    return databaseDataPaths.contains { image.hasPrefix($0.image) || image.contains("/\($0.image)") }
  }

  /// Whether a host name in an env file is this Mac.
  static func isThisMac(_ host: String) -> Bool {
    ["localhost", "127.0.0.1", "0.0.0.0", "::1", "[::1]"].contains(host.lowercased())
  }

  /// `KEY=value` pairs of a dotenv file (quotes removed).
  static func parseEnv(_ text: String) -> [String: String] {
    var values: [String: String] = [:]
    for line in text.components(separatedBy: "\n") {
      guard let key = TaskEnvironment.key(of: line), let equals = line.firstIndex(of: "=") else { continue }
      var value = line[line.index(after: equals)...].trimmingCharacters(in: .whitespaces)
      if value.count >= 2, let first = value.first, first == value.last, first == "\"" || first == "'" {
        value = String(value.dropFirst().dropLast())
      }
      values[key] = value
    }
    return values
  }

  /// The names under `"scripts"` in a package.json or composer.json.
  static func scripts(_ json: String) -> [String]? {
    guard let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
      let scripts = object["scripts"] as? [String: Any]
    else { return nil }
    return scripts.keys.sorted()
  }

  /// A Makefile's plain targets (`build:`), not variables, patterns or
  /// special targets.
  static func makeTargets(_ text: String) -> [String] {
    text.components(separatedBy: "\n").compactMap { line in
      guard let first = line.first, first.isLetter || first.isNumber, let colon = line.firstIndex(of: ":") else {
        return nil
      }
      let name = String(line[..<colon])
      guard !name.contains(where: { " %=$.".contains($0) }), !line.dropFirst(name.count).hasPrefix(":=") else {
        return nil
      }
      return name
    }
  }
}
