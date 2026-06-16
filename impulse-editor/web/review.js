"use strict";

// ===========================================================================
// Review Changes — stacked-diff renderer + host bridge.
//
// Host -> JS:  window.__applyReviewCommand(cmd)  (object or JSON string)
//              dispatches on cmd.type ("Render" / "SetDiff" / "SetTheme").
// JS -> Host:  ReviewEvents posted to messageHandlers.impulseReview
//              (e.g. { "type": "Ready" }, { "type": "RequestDiff", "path": ... }).
//
// PERFORMANCE: Monaco diff editors are expensive. We virtualize the list so
// only sections at/near the viewport hold a live Monaco instance. A section
// far out of view is disposed and replaced by a spacer of its last-known
// height (so scroll position is stable); it re-mounts from cached diff content
// when it re-approaches. This keeps a handful of live editors regardless of
// how many files changed.
// ===========================================================================

// ---------------------------------------------------------------------------
// Host communication (mirrors editor.js's sendToHost, different handler name)
// ---------------------------------------------------------------------------
function sendToHost(msgObj) {
  const json = JSON.stringify(msgObj);
  if (
    window.webkit &&
    window.webkit.messageHandlers &&
    window.webkit.messageHandlers.impulseReview
  ) {
    // macOS WKWebView
    window.webkit.messageHandlers.impulseReview.postMessage(json);
  } else {
    // Fallback (e.g. future Linux frontend intercepting console messages)
    console.log("IMPULSE_REVIEW_EVENT:" + json);
  }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
let monacoReady = false;
let pendingCommands = [];
let currentThemeColors = null;

// Virtualization margin: mount/keep a section's Monaco editor when it is within
// this many pixels of the viewport; dispose it once it scrolls farther away.
const VIRTUALIZE_ROOT_MARGIN = "600px 0px";

// path -> section record
//   { path, status, oldPath, added, removed, isBinary, expanded,
//     diffRequested, diffLoaded, diffData, isBinaryDiff, isTooLarge,
//     near, lastHeight,
//     el, bodyEl, diffContainerEl,
//     diffEditor, originalModel, modifiedModel,
//     contentSizeListener, updateDiffListener, layoutRaf }
const sections = new Map();

// Shared IntersectionObserver: tracks which sections are near the viewport so
// we can mount/dispose their Monaco editors. Created lazily once Monaco is up.
let viewportObserver = null;

// ---------------------------------------------------------------------------
// Monaco loader wiring (IDENTICAL to editor.js so file:// paths resolve)
// ---------------------------------------------------------------------------
require.config({
  paths: { vs: "./vs" },
});

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
  // Disable Monaco's built-in TS/JS diagnostics (Impulse owns its LSP, and the
  // review view is read-only anyway).
  try {
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
  } catch (e) {
    /* older bundles may lack the TS defaults */
  }

  monacoReady = true;
  createViewportObserver();

  // Flush any commands that arrived before Monaco finished loading.
  var queued = pendingCommands;
  pendingCommands = [];
  queued.forEach(handleCommand);

  sendToHost({ type: "Ready" });
});

// ---------------------------------------------------------------------------
// Host -> JS bridge
// ---------------------------------------------------------------------------
window.__applyReviewCommand = function (cmd) {
  if (typeof cmd === "string") {
    try {
      cmd = JSON.parse(cmd);
    } catch (e) {
      console.error("Failed to parse review command:", e);
      return;
    }
  }
  if (!cmd || typeof cmd !== "object") return;

  if (!monacoReady) {
    pendingCommands.push(cmd);
    return;
  }
  handleCommand(cmd);
};

function handleCommand(cmd) {
  try {
    switch (cmd.type) {
      case "Render":
        handleRender(cmd);
        break;
      case "SetDiff":
        handleSetDiff(cmd);
        break;
      case "SetTheme":
        handleSetTheme(cmd);
        break;
      default:
        console.warn("Unknown review command:", cmd.type);
    }
  } catch (e) {
    console.error("Review command handler error for", cmd.type, ":", e);
  }
}

