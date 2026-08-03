// ImpulseLSP — Swift port of the Rust LSP client (impulse-core/src/lsp.rs
// plus the LSP glue from impulse-ffi/src/lib.rs). Foundation only.
//
// The public surface (`LSPRegistry`, `ManagedServers`, `DocumentCache`,
// `LSPConfig`) mirrors the synchronous FFI call surface the app uses today
// (`impulse_lsp_*` functions), including the exact JSON envelopes emitted by
// `impulse_lsp_poll_event` and the managed-status JSON.

import Foundation

/// Version reported to servers via `clientInfo.version` during `initialize`.
/// The Rust client used `CARGO_PKG_VERSION`; keep in sync with the workspace
/// version in Cargo.toml.
let lspClientVersion = "0.29.0"

/// The Rust code returns `Result<T, String>` everywhere; mirror that shape
/// so error strings flow through unchanged.
extension String: @retroactive Error {}

/// Minimal logging shim standing in for the Rust `log` crate.
func lspLog(_ message: String) {
  NSLog("ImpulseLSP: %@", message)
}

// MARK: - JSON helpers

enum JSONUtil {
  /// Compact JSON encoding with sorted keys. `serde_json` uses a `BTreeMap`
  /// for objects, so its output is also alphabetically ordered — sorting here
  /// keeps the envelopes byte-comparable with what the FFI produced.
  static func encode(_ value: Any) -> String? {
    guard let data = encodeData(value) else { return nil }
    return String(data: data, encoding: .utf8)
  }

  static func encodeData(_ value: Any) -> Data? {
    let options: JSONSerialization.WritingOptions = [
      .sortedKeys, .withoutEscapingSlashes, .fragmentsAllowed,
    ]
    return try? JSONSerialization.data(withJSONObject: value, options: options)
  }

  static func parse(_ json: String) -> Any? {
    guard let data = json.data(using: .utf8) else { return nil }
    return try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
  }

  /// `serde_json::Value::as_i64` equivalent: integers only, never bools or
  /// floating-point values.
  static func asInt64(_ value: Any?) -> Int64? {
    guard let number = value as? NSNumber else { return nil }
    if CFGetTypeID(number) == CFBooleanGetTypeID() { return nil }
    switch String(cString: number.objCType) {
    case "c", "C", "s", "S", "i", "I", "l", "L", "q", "Q":
      return number.int64Value
    default:
      return nil
    }
  }

  /// Strict `u32` parse mirroring serde: integral, non-negative, in range.
  static func asUInt32(_ value: Any?) -> UInt32? {
    guard let v = asInt64(value), v >= 0, v <= Int64(UInt32.max) else { return nil }
    return UInt32(v)
  }
}

// MARK: - File URIs

/// Port of the `file://` URI helpers used by lsp.rs (`url::Url::from_file_path`
/// / `to_file_path`). No trailing slash is added for directories, matching the
/// Rust behavior for root URIs and client keys.
public enum FileURI {
  /// Converts an absolute file path to a `file://` URI. Returns `nil` for
  /// relative paths (like `Url::from_file_path`).
  public static func fromPath(_ path: String) -> String? {
    guard path.hasPrefix("/") else { return nil }
    guard let encoded = path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) else {
      return nil
    }
    return "file://" + encoded
  }

  /// Converts a `file://` URI back to a local file path. Returns `nil` when
  /// the URI is not a file URI.
  public static func toPath(_ uri: String) -> String? {
    guard uri.hasPrefix("file://") else { return nil }
    let rest = uri.dropFirst("file://".count)
    guard let slashIndex = rest.firstIndex(of: "/") else { return nil }
    return String(rest[slashIndex...]).removingPercentEncoding
  }

  /// Port of `workspace_folder_name`: last path component of the root URI,
  /// falling back to "workspace".
  static func workspaceFolderName(_ rootUri: String) -> String {
    guard let path = toPath(rootUri) else { return "workspace" }
    let name = (path as NSString).lastPathComponent
    if name.isEmpty || name == "/" { return "workspace" }
    return name
  }
}

// MARK: - Events

/// Port of `impulse_core::lsp::LspEvent`, with diagnostics pre-parsed the same
/// way the FFI's `lsp_types::Diagnostic` deserialization behaved.
struct LSPDiagnostic {
  var severity: Int64?
  var startLine: UInt32
  var startColumn: UInt32
  var endLine: UInt32
  var endColumn: UInt32
  var message: String
  var source: String?
}

enum LSPEvent {
  case diagnostics(uri: String, version: Int64?, diagnostics: [LSPDiagnostic])
  case initialized(clientKey: String, serverId: String)
  case serverError(clientKey: String, serverId: String, message: String)
  case serverExited(clientKey: String, serverId: String)

  /// Encodes the event as the exact JSON envelope `impulse_lsp_poll_event`
  /// emitted (same key names and casing; severity mapped to 1–4).
  func encodeJSON() -> String? {
    let object: [String: Any]
    switch self {
    case .diagnostics(let uri, let version, let diagnostics):
      let diagJSON: [[String: Any]] = diagnostics.map { d in
        // DiagnosticSeverity ERROR..HINT map to 1..4; anything else maps to
        // 1, and a missing severity defaults to 1 (unwrap_or(1) in the FFI).
        let severity: Int64
        if let s = d.severity, (1...4).contains(s) {
          severity = s
        } else {
          severity = 1
        }
        return [
          "severity": severity,
          "startLine": d.startLine,
          "startColumn": d.startColumn,
          "endLine": d.endLine,
          "endColumn": d.endColumn,
          "message": d.message,
          "source": d.source ?? NSNull(),
        ]
      }
      object = [
        "type": "diagnostics",
        "uri": uri,
        "version": version ?? NSNull(),
        "diagnostics": diagJSON,
      ]
    case .initialized(let clientKey, let serverId):
      object = [
        "type": "initialized",
        "clientKey": clientKey,
        "serverId": serverId,
      ]
    case .serverError(let clientKey, let serverId, let message):
      object = [
        "type": "serverError",
        "clientKey": clientKey,
        "serverId": serverId,
        "message": message,
      ]
    case .serverExited(let clientKey, let serverId):
      object = [
        "type": "serverExited",
        "clientKey": clientKey,
        "serverId": serverId,
      ]
    }
    return JSONUtil.encode(object)
  }
}
