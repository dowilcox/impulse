import CImpulseFFI
import Foundation

// MARK: - Search Result

/// A single search result returned from the Rust search APIs.
///
/// The JSON encoding uses snake_case keys to match the Rust `SearchResult`
/// serialization produced by impulse-core.
struct SearchResult: Codable {
    let path: String
    let name: String
    let lineNumber: UInt32?
    let lineContent: String?
    let columnStart: UInt32?
    let columnEnd: UInt32?
    let matchType: String

    enum CodingKeys: String, CodingKey {
        case path, name
        case lineNumber = "line_number"
        case lineContent = "line_content"
        case columnStart = "column_start"
        case columnEnd = "column_end"
        case matchType = "match_type"
    }
}

// MARK: - ImpulseCore FFI Bridge

/// Swift wrapper around the C FFI functions from impulse-ffi.
///
/// This class provides a clean Swift API over the C function calls exposed by
/// the `impulse-ffi` static library. All returned C strings are freed through
/// `impulse_free_string` to prevent memory leaks.
///
/// Methods that return heap-allocated C strings use a helper that converts to
/// a Swift `String` and immediately frees the C pointer. The one exception is
/// `impulse_get_editor_html()`, which returns a process-lifetime static
/// pointer that must NOT be freed.
final class ImpulseCore {

    /// Opaque handle to the Rust LSP registry. `nil` until the first working
    /// directory is established via `initializeLsp(rootUri:)`.
    private var lspRegistry: OpaquePointer?

    init() {}

    deinit {
        shutdownLsp()
    }

    // MARK: - Private Helpers

    /// Converts an owned C string to a Swift String and frees the C pointer.
    private static func consumeCString(_ ptr: UnsafeMutablePointer<CChar>?) -> String? {
        guard let ptr = ptr else { return nil }
        let result = String(cString: ptr)
        impulse_free_string(ptr)
        return result
    }

    // MARK: - Monaco Assets

    /// Ensures Monaco editor files are extracted to the platform cache
    /// directory. Returns the extraction directory path on success, or a
    /// descriptive error string on failure.
    static func ensureMonacoExtracted() -> Result<String, String> {
        guard let raw = impulse_ensure_monaco_extracted() else {
            return .failure("impulse_ensure_monaco_extracted returned null")
        }
        let str = String(cString: raw)
        impulse_free_string(raw)
        if str.hasPrefix("ERROR:") {
            return .failure(String(str.dropFirst(6)))
        }
        return .success(str)
    }

    /// Returns the embedded editor HTML content that hosts Monaco inside a
    /// WKWebView. The returned pointer is process-static on the Rust side
    /// and must NOT be freed.
    static func getEditorHTML() -> String {
        guard let cStr = impulse_get_editor_html() else { return "" }
        return String(cString: cStr)
    }

    // MARK: - Shell Integration

    /// Returns the shell integration script for the given shell name.
    ///
    /// - Parameter shell: One of `"bash"`, `"zsh"`, or `"fish"`.
    /// - Returns: The integration script, or `nil` for an unrecognized shell.
    static func getShellIntegrationScript(shell: String) -> String? {
        return consumeCString(impulse_get_shell_integration_script(shell))
    }

    /// Returns the user's default login shell path (e.g. `/bin/zsh`).
    static func getUserLoginShell() -> String {
        return consumeCString(impulse_get_user_login_shell()) ?? "/bin/zsh"
    }

    /// Returns the user's default login shell name (e.g. `"fish"`, `"zsh"`).
    static func getUserLoginShellName() -> String {
        return consumeCString(impulse_get_user_login_shell_name()) ?? "zsh"
    }

    // MARK: - Search

    /// Searches for files by name under `root` matching `query`.
    ///
    /// - Parameters:
    ///   - root: The directory to search within.
    ///   - query: The filename substring to match.
    /// - Returns: An array of `SearchResult` values decoded from the JSON
    ///   response, or an empty array on failure.
    static func searchFiles(root: String, query: String) -> [SearchResult] {
        guard let json = consumeCString(impulse_search_files(root, query)) else {
            return []
        }
        return decodeSearchResults(json)
    }

    /// Searches file contents under `root` for `query`.
    ///
    /// - Parameters:
    ///   - root: The directory to search within.
    ///   - query: The content substring or pattern to match.
    ///   - caseSensitive: Whether the search should be case-sensitive.
    /// - Returns: An array of `SearchResult` values decoded from the JSON
    ///   response, or an empty array on failure.
    static func searchContent(root: String, query: String, caseSensitive: Bool) -> [SearchResult] {
        guard let json = consumeCString(impulse_search_content(root, query, caseSensitive)) else {
            return []
        }
        return decodeSearchResults(json)
    }