// ---------------------------------------------------------------------------
// Viewport virtualization
//
// The observer watches each section element. When a section is near the
// viewport it becomes "near" and (if expanded + diff available) mounts a live
// Monaco editor; when it leaves the margin it is disposed and the container is
// replaced by a spacer of its last-known height so scroll position is stable.
// ---------------------------------------------------------------------------
function createViewportObserver() {
  if (viewportObserver || typeof IntersectionObserver === "undefined") return;
  viewportObserver = new IntersectionObserver(
    function (entries) {
      entries.forEach(function (entry) {
        const path = entry.target.getAttribute("data-review-path");
        if (!path) return;
        const rec = sections.get(path);
        if (!rec) return;
        const near = entry.isIntersecting;
        if (near === rec.near) return;
        rec.near = near;
        reconcileSection(rec);
      });
    },
    { root: null, rootMargin: VIRTUALIZE_ROOT_MARGIN, threshold: 0 },
  );
}

function observeSection(rec) {
  if (viewportObserver && rec.el) viewportObserver.observe(rec.el);
}

// Bring a section's DOM in line with its (expanded, near, diff) state. This is
// the single funnel for mount/dispose decisions so expand/collapse, viewport
// changes, and diff arrival all converge here.
function reconcileSection(rec) {
  if (!rec.expanded) {
    // Collapsed sections never hold a live editor.
    if (rec.diffEditor) disposeSectionDiff(rec);
    return;
  }

  // Expanded but no diff content yet: request it (once) when near; binaries and
  // too-large files render placeholders without any host round-trip.
  if (rec.isBinaryDiff || rec.isTooLarge) {
    renderPlaceholder(rec);
    return;
  }

  if (!rec.diffData) {
    if (rec.near && !rec.diffRequested) {
      rec.diffRequested = true;
      sendToHost({ type: "RequestDiff", path: rec.path });
    }
    return;
  }

  // We have cached diff content.
  if (rec.near) {
    if (!rec.diffEditor) mountSectionDiff(rec);
    else scheduleLayout(rec);
  } else if (rec.diffEditor) {
    // Far from viewport: dispose the editor, keep a spacer of last height.
    disposeSectionDiff(rec, /* keepSpacer */ true);
  }
}

// ---------------------------------------------------------------------------
// Render: rebuild #review-root with one collapsible section per file.
// ---------------------------------------------------------------------------
function handleRender(cmd) {
  const files = cmd.files || [];
  const root = document.getElementById("review-root");
  if (!root) return;

  // Dispose all existing editors/models + stop observing before tearing down.
  sections.forEach(function (rec) {
    if (viewportObserver && rec.el) {
      try {
        viewportObserver.unobserve(rec.el);
      } catch (e) {
        /* ignore */
      }
    }
    disposeSectionDiff(rec);
  });
  sections.clear();
  root.textContent = "";

  if (files.length === 0) {
    const empty = document.createElement("div");
    empty.className = "review-empty";
    empty.textContent = "No changes to review.";
    root.appendChild(empty);
    return;
  }

  files.forEach(function (f) {
    // All sections start collapsed; the diff is requested and the Monaco editor
    // mounted lazily the first time the user expands a section (and it is near
    // the viewport — see reconcileSection / the IntersectionObserver).
    const rec = buildSection(f, false);
    sections.set(f.path, rec);
    root.appendChild(rec.el);
    observeSection(rec);
  });
}

function statusGlyphClass(status) {
  switch (status) {
    case "A":
      return "review-status-A";
    case "M":
      return "review-status-M";
    case "D":
      return "review-status-D";
    case "R":
      return "review-status-R";
    case "?":
      return "review-status-Q";
    default:
      return "review-status-M";
  }
}

function statusGlyphText(status) {
  return status === "?" ? "?" : status || "M";
}

