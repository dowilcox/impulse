import CImpulseFFI
import Foundation
import ImpulseGit
import ImpulseKit
import ImpulseLSP

// MARK: - Error Type

/// Simple error wrapper so we can use `Result<String, ImpulseError>` (Swift
/// requires the failure type to conform to `Error`).
struct ImpulseError: Error, CustomStringConvertible {
    let message: String
    var description: String { message }
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

    /// The Swift LSP registry. `nil` until the first working directory is
    /// established via `initializeLsp(rootUri:)`.
    private var lspRegistry: LSPRegistry?

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

    // MARK: - Search

    /// Searches for files by name under `root` matching `query`.
    ///
    /// - Parameters:
    ///   - root: The directory to search within.
    ///   - query: The filename substring to match.
    /// - Returns: An array of `SearchResult` values decoded from the JSON
    ///   response, or an empty array on failure.
    static func searchFiles(root: String, query: String) -> [SearchResult] {
        return FileSearch.searchFilenames(root: root, query: query, limit: 200)
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
        return FileSearch.searchContents(
            root: root, query: query, limit: 500, caseSensitive: caseSensitive)
    }

    /// Runs filename and content searches concurrently under `root` and returns
    /// a merged, deduplicated result list. Filename hits that are also present
    /// as content hits are dropped (content hits carry line info, so they're
    /// more useful).
    ///
    /// Call off the main thread — both FFI calls block until the ripgrep/walk
    /// completes.
    static func searchAll(root: String, query: String, caseSensitive: Bool) -> [SearchResult] {
        // Kick off both searches in parallel using background queues.
        let group = DispatchGroup()
        let queue = DispatchQueue.global(qos: .userInitiated)

        var fileResults: [SearchResult] = []
        var contentResults: [SearchResult] = []

        group.enter()
        queue.async {
            fileResults = searchFiles(root: root, query: query)
            group.leave()
        }
        group.enter()
        queue.async {
            contentResults = searchContent(root: root, query: query, caseSensitive: caseSensitive)
            group.leave()
        }
        group.wait()

        if contentResults.isEmpty { return fileResults }
        if fileResults.isEmpty { return contentResults }

        var seen = Set<String>()
        seen.reserveCapacity(contentResults.count)
        for r in contentResults { seen.insert(r.path) }

        var combined: [SearchResult] = []
        combined.reserveCapacity(fileResults.count + contentResults.count)
        for r in fileResults where !seen.contains(r.path) {
            combined.append(r)
        }
        combined.append(contentsOf: contentResults)
        return combined
    }

    // MARK: - Git (ImpulseGit-backed)

    /// Re-encode an ImpulseGit/ImpulseKit value into the bridge's Codable
    /// model via JSON — both sides pin the same serde-compatible keys.
    private static func reencode<T: Encodable, U: Decodable>(_ value: T, as type: U.Type) -> U? {
        guard let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(U.self, from: data)
    }

    /// Returns the current git branch for the directory at `path`, or `nil`
    /// if the path is not inside a git repository.
    static func gitBranch(path: String) -> String? {
        return GitClient.branch(forPath: path)
    }

    /// Returns git status for files in a directory as a dictionary mapping
    /// filenames to status codes (e.g. `["file.rs": "M", "new.txt": "?"]`).
    /// Returns an empty dictionary if the path is not in a git repo.
    static func gitStatusForDirectory(path: String) -> [String: String] {
        return GitClient.statusForDirectory(path) ?? [:]
    }

    /// Batch-fetch git status for the entire repository in a single call.
    ///
    /// Returns a nested dictionary: outer key = directory absolute path,
    /// inner key = filename, value = status code. Parent directories receive
    /// the highest-priority status among their descendants.
    static func getAllGitStatuses(repoPath: String) -> [String: [String: String]] {
        return GitClient.allStatuses(root: repoPath) ?? [:]
    }

    /// Codable struct matching the Rust `FileEntry` serialization.
    struct FileEntryFFI: Codable {
        let name: String
        let path: String
        let is_dir: Bool
        let is_symlink: Bool
        let size: UInt64
        let modified: UInt64
        let git_status: String?
    }

