"use strict";

// ---------------------------------------------------------------------------
// Platform-abstracted host communication
// ---------------------------------------------------------------------------
function sendToHost(msgObj) {
  const json = JSON.stringify(msgObj);
  if (
    window.webkit &&
    window.webkit.messageHandlers &&
    window.webkit.messageHandlers.impulse
  ) {
    // macOS WKWebView
    window.webkit.messageHandlers.impulse.postMessage(json);
  } else {
    // Qt WebEngine: QML intercepts console messages with this prefix
    console.log("IMPULSE_EVENT:" + json);
  }
}

// ---------------------------------------------------------------------------
// Font family normalization
// ---------------------------------------------------------------------------
// Monaco's fontFamily option is a CSS font-family string. Font names with
// spaces must be quoted (e.g. 'SF Mono'), and a monospace fallback should
// always be present so the editor never falls back to a proportional font.
function normalizeFontFamily(raw) {
  // Split on commas, trim each part, and quote unquoted multi-word names.
  const parts = raw
    .split(",")
    .map((s) => {
      s = s.trim();
      if (!s) return null;
      // Already quoted — leave as-is.
      if (
        (s.startsWith("'") && s.endsWith("'")) ||
        (s.startsWith('"') && s.endsWith('"'))
      )
        return s;
      // Generic families (monospace, sans-serif, etc.) must not be quoted.
      if (/^[a-z-]+$/.test(s)) return s;
      // Multi-word name without quotes — wrap in single quotes.
      if (s.includes(" ")) return "'" + s + "'";
      return s;
    })
    .filter(Boolean);
  // Ensure a monospace fallback is always present.
  const lower = parts.map((p) => p.toLowerCase().replace(/['"]/g, ""));
  if (!lower.includes("monospace")) parts.push("monospace");
  return parts.join(", ");
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
let editor = null;
let currentModel = null;
let currentFilePath = "";
let requestSeq = 0;
const pendingCompletions = new Map();
const pendingHovers = new Map();
const pendingDefinitions = new Map();
const pendingFormatting = new Map();
const pendingSignatureHelp = new Map();
const pendingReferences = new Map();
const pendingCodeActions = new Map();
const pendingRename = new Map();
const pendingPrepareRename = new Map();
let contentVersion = 0;
let currentDiffDecorations = [];
let pendingCommands = [];

// ---------------------------------------------------------------------------
// Monaco initialization
// ---------------------------------------------------------------------------
require.config({
  paths: { vs: "./vs" },
});

// Configure web workers — use document.baseURI so file:// paths resolve correctly
window.MonacoEnvironment = {
  getWorker: function (moduleId, label) {
    var baseUri = document.baseURI.substring(
      0,
      document.baseURI.lastIndexOf("/") + 1,
    );
    var workerUrl = baseUri + "vs/base/worker/workerMain.js";
    var blob = new Blob(
      [
        "self.MonacoEnvironment={baseUrl:" +
          JSON.stringify(baseUri) +
          "};importScripts(" +
          JSON.stringify(workerUrl) +
          ");",
      ],
      { type: "application/javascript" },
    );
    var blobUrl = URL.createObjectURL(blob);
    var worker = new Worker(blobUrl);
    URL.revokeObjectURL(blobUrl);
    return worker;
  },
};

require(["vs/editor/editor.main"], function () {
  document.getElementById("loading").style.display = "none";
  document.getElementById("container").style.display = "block";

  // ---------------------------------------------------------------------------
  // Disable Monaco's built-in TypeScript/JavaScript diagnostics.
  // Impulse uses its own LSP client (typescript-language-server) which provides
  // diagnostics that respect the project's tsconfig.json. Monaco's built-in
  // checker runs without project context, producing false positives.
  // ---------------------------------------------------------------------------
  monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions({
    noSemanticValidation: true,
    noSyntaxValidation: true,
    noSuggestionDiagnostics: true,
  });
  monaco.languages.typescript.javascriptDefaults.setDiagnosticsOptions({
    noSemanticValidation: true,
    noSyntaxValidation: true,
    noSuggestionDiagnostics: true,
  });

  // ---------------------------------------------------------------------------
  // Register JSON/JSONC Monarch tokenizer (the vendored Monaco bundle lacks one)
  // ---------------------------------------------------------------------------
  monaco.languages.setMonarchTokensProvider("json", {
    tokenPostfix: ".json",
    keywords: ["true", "false", "null"],
    tokenizer: {
      root: [
        // Whitespace & comments (JSONC)
        [/\/\/.*$/, "comment"],
        [/\/\*/, "comment", "@comment"],
        { include: "@whitespace" },
        // Object key (string before colon)
        [/"(?:[^"\\]|\\.)*"(?=\s*:)/, "string.key"],
        // String value
        [/"/, "string", "@string"],
        // Numbers
        [/-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/, "number"],
        // Keywords
        [/\b(?:true|false|null)\b/, "keyword.constant"],
        // Delimiters
        [/[{}[\]]/, "delimiter.bracket"],
        [/[,:]/, "delimiter"],
      ],
      string: [
        [/\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4})/, "string.escape"],
        [/\\./, "string.escape.invalid"],
        [/[^"\\]+/, "string"],
        [/"/, "string", "@pop"],
      ],
      comment: [
        [/[^/*]+/, "comment"],
        [/\*\//, "comment", "@pop"],
        [/./, "comment"],
      ],
      whitespace: [[/\s+/, ""]],
    },
  });

  editor = monaco.editor.create(document.getElementById("container"), {
    value: "",
    language: "plaintext",
    theme: "vs-dark",
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 14,
    fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
    fontLigatures: true,
    renderWhitespace: "selection",
    bracketPairColorization: { enabled: true },
    guides: { bracketPairs: true, indentation: true },
    smoothScrolling: false,
    mouseWheelScrollSensitivity: 1,
    cursorBlinking: "smooth",
    cursorSmoothCaretAnimation: "on",
    stickyScroll: { enabled: false },
    padding: { left: 8 },
    // Native-style overlay scrollbars: thin, no shadows, auto-fade via CSS
    scrollbar: {
      verticalScrollbarSize: 10,
      horizontalScrollbarSize: 10,
      verticalSliderSize: 10,
      horizontalSliderSize: 10,
      useShadows: false,
      verticalHasArrows: false,
      horizontalHasArrows: false,
    },
    suggest: {
      showIcons: true,
      showStatusBar: true,
      preview: true,
      shareSuggestSelections: true,
    },
    hover: { delay: 300 },
    // Use Alt for multi-cursor so Cmd+click (macOS) / Ctrl+click (Linux)
    // triggers go-to-definition instead of adding a cursor.
    multiCursorModifier: "alt",
    folding: true,
    foldingStrategy: "auto",
    showFoldingControls: "mouseover",
    lineNumbers: "on",
    glyphMargin: false,
    lineDecorationsWidth: 10,
    wordWrap: "off",
    tabSize: 4,
    insertSpaces: true,
    formatOnPaste: false,
    formatOnType: false,
    // Auto-rename matching HTML/JSX tags
    linkedEditing: true,
    // Hide line highlight when editor is not focused
    renderLineHighlightOnlyWhenFocus: true,
    // Keep context lines visible around cursor when scrolling
    cursorSurroundingLines: 3,
  });

  // --- Content change listener ---
  editor.onDidChangeModelContent(function (e) {
    contentVersion++;
    scheduleGitDiff(250);
    scheduleConflictLenses();
    sendToHost({
      type: "ContentChanged",
      changes: e.changes.map(function (change) {
        return {
          range: {
            start_line: change.range.startLineNumber,
            start_column: change.range.startColumn,
            end_line: change.range.endLineNumber,
            end_column: change.range.endColumn,
          },
          range_offset: change.rangeOffset,
          range_length: change.rangeLength,
          text: change.text,
        };
      }),
      version: contentVersion,
    });
  });

  // --- Cursor change listener (debounced) ---
  var cursorDebounceTimer = null;
  editor.onDidChangeCursorPosition(function (e) {
    scheduleBlame(e.position.lineNumber);
    clearTimeout(cursorDebounceTimer);
    cursorDebounceTimer = setTimeout(function () {
      sendToHost({
        type: "CursorMoved",
        line: e.position.lineNumber,
        column: e.position.column,
      });
    }, 50);
  });

  installGitPeekHandler();

  // --- Focus listeners ---
  editor.onDidFocusEditorText(function () {
    sendToHost({ type: "FocusChanged", focused: true });
  });
  editor.onDidBlurEditorText(function () {
    sendToHost({ type: "FocusChanged", focused: false });
  });

  // --- Ctrl+S keybinding ---
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, function () {
    sendToHost({ type: "SaveRequested" });
  });

  // ⌥⌘↑/↓ move focus between the app's split panes; give them up here
  // (Monaco binds them to "add cursor above/below").
  try {
    monaco.editor.addKeybindingRules([
      {
        keybinding: monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.UpArrow,
        command: "-editor.action.insertCursorAbove",
      },
      {
        keybinding: monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.DownArrow,
        command: "-editor.action.insertCursorBelow",
      },
    ]);
  } catch (e) {
    // Older Monaco without keybinding rules: the editor keeps them.
  }

  // --- Register LSP Completion Provider ---
  monaco.languages.registerCompletionItemProvider("*", {
    triggerCharacters: [".", ":", "<", '"', "/", "@", "\\", " "],
    provideCompletionItems: function (model, position) {
      const id = ++requestSeq;
      sendToHost({
        type: "CompletionRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve) {
        pendingCompletions.set(id, resolve);
        setTimeout(function () {
          if (pendingCompletions.has(id)) {
            pendingCompletions.delete(id);
            resolve({ suggestions: [] });
          }
        }, 5000);
      });
    },
  });

  // --- Register LSP Hover Provider ---
  monaco.languages.registerHoverProvider("*", {
    provideHover: function (model, position) {
      const id = ++requestSeq;
      sendToHost({
        type: "HoverRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve) {
        pendingHovers.set(id, resolve);
        setTimeout(function () {
          if (pendingHovers.has(id)) {
            pendingHovers.delete(id);
            resolve(null);
          }
        }, 5000);
      });
    },
  });

  // --- Register LSP Definition Provider ---
  monaco.languages.registerDefinitionProvider("*", {
    provideDefinition: function (model, position) {
      var id = ++requestSeq;
      sendToHost({
        type: "DefinitionRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve) {
        pendingDefinitions.set(id, resolve);
        setTimeout(function () {
          if (pendingDefinitions.has(id)) {
            pendingDefinitions.delete(id);
            resolve(null);
          }
        }, 5000);
      });
    },
  });

  // --- Register LSP Document Formatting Provider ---
  monaco.languages.registerDocumentFormattingEditProvider("*", {
    provideDocumentFormattingEdits: function (model, options) {
      var id = ++requestSeq;
      sendToHost({
        type: "FormattingRequested",
        request_id: id,
        tab_size: options.tabSize,
        insert_spaces: options.insertSpaces,
      });
      return new Promise(function (resolve) {
        pendingFormatting.set(id, resolve);
        setTimeout(function () {
          if (pendingFormatting.has(id)) {
            pendingFormatting.delete(id);
            resolve([]);
          }
        }, 10000);
      });
    },
  });

  // --- Register LSP Signature Help Provider ---
  monaco.languages.registerSignatureHelpProvider("*", {
    signatureHelpTriggerCharacters: ["(", ","],
    provideSignatureHelp: function (model, position) {
      var id = ++requestSeq;
      sendToHost({
        type: "SignatureHelpRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve) {
        pendingSignatureHelp.set(id, resolve);
        setTimeout(function () {
          if (pendingSignatureHelp.has(id)) {
            pendingSignatureHelp.delete(id);
            resolve(null);
          }
        }, 5000);
      });
    },
  });

  // --- Register LSP Reference Provider ---
  monaco.languages.registerReferenceProvider("*", {
    provideReferences: function (model, position) {
      var id = ++requestSeq;
      sendToHost({
        type: "ReferencesRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve) {
        pendingReferences.set(id, resolve);
        setTimeout(function () {
          if (pendingReferences.has(id)) {
            pendingReferences.delete(id);
            resolve([]);
          }
        }, 15000);
      });
    },
  });

  // --- Register LSP Code Action Provider ---
  monaco.languages.registerCodeActionProvider("*", {
    provideCodeActions: function (model, range, context) {
      var id = ++requestSeq;
      var diagnostics = (context.markers || []).map(function (m) {
        return {
          severity: m.severity,
          start_line: m.startLineNumber - 1,
          start_column: m.startColumn - 1,
          end_line: m.endLineNumber - 1,
          end_column: m.endColumn - 1,
          message: m.message,
          source: m.source || null,
        };
      });
      sendToHost({
        type: "CodeActionRequested",
        request_id: id,
        start_line: range.startLineNumber - 1,
        start_column: range.startColumn - 1,
        end_line: range.endLineNumber - 1,
        end_column: range.endColumn - 1,
        diagnostics: diagnostics,
      });
      return new Promise(function (resolve) {
        pendingCodeActions.set(id, resolve);
        setTimeout(function () {
          if (pendingCodeActions.has(id)) {
            pendingCodeActions.delete(id);
            resolve({ actions: [], dispose: function () {} });
          }
        }, 10000);
      });
    },
  });

  // --- Register LSP Rename Provider ---
  monaco.languages.registerRenameProvider("*", {
    provideRenameEdits: function (model, position, newName) {
      var id = ++requestSeq;
      sendToHost({
        type: "RenameRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
        new_name: newName,
      });
      return new Promise(function (resolve, reject) {
        pendingRename.set(id, { resolve: resolve, reject: reject });
        setTimeout(function () {
          if (pendingRename.has(id)) {
            pendingRename.delete(id);
            reject(new Error("Rename request timed out"));
          }
        }, 15000);
      });
    },
    resolveRenameLocation: function (model, position) {
      var id = ++requestSeq;
      sendToHost({
        type: "PrepareRenameRequested",
        request_id: id,
        line: position.lineNumber - 1,
        character: position.column - 1,
      });
      return new Promise(function (resolve, reject) {
        pendingPrepareRename.set(id, { resolve: resolve, reject: reject });
        setTimeout(function () {
          if (pendingPrepareRename.has(id)) {
            pendingPrepareRename.delete(id);
            reject(new Error("Prepare rename request timed out"));
          }
        }, 5000);
      });
    },
  });

  // --- Cross-file go-to-definition ---
  // Monaco calls this when Cmd+click resolves to a definition in a different
  // file URI. We forward the request to the host to open the target file.
  monaco.editor.registerEditorOpener({
    openCodeEditor: function (source, resource, selectionOrPosition) {
      var line = 0;
      var column = 0;
      if (selectionOrPosition) {
        if (typeof selectionOrPosition.lineNumber === "number") {
          line = selectionOrPosition.lineNumber - 1;
          column = (selectionOrPosition.column || 1) - 1;
        } else if (typeof selectionOrPosition.startLineNumber === "number") {
          line = selectionOrPosition.startLineNumber - 1;
          column = (selectionOrPosition.startColumn || 1) - 1;
        }
      }
      sendToHost({
        type: "OpenFileRequested",
        uri: resource.toString(),
        line: line,
        character: column,
      });
      return true;
    },
  });

  // Flush any commands that arrived before Monaco was ready
  pendingCommands.forEach(handleCommand);
  pendingCommands = [];

  // Force font remeasurement after layout settles — WebKit can measure
  // fonts before they are fully loaded, causing hit-test coordinate drift.
  requestAnimationFrame(function () {
    monaco.editor.remeasureFonts();
  });

  // Signal ready
  sendToHost({ type: "Ready" });
});

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------
function handleCommand(cmd) {
  try {
    switch (cmd.type) {
      case "OpenFile":
        handleOpenFile(cmd);
        break;
      case "SetTheme":
        handleSetTheme(cmd);
        break;
      case "UpdateSettings":
        handleUpdateSettings(cmd);
        break;
      case "ApplyDiagnostics":
        handleApplyDiagnostics(cmd);
        break;
      case "ResolveCompletions":
        handleResolveCompletions(cmd);
        break;
      case "ResolveHover":
        handleResolveHover(cmd);
        break;
      case "ResolveDefinition":
        handleResolveDefinition(cmd);
        break;
      case "GoToPosition":
        handleGoToPosition(cmd);
        break;
      case "SetReadOnly":
        editor.updateOptions({ readOnly: cmd.read_only });
        break;
      case "ApplyDiffDecorations":
        handleApplyDiffDecorations(cmd);
        break;
      case "SetGitBase":
        handleSetGitBase(cmd);
        break;
      case "ResolveFormatting":
        handleResolveFormatting(cmd);
        break;
      case "ResolveSignatureHelp":
        handleResolveSignatureHelp(cmd);
        break;
      case "ResolveReferences":
        handleResolveReferences(cmd);
        break;
      case "ResolveCodeActions":
        handleResolveCodeActions(cmd);
        break;
      case "ResolveRename":
        handleResolveRename(cmd);
        break;
      case "ResolvePrepareRename":
        handleResolvePrepareRename(cmd);
        break;
      default:
        console.warn("Unknown command:", cmd.type);
    }
  } catch (e) {
    console.error("Command handler error for", cmd.type, ":", e);
  }
}

// Expose handleCommand globally for Qt WebEngine (QML calls window.handleCommand directly)
window.handleCommand = handleCommand;

// ---------------------------------------------------------------------------
// Command handler: called from Rust via evaluate_javascript
// ---------------------------------------------------------------------------
window.impulseReceiveCommand = function (jsonString) {
  let cmd;
  try {
    cmd = JSON.parse(jsonString);
  } catch (e) {
    console.error("Failed to parse command:", e);
    return;
  }

  if (!editor) {
    pendingCommands.push(cmd);
    return;
  }

  handleCommand(cmd);
};

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

function handleOpenFile(cmd) {
  currentFilePath = cmd.file_path || "";
  const language = cmd.language || "plaintext";

  // Clear diff decorations from previous file
  currentDiffDecorations = editor.deltaDecorations(currentDiffDecorations, []);
  resetGitState();

  // Dispose old model if it exists
  if (currentModel) {
    currentModel.dispose();
  }

  // Clear pending LSP requests from previous file
  pendingCompletions.clear();
  pendingHovers.clear();
  pendingDefinitions.clear();
  pendingFormatting.clear();
  pendingSignatureHelp.clear();
  pendingReferences.clear();
  pendingCodeActions.clear();
  pendingRename.clear();
  pendingPrepareRename.clear();

  const uri = monaco.Uri.file(currentFilePath);
  currentModel = monaco.editor.createModel(cmd.content || "", language, uri);
  editor.setModel(currentModel);
  contentVersion = 0;
  conflictZones = [];
  conflictDecorations = [];
  lastConflictCount = 0;
  updateConflictLenses();

  // Reset undo stack by setting the model fresh
  editor.focus();
  sendToHost({ type: "FileOpened" });
}

function handleSetTheme(cmd) {
  const theme = cmd.theme;
  if (!theme) return;

  monaco.editor.defineTheme("impulse-theme", {
    base: theme.base || "vs-dark",
    inherit: theme.inherit !== false,
    rules: (theme.rules || []).map(function (r) {
      const rule = { token: r.token };
      if (r.foreground) rule.foreground = r.foreground;
      if (r.font_style) rule.fontStyle = r.font_style;
      return rule;
    }),
    colors: theme.colors || {},
  });
  monaco.editor.setTheme("impulse-theme");
  if (theme.colors) updateDiffGutterColors(theme.colors);
}

function handleUpdateSettings(cmd) {
  const opts = cmd.options || {};
  const update = {};
  if (opts.font_size != null) update.fontSize = opts.font_size;
  if (opts.font_family != null)
    update.fontFamily = normalizeFontFamily(opts.font_family);
  if (opts.tab_size != null) update.tabSize = opts.tab_size;
  if (opts.insert_spaces != null) update.insertSpaces = opts.insert_spaces;
  if (opts.word_wrap != null) update.wordWrap = opts.word_wrap;
  if (opts.minimap_enabled != null)
    update.minimap = { enabled: opts.minimap_enabled };
  if (opts.line_numbers != null) update.lineNumbers = opts.line_numbers;
  if (opts.render_whitespace != null)
    update.renderWhitespace = opts.render_whitespace;
  if (opts.render_line_highlight != null)
    update.renderLineHighlight = opts.render_line_highlight;
  if (opts.rulers != null) update.rulers = opts.rulers;
  if (opts.sticky_scroll != null)
    update.stickyScroll = { enabled: opts.sticky_scroll };
  if (opts.bracket_pair_colorization != null)
    update.bracketPairColorization = {
      enabled: opts.bracket_pair_colorization,
    };
  if (opts.indent_guides != null)
    update.guides = { indentation: opts.indent_guides };
  if (opts.font_ligatures != null) update.fontLigatures = opts.font_ligatures;
  if (opts.folding != null) update.folding = opts.folding;
  if (opts.scroll_beyond_last_line != null)
    update.scrollBeyondLastLine = opts.scroll_beyond_last_line;
  if (opts.smooth_scrolling != null)
    update.smoothScrolling = opts.smooth_scrolling;
  if (opts.cursor_style != null) update.cursorStyle = opts.cursor_style;
  if (opts.cursor_blinking != null)
    update.cursorBlinking = opts.cursor_blinking;
  if (opts.line_height != null) update.lineHeight = opts.line_height;
  if (opts.auto_closing_brackets != null)
    update.autoClosingBrackets = opts.auto_closing_brackets;
  if (opts.cursor_surrounding_lines != null)
    update.cursorSurroundingLines = opts.cursor_surrounding_lines;
  if (opts.selection_highlight != null)
    update.selectionHighlight = opts.selection_highlight;
  if (opts.occurrences_highlight != null)
    update.occurrencesHighlight = opts.occurrences_highlight;
  if (opts.word_based_suggestions != null)
    update.wordBasedSuggestions = opts.word_based_suggestions;
  editor.updateOptions(update);

  // Also update model options if tab settings changed
  if (currentModel && (opts.tab_size != null || opts.insert_spaces != null)) {
    currentModel.updateOptions({
      tabSize: opts.tab_size || currentModel.getOptions().tabSize,
      insertSpaces:
        opts.insert_spaces != null
          ? opts.insert_spaces
          : currentModel.getOptions().insertSpaces,
    });
  }
}

function handleApplyDiagnostics(cmd) {
  if (!currentModel) return;
  const markers = (cmd.markers || []).map(function (m) {
    return {
      severity: m.severity,
      startLineNumber: m.start_line + 1,
      startColumn: m.start_column + 1,
      endLineNumber: m.end_line + 1,
      endColumn: m.end_column + 1,
      message: m.message,
      source: m.source || "lsp",
    };
  });
  monaco.editor.setModelMarkers(currentModel, "lsp", markers);
}

function handleResolveCompletions(cmd) {
  const resolve = pendingCompletions.get(cmd.request_id);
  if (!resolve) return;
  pendingCompletions.delete(cmd.request_id);

  const suggestions = (cmd.items || []).map(function (item) {
    const suggestion = {
      label: item.label,
      kind: item.kind,
      insertText: item.insert_text || item.label,
      detail: item.detail || "",
    };
    if (item.insert_text_rules) {
      suggestion.insertTextRules = item.insert_text_rules;
    }
    if (item.range) {
      suggestion.range = {
        startLineNumber: item.range.start_line + 1,
        startColumn: item.range.start_column + 1,
        endLineNumber: item.range.end_line + 1,
        endColumn: item.range.end_column + 1,
      };
    }
    if (item.additional_text_edits && item.additional_text_edits.length > 0) {
      suggestion.additionalTextEdits = item.additional_text_edits.map(
        function (edit) {
          return {
            range: {
              startLineNumber: edit.range.start_line + 1,
              startColumn: edit.range.start_column + 1,
              endLineNumber: edit.range.end_line + 1,
              endColumn: edit.range.end_column + 1,
            },
            text: edit.text,
          };
        },
      );
    }
    return suggestion;
  });

  resolve({ suggestions: suggestions });
}

function handleResolveHover(cmd) {
  const resolve = pendingHovers.get(cmd.request_id);
  if (!resolve) return;
  pendingHovers.delete(cmd.request_id);

  const contents = (cmd.contents || []).map(function (c) {
    return { value: c.value, isTrusted: false };
  });

  if (contents.length === 0) {
    resolve(null);
  } else {
    resolve({ contents: contents });
  }
}

function handleResolveDefinition(cmd) {
  var resolve = pendingDefinitions.get(cmd.request_id);
  if (!resolve) return;
  pendingDefinitions.delete(cmd.request_id);

  if (cmd.uri && cmd.line != null && cmd.column != null) {
    // Return a Location so Monaco can show the underline link on Cmd+hover.
    // For same-file definitions Monaco navigates directly; for cross-file
    // definitions the host handles navigation via the DefinitionRequested flow.
    resolve({
      uri: monaco.Uri.parse(cmd.uri),
      range: {
        startLineNumber: cmd.line + 1,
        startColumn: cmd.column + 1,
        endLineNumber: cmd.line + 1,
        endColumn: cmd.column + 1,
      },
    });
  } else {
    resolve(null);
  }
}

// Positions from the host are 1-based, like Monaco's.
function handleGoToPosition(cmd) {
  const line = Math.max(1, cmd.line || 1);
  const column = Math.max(1, cmd.column || 1);
  editor.setPosition({ lineNumber: line, column: column });
  editor.revealPositionInCenter({ lineNumber: line, column: column });
  editor.focus();
}

function handleApplyDiffDecorations(cmd) {
  const decorations = (cmd.decorations || []).map(function (d) {
    var className;
    switch (d.status) {
      case "added":
        className = "diff-gutter-added";
        break;
      case "modified":
        className = "diff-gutter-modified";
        break;
      case "deleted":
        className = "diff-gutter-deleted";
        break;
      default:
        className = "diff-gutter-added";
    }
    return {
      range: new monaco.Range(d.line, 1, d.line, 1),
      options: {
        isWholeLine: true,
        linesDecorationsClassName: className,
      },
    };
  });
  currentDiffDecorations = editor.deltaDecorations(
    currentDiffDecorations,
    decorations,
  );
}

function handleResolveFormatting(cmd) {
  var resolve = pendingFormatting.get(cmd.request_id);
  if (!resolve) return;
  pendingFormatting.delete(cmd.request_id);

  var edits = (cmd.edits || []).map(function (e) {
    return {
      range: {
        startLineNumber: e.range.start_line + 1,
        startColumn: e.range.start_column + 1,
        endLineNumber: e.range.end_line + 1,
        endColumn: e.range.end_column + 1,
      },
      text: e.text,
    };
  });
  resolve(edits);
}

function handleResolveSignatureHelp(cmd) {
  var resolve = pendingSignatureHelp.get(cmd.request_id);
  if (!resolve) return;
  pendingSignatureHelp.delete(cmd.request_id);

  if (!cmd.signature_help) {
    resolve(null);
    return;
  }

  var sh = cmd.signature_help;
  var signatures = (sh.signatures || []).map(function (sig) {
    var params = (sig.parameters || []).map(function (p) {
      var param = { label: p.label };
      if (p.documentation) {
        param.documentation = { value: p.documentation };
      }
      return param;
    });
    var result = {
      label: sig.label,
      parameters: params,
    };
    if (sig.documentation) {
      result.documentation = { value: sig.documentation };
    }
    return result;
  });

  resolve({
    value: {
      signatures: signatures,
      activeSignature: sh.active_signature,
      activeParameter: sh.active_parameter,
    },
    dispose: function () {},
  });
}

function handleResolveReferences(cmd) {
  var resolve = pendingReferences.get(cmd.request_id);
  if (!resolve) return;
  pendingReferences.delete(cmd.request_id);

  var locations = (cmd.locations || []).map(function (loc) {
    return {
      uri: monaco.Uri.parse(loc.uri),
      range: {
        startLineNumber: loc.range.start_line + 1,
        startColumn: loc.range.start_column + 1,
        endLineNumber: loc.range.end_line + 1,
        endColumn: loc.range.end_column + 1,
      },
    };
  });
  resolve(locations);
}

function handleResolveCodeActions(cmd) {
  var resolve = pendingCodeActions.get(cmd.request_id);
  if (!resolve) return;
  pendingCodeActions.delete(cmd.request_id);

  var actions = (cmd.actions || []).map(function (action) {
    var workspaceEdits = (action.edits || []).map(function (e) {
      return {
        resource: monaco.Uri.parse(e.uri),
        textEdit: {
          range: {
            startLineNumber: e.range.start_line + 1,
            startColumn: e.range.start_column + 1,
            endLineNumber: e.range.end_line + 1,
            endColumn: e.range.end_column + 1,
          },
          text: e.text,
        },
      };
    });
    return {
      title: action.title,
      kind: action.kind || undefined,
      isPreferred: action.is_preferred,
      edit: { edits: workspaceEdits },
    };
  });

  resolve({ actions: actions, dispose: function () {} });
}

function handleResolveRename(cmd) {
  var pending = pendingRename.get(cmd.request_id);
  if (!pending) return;
  pendingRename.delete(cmd.request_id);

  var edits = (cmd.edits || []).map(function (e) {
    return {
      resource: monaco.Uri.parse(e.uri),
      textEdit: {
        range: {
          startLineNumber: e.range.start_line + 1,
          startColumn: e.range.start_column + 1,
          endLineNumber: e.range.end_line + 1,
          endColumn: e.range.end_column + 1,
        },
        text: e.text,
      },
    };
  });
  pending.resolve({ edits: edits });
}

function handleResolvePrepareRename(cmd) {
  var pending = pendingPrepareRename.get(cmd.request_id);
  if (!pending) return;
  pendingPrepareRename.delete(cmd.request_id);

  if (cmd.range) {
    pending.resolve({
      range: {
        startLineNumber: cmd.range.start_line + 1,
        startColumn: cmd.range.start_column + 1,
        endLineNumber: cmd.range.end_line + 1,
        endColumn: cmd.range.end_column + 1,
      },
      text: cmd.placeholder || "",
    });
  } else {
    pending.reject(new Error("Symbol cannot be renamed"));
  }
}

function isValidCssColor(c) {
  return (
    typeof c === "string" &&
    /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(c)
  );
}

function updateDiffGutterColors(colors) {
  var addedColor = colors["impulse.diffAddedColor"];
  var modifiedColor = colors["impulse.diffModifiedColor"];
  var deletedColor = colors["impulse.diffDeletedColor"];
  if (!addedColor && !modifiedColor && !deletedColor) return;

  var safeAdded = isValidCssColor(addedColor) ? addedColor : "#9ece6a";
  var safeModified = isValidCssColor(modifiedColor) ? modifiedColor : "#e0af68";
  var safeDeleted = isValidCssColor(deletedColor) ? deletedColor : "#f7768e";

  var root = document.documentElement.style;
  root.setProperty("--git-added", safeAdded);
  root.setProperty("--git-modified", safeModified);
  root.setProperty("--git-removed", safeDeleted);
  if (isValidCssColor(colors["editorLineNumber.foreground"]))
    root.setProperty("--git-blame", colors["editorLineNumber.foreground"]);
  if (isValidCssColor(colors["editorWidget.background"]))
    root.setProperty("--git-peek-bg", colors["editorWidget.background"]);
  if (isValidCssColor(colors["editor.foreground"]))
    root.setProperty("--git-peek-fg", colors["editor.foreground"]);

  var styleId = "impulse-diff-gutter-style";
  var existing = document.getElementById(styleId);
  if (existing) existing.remove();

  var style = document.createElement("style");
  style.id = styleId;
  style.textContent =
    ".diff-gutter-added { background: " +
    safeAdded +
    "; }" +
    ".diff-gutter-modified { background: " +
    safeModified +
    "; }" +
    ".diff-gutter-deleted { background: " +
    safeDeleted +
    "; }";
  document.head.appendChild(style);
}


// ===========================================================================
// Git: change marks against the index version, peek, and inline blame
//
// The host sends the file's base (its index version) with SetGitBase after
// opening/saving and when the index changes. Marks are computed here from the
// live buffer, so they follow typing. Clicking a mark opens a peek with the
// original lines and Revert / Stage / Review actions.
// ===========================================================================
let gitBaseLines = null; // null: not in a repository (no marks)
let gitHunks = [];
let gitDecorations = [];
let gitDiffTimer = null;
let gitBlame = new Map();
let gitBlameVersion = -1;
let blameDecorations = [];
let blameTimer = null;
let gitPeek = null; // { zoneId, hunk }

function resetGitState() {
  gitBaseLines = null;
  gitHunks = [];
  gitBlame = new Map();
  if (editor) {
    gitDecorations = editor.deltaDecorations(gitDecorations, []);
    blameDecorations = editor.deltaDecorations(blameDecorations, []);
  }
  closeGitPeek();
}

function handleSetGitBase(cmd) {
  gitBaseLines = typeof cmd.base === "string" ? splitGitLines(cmd.base) : null;
  gitBlame = new Map();
  (cmd.blame || []).forEach(function (b) {
    gitBlame.set(b.line, b);
  });
  gitBlameVersion = contentVersion;
  // The old marks (from the previous base) are now meaningless.
  currentDiffDecorations = editor.deltaDecorations(currentDiffDecorations, []);
  scheduleGitDiff(0);
}

function splitGitLines(text) {
  if (text === "") return [];
  var lines = text.split(/\r?\n/);
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

function scheduleGitDiff(delay) {
  clearTimeout(gitDiffTimer);
  gitDiffTimer = setTimeout(updateGitDiff, delay);
}

function updateGitDiff() {
  if (!editor || !currentModel) return;
  if (gitBaseLines == null) {
    gitHunks = [];
    gitDecorations = editor.deltaDecorations(gitDecorations, []);
    return;
  }
  var current = currentModel.getLinesContent().slice();
  if (current.length && current[current.length - 1] === "") current.pop();
  gitHunks = diffLineHunks(gitBaseLines, current);
  var lineCount = currentModel.getLineCount();
  var decorations = [];
  gitHunks.forEach(function (h) {
    var kind = h.newCount === 0 ? "deleted" : h.oldLines.length === 0 ? "added" : "modified";
    var className = "diff-gutter-" + kind;
    var color = kind === "added" ? "--git-added" : kind === "modified" ? "--git-modified" : "--git-removed";
    var ruler = getComputedStyle(document.documentElement).getPropertyValue(color).trim() || "#888";
    if (kind === "deleted") {
      var line = Math.min(Math.max(h.newStart - 1, 1), lineCount);
      decorations.push({
        range: new monaco.Range(line, 1, line, 1),
        options: {
          linesDecorationsClassName: "diff-gutter-deleted",
          overviewRuler: { color: ruler, position: monaco.editor.OverviewRulerLane.Left },
        },
      });
    } else {
      decorations.push({
        range: new monaco.Range(h.newStart, 1, h.newStart + h.newCount - 1, 1),
        options: {
          isWholeLine: true,
          linesDecorationsClassName: className,
          overviewRuler: { color: ruler, position: monaco.editor.OverviewRulerLane.Left },
        },
      });
    }
  });
  gitDecorations = editor.deltaDecorations(gitDecorations, decorations);
  if (gitPeek) {
    // Keep the peek on its hunk if it still exists; otherwise close it.
    var same = gitHunks.find(function (h) {
      return h.newStart === gitPeek.hunk.newStart;
    });
    if (same) renderGitPeek(same);
    else closeGitPeek();
  }
}

// Line diff: trim the common prefix/suffix, then Myers on the middle.
function diffLineHunks(a, b) {
  var start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  var endA = a.length;
  var endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  var midA = a.slice(start, endA);
  var midB = b.slice(start, endB);
  if (midA.length === 0 && midB.length === 0) return [];
  var ops = myersOps(midA, midB);
  if (!ops) {
    // Too different to diff cheaply: one hunk covering the middle.
    return [{ oldStart: start + 1, oldLines: midA, newStart: start + 1, newCount: midB.length }];
  }
  var hunks = [];
  var current = null;
  var ai = 0;
  var bi = 0;
  ops.forEach(function (op) {
    if (op === "=") {
      if (current) {
        hunks.push(current);
        current = null;
      }
      ai++;
      bi++;
      return;
    }
    if (!current) current = { oldStart: start + ai + 1, oldLines: [], newStart: start + bi + 1, newCount: 0 };
    if (op === "-") {
      current.oldLines.push(midA[ai]);
      ai++;
    } else {
      current.newCount++;
      bi++;
    }
  });
  if (current) hunks.push(current);
  return hunks;
}

// Myers O(ND) edit script as a list of "=", "-", "+"; null if D is huge.
// Each step's trace keeps only the diagonals it can reach (k in [-d, d]),
// so memory is O(D^2) rather than O(D * (N + M)).
function myersOps(a, b) {
  var n = a.length;
  var m = b.length;
  var max = n + m;
  var limit = Math.min(max, 1500);
  var offset = max + 1;
  var v = new Int32Array(2 * max + 3);
  var trace = [];
  for (var d = 0; d <= limit; d++) {
    // Snapshot diagonals -d-1 .. d+1 before this step mutates them.
    trace.push(v.slice(offset - d - 1, offset + d + 2));
    for (var k = -d; k <= d; k += 2) {
      var x;
      if (k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])) {
        x = v[offset + k + 1];
      } else {
        x = v[offset + k - 1] + 1;
      }
      var y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        return backtrack(trace, n, m, d);
      }
    }
  }
  return null;
}

function backtrack(trace, n, m, dFinal) {
  var x = n;
  var y = m;
  var ops = [];
  for (var d = dFinal; d > 0; d--) {
    var band = trace[d];
    var at = function (k) {
      return band[k + d + 1];
    };
    var k = x - y;
    var prevK = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
    var prevX = at(prevK);
    var prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      ops.push("=");
      x--;
      y--;
    }
    if (x === prevX) {
      ops.push("+");
      y--;
    } else {
      ops.push("-");
      x--;
    }
  }
  while (x > 0 && y > 0) {
    ops.push("=");
    x--;
    y--;
  }
  return ops.reverse();
}

// --- Peek -------------------------------------------------------------------

function gitHunkAtLine(line) {
  return gitHunks.find(function (h) {
    if (h.newCount === 0) return line === Math.max(h.newStart - 1, 1);
    return line >= h.newStart && line < h.newStart + h.newCount;
  });
}

function installGitPeekHandler() {
  if (!editor || editor.__gitPeekInstalled) return;
  editor.__gitPeekInstalled = true;
  // Blame: click the ghost text (or use the context menu) to open the
  // line's commit in History.
  editor.onMouseDown(function (e) {
    var injected = e.target && e.target.detail && e.target.detail.injectedText;
    if (!injected || !injected.options || injected.options.inlineClassName !== "git-blame-ghost") return;
    showLineCommit(e.target.position && e.target.position.lineNumber);
  });
  editor.addAction({
    id: "impulse.nextConflict",
    label: "Go to Next Merge Conflict",
    precondition: null,
    run: function () { goToConflict(1); },
  });
  editor.addAction({
    id: "impulse.previousConflict",
    label: "Go to Previous Merge Conflict",
    run: function () { goToConflict(-1); },
  });
  editor.addAction({
    id: "impulse.showLineCommit",
    label: "Show Commit for This Line",
    contextMenuGroupId: "9_git",
    run: function (ed) {
      var position = ed.getPosition();
      showLineCommit(position && position.lineNumber);
    },
  });
  editor.onMouseDown(function (e) {
    if (!e.target || e.target.type !== monaco.editor.MouseTargetType.GUTTER_LINE_DECORATIONS) return;
    var line = e.target.position && e.target.position.lineNumber;
    var hunk = line && gitHunkAtLine(line);
    if (!hunk) return;
    if (gitPeek && gitPeek.hunk.newStart === hunk.newStart) closeGitPeek();
    else renderGitPeek(hunk);
  });
}

function showLineCommit(line) {
  var info = line && gitBlame.get(line);
  if (!info || /^0+$/.test(info.sha)) return;
  sendToHost({ type: "GitAction", action: "commit:" + info.sha, line: line });
}

function renderGitPeek(hunk) {
  closeGitPeek();
  var node = document.createElement("div");
  node.className = "git-peek";
  var bar = document.createElement("div");
  bar.className = "git-peek-bar";
  var title = document.createElement("span");
  title.className = "git-peek-title";
  var removed = hunk.oldLines.length;
  title.textContent =
    hunk.newCount === 0
      ? removed + " line" + (removed === 1 ? "" : "s") + " removed"
      : removed === 0
        ? hunk.newCount + " line" + (hunk.newCount === 1 ? "" : "s") + " added"
        : "Changed " + hunk.newCount + " line" + (hunk.newCount === 1 ? "" : "s") + " (was " + removed + ")";
  bar.appendChild(title);
  bar.appendChild(peekButton("Revert", "Restore the original lines in the editor", function () {
    revertGitHunk(hunk);
  }));
  bar.appendChild(peekButton("Stage", "Stage this change (saves first)", function () {
    sendToHost({ type: "GitAction", action: "stage", line: anchorLine(hunk) });
  }));
  bar.appendChild(peekButton("Review", "Open in the review", function () {
    sendToHost({ type: "GitAction", action: "review", line: anchorLine(hunk) });
  }));
  bar.appendChild(peekButton("✕", "Close", closeGitPeek));
  node.appendChild(bar);
  if (hunk.oldLines.length) {
    var pre = document.createElement("pre");
    pre.className = "git-peek-old";
    pre.textContent = hunk.oldLines.join("\n");
    node.appendChild(pre);
  }
  var lineHeight = editor.getOption(monaco.editor.EditorOption.lineHeight);
  var heightInLines = 1.6 + Math.min(hunk.oldLines.length, 12);
  var afterLine = hunk.newCount === 0 ? Math.max(hunk.newStart - 1, 0) : hunk.newStart + hunk.newCount - 1;
  var zoneId = null;
  editor.changeViewZones(function (accessor) {
    zoneId = accessor.addZone({
      afterLineNumber: afterLine,
      heightInPx: Math.round(heightInLines * lineHeight) + 8,
      domNode: node,
      suppressMouseDown: true,
    });
  });
  gitPeek = { zoneId: zoneId, hunk: hunk };
}

// --- Merge conflict lenses ---
// Above each <<<<<<< block: Accept Current / Incoming / Both, applied as one
// undoable edit; the two sides are tinted.

var conflictZones = [];
var conflictDecorations = [];
var conflictTimer = null;

function scheduleConflictLenses() {
  clearTimeout(conflictTimer);
  conflictTimer = setTimeout(updateConflictLenses, 150);
}

/** Conflict blocks as 1-based line numbers: start (<<<<<<<), optional
 *  base (|||||||), mid (=======), end (>>>>>>>). */
function findConflicts(model) {
  var blocks = [];
  if (model.findMatches("<<<<<<<", false, false, true, null, false, 1).length === 0) return blocks;
  var lines = model.getLinesContent();
  for (var i = 0; i < lines.length; i++) {
    if (lines[i].indexOf("<<<<<<<") !== 0) continue;
    var base = -1, mid = -1, end = -1;
    for (var j = i + 1; j < lines.length; j++) {
      if (lines[j].indexOf("<<<<<<<") === 0) break;
      if (lines[j].indexOf("|||||||") === 0 && mid < 0 && base < 0) base = j;
      else if (lines[j].indexOf("=======") === 0 && mid < 0) mid = j;
      else if (lines[j].indexOf(">>>>>>>") === 0 && mid >= 0) { end = j; break; }
    }
    if (mid < 0 || end < 0) continue;
    blocks.push({
      start: i + 1, base: base >= 0 ? base + 1 : -1, mid: mid + 1, end: end + 1,
      currentLabel: lines[i].slice(7).trim() || "current",
      incomingLabel: lines[end].slice(7).trim() || "incoming",
    });
    i = end;
  }
  return blocks;
}

function updateConflictLenses() {
  var model = currentModel;
  if (!editor || !model) return;
  var blocks = findConflicts(model);
  var decorations = [];
  function whole(from, to, className) {
    if (from > to) return;
    decorations.push({ range: new monaco.Range(from, 1, to, 1), options: { isWholeLine: true, className: className } });
  }
  blocks.forEach(function (b) {
    var currentEnd = (b.base > 0 ? b.base : b.mid) - 1;
    whole(b.start, b.start, "conflict-marker-line");
    whole(b.start + 1, currentEnd, "conflict-current");
    if (b.base > 0) whole(b.base, b.mid - 1, "conflict-marker-line");
    whole(b.mid, b.mid, "conflict-marker-line");
    whole(b.mid + 1, b.end - 1, "conflict-incoming");
    whole(b.end, b.end, "conflict-marker-line");
  });
  conflictDecorations = editor.deltaDecorations(conflictDecorations, decorations);
  editor.changeViewZones(function (accessor) {
    conflictZones.forEach(function (id) { accessor.removeZone(id); });
    conflictZones = blocks.map(function (b, index) {
      var bar = document.createElement("div");
      bar.className = "conflict-bar";
      bar.appendChild(peekButton("Accept Current", "Keep " + b.currentLabel, function () { resolveConflict(b, "current"); }));
      bar.appendChild(peekButton("Accept Incoming", "Take " + b.incomingLabel, function () { resolveConflict(b, "incoming"); }));
      bar.appendChild(peekButton("Accept Both", "Current, then incoming", function () { resolveConflict(b, "both"); }));
      var note = document.createElement("span");
      note.className = "git-peek-title";
      note.textContent = b.currentLabel + " ⟷ " + b.incomingLabel;
      bar.appendChild(note);
      if (blocks.length > 1) {
        var count = document.createElement("span");
        count.className = "git-peek-title conflict-count";
        count.textContent = index + 1 + " of " + blocks.length;
        bar.appendChild(count);
        bar.appendChild(peekButton("↑", "Previous conflict", function () { goToConflict(-1, b.start); }));
        bar.appendChild(peekButton("↓", "Next conflict", function () { goToConflict(1, b.start); }));
      }
      return accessor.addZone({ afterLineNumber: b.start - 1, heightInPx: 24, domNode: bar, suppressMouseDown: true });
    });
  });
  if (blocks.length !== lastConflictCount) {
    lastConflictCount = blocks.length;
    sendToHost({ type: "GitAction", action: blocks.length ? "conflicts" : "conflicts-resolved", line: blocks.length });
  }
}
var lastConflictCount = 0;

/** Move to the next (+1) or previous (-1) conflict after/before `fromLine`
 *  (the cursor when omitted), wrapping around. */
function goToConflict(direction, fromLine) {
  if (!editor || !currentModel) return;
  var blocks = findConflicts(currentModel);
  if (!blocks.length) return;
  var line = fromLine || (editor.getPosition() || { lineNumber: 1 }).lineNumber;
  var target = direction > 0
    ? blocks.find(function (b) { return b.start > line; }) || blocks[0]
    : blocks.slice().reverse().find(function (b) { return b.start < line; }) || blocks[blocks.length - 1];
  editor.setPosition({ lineNumber: target.start + 1, column: 1 });
  editor.revealLineInCenter(target.start);
  editor.focus();
}

function resolveConflict(b, choice) {
  var model = currentModel;
  if (!model) return;
  // Re-find the block: earlier edits may have moved it.
  var fresh = findConflicts(model).filter(function (c) { return c.currentLabel === b.currentLabel && c.incomingLabel === b.incomingLabel; });
  var block = fresh.reduce(function (best, c) {
    return best === null || Math.abs(c.start - b.start) < Math.abs(best.start - b.start) ? c : best;
  }, null);
  if (!block) return;
  var lines = model.getLinesContent();
  var current = lines.slice(block.start, (block.base > 0 ? block.base : block.mid) - 1);
  var incoming = lines.slice(block.mid, block.end - 1);
  var kept = choice === "current" ? current : choice === "incoming" ? incoming : current.concat(incoming);
  var eol = model.getEOL();
  var lineCount = model.getLineCount();
  var range, text;
  if (block.end < lineCount) {
    range = new monaco.Range(block.start, 1, block.end + 1, 1);
    text = kept.length ? kept.join(eol) + eol : "";
  } else {
    range = new monaco.Range(block.start, 1, block.end, model.getLineMaxColumn(block.end));
    text = kept.join(eol);
  }
  editor.pushUndoStop();
  editor.executeEdits("conflict", [{ range: range, text: text, forceMoveMarkers: true }]);
  editor.pushUndoStop();
  editor.focus();
}

function anchorLine(hunk) {
  return hunk.newCount === 0 ? Math.max(hunk.newStart - 1, 1) : hunk.newStart;
}

function peekButton(label, title, onClick) {
  var b = document.createElement("button");
  b.className = "git-peek-button";
  b.textContent = label;
  b.title = title;
  b.addEventListener("mousedown", function (e) {
    e.preventDefault();
    e.stopPropagation();
    onClick();
  });
  return b;
}

function closeGitPeek() {
  if (!gitPeek || !editor) {
    gitPeek = null;
    return;
  }
  var id = gitPeek.zoneId;
  editor.changeViewZones(function (accessor) {
    accessor.removeZone(id);
  });
  gitPeek = null;
}

function revertGitHunk(hunk) {
  var model = currentModel;
  if (!model) return;
  var lineCount = model.getLineCount();
  var range;
  var text;
  if (hunk.newCount === 0) {
    // Re-insert removed lines before newStart.
    var at = Math.min(hunk.newStart, lineCount + 1);
    if (at > lineCount) {
      var lastCol = model.getLineMaxColumn(lineCount);
      range = new monaco.Range(lineCount, lastCol, lineCount, lastCol);
      text = "\n" + hunk.oldLines.join("\n");
    } else {
      range = new monaco.Range(at, 1, at, 1);
      text = hunk.oldLines.join("\n") + "\n";
    }
  } else if (hunk.oldLines.length === 0) {
    // Remove added lines (including their line breaks).
    var first = hunk.newStart;
    var last = hunk.newStart + hunk.newCount - 1;
    if (last < lineCount) {
      range = new monaco.Range(first, 1, last + 1, 1);
    } else if (first > 1) {
      range = new monaco.Range(first - 1, model.getLineMaxColumn(first - 1), last, model.getLineMaxColumn(last));
    } else {
      range = new monaco.Range(first, 1, last, model.getLineMaxColumn(last));
    }
    text = "";
  } else {
    var end = hunk.newStart + hunk.newCount - 1;
    range = new monaco.Range(hunk.newStart, 1, end, model.getLineMaxColumn(end));
    text = hunk.oldLines.join("\n");
  }
  editor.pushUndoStop();
  editor.executeEdits("git-revert", [{ range: range, text: text, forceMoveMarkers: true }]);
  editor.pushUndoStop();
  closeGitPeek();
}

// --- Inline blame -------------------------------------------------------------

function scheduleBlame(line) {
  clearTimeout(blameTimer);
  if (blameDecorations.length) blameDecorations = editor.deltaDecorations(blameDecorations, []);
  blameTimer = setTimeout(function () {
    showBlame(line);
  }, 450);
}

function showBlame(line) {
  if (!editor || !currentModel || gitBlame.size === 0) return;
  // Blame describes the saved file; skip once the buffer has diverged, and
  // on lines with uncommitted edits.
  if (contentVersion !== gitBlameVersion) return;
  if (gitHunkAtLine(line)) return;
  var info = gitBlame.get(line);
  if (!info || /^0+$/.test(info.sha)) return;
  var text = "    " + info.author + ", " + relativeTime(info.time) + " · " + info.summary;
  var col = currentModel.getLineMaxColumn(line);
  blameDecorations = editor.deltaDecorations(blameDecorations, [
    {
      range: new monaco.Range(line, col, line, col),
      options: { after: { content: text, inlineClassName: "git-blame-ghost" } },
    },
  ]);
}

function relativeTime(seconds) {
  var delta = Date.now() / 1000 - seconds;
  var units = [
    ["year", 31536000],
    ["month", 2592000],
    ["week", 604800],
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ];
  for (var i = 0; i < units.length; i++) {
    var n = Math.floor(delta / units[i][1]);
    if (n >= 1) return n + " " + units[i][0] + (n === 1 ? "" : "s") + " ago";
  }
  return "just now";
}