function buildSection(f, expanded) {
  const rec = {
    path: f.path,
    status: f.status,
    oldPath: f.old_path || null,
    added: f.added || 0,
    removed: f.removed || 0,
    isBinary: !!f.is_binary,
    expanded: !!expanded,
    diffRequested: false,
    diffLoaded: false,
    // Cached diff payload from SetDiff so a re-mount needs no host round-trip.
    diffData: null, // { original, modified, language }
    isBinaryDiff: !!f.is_binary,
    isTooLarge: false,
    near: false,
    lastHeight: 0,
    el: null,
    bodyEl: null,
    diffContainerEl: null,
    diffEditor: null,
    originalModel: null,
    modifiedModel: null,
    contentSizeListener: null,
    updateDiffListener: null,
    layoutRaf: 0,
  };

  const section = document.createElement("div");
  section.className = "review-section" + (expanded ? "" : " collapsed");
  section.setAttribute("data-review-path", f.path);

  // --- Header row ---
  const header = document.createElement("div");
  header.className = "review-section-header";

  const chevron = document.createElement("span");
  chevron.className = "review-chevron";
  chevron.innerHTML =
    '<svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 4.5L6 7.5L9 4.5"/></svg>';
  header.appendChild(chevron);

  const glyph = document.createElement("span");
  glyph.className = "review-status-glyph " + statusGlyphClass(f.status);
  glyph.textContent = statusGlyphText(f.status);
  header.appendChild(glyph);

  const pathEl = document.createElement("span");
  pathEl.className = "review-path";
  if (rec.oldPath && rec.oldPath !== rec.path) {
    const oldEl = document.createElement("span");
    oldEl.className = "review-old-path";
    oldEl.textContent = rec.oldPath + " → ";
    pathEl.appendChild(oldEl);
    pathEl.appendChild(document.createTextNode(rec.path));
  } else {
    pathEl.textContent = rec.path;
  }
  pathEl.title = rec.path;
  header.appendChild(pathEl);

  const badge = document.createElement("span");
  badge.className = "review-badge";
  const addedEl = document.createElement("span");
  addedEl.className = "added";
  addedEl.textContent = "+" + rec.added;
  const removedEl = document.createElement("span");
  removedEl.className = "removed";
  removedEl.textContent = "-" + rec.removed;
  badge.appendChild(addedEl);
  badge.appendChild(removedEl);
  header.appendChild(badge);

  const discard = document.createElement("button");
  discard.className = "review-discard";
  discard.title = "Discard changes to this file";
  discard.textContent = "Discard";
  discard.addEventListener("click", function (ev) {
    ev.stopPropagation();
    sendToHost({ type: "Discard", path: rec.path });
  });
  header.appendChild(discard);

  header.addEventListener("click", function () {
    toggleSection(rec);
  });

  section.appendChild(header);

  // --- Body (diff container) ---
  const body = document.createElement("div");
  body.className = "review-section-body";
  const diffContainer = document.createElement("div");
  diffContainer.className = "review-diff-container";
  body.appendChild(diffContainer);
  section.appendChild(body);

  rec.el = section;
  rec.bodyEl = body;
  rec.diffContainerEl = diffContainer;

  return rec;
}

// ---------------------------------------------------------------------------
// Expand / collapse
// ---------------------------------------------------------------------------
function toggleSection(rec) {
  const next = !rec.expanded;
  rec.expanded = next;
  rec.el.classList.toggle("collapsed", !next);
  sendToHost({ type: "ToggleFile", path: rec.path, expanded: next });

  if (!next) {
    // Collapsed: free the editor immediately (no spacer needed — body hidden).
    disposeSectionDiff(rec);
    return;
  }

  // Expanded: let the reconciler decide whether to request/mount based on
  // whether this section is currently near the viewport. Re-expanding a section
  // that is on-screen will mount synchronously here; off-screen ones wait for
  // the IntersectionObserver to flag them near.
  reconcileSection(rec);
}

// ---------------------------------------------------------------------------
// SetDiff: cache the diff payload for one section, then reconcile.
// ---------------------------------------------------------------------------
function handleSetDiff(cmd) {
  const rec = sections.get(cmd.path);
  if (!rec) return;

  if (cmd.is_binary || cmd.too_large) {
    // Placeholder content — no editor, no round-trip needed again.
    rec.isBinaryDiff = !!cmd.is_binary;
    rec.isTooLarge = !!cmd.too_large;
    rec.diffLoaded = true;
    rec.diffRequested = false;
    rec.diffData = null;
    reconcileSection(rec);
    return;
  }

  // Cache the real diff content so we can rebuild on re-mount without asking
  // the host again.
  rec.diffData = {
    original: cmd.original != null ? cmd.original : "",
    modified: cmd.modified != null ? cmd.modified : "",
    language: cmd.language || "plaintext",
  };
  rec.diffLoaded = true;
  rec.diffRequested = false;

  // If the section was collapsed again before the diff arrived, keep the cached
  // data but build nothing now. A later expand reconciles from cache (CORRECTNESS
  // fix: previously this marked loaded with no editor and never re-rendered).
  reconcileSection(rec);
}