    /// Codable struct matching the Rust `FileTreeNode` patch serialization.
    struct FileTreePatchNode: Codable {
        let id: String
        let parent_id: String?
        let name: String
        let path: String
        let is_dir: Bool
        let is_symlink: Bool
        let size: UInt64
        let modified: UInt64
        let git_status: String?
    }

    struct FileTreePatchBatch: Codable {
        let root_id: String
        let patches: [FileTreePatch]
    }

    struct FileTreePatch: Codable {
        let parent_id: String
        let operations: [FileTreeOperation]
    }

    enum FileTreeOperation: Codable {
        case remove(id: String)
        case upsert(parentId: String, index: Int, node: FileTreePatchNode)

        private enum CodingKeys: String, CodingKey {
            case type
            case id
            case parentId = "parent_id"
            case index
            case node
        }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let type = try container.decode(String.self, forKey: .type)
            switch type {
            case "remove":
                self = .remove(id: try container.decode(String.self, forKey: .id))
            case "upsert":
                self = .upsert(
                    parentId: try container.decode(String.self, forKey: .parentId),
                    index: try container.decode(Int.self, forKey: .index),
                    node: try container.decode(FileTreePatchNode.self, forKey: .node)
                )
            default:
                throw DecodingError.dataCorruptedError(
                    forKey: .type,
                    in: container,
                    debugDescription: "Unknown file-tree operation type: \(type)"
                )
            }
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            switch self {
            case .remove(let id):
                try container.encode("remove", forKey: .type)
                try container.encode(id, forKey: .id)
            case .upsert(let parentId, let index, let node):
                try container.encode("upsert", forKey: .type)
                try container.encode(parentId, forKey: .parentId)
                try container.encode(index, forKey: .index)
                try container.encode(node, forKey: .node)
            }
        }
    }

    struct FileTreeWatchEvent: Codable {
        let kind: String
        let paths: [String]
    }

    /// Read directory contents (sorted dirs-first) with git status enrichment,
    /// mirroring the Rust `read_directory_with_git_status`.
    static func readDirectoryWithGitStatus(path: String, showHidden: Bool) -> [FileEntryFFI]? {
        guard let entries = readDirectoryEntriesEnriched(path: path, showHidden: showHidden)
        else { return nil }
        return entries.map {
            FileEntryFFI(
                name: $0.name, path: $0.path, is_dir: $0.isDir, is_symlink: $0.isSymlink,
                size: $0.size, modified: $0.modified, git_status: $0.gitStatus)
        }
    }

    private static func readDirectoryEntriesEnriched(path: String, showHidden: Bool)
        -> [ImpulseKit.FileEntry]?
    {
        guard
            var entries = try? DirectoryLister.readDirectoryEntries(
                path: path, showHidden: showHidden)
        else { return nil }
        if let status = GitClient.statusForDirectory(path) {
            for index in entries.indices {
                if let code = status[entries[index].name] {
                    entries[index].gitStatus = code
                }
            }
        }
        return entries
    }

    /// Build a file-tree patch batch from watcher events and the directories
    /// currently loaded by the UI.
    static func buildFileTreePatchBatch(
        rootPath: String,
        events: [FileTreeWatchEvent],
        beforeByParent: [String: [FileEntryFFI]],
        showHidden: Bool
    ) -> FileTreePatchBatch? {
        let kitEvents = events.map { event in
            ImpulseKit.FileTreeWatchEvent(
                kind: ImpulseKit.FileTreeWatchEventKind(rawValue: event.kind) ?? .any,
                paths: event.paths)
        }
        let kitBefore = beforeByParent.mapValues { entries in
            entries.map { entry in
                ImpulseKit.FileEntry(
                    name: entry.name, path: entry.path, isDir: entry.is_dir,
                    isSymlink: entry.is_symlink, size: entry.size, modified: entry.modified,
                    gitStatus: entry.git_status)
            }
        }
        guard
            let batch = FileTreePatcher.buildPatchBatchFromFilesystem(
                rootPath: rootPath,
                events: kitEvents,
                beforeByParent: kitBefore,
                showHidden: showHidden,
                readDirectory: { path, showHidden in
                    readDirectoryEntriesEnriched(path: path, showHidden: showHidden)
                })
        else { return nil }
        return reencode(batch, as: FileTreePatchBatch.self)
    }

    // MARK: - LSP (ImpulseLSP-backed)

    func initializeLsp(rootUri: String) {
        shutdownLsp()
        lspRegistry = LSPRegistry(rootUri: rootUri)
        lspRegistry?.onEventsAvailable = lspEventsAvailable
    }

    /// Called from a language server's reader thread when events are queued.
    var lspEventsAvailable: (() -> Void)? {
        didSet { lspRegistry?.onEventsAvailable = lspEventsAvailable }
    }

    /// Ensures LSP servers are running for the given language and file
    /// using the instance registry.
    @discardableResult
    func lspEnsureServers(languageId: String, fileUri: String) -> Int32 {
        guard let reg = lspRegistry else { return -1 }
        return Int32(reg.ensureServers(languageId: languageId, fileUri: fileUri))
    }

    /// Sends a synchronous LSP request using the instance registry.
    func lspRequest(languageId: String, fileUri: String, method: String, paramsJson: String) -> String? {
        guard let reg = lspRegistry else { return nil }
        return reg.request(
            languageId: languageId, fileUri: fileUri, method: method, paramsJSON: paramsJson)
    }

    /// Sends an LSP notification using the instance registry.
    @discardableResult
    func lspNotify(languageId: String, fileUri: String, method: String, paramsJson: String) -> Int32 {
        guard let reg = lspRegistry else { return -1 }
        return reg.notify(
            languageId: languageId, fileUri: fileUri, method: method, paramsJSON: paramsJson)
            ? 0 : -1
    }

    /// Sends a capability-aware textDocument/didChange using the instance registry.
    @discardableResult
    func lspDidChange(languageId: String, fileUri: String, version: Int32, fullText: String?, changesJson: String) -> Int32 {
        guard let reg = lspRegistry else { return -1 }
        return reg.didChange(
            languageId: languageId, fileUri: fileUri, version: version,
            fullText: fullText, changesJSON: changesJson)
            ? 0 : -1
    }

    /// Polls for the next asynchronous LSP event using the instance registry.
    func lspPollEvent() -> String? {
        guard let reg = lspRegistry else { return nil }
        return reg.pollEvent()
    }

    /// Shuts down all running LSP servers and releases the instance registry.
    func shutdownLsp() {
        lspRegistry?.shutdownAll()
        lspRegistry = nil
    }

    // MARK: - Instance convenience wrappers for non-LSP calls

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
        decodeStatusJSON(ManagedServers.checkStatusJSON())
    }

    private static func decodeStatusJSON(_ json: String) -> [[String: Any]] {
        guard let data = json.data(using: .utf8),
              let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            return []
        }
        return array
    }

    /// Installs managed web LSP servers. Returns the installation root path
    /// on success, or a descriptive error on failure.
    static func lspInstall() -> Result<String, ImpulseError> {
        switch ManagedServers.install() {
        case .success(let path): return .success(path)
        case .failure(let message): return .failure(ImpulseError(message: message))
        }
    }

    /// Returns whether npm is available on the system PATH.
    static func npmIsAvailable() -> Bool {
        ManagedServers.npmIsAvailable()
    }

    // MARK: - Terminal Backend

    /// Creates a new terminal backend and returns an opaque handle.
    ///
    /// - Parameters:
    ///   - configJson: JSON-encoded `TerminalBackendConfig`.
    ///   - cols: Initial column count.
    ///   - rows: Initial row count.
    ///   - cellWidth: Cell width in pixels (for pixel-accurate resize).
    ///   - cellHeight: Cell height in pixels.
    /// - Returns: An opaque handle, or `nil` on failure.
    static func terminalCreate(configJson: String, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) -> OpaquePointer? {
        let raw = configJson.withCString { ptr in
            impulse_terminal_create(ptr, CUnsignedShort(cols), CUnsignedShort(rows), CUnsignedShort(cellWidth), CUnsignedShort(cellHeight))
        }
        guard let raw else { return nil }
        return OpaquePointer(raw)
    }

    /// Destroys a terminal backend handle.
    static func terminalDestroy(handle: OpaquePointer) {
        impulse_terminal_destroy(UnsafeMutableRawPointer(handle))
    }

    /// Register (or clear, with nil) the callback the PTY reader thread calls
    /// when events are waiting.
    static func terminalSetWakeupCallback(
        handle: OpaquePointer,
        callback: (@convention(c) (UnsafeMutableRawPointer?) -> Void)?,
        context: UnsafeMutableRawPointer?
    ) {
        impulse_terminal_set_wakeup_callback(UnsafeMutableRawPointer(handle), callback, context)
    }

    /// Writes raw data to the terminal's PTY input.
    static func terminalWrite(handle: OpaquePointer, data: Data) {
        data.withUnsafeBytes { rawBuf in
            guard let ptr = rawBuf.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
            impulse_terminal_write(UnsafeMutableRawPointer(handle), ptr, UInt(rawBuf.count))
        }
    }

    /// Resizes the terminal grid and notifies the PTY.
    static func terminalResize(handle: OpaquePointer, cols: UInt16, rows: UInt16, cellWidth: UInt16, cellHeight: UInt16) {
        impulse_terminal_resize(UnsafeMutableRawPointer(handle), CUnsignedShort(cols), CUnsignedShort(rows), CUnsignedShort(cellWidth), CUnsignedShort(cellHeight))
    }

    /// Fills `buffer` with a binary grid snapshot and returns the number of
    /// bytes written.
    static func terminalGridSnapshot(handle: OpaquePointer, buffer: UnsafeMutablePointer<UInt8>, bufferSize: Int) -> Int {
        return Int(impulse_terminal_grid_snapshot(UnsafeMutableRawPointer(handle), buffer, UInt(bufferSize)))
    }

    /// Returns the buffer size needed for a grid snapshot at the current
    /// terminal dimensions.
    static func terminalGridSnapshotSize(handle: OpaquePointer) -> Int {
        return Int(impulse_terminal_grid_snapshot_size(UnsafeMutableRawPointer(handle)))
    }

    /// Takes the damage accumulated since the last call and resets tracking.
    /// Returns -1 when a full repaint is required, otherwise the number of
    /// damaged viewport row indices written to `buffer`.
    static func terminalTakeDamage(handle: OpaquePointer, buffer: UnsafeMutablePointer<UInt16>, cap: Int) -> Int {
        return Int(impulse_terminal_take_damage(UnsafeMutableRawPointer(handle), buffer, UInt(cap)))
    }

    /// Polls for pending terminal events. Returns a JSON array string, or
    /// `nil` if no events are pending.
    static func terminalPollEvents(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_poll_events(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Returns command block metadata as a JSON array string.
    static func terminalCommandBlocks(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_command_blocks(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Returns command-block availability flags: bit 0 command, bit 1 output, bit 2 failed.
    static func terminalCommandBlockFlags(handle: OpaquePointer) -> UInt32 {
        return impulse_terminal_command_block_flags(UnsafeMutableRawPointer(handle))
    }

    /// Returns viewport-mapped block regions for block decorations as a JSON string.
    static func terminalBlockOverlay(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_block_overlay(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Returns up to `limit` recent command strings from the terminal's block
    /// history (newest first), for the Swift-side input completion.
    static func terminalRecentCommands(handle: OpaquePointer, limit: Int) -> [String] {
        let ptr = impulse_terminal_recent_commands(
            UnsafeMutableRawPointer(handle), CUnsignedLong(limit))
        guard let json = consumeCString(ptr), let data = json.data(using: .utf8) else { return [] }
        return (try? JSONDecoder().decode([String].self, from: data)) ?? []
    }

    /// Searches completed terminal command history and returns a JSON array string.
    static func terminalCommandHistorySearch(handle: OpaquePointer, queryJson: String) -> String? {
        let ptr = queryJson.withCString { queryPtr in
            impulse_terminal_command_history_search(UnsafeMutableRawPointer(handle), queryPtr)
        }
        guard let ptr else { return nil }
        return consumeCString(ptr)
    }

    /// Reruns a command by writing backend-approved interactive input to the PTY.
    static func terminalRerunCommand(handle: OpaquePointer, command: String) -> Bool {
        return command.withCString { commandPtr in
            impulse_terminal_rerun_command(UnsafeMutableRawPointer(handle), commandPtr)
        }
    }

    /// Starts a text selection at the given grid position.
    ///
    /// - Parameter kind: Selection kind — 0 = simple, 1 = block, 2 = semantic, 3 = line.
    static func terminalStartSelection(handle: OpaquePointer, col: UInt16, row: UInt16, kind: UInt8) {
        impulse_terminal_start_selection(UnsafeMutableRawPointer(handle), CUnsignedShort(col), CUnsignedShort(row), kind)
    }

    /// Extends the current selection to the given grid position.
    static func terminalUpdateSelection(handle: OpaquePointer, col: UInt16, row: UInt16) {
        impulse_terminal_update_selection(UnsafeMutableRawPointer(handle), CUnsignedShort(col), CUnsignedShort(row))
    }

    /// Clears the current text selection.
    static func terminalClearSelection(handle: OpaquePointer) {
        impulse_terminal_clear_selection(UnsafeMutableRawPointer(handle))
    }

    /// Returns the currently selected text, or `nil` if nothing is selected.
    static func terminalSelectedText(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_selected_text(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    static func terminalTranscript(handle: OpaquePointer, maxRows: Int) -> String? {
        guard
            let ptr = impulse_terminal_transcript(
                UnsafeMutableRawPointer(handle), UInt32(clamping: maxRows), true)
        else { return nil }
        return consumeCString(ptr)
    }

    /// Scrolls the terminal viewport by `delta` lines (negative = up).
    static func terminalScroll(handle: OpaquePointer, delta: Int32) {
        impulse_terminal_scroll(UnsafeMutableRawPointer(handle), delta)
    }

    /// Scrolls the terminal viewport to the bottom (most recent output).
    static func terminalScrollToBottom(handle: OpaquePointer) {
        impulse_terminal_scroll_to_bottom(UnsafeMutableRawPointer(handle))
    }

    /// Scrolls the terminal viewport to the approximate start of a command block.
    static func terminalScrollToCommandBlock(handle: OpaquePointer, blockId: UInt64) -> Bool {
        return impulse_terminal_scroll_to_command_block(UnsafeMutableRawPointer(handle), blockId)
    }

    /// Returns terminal mode flags as a JSON string.
    static func terminalMode(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_mode(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Notifies the terminal of focus changes (for focus-in/out reporting).
    static func terminalSetFocus(handle: OpaquePointer, focused: Bool) {
        impulse_terminal_set_focus(UnsafeMutableRawPointer(handle), focused)
    }

    /// Returns the child process PID, or 0 if not available.
    static func terminalChildPid(handle: OpaquePointer) -> UInt32 {
        return impulse_terminal_child_pid(UnsafeMutableRawPointer(handle))
    }

    /// Starts a search in the terminal scrollback. Returns a JSON result string.
    static func terminalSearch(handle: OpaquePointer, pattern: String) -> String? {
        return pattern.withCString { ptr in
            guard let result = impulse_terminal_search(UnsafeMutableRawPointer(handle), ptr) else { return nil }
            return consumeCString(result)
        }
    }

    /// Advances to the next search match. Returns a JSON result string.
    static func terminalSearchNext(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_search_next(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Advances to the previous search match. Returns a JSON result string.
    static func terminalSearchPrev(handle: OpaquePointer) -> String? {
        guard let ptr = impulse_terminal_search_prev(UnsafeMutableRawPointer(handle)) else { return nil }
        return consumeCString(ptr)
    }

    /// Clears the current terminal search.
    static func terminalSearchClear(handle: OpaquePointer) {
        impulse_terminal_search_clear(UnsafeMutableRawPointer(handle))
    }

    /// Updates the terminal's color palette at runtime.
    static func terminalSetColors(handle: OpaquePointer, configJson: String) {
        configJson.withCString { ptr in
            impulse_terminal_set_colors(UnsafeMutableRawPointer(handle), ptr)
        }
    }

    /// Returns the OSC 8 hyperlink URI at the given grid cell, or nil.
    static func terminalHyperlinkAt(handle: OpaquePointer, col: UInt32, row: UInt32) -> String? {
        guard let cStr = impulse_terminal_hyperlink_at(UnsafeMutableRawPointer(handle), col, row) else {
            return nil
        }
        defer { impulse_free_string(cStr) }
        return String(cString: cStr)
    }

    /// Returns the installation status of system (non-managed) LSP servers
    /// as an array of dictionaries with `command`, `installed`, and
    /// `resolvedPath` keys.
    static func systemLspStatus() -> [[String: Any]] {
        decodeStatusJSON(ManagedServers.systemStatusJSON())
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
