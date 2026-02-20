import AppKit
import os.log

// MARK: - LSP Integration

extension MainWindowController {

    func startLspPolling() {
        lspPollTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.pollLspEventsInBackground()
        }
    }

    func stopLspPolling() {
        lspPollTimer?.invalidate()
        lspPollTimer = nil
    }

    /// Polls for LSP events on a background queue and dispatches parsed results
    /// back to the main thread for UI updates. Limits events per tick to avoid
    /// unbounded processing.
    private func pollLspEventsInBackground() {
        lspQueue.async { [weak self] in
            guard let self else { return }
            var events: [(String, [[String: Any]])] = []
            var count = 0
            let maxEventsPerTick = 50
            while count < maxEventsPerTick, let json = self.core.lspPollEvent() {
                count += 1
                guard let data = json.data(using: .utf8),
                      let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let type = event["type"] as? String else { continue }

                switch type {
                case "diagnostics":
                    guard let uri = event["uri"] as? String,
                          let diagnosticsArray = event["diagnostics"] as? [[String: Any]] else { continue }
                    events.append((uri, diagnosticsArray))
                default:
                    break
                }
            }

            guard !events.isEmpty else { return }

            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                for (uri, diagnosticsArray) in events {
                    let filePath = self.uriToFilePath(uri)
                    guard let editorTab = self.findEditorTab(forPath: filePath) else { continue }

                    let markers: [MonacoDiagnostic] = diagnosticsArray.compactMap { d in
                        guard let severity = (d["severity"] as? NSNumber)?.uint8Value,
                              let startLine = (d["startLine"] as? NSNumber)?.uint32Value,
                              let startColumn = (d["startColumn"] as? NSNumber)?.uint32Value,
                              let endLine = (d["endLine"] as? NSNumber)?.uint32Value,
                              let endColumn = (d["endColumn"] as? NSNumber)?.uint32Value,
                              let message = d["message"] as? String else { return nil }
                        return MonacoDiagnostic(
                            severity: diagnosticSeverityToMonaco(severity),
                            startLine: startLine + 1,   // LSP 0-based -> Monaco 1-based
                            startColumn: startColumn + 1,
                            endLine: endLine + 1,
                            endColumn: endColumn + 1,
                            message: message,
                            source: d["source"] as? String
                        )
                    }
                    editorTab.applyDiagnostics(uri: uri, markers: markers)
                }
            }
        }
    }

    /// Sends LSP didOpen for a file if not already tracked.
    func lspDidOpenIfNeeded(path: String) {
        let uri = filePathToUri(path)
        guard !lspOpenFiles.contains(uri) else { return }

        guard let editorTab = findEditorTab(forPath: path) else {
            // Tab not created yet (async). didOpen will be sent when the
            // editor fires FileOpened via the notification observer.
            return
        }
        lspOpenFiles.insert(uri)
        lspDocVersions[uri] = 1

        let language = editorTab.language
        let content = editorTab.content

        lspQueue.async { [weak self] in
            guard let self else { return }
            self.core.lspEnsureServers(languageId: language, fileUri: uri)
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))","languageId":"\(self.jsonEscape(language))","version":1,"text":"\(self.jsonEscape(content))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didOpen", paramsJson: params)
        }
    }

    /// Sends LSP didChange for a content update.
    func lspDidChange(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }

        let version = (lspDocVersions[uri] ?? 1) + 1
        lspDocVersions[uri] = version

        let language = editor.language
        let content = editor.content

        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))","version":\(version)},"contentChanges":[{"text":"\(self.jsonEscape(content))"}]}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didChange", paramsJson: params)
        }
    }

    /// Sends LSP didClose when an editor tab is closed.
    func lspDidClose(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }
        lspOpenFiles.remove(uri)
        lspDocVersions.removeValue(forKey: uri)

        let language = editor.language
        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didClose", paramsJson: params)
        }
    }

    /// Sends LSP didSave after a file is saved.
    func lspDidSave(editor: EditorTab) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        guard lspOpenFiles.contains(uri) else { return }

        let language = editor.language
        lspQueue.async { [weak self] in
            guard let self else { return }
            let params = """
            {"textDocument":{"uri":"\(self.jsonEscape(uri))"}}
            """
            self.core.lspNotify(languageId: language, fileUri: uri, method: "textDocument/didSave", paramsJson: params)
        }
    }

    /// Handles a completion request from the editor by forwarding it to the LSP.
    func handleCompletionRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        latestCompletionReq[uri] = requestId

        // Cancel any previous in-flight completion request for this URI.
        completionWorkItems[uri]?.cancel()

        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        let workItem = DispatchWorkItem { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/completion", paramsJson: params
            ) else { return }

            let items = self.parseCompletionResponse(response)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                // Only resolve if this is still the latest request for this URI.
                guard self.latestCompletionReq[uri] == requestId else { return }
                editor.resolveCompletions(requestId: requestId, items: items)
            }
        }
        completionWorkItems[uri] = workItem
        lspQueue.async(execute: workItem)
    }

    /// Handles a hover request from the editor by forwarding it to the LSP.
    func handleHoverRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else { return }
        let uri = filePathToUri(path)
        latestHoverReq[uri] = requestId

        // Cancel any previous in-flight hover request for this URI.
        hoverWorkItems[uri]?.cancel()

        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        let workItem = DispatchWorkItem { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/hover", paramsJson: params
            ) else { return }

            let contents = self.parseHoverResponse(response)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard self.latestHoverReq[uri] == requestId else { return }
                editor.resolveHover(requestId: requestId, contents: contents)
            }
        }
        hoverWorkItems[uri] = workItem
        lspQueue.async(execute: workItem)
    }

    /// Handles a go-to-definition request from the editor.
    ///
    /// This is called on Cmd+hover (to show the underline) AND on Cmd+click.
    /// We only resolve the Monaco promise here — actual navigation for
    /// cross-file definitions happens via ``handleOpenFileRequested`` when
    /// Monaco's editor opener fires on click.
    func handleDefinitionRequest(editor: EditorTab, requestId: UInt64, line: UInt32, character: UInt32) {
        guard let path = editor.filePath else {
            editor.resolveDefinition(requestId: requestId, uri: nil, line: nil, column: nil)
            return
        }
        let uri = filePathToUri(path)
        let language = editor.language
        let params = """
        {"textDocument":{"uri":"\(jsonEscape(uri))"},"position":{"line":\(line),"character":\(character)}}
        """

        lspQueue.async { [weak self] in
            guard let self else { return }
            guard let response = self.core.lspRequest(
                languageId: language, fileUri: uri,
                method: "textDocument/definition", paramsJson: params
            ) else {
                DispatchQueue.main.async { editor.resolveDefinition(requestId: requestId, uri: nil, line: nil, column: nil) }
                return
            }

            guard let def = self.parseDefinitionResponse(response) else {
                DispatchQueue.main.async { editor.resolveDefinition(requestId: requestId, uri: nil, line: nil, column: nil) }
                return
            }

            DispatchQueue.main.async {
                // Resolve the Monaco promise with the definition location.
                // Monaco shows an underline on hover and navigates on click.
                // Same-file: Monaco navigates directly.
                // Cross-file: Monaco calls registerEditorOpener → OpenFileRequested.
                editor.resolveDefinition(requestId: requestId, uri: def.uri, line: def.line, column: def.character)
            }
        }
    }

    /// Handles Monaco's request to open a different file (cross-file go-to-definition).
    /// Fired by registerEditorOpener when the user actually Cmd+clicks.
    func handleOpenFileRequested(uri: String, line: UInt32, character: UInt32) {
        // Only open file:// URIs or plain paths (no scheme)
        guard uri.hasPrefix("file://") || !uri.contains("://") else {
            NSLog("Blocked opening non-file URI: \(uri)")
            return
        }
        let targetPath = uriToFilePath(uri)
        tabManager.addEditorTab(
            path: targetPath,
            goToLine: line + 1,
            goToColumn: character + 1
        )
        lspDidOpenIfNeeded(path: targetPath)
        if tabManager.selectedIndex >= 0,
           tabManager.selectedIndex < tabManager.tabs.count,
           case .editor(let targetEditor) = tabManager.tabs[tabManager.selectedIndex] {
            trackEditorTab(targetEditor, forPath: targetPath)
        }
    }

    // MARK: LSP Response Parsing

    private func parseCompletionResponse(_ json: String) -> [MonacoCompletionItem] {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) else { return [] }

        let items: [[String: Any]]
        if let list = response as? [String: Any],
           let listItems = list["items"] as? [[String: Any]] {
            items = listItems
        } else if let array = response as? [[String: Any]] {
            items = array
        } else {
            return []
        }

        return items.compactMap { item in
            guard let label = item["label"] as? String else { return nil }
            let kind = (item["kind"] as? NSNumber)?.intValue ?? 1
            let insertText = (item["insertText"] as? String) ?? label
            let detail = item["detail"] as? String
            let insertTextFormat = (item["insertTextFormat"] as? NSNumber)?.intValue ?? 1

            return MonacoCompletionItem(
                label: label,
                kind: lspCompletionKindFromInt(kind),
                detail: detail,
                insertText: insertText,
                insertTextRules: insertTextFormat == 2 ? 4 : nil
            )
        }
    }

    private func parseHoverResponse(_ json: String) -> [MonacoHoverContent] {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let contents = response["contents"] else { return [] }

        if let markup = contents as? [String: Any], let value = markup["value"] as? String {
            return [MonacoHoverContent(value: value, isTrusted: false)]
        } else if let str = contents as? String {
            return [MonacoHoverContent(value: str, isTrusted: false)]
        } else if let array = contents as? [[String: Any]] {
            return array.compactMap { item in
                guard let value = item["value"] as? String else { return nil }
                return MonacoHoverContent(value: value, isTrusted: false)
            }
        }
        return []
    }

    private func parseDefinitionResponse(_ json: String) -> (uri: String, line: UInt32, character: UInt32)? {
        guard let data = json.data(using: .utf8),
              let response = try? JSONSerialization.jsonObject(with: data) else { return nil }

        let location: [String: Any]?
        if let loc = response as? [String: Any], loc["uri"] != nil {
            location = loc
        } else if let array = response as? [[String: Any]], let first = array.first {
            if first["targetUri"] != nil {
                // LocationLink format
                guard let uri = first["targetUri"] as? String,
                      let range = (first["targetSelectionRange"] as? [String: Any])
                          ?? (first["targetRange"] as? [String: Any]),
                      let start = range["start"] as? [String: Any],
                      let line = (start["line"] as? NSNumber)?.uint32Value,
                      let character = (start["character"] as? NSNumber)?.uint32Value else { return nil }
                return (uri: uri, line: line, character: character)
            }
            location = first
        } else {
            return nil
        }

        guard let loc = location,
              let uri = loc["uri"] as? String,
              let range = loc["range"] as? [String: Any],
              let start = range["start"] as? [String: Any],
              let line = (start["line"] as? NSNumber)?.uint32Value,
              let character = (start["character"] as? NSNumber)?.uint32Value else { return nil }
        return (uri: uri, line: line, character: character)
    }

    // MARK: LSP Helpers

    /// Maps an LSP CompletionItemKind integer to a Monaco CompletionItemKind value.
    private func lspCompletionKindFromInt(_ kind: Int) -> UInt32 {
        switch kind {
        case 1:  return 18  // Text
        case 2:  return 0   // Method
        case 3:  return 1   // Function
        case 4:  return 2   // Constructor
        case 5:  return 3   // Field
        case 6:  return 4   // Variable
        case 7:  return 5   // Class
        case 8:  return 7   // Interface
        case 9:  return 8   // Module
        case 10: return 9   // Property
        case 11: return 12  // Unit
        case 12: return 13  // Value
        case 13: return 15  // Enum
        case 14: return 17  // Keyword
        case 15: return 27  // Snippet
        case 16: return 19  // Color
        case 17: return 20  // File
        case 18: return 21  // Reference
        case 19: return 23  // Folder
        case 20: return 16  // EnumMember
        case 21: return 14  // Constant
        case 22: return 6   // Struct
        case 23: return 10  // Event
        case 24: return 11  // Operator
        case 25: return 24  // TypeParameter
        default: return 18  // Text
        }
    }

    /// Converts an absolute file path to a file:// URI.
    private func filePathToUri(_ path: String) -> String {
        return URL(fileURLWithPath: path).absoluteString
    }

    /// Extracts the file path from a file:// URI.
    func uriToFilePath(_ uri: String) -> String {
        if let url = URL(string: uri), url.scheme == "file" {
            return url.path
        }
        return uri
    }

    /// Escapes a string for safe embedding in a JSON string literal.
    /// Uses a single-pass scanner instead of chained replacingOccurrences calls.
    private func jsonEscape(_ s: String) -> String {
        var result = ""
        result.reserveCapacity(s.count)
        for char in s {
            switch char {
            case "\\": result += "\\\\"
            case "\"": result += "\\\""
            case "\n": result += "\\n"
            case "\r": result += "\\r"
            case "\t": result += "\\t"
            default:
                if char.asciiValue.map({ $0 < 32 }) == true {
                    result += String(format: "\\u%04x", char.asciiValue!)
                } else {
                    result.append(char)
                }
            }
        }
        return result
    }

    /// Finds the editor tab that has the given file path open.
    /// Uses the editorTabsByPath dictionary for O(1) lookup, falling back
    /// to a linear scan if the dictionary is out of sync.
    func findEditorTab(forPath path: String) -> EditorTab? {
        if let editor = editorTabsByPath[path] {
            return editor
        }
        // Fallback: linear scan (keeps behavior correct if dictionary is stale).
        for tab in tabManager.tabs {
            if case .editor(let editor) = tab, editor.filePath == path {
                // Re-track it for future lookups.
                editorTabsByPath[path] = editor
                return editor
            }
        }
        return nil
    }
}
