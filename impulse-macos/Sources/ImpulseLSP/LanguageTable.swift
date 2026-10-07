// Port of `LspConfig` / `LspServerConfig` from impulse-core/src/lsp.rs: the
// language -> server table, user-override loading (trusted global config,
// untrusted project-local config), workspace root detection, and the default
// workspace settings / initialization options for known servers.

import Foundation

public struct LSPServerConfig {
  public var command: String
  public var args: [String]
  /// JSON value (dictionary/array/scalar) or nil.
  public var initializationOptions: Any?

  public init(command: String, args: [String] = [], initializationOptions: Any? = nil) {
    self.command = command
    self.args = args
    self.initializationOptions = initializationOptions
  }
}

public struct LSPConfig {
  public var servers: [String: LSPServerConfig]
  public var languageServers: [String: [String]]
  public var rootMarkers: [String]

  // MARK: Loading

  /// Port of `LspConfig::load`: defaults, then the trusted global config,
  /// then untrusted project-local configs in `projectFolder`.
  public static func load(globalConfigPath: String?, projectFolder: String?) -> LSPConfig {
    var cfg = defaultConfig()
    if let globalConfigPath {
      cfg.applyFile(globalConfigPath, trusted: true)
    }
    if let projectFolder {
      cfg.applyProjectConfig(in: projectFolder)
    }
    return cfg
  }

  /// Apply the project-local configs in `folder`. They are untrusted: they
  /// cannot define new server commands, only remap language->server
  /// associations and root markers. This prevents malicious repos from
  /// executing arbitrary binaries.
  mutating func applyProjectConfig(in folder: String) {
    for path in Self.projectConfigPaths(in: folder) {
      applyFile(path, trusted: false)
    }
  }

  /// A folder's project-local config files, in the order they apply.
  static func projectConfigPaths(in folder: String) -> [String] {
    [".impulse/lsp.json", ".impulse-lsp.json"].map { (folder as NSString).appendingPathComponent($0) }
  }

  /// The folder whose project config applies to files in `directory`: the
  /// nearest one, from `directory` up, that has `.impulse/lsp.json` or
  /// `.impulse-lsp.json`. A repository's root is as far up as it looks.
  static func projectConfigFolder(forDirectory directory: String) -> String? {
    let fm = FileManager.default
    var dir = directory
    while !dir.isEmpty {
      if projectConfigPaths(in: dir).contains(where: { fm.fileExists(atPath: $0) }) { return dir }
      if fm.fileExists(atPath: (dir as NSString).appendingPathComponent(".git")) { return nil }
      let parent = (dir as NSString).deletingLastPathComponent
      if parent == dir { break }
      dir = parent
    }
    return nil
  }

  /// Changes when the file at `path` is written, replaced or removed (nil
  /// when there's no file).
  static func fileStamp(_ path: String) -> String? {
    var st = stat()
    guard stat(path, &st) == 0 else { return nil }
    return "\(st.st_mtimespec.tv_sec).\(st.st_mtimespec.tv_nsec):\(st.st_size):\(st.st_ino)"
  }

  static func globalLspConfigPath() -> String? {
    let env = ProcessInfo.processInfo.environment
    if let xdg = env["XDG_CONFIG_HOME"] {
      return (xdg as NSString).appendingPathComponent("impulse/lsp.json")
    }
    guard let home = env["HOME"] else { return nil }
    return (home as NSString).appendingPathComponent(".config/impulse/lsp.json")
  }

  /// Parsed overrides file. Mirrors `LspConfigOverrides` including serde's
  /// all-or-nothing strictness: any structural error rejects the whole file.
  private struct Overrides {
    var servers: [String: LSPServerConfig]?
    var languageServers: [String: [String]]?
    var rootMarkers: [String]?
  }

  mutating func applyFile(_ path: String, trusted: Bool) {
    guard let data = FileManager.default.contents(atPath: path) else { return }
    guard let overrides = Self.parseOverrides(data) else {
      lspLog("Invalid LSP config at \(path)")
      return
    }

    if let servers = overrides.servers {
      if trusted {
        // Global config may define arbitrary server commands.
        for (id, config) in servers {
          self.servers[id] = config
        }
      } else {
        // Project-local configs are untrusted and cannot modify server
        // configuration (neither command nor args) to prevent argument
        // injection attacks.
        for id in servers.keys {
          lspLog(
            "Project LSP config tried to configure server '\(id)' — ignoring (only global config can modify servers)"
          )
        }
      }
    }
    if let languageServers = overrides.languageServers {
      if trusted {
        for (lang, serverIds) in languageServers {
          self.languageServers[lang] = serverIds
        }
      } else {
        // Only allow mapping to servers that already exist.
        for (lang, serverIds) in languageServers {
          let validIds = serverIds.filter { servers[$0] != nil }
          if !validIds.isEmpty {
            self.languageServers[lang] = validIds
          }
        }
      }
    }
    if let rootMarkers = overrides.rootMarkers {
      let validMarkers = rootMarkers.filter { marker in
        !marker.isEmpty && !marker.contains("/") && !marker.contains("\\")
          && !marker.contains("..")
      }
      if !validMarkers.isEmpty {
        self.rootMarkers = validMarkers
      }
    }
  }