// Render the binary / too-large placeholder directly into the container.
function renderPlaceholder(rec) {
  if (rec.diffEditor) disposeSectionDiff(rec);
  const container = rec.diffContainerEl;
  if (!container) return;
  container.textContent = "";
  const ph = document.createElement("div");
  ph.className = "review-placeholder";
  ph.textContent = rec.isBinaryDiff
    ? "Binary file not shown"
    : "File too large to display";
  container.appendChild(ph);
  container.style.height = "auto";
}

// ---------------------------------------------------------------------------
// Mount a live Monaco diff editor from cached diff content.
// ---------------------------------------------------------------------------
function mountSectionDiff(rec) {
  const data = rec.diffData;
  if (!data) return;

  // Replace any spacer / stale content.
  disposeSectionDiff(rec);
  const container = rec.diffContainerEl;
  if (!container) return;
  container.textContent = "";
  // Seed the container with the last-known height so layout doesn't collapse to
  // zero (and jump the page) during the first frame before Monaco measures.
  if (rec.lastHeight > 0) container.style.height = rec.lastHeight + "px";

  const language = data.language || "plaintext";

  rec.originalModel = monaco.editor.createModel(data.original, language);
  rec.modifiedModel = monaco.editor.createModel(data.modified, language);

  rec.diffEditor = monaco.editor.createDiffEditor(container, {
    renderSideBySide: false,
    readOnly: true,
    originalEditable: false,
    automaticLayout: false,
    scrollBeyondLastLine: false,
    hideUnchangedRegions: { enabled: true },
    renderOverviewRuler: false,
    overviewRulerLanes: 0,
    minimap: { enabled: false },
    glyphMargin: false,
    folding: false,
    // --- UI polish: breathing room in the gutter ---
    // Reserve enough columns for the line numbers and push the code away from
    // the +/- change markers with extra decoration + padding space.
    lineNumbers: "on",
    lineNumbersMinChars: 4,
    lineDecorationsWidth: 18,
    renderLineHighlight: "none",
    padding: { top: 10, bottom: 10 },
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
    fontSize: 13,
    lineHeight: 20,
    scrollbar: {
      vertical: "hidden",
      handleMouseWheel: false,
      alwaysConsumeMouseWheel: false,
    },
  });

  if (currentThemeColors) {
    // Apply the already-defined theme to this freshly created editor.
    monaco.editor.setTheme("impulse-review-theme");
  }

  rec.diffEditor.setModel({
    original: rec.originalModel,
    modified: rec.modifiedModel,
  });

  // AUTO-HEIGHT: size the container to the inline diff's content height so the
  // OUTER page scrolls, never the inner editor.
  rec.updateDiffListener = rec.diffEditor.onDidUpdateDiff(function () {
    scheduleLayout(rec);
  });

  const modEditor = rec.diffEditor.getModifiedEditor();
  if (modEditor && modEditor.onDidContentSizeChange) {
    rec.contentSizeListener = modEditor.onDidContentSizeChange(function () {
      scheduleLayout(rec);
    });
  }

  scheduleLayout(rec);
}

// ---------------------------------------------------------------------------
// Auto-height layout (single coalesced rAF per content-size change).
//
// IMPORTANT: when the container is not measurable (width 0 — e.g. still
// collapsed, or the section is virtualized out), we BAIL instead of
// self-rescheduling. A spinning rAF wastes a frame every tick; the next
// IntersectionObserver "near" event or expand will re-run layout for us.
// ---------------------------------------------------------------------------
function scheduleLayout(rec) {
  if (!rec.diffEditor) return;
  if (rec.layoutRaf) cancelAnimationFrame(rec.layoutRaf);
  rec.layoutRaf = requestAnimationFrame(function () {
    rec.layoutRaf = 0;
    layoutSection(rec);
  });
}

