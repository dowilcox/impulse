// Port of the managed web LSP server support from impulse-core/src/lsp.rs:
// command resolution (PATH + managed npm bin dir), npm probing, npm-based
// installation into the managed dir, and the status JSON the FFI emitted
// (`impulse_lsp_check_status` / `impulse_system_lsp_status`).

import Foundation
import ImpulseKit

public enum ManagedServers {
  public static let recommendedWebLspPackages: [String] = [
    "typescript",
    "typescript-language-server",
    "intelephense",
    "vscode-langservers-extracted",
    "@tailwindcss/language-server",
    "@vue/language-server",
    "svelte-language-server",
    "graphql-language-service-cli",
    "emmet-ls",
    "yaml-language-server",
    "dockerfile-language-server-nodejs",
    "bash-language-server",
  ]

  public static let managedNpmServerCommands: [String] = [
    "typescript-language-server",
    "intelephense",
    "vscode-html-language-server",
    "vscode-css-language-server",
    "vscode-json-language-server",
    "vscode-eslint-language-server",
    "tailwindcss-language-server",
    "vue-language-server",
    "svelteserver",
    "graphql-lsp",
    "emmet-ls",
    "yaml-language-server",
    "docker-langserver",
    "bash-language-server",
  ]

  static let installHint =
    "Install them with \"Install Web LSP Servers\" in the command palette or Settings → Language Servers."

  static let systemLspServers: [(id: String, command: String)] = [
    ("rust-analyzer", "rust-analyzer"),
    ("pyright", "pyright-langserver"),
    ("clangd", "clangd"),
    // Ships with Xcode and the Command Line Tools (Swift files use it).
    ("sourcekit-lsp", "sourcekit-lsp"),
  ]

  // MARK: Managed directories

  /// Port of `managed_lsp_root_dir`. The install script uses the XDG
  /// convention (`~/.local/share`); `dirs::data_dir()` on macOS is
  /// `~/Library/Application Support`. Prefer whichever location actually has
  /// an installation, then fall back to the platform-native location.
  public static func managedLspRootDir() -> String? {
    let env = ProcessInfo.processInfo.environment
    let xdgBase = env["XDG_DATA_HOME"]
      ?? (NSHomeDirectory() as NSString).appendingPathComponent(".local/share")
    let xdg = (xdgBase as NSString).appendingPathComponent("impulse/lsp")
    let native = (NSHomeDirectory() as NSString).appendingPathComponent(
      "Library/Application Support/impulse/lsp")

    for candidate in [xdg, native] {
      var isDirectory: ObjCBool = false
      let nodeModules = (candidate as NSString).appendingPathComponent("node_modules")
      if FileManager.default.fileExists(atPath: nodeModules, isDirectory: &isDirectory),
        isDirectory.boolValue
      {
        return candidate
      }
    }
    // Nothing installed yet — return the native path for future installs.
    return native
  }

  public static func managedLspBinDir() -> String? {
    managedLspRootDir().map { ($0 as NSString).appendingPathComponent("node_modules/.bin") }
  }

  // MARK: Command resolution

  static func commandLooksLikePath(_ command: String) -> Bool {
    command.contains("/")
  }

  static func isExecutableFile(_ path: String) -> Bool {
    var st = stat()
    guard stat(path, &st) == 0 else { return false }
    return (st.st_mode & S_IFMT) == S_IFREG && (st.st_mode & 0o111) != 0
  }

  static func findCommandInPath(_ command: String) -> String? {
    if commandLooksLikePath(command) {
      return isExecutableFile(command) ? command : nil
    }
    // The login shell's PATH: launched from the Dock, the app's own is
    // launchd's /usr/bin:/bin:/usr/sbin:/sbin, without Homebrew, ~/.cargo/bin…
    let pathEnv = LoginShell.loginPath()
    for dir in pathEnv.split(separator: ":", omittingEmptySubsequences: false) {
      let candidate = (String(dir) as NSString).appendingPathComponent(command)
      if isExecutableFile(candidate) {
        return candidate
      }
    }
    return nil
  }

  static func findManagedCommand(_ command: String) -> String? {
    guard let bin = managedLspBinDir() else { return nil }
    let candidate = (bin as NSString).appendingPathComponent(command)
    return isExecutableFile(candidate) ? candidate : nil
  }

  /// Port of `resolve_lsp_command_path`: PATH first, then the managed npm
  /// bin directory.
  public static func resolveLspCommandPath(_ command: String) -> String? {
    findCommandInPath(command) ?? findManagedCommand(command)
  }