  private static func parseOverrides(_ data: Data) -> Overrides? {
    guard let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
      return nil
    }
    var overrides = Overrides()

    if let rawServers = root["servers"], !(rawServers is NSNull) {
      guard let dict = rawServers as? [String: Any] else { return nil }
      var servers: [String: LSPServerConfig] = [:]
      for (id, value) in dict {
        guard let obj = value as? [String: Any],
          let command = obj["command"] as? String
        else { return nil }
        var args: [String] = []
        if let rawArgs = obj["args"] {
          guard let parsed = rawArgs as? [String] else { return nil }
          args = parsed
        }
        var initOptions: Any?
        if let rawOptions = obj["initialization_options"], !(rawOptions is NSNull) {
          initOptions = rawOptions
        }
        servers[id] = LSPServerConfig(
          command: command, args: args, initializationOptions: initOptions)
      }
      overrides.servers = servers
    }

    if let rawLanguageServers = root["language_servers"], !(rawLanguageServers is NSNull) {
      guard let dict = rawLanguageServers as? [String: Any] else { return nil }
      var languageServers: [String: [String]] = [:]
      for (lang, value) in dict {
        guard let ids = value as? [String] else { return nil }
        languageServers[lang] = ids
      }
      overrides.languageServers = languageServers
    }

    if let rawMarkers = root["root_markers"], !(rawMarkers is NSNull) {
      guard let markers = rawMarkers as? [String] else { return nil }
      overrides.rootMarkers = markers
    }