    /// Decodes a JSON array string into an array of `SearchResult`.
    private static func decodeSearchResults(_ json: String) -> [SearchResult] {
        guard let data = json.data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([SearchResult].self, from: data)) ?? []
    }

    // MARK: - Git

    /// Returns the current git branch for the directory at `path`, or `nil`
    /// if the path is not inside a git repository.
    static func gitBranch(path: String) -> String? {
        return consumeCString(impulse_git_branch(path))
    }

    /// Returns git blame info for a specific 1-based line in a file.
    ///
    /// - Parameters:
    ///   - filePath: The absolute path to the file.
    ///   - line: The 1-based line number.
    /// - Returns: A dictionary with `author`, `date`, `commitHash`, and
    ///   `summary` keys, or `nil` if blame info is unavailable.
    static func gitBlame(filePath: String, line: UInt32) -> [String: String]? {
        guard let json = consumeCString(impulse_git_blame(filePath, line)) else { return nil }
        guard let data = json.data(using: .utf8),
              let dict = try? JSONSerialization.jsonObject(with: data) as? [String: String] else { return nil }
        return dict
    }

    /// Returns diff markers for the file at `path` as a `DiffDecoration` array.
    ///
    /// Each element contains a 1-based line number and a status string
    /// (`"added"`, `"modified"`, or `"deleted"`).
    static func gitDiffMarkers(filePath: String) -> [DiffDecoration] {
        guard let json = consumeCString(impulse_git_diff_markers(filePath)) else { return [] }
        guard let data = json.data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([DiffDecoration].self, from: data)) ?? []
    }

    // MARK: - LSP

    /// Creates a new LSP registry for the given workspace root URI.
    ///
    /// - Parameter rootUri: The workspace root as a `file://` URI.
    /// - Returns: An opaque pointer to the registry handle, or `nil` on failure.
    static func createLspRegistry(rootUri: String) -> OpaquePointer? {
        return OpaquePointer(impulse_lsp_registry_new(rootUri))
    }

    /// Ensures LSP servers are running for the given language and file.
    ///
    /// - Parameters:
    ///   - handle: The LSP registry handle.
    ///   - languageId: The LSP language identifier (e.g. `"typescript"`).
    ///   - fileUri: The file URI (e.g. `"file:///path/to/file.ts"`).
    /// - Returns: The number of clients started/found, or `-1` on error.
    static func lspEnsureServers(handle: OpaquePointer, languageId: String, fileUri: String) -> Int32 {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        return impulse_lsp_ensure_servers(typed, languageId, fileUri)
    }

    /// Sends a synchronous LSP request and returns the JSON response.
    ///
    /// - Parameters:
    ///   - handle: The LSP registry handle.
    ///   - languageId: The LSP language identifier.
    ///   - fileUri: The file URI.
    ///   - method: The LSP method name (e.g. `"textDocument/completion"`).
    ///   - params: JSON-encoded parameters, or `nil` for no params.
    /// - Returns: A JSON string with the result.
    static func lspRequest(handle: OpaquePointer, languageId: String, fileUri: String, method: String, params: String?) -> String {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        let result = impulse_lsp_request(typed, languageId, fileUri, method, params)
        return consumeCString(result) ?? "{\"error\":\"null response\"}"
    }

    /// Sends an LSP notification (no response expected).
    ///
    /// - Parameters:
    ///   - handle: The LSP registry handle.
    ///   - languageId: The LSP language identifier.
    ///   - fileUri: The file URI.
    ///   - method: The LSP method name (e.g. `"textDocument/didOpen"`).
    ///   - params: JSON-encoded parameters, or `nil` for no params.
    /// - Returns: `true` on success, `false` on error.
    static func lspNotify(handle: OpaquePointer, languageId: String, fileUri: String, method: String, params: String?) -> Bool {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        return impulse_lsp_notify(typed, languageId, fileUri, method, params) == 0
    }

    /// Polls for the next asynchronous LSP event (diagnostics, lifecycle).
    ///
    /// - Parameter handle: The LSP registry handle.
    /// - Returns: A JSON string describing the event, or `nil` if no events
    ///   are pending.
    static func lspPollEvent(handle: OpaquePointer) -> String? {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        return consumeCString(impulse_lsp_poll_event(typed))
    }

    /// Shuts down all LSP servers managed by the given registry.
    static func lspShutdownAll(handle: OpaquePointer) {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        impulse_lsp_shutdown_all(typed)
    }

    /// Frees an LSP registry handle. This also shuts down all servers.
    static func lspRegistryFree(handle: OpaquePointer) {
        let raw = UnsafeMutableRawPointer(handle)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        impulse_lsp_registry_free(typed)
    }

    // MARK: - Instance LSP Management

    /// Initializes the instance LSP registry for the given root URI.
    /// Shuts down any previously active registry first.
    func initializeLsp(rootUri: String) {
        shutdownLsp()
        lspRegistry = ImpulseCore.createLspRegistry(rootUri: rootUri)
    }

    /// Ensures LSP servers are running for the given language and file
    /// using the instance registry.
    @discardableResult
    func lspEnsureServers(languageId: String, fileUri: String) -> Int32 {
        guard let reg = lspRegistry else { return -1 }
        return ImpulseCore.lspEnsureServers(handle: reg, languageId: languageId, fileUri: fileUri)
    }

    /// Sends a synchronous LSP request using the instance registry.
    func lspRequest(languageId: String, fileUri: String, method: String, paramsJson: String) -> String? {
        guard let reg = lspRegistry else { return nil }
        return ImpulseCore.lspRequest(handle: reg, languageId: languageId, fileUri: fileUri, method: method, params: paramsJson)
    }

    /// Sends an LSP notification using the instance registry.
    @discardableResult
    func lspNotify(languageId: String, fileUri: String, method: String, paramsJson: String) -> Int32 {
        guard let reg = lspRegistry else { return -1 }
        let raw = UnsafeMutableRawPointer(reg)
        let typed = raw.assumingMemoryBound(to: LspRegistryHandle.self)
        return impulse_lsp_notify(typed, languageId, fileUri, method, paramsJson)
    }

    /// Polls for the next asynchronous LSP event using the instance registry.
    func lspPollEvent() -> String? {
        guard let reg = lspRegistry else { return nil }
        return ImpulseCore.lspPollEvent(handle: reg)
    }

    /// Shuts down all running LSP servers and releases the instance registry.
    func shutdownLsp() {
        guard let reg = lspRegistry else { return }
        ImpulseCore.lspShutdownAll(handle: reg)
        ImpulseCore.lspRegistryFree(handle: reg)
        lspRegistry = nil
    }

    // MARK: - Instance convenience wrappers for non-LSP calls

    /// Extracts Monaco assets. Instance wrapper around the static method.
    func ensureMonacoExtracted() -> String? {
        switch ImpulseCore.ensureMonacoExtracted() {
        case .success(let path): return path
        case .failure: return nil
        }
    }

    /// Returns the editor HTML. Instance wrapper around the static method.
    func editorHTML() -> String? {
        let html = ImpulseCore.getEditorHTML()
        return html.isEmpty ? nil : html
    }

    /// Returns the user's login shell path. Instance wrapper.
    func userLoginShell() -> String {
        return ImpulseCore.getUserLoginShell()
    }

    /// Returns the short name of the user's login shell. Instance wrapper.
    func userLoginShellName() -> String {
        return ImpulseCore.getUserLoginShellName()
    }

    /// Returns the shell integration script. Instance wrapper.
    func shellIntegrationScript(shell: String) -> String? {
        return ImpulseCore.getShellIntegrationScript(shell: shell)
    }

    /// Searches for files by name. Instance wrapper.
    func searchFiles(root: String, query: String) -> String? {
        let results = ImpulseCore.searchFiles(root: root, query: query)
        guard let data = try? JSONEncoder().encode(results),
              let json = String(data: data, encoding: .utf8) else {
            return nil
        }
        return json
    }

    /// Searches file contents. Instance wrapper.
    func searchContent(root: String, query: String, caseSensitive: Bool) -> String? {
        let results = ImpulseCore.searchContent(root: root, query: query, caseSensitive: caseSensitive)
        guard let data = try? JSONEncoder().encode(results),
              let json = String(data: data, encoding: .utf8) else {
            return nil
        }
        return json
    }

    // MARK: - Managed LSP

    /// Returns the installation status of managed web LSP servers as an
    /// array of dictionaries with `command`, `installed`, and
    /// `resolvedPath` keys.
    static func lspCheckStatus() -> [[String: Any]] {
        guard let raw = impulse_lsp_check_status() else { return [] }
        let json = String(cString: raw)
        impulse_free_string(raw)

        guard let data = json.data(using: .utf8),
              let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            return []
        }
        return array
    }

    /// Installs managed web LSP servers. Returns the installation root path
    /// on success, or a descriptive error on failure.
    static func lspInstall() -> Result<String, String> {
        guard let raw = impulse_lsp_install() else {
            return .failure("impulse_lsp_install returned null")
        }
        let str = String(cString: raw)
        impulse_free_string(raw)
        if str.hasPrefix("ERROR:") {
            return .failure(String(str.dropFirst(6)))
        }
        return .success(str)
    }

    /// Instance wrapper for `lspCheckStatus`.
    func lspCheckStatus() -> String? {
        let statuses = ImpulseCore.lspCheckStatus()
        guard let data = try? JSONSerialization.data(withJSONObject: statuses),
              let json = String(data: data, encoding: .utf8) else {
            return nil
        }
        return json
    }

    /// Instance wrapper for `lspInstall`.
    func lspInstall() -> String? {
        switch ImpulseCore.lspInstall() {
        case .success(let path): return path
        case .failure: return nil
        }
    }
}