function layoutSection(rec) {
  if (!rec.diffEditor) return;
  const container = rec.diffContainerEl;
  const width = container.clientWidth || rec.el.clientWidth || 0;
  if (width === 0) {
    // Not measurable yet — DO NOT spin. Wait for the next visibility/expand
    // event to call scheduleLayout again.
    return;
  }

  const modEditor = rec.diffEditor.getModifiedEditor();
  const origEditor = rec.diffEditor.getOriginalEditor();
  let height = 0;
  if (modEditor) height = Math.max(height, modEditor.getContentHeight());
  // Inline diff renders original deletions inside the modified pane, but guard
  // for any bundle that exposes original content height as the taller side.
  if (origEditor) height = Math.max(height, origEditor.getContentHeight());
  if (height === 0) height = 40;

  rec.lastHeight = height;
  container.style.height = height + "px";
  rec.diffEditor.layout({ width: width, height: height });
}

// ---------------------------------------------------------------------------
// Dispose helpers (free Monaco editors + models)
//
// keepSpacer: when true (virtualized-out), leave a spacer div of the last-known
// height so the page scroll position does not jump. When false (collapsed), the
// body is hidden anyway so the container is just reset.
// ---------------------------------------------------------------------------
function disposeSectionDiff(rec, keepSpacer) {
  if (!rec) return;
  if (rec.layoutRaf) {
    cancelAnimationFrame(rec.layoutRaf);
    rec.layoutRaf = 0;
  }
  if (rec.updateDiffListener) {
    try {
      rec.updateDiffListener.dispose();
    } catch (e) {
      /* ignore */
    }
    rec.updateDiffListener = null;
  }
  if (rec.contentSizeListener) {
    try {
      rec.contentSizeListener.dispose();
    } catch (e) {
      /* ignore */
    }
    rec.contentSizeListener = null;
  }
  if (rec.diffEditor) {
    try {
      rec.diffEditor.setModel(null);
      rec.diffEditor.dispose();
    } catch (e) {
      /* ignore */
    }
    rec.diffEditor = null;
  }
  if (rec.originalModel) {
    try {
      rec.originalModel.dispose();
    } catch (e) {
      /* ignore */
    }
    rec.originalModel = null;
  }
  if (rec.modifiedModel) {
    try {
      rec.modifiedModel.dispose();
    } catch (e) {
      /* ignore */
    }
    rec.modifiedModel = null;
  }
  if (rec.diffContainerEl) {
    rec.diffContainerEl.textContent = "";
    if (keepSpacer && rec.lastHeight > 0) {
      // Preserve scroll position with a placeholder spacer of the last height.
      const spacer = document.createElement("div");
      spacer.className = "review-spacer";
      spacer.style.height = rec.lastHeight + "px";
      rec.diffContainerEl.appendChild(spacer);
      rec.diffContainerEl.style.height = rec.lastHeight + "px";
    } else {
      rec.diffContainerEl.style.height = "auto";
    }
  }
}

// ---------------------------------------------------------------------------
// SetTheme: define + apply the Monaco theme, and map colors to CSS variables
// used by the section headers/rows (mirrors editor.js's theme application).
// ---------------------------------------------------------------------------
function handleSetTheme(cmd) {
  const theme = cmd.theme;
  if (!theme) return;

  monaco.editor.defineTheme("impulse-review-theme", {
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
  monaco.editor.setTheme("impulse-review-theme");

  if (theme.colors) {
    currentThemeColors = theme.colors;
    applyThemeCssVars(theme.colors);
  }
}

function isValidCssColor(c) {
  return (
    typeof c === "string" &&
    /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(c)
  );
}

function applyThemeCssVars(colors) {
  const root = document.documentElement.style;
  const set = function (cssVar, value, fallback) {
    root.setProperty(cssVar, isValidCssColor(value) ? value : fallback);
  };

  set("--review-bg", colors["editor.background"], "#1a1b26");
  set("--review-fg", colors["editor.foreground"], "#c0caf5");
  set("--review-header-bg", colors["editorGutter.background"], "#1f2335");
  set(
    "--review-header-hover-bg",
    colors["editor.lineHighlightBackground"],
    "#292e42",
  );
  set("--review-border", colors["editor.lineHighlightBackground"], "#292e42");
  set("--review-muted", colors["editorLineNumber.foreground"], "#565f89");
  set("--review-added", colors["impulse.diffAddedColor"], "#9ece6a");
  set("--review-modified", colors["impulse.diffModifiedColor"], "#e0af68");
  set("--review-deleted", colors["impulse.diffDeletedColor"], "#f7768e");
  // Renamed reuses the modified accent unless a dedicated color is provided.
  set("--review-renamed", colors["impulse.diffModifiedColor"], "#7aa2f7");
}