    return overrides
  }

  // MARK: Defaults

  /// Port of `impl Default for LspConfig`.
  public static func defaultConfig() -> LSPConfig {
    var servers: [String: LSPServerConfig] = [:]
    servers["rust-analyzer"] = LSPServerConfig(command: "rust-analyzer")
    servers["pyright"] = LSPServerConfig(command: "pyright-langserver", args: ["--stdio"])
    servers["clangd"] = LSPServerConfig(command: "clangd")
    servers["typescript-language-server"] = LSPServerConfig(
      command: "typescript-language-server", args: ["--stdio"])
    servers["intelephense"] = LSPServerConfig(command: "intelephense", args: ["--stdio"])
    servers["vscode-html-language-server"] = LSPServerConfig(
      command: "vscode-html-language-server", args: ["--stdio"])
    servers["vscode-css-language-server"] = LSPServerConfig(
      command: "vscode-css-language-server", args: ["--stdio"])
    servers["vscode-json-language-server"] = LSPServerConfig(
      command: "vscode-json-language-server", args: ["--stdio"])
    servers["vscode-eslint-language-server"] = LSPServerConfig(
      command: "vscode-eslint-language-server", args: ["--stdio"])
    servers["tailwindcss-language-server"] = LSPServerConfig(
      command: "tailwindcss-language-server", args: ["--stdio"])
    servers["vue-language-server"] = LSPServerConfig(
      command: "vue-language-server", args: ["--stdio"])
    servers["svelteserver"] = LSPServerConfig(command: "svelteserver", args: ["--stdio"])
    servers["graphql-lsp"] = LSPServerConfig(
      command: "graphql-lsp", args: ["server", "-m", "stream"])
    servers["emmet-ls"] = LSPServerConfig(command: "emmet-ls", args: ["--stdio"])
    servers["yaml-language-server"] = LSPServerConfig(
      command: "yaml-language-server", args: ["--stdio"])
    servers["docker-langserver"] = LSPServerConfig(
      command: "docker-langserver", args: ["--stdio"])
    servers["bash-language-server"] = LSPServerConfig(
      command: "bash-language-server", args: ["start"])
    // Ships with Xcode and the Command Line Tools.
    servers["sourcekit-lsp"] = LSPServerConfig(command: "sourcekit-lsp")

    let webStack = [
      "typescript-language-server",
      "vscode-eslint-language-server",
      "tailwindcss-language-server",
      "emmet-ls",
    ]

    var languageServers: [String: [String]] = [:]
    languageServers["rust"] = ["rust-analyzer"]
    languageServers["python"] = ["pyright"]
    languageServers["c"] = ["clangd"]
    languageServers["cpp"] = ["clangd"]
    languageServers["javascript"] = webStack
    languageServers["javascriptreact"] = webStack
    languageServers["typescript"] = webStack
    languageServers["typescriptreact"] = webStack
    languageServers["php"] = ["intelephense"]
    languageServers["html"] = [
      "vscode-html-language-server", "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["css"] = [
      "vscode-css-language-server", "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["scss"] = [
      "vscode-css-language-server", "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["less"] = [
      "vscode-css-language-server", "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["json"] = ["vscode-json-language-server"]
    languageServers["jsonc"] = ["vscode-json-language-server"]
    languageServers["yaml"] = ["yaml-language-server"]
    languageServers["vue"] = [
      "vue-language-server", "vscode-eslint-language-server",
      "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["svelte"] = [
      "svelteserver", "vscode-eslint-language-server",
      "tailwindcss-language-server", "emmet-ls",
    ]
    languageServers["graphql"] = ["graphql-lsp"]
    languageServers["dockerfile"] = ["docker-langserver"]
    languageServers["shellscript"] = ["bash-language-server"]
    languageServers["swift"] = ["sourcekit-lsp"]

    let rootMarkers = [
      "Cargo.toml",
      "package.json",
      "tsconfig.json",
      "jsconfig.json",
      "pnpm-workspace.yaml",
      "yarn.lock",
      "package-lock.json",
      "bun.lockb",
      "turbo.json",
      "nx.json",
      "go.mod",
      "pyproject.toml",
      "setup.py",
      "composer.json",
      "Gemfile",
      "deno.json",
      "deno.jsonc",
      "Package.swift",
    ]

    return LSPConfig(
      servers: servers, languageServers: languageServers, rootMarkers: rootMarkers)
  }

  // MARK: Root detection

  /// Port of `detect_project_root`: walk up from the file's directory. The
  /// innermost directory containing a root marker wins; the innermost `.git`
  /// directory is the fallback when no marker matches anywhere.
  public static func detectProjectRoot(fileUri: String, markers: [String]) -> String? {
    guard let path = FileURI.toPath(fileUri), path != "/" else { return nil }
    var dir = (path as NSString).deletingLastPathComponent
    if dir.isEmpty { return nil }
    var best: String?
    var gitRoot: String?
    let fm = FileManager.default

    while true {
      if best == nil {
        for marker in markers {
          if fm.fileExists(atPath: (dir as NSString).appendingPathComponent(marker)) {
            best = FileURI.fromPath(dir)
            break
          }
        }
      }
      if gitRoot == nil, fm.fileExists(atPath: (dir as NSString).appendingPathComponent(".git")) {
        gitRoot = FileURI.fromPath(dir)
      }

      let parent = (dir as NSString).deletingLastPathComponent
      if parent == dir || parent.isEmpty { break }
      dir = parent
    }

    return best ?? gitRoot
  }

  // MARK: Default server settings

  /// Port of `get_default_init_options`: hardcoded initialization options
  /// for known LSP servers that require them for proper operation.
  static func defaultInitOptions(serverId: String) -> Any? {
    switch serverId {
    case "intelephense":
      let base =
        ManagedServers.managedLspRootDir()
        ?? (NSHomeDirectory() as NSString).appendingPathComponent(
          "Library/Application Support/impulse/lsp")
      let storage = (base as NSString).appendingPathComponent("intelephense")
      return [
        "storagePath": storage,
        "globalStoragePath": storage,
      ]
    case "typescript-language-server":
      return [
        "hostInfo": "Impulse",
        "preferences": [
          "quotePreference": "auto",
          "importModuleSpecifierPreference": "shortest",
          "jsxAttributeCompletionStyle": "auto",
        ],
      ]
    case "vscode-eslint-language-server":
      return ["validate": "probe"]
    default:
      return nil
    }
  }
}

/// Port of `workspace_configuration_for_section`, used when responding to
/// `workspace/configuration` requests from LSP servers.
func workspaceConfigurationForSection(_ section: String) -> [String: Any] {
  switch section {
  case "typescript", "javascript":
    return tsJsWorkspaceSettings()
  case "typescript.format", "javascript.format":
    return [
      "insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces": true
    ]
  default:
    return [:]
  }
}

/// Port of `default_workspace_settings`, pushed via
/// `workspace/didChangeConfiguration` after `initialized`.
func defaultWorkspaceSettings() -> [String: Any] {
  let tsJs = tsJsWorkspaceSettings()
  return [
    "typescript": tsJs,
    "javascript": tsJs,
  ]
}

private func tsJsWorkspaceSettings() -> [String: Any] {
  return [
    "implicitProjectConfiguration": [
      "checkJs": false,
      "experimentalDecorators": false,
      "module": "ESNext",
      "strictNullChecks": true,
      "strictFunctionTypes": true,
      "target": "ES2020",
    ],
    "suggest": [
      "autoImports": true,
      "completeFunctionCalls": false,
    ],
    "preferences": [
      "quotePreference": "auto",
      "importModuleSpecifierPreference": "shortest",
      "jsxAttributeCompletionStyle": "auto",
    ],
    "format": [
      "insertSpaceAfterOpeningAndBeforeClosingNonemptyBraces": true
    ],
  ]
}