  static func isManagedNpmServerCommand(_ command: String) -> Bool {
    managedNpmServerCommands.contains(command)
  }

  /// Port of `missing_command_message` — exact user-facing strings.
  static func missingCommandMessage(serverId: String, command: String) -> String {
    if isManagedNpmServerCommand(command) {
      return
        "LSP server '\(serverId)' requires '\(command)' but it is not installed. \(installHint)"
    } else {
      return
        "LSP server '\(serverId)' requires '\(command)' but it is not in PATH. Install it or override `servers.\(serverId)` in lsp.json."
    }
  }

  /// The environment for language servers and npm: the app's, with the
  /// login shell's PATH (plus the managed bin folder) so `node`, `npm` and
  /// servers installed with Homebrew, cargo or nvm are found.
  public static func toolEnvironment() -> [String: String] {
    var environment = ProcessInfo.processInfo.environment
    environment["PATH"] = LoginShell.mergePaths(managedLspBinDir() ?? "", LoginShell.loginPath())
    return environment
  }

  // MARK: npm

  /// Port of `npm_is_available`: `npm --version` succeeds.
  public static func npmIsAvailable() -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["npm", "--version"]
    process.environment = toolEnvironment()
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    do {
      try process.run()
    } catch {
      return false
    }
    process.waitUntilExit()
    return process.terminationStatus == 0
  }

  /// Port of `install_managed_web_lsp_servers`: npm-installs the recommended
  /// web LSP packages into the managed dir. Returns the managed bin directory
  /// on success, or an error message on failure.
  public static func install() -> Result<String, String> {
    guard npmIsAvailable() else {
      return .failure(
        "npm is required but was not found in PATH. Install Node.js + npm first.")
    }

    guard let root = managedLspRootDir() else {
      return .failure("Unable to determine data directory for managed LSPs")
    }
    do {
      try FileManager.default.createDirectory(atPath: root, withIntermediateDirectories: true)
    } catch {
      return .failure("Failed to create \(root): \(error.localizedDescription)")
    }

    let packageJson = (root as NSString).appendingPathComponent("package.json")
    if !FileManager.default.fileExists(atPath: packageJson) {
      let packageDoc: [String: Any] = [
        "name": "impulse-lsp-servers",
        "private": true,
        "description": "Managed web LSP dependencies for Impulse",
        "license": "UNLICENSED",
      ]
      guard
        let data = try? JSONSerialization.data(
          withJSONObject: packageDoc, options: [.prettyPrinted, .sortedKeys])
      else {
        return .failure("Failed to serialize package.json")
      }
      do {
        try data.write(to: URL(fileURLWithPath: packageJson))
      } catch {
        return .failure("Failed to write \(packageJson): \(error.localizedDescription)")
      }
    }

    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments =
      ["npm", "install", "--prefix", root, "--no-audit", "--no-fund"]
      + recommendedWebLspPackages
    process.environment = toolEnvironment()
    do {
      try process.run()
    } catch {
      return .failure("Failed to run npm install: \(error.localizedDescription)")
    }
    process.waitUntilExit()
    guard process.terminationStatus == 0 else {
      return .failure(
        "npm install failed with status exit status: \(process.terminationStatus) while installing managed LSP servers"
      )
    }

    guard let bin = managedLspBinDir() else {
      return .failure(
        "Installation completed but managed bin directory could not be determined")
    }
    return .success(bin)
  }

  // MARK: Status JSON

  /// Port of `impulse_lsp_check_status`: JSON array of
  /// `{"command","installed","resolvedPath"}` for the managed web servers.
  public static func checkStatusJSON() -> String {
    statusJSON(
      for: managedNpmServerCommands.map { ($0, resolveLspCommandPath($0)) })
  }

  /// Port of `impulse_system_lsp_status`: same shape for the system
  /// (non-managed) servers, resolved via PATH only.
  public static func systemStatusJSON() -> String {
    statusJSON(
      for: systemLspServers.map { ($0.id, findCommandInPath($0.command)) })
  }

  private static func statusJSON(for entries: [(command: String, resolvedPath: String?)]) -> String {
    let array: [[String: Any]] = entries.map { entry in
      [
        "command": entry.command,
        "installed": entry.resolvedPath != nil,
        "resolvedPath": entry.resolvedPath ?? NSNull(),
      ]
    }
    return JSONUtil.encode(array) ?? "[]"
  }
}
