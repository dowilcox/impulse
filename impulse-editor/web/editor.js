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
    window.webkit.messageHandlers.impulse.postMessage(json);
  }
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
let contentChangeTimer = null;
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
    return new Worker(URL.createObjectURL(blob));
  },
};

require(["vs/editor/editor.main"], function () {
  document.getElementById("loading").style.display = "none";
  document.getElementById("container").style.display = "block";

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
    padding: { top: 4 },
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
  });

  // --- Content change listener (debounced) ---
  editor.onDidChangeModelContent(function () {
    contentVersion++;
    if (contentChangeTimer) clearTimeout(contentChangeTimer);
    contentChangeTimer = setTimeout(function () {
      sendToHost({
        type: "ContentChanged",
        content: editor.getValue(),
        version: contentVersion,
      });
    }, 300);
  });

  // --- Cursor change listener (debounced) ---
  var cursorDebounceTimer = null;
  editor.onDidChangeCursorPosition(function (e) {
    clearTimeout(cursorDebounceTimer);
    cursorDebounceTimer = setTimeout(function () {
      sendToHost({
        type: "CursorMoved",
        line: e.position.lineNumber,
        column: e.position.column,
      });
    }, 50);
  });

  // --- Focus listeners ---
  editor.onDidFocusEditorText(function () {
    sendToHost({ type: "FocusChanged", focused: true });
  });
  editor.onDidBlurEditorText(function () {
    sendToHost({ type: "FocusChanged", focused: false });
  });

  // --- Ctrl+S keybinding ---
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, function () {
    // Flush any pending debounced content change so the host has the
    // latest content before we request a save.
    if (contentChangeTimer) {
      clearTimeout(contentChangeTimer);
      contentChangeTimer = null;
      contentVersion++;
      sendToHost({
        type: "ContentChanged",
        content: editor.getValue(),
        version: contentVersion,
      });
    }
    sendToHost({ type: "SaveRequested" });
  });

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
      default:
        console.warn("Unknown command:", cmd.type);
    }
  } catch (e) {
    console.error("Command handler error for", cmd.type, ":", e);
  }
}

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

  // Dispose old model if it exists
  if (currentModel) {
    currentModel.dispose();
  }

  const uri = monaco.Uri.file(currentFilePath);
  currentModel = monaco.editor.createModel(cmd.content || "", language, uri);
  editor.setModel(currentModel);
  contentVersion = 0;

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
  if (opts.font_family != null) update.fontFamily = opts.font_family;
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

function handleGoToPosition(cmd) {
  const line = (cmd.line || 0) + 1;
  const column = (cmd.column || 0) + 1;
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

function isValidCssColor(c) {
  return typeof c === "string" && /^#[0-9a-fA-F]{3,8}$/.test(c);
}

function updateDiffGutterColors(colors) {
  var addedColor = colors["impulse.diffAddedColor"];
  var modifiedColor = colors["impulse.diffModifiedColor"];
  var deletedColor = colors["impulse.diffDeletedColor"];
  if (!addedColor && !modifiedColor && !deletedColor) return;

  var safeAdded = isValidCssColor(addedColor) ? addedColor : "#9ece6a";
  var safeModified = isValidCssColor(modifiedColor) ? modifiedColor : "#e0af68";
  var safeDeleted = isValidCssColor(deletedColor) ? deletedColor : "#f7768e";

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
