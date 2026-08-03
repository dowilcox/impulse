"use strict";

// ===========================================================================
// Review Changes — stacked unified-diff renderer + host bridge.
//
// Host -> JS:  window.__applyReviewCommand(cmd)  (object or JSON string)
//              dispatches on cmd.type ("Render" / "SetHunks" / "SetTheme").
// JS -> Host:  ReviewEvents posted to messageHandlers.impulseReview
//              (e.g. { "type": "Ready" }, { "type": "RequestDiff", "path": ... }).
//
// The host computes unified-diff hunks in Rust (only changed regions + a few
// context lines, never whole files) and sends them via SetHunks. We render them
// as plain DOM rows — old/new line-number gutters, a +/- marker, and the line
// content syntax-colored via monaco.editor.colorizeModelLine plus word-level
// emphasis from the per-line spans. No client-side diffing, no Monaco editor
// instances. We still virtualize: only sections near the viewport build their
// rows; off-screen ones collapse to a spacer of last-known height so scroll
// position stays stable.
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

// Virtualization margin: build/keep a section's rows when it is within this
// many pixels of the viewport; collapse to a spacer once it scrolls away.
const VIRTUALIZE_ROOT_MARGIN = "600px 0px";

// path -> section record
//   { path, status, oldPath, added, removed, isBinary, expanded,
//     diffRequested, isBinaryDiff, isTooLarge,
//     hunksData,        // cached FileHunks payload from SetHunks
//     rendered,         // whether rows are currently mounted in the DOM
//     near, lastHeight,
//     el, bodyEl, diffContainerEl }
const sections = new Map();

// Shared IntersectionObserver: tracks which sections are near the viewport so
// we can build/collapse their rows. Created lazily once Monaco is up.
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
      case "SetHunks":
        handleSetHunks(cmd);
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
// viewport it becomes "near" and (if expanded + hunks available) builds its
// rows; when it leaves the margin the rows are cleared and the container is
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

// Bring a section's DOM in line with its (expanded, near, hunks) state. This is
// the single funnel for build/clear decisions so expand/collapse, viewport
// changes, and hunk arrival all converge here.
function reconcileSection(rec) {
  if (!rec.expanded) {
    // Collapsed sections never hold rows.
    if (rec.rendered) clearSection(rec);
    return;
  }

  // Binaries and too-large files render placeholders without a host round-trip.
  if (rec.isBinaryDiff || rec.isTooLarge) {
    renderPlaceholder(rec);
    return;
  }

  // Expanded but no hunks yet: request them (once) when near.
  if (!rec.hunksData) {
    if (rec.near && !rec.diffRequested) {
      rec.diffRequested = true;
      sendToHost({ type: "RequestDiff", path: rec.path });
    }
    return;
  }

  // We have cached hunks.
  if (rec.near) {
    if (!rec.rendered) renderHunks(rec);
  } else if (rec.rendered) {
    // Far from viewport: clear the rows, keep a spacer of last height.
    clearSection(rec, /* keepSpacer */ true);
  }
}

// ---------------------------------------------------------------------------
// Render: rebuild #review-root with one collapsible section per file.
// ---------------------------------------------------------------------------
function handleRender(cmd) {
  const files = cmd.files || [];
  const root = document.getElementById("review-root");
  if (!root) return;

  // Clear all existing rows + stop observing before tearing down.
  sections.forEach(function (rec) {
    if (viewportObserver && rec.el) {
      try {
        viewportObserver.unobserve(rec.el);
      } catch (e) {
        /* ignore */
      }
    }
    clearSection(rec);
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
    // Cached FileHunks payload from SetHunks so a re-build needs no round-trip.
    hunksData: null,
    rendered: false,
    isBinaryDiff: !!f.is_binary,
    isTooLarge: false,
    near: false,
    lastHeight: 0,
    el: null,
    bodyEl: null,
    diffContainerEl: null,
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
    // Collapsed: clear the rows immediately (no spacer needed — body hidden).
    clearSection(rec);
    return;
  }

  // Expanded: let the reconciler decide whether to request/build based on
  // whether this section is currently near the viewport. Re-expanding a section
  // that is on-screen will build synchronously here; off-screen ones wait for
  // the IntersectionObserver to flag them near.
  reconcileSection(rec);
}

// ---------------------------------------------------------------------------
// SetHunks: cache the unified-diff hunks for one section, then reconcile.
// ---------------------------------------------------------------------------
function handleSetHunks(cmd) {
  const rec = sections.get(cmd.path);
  if (!rec) return;
  const data = cmd.hunks || null;
  rec.diffRequested = false;

  if (data && (data.is_binary || data.too_large)) {
    // Placeholder content — no rows, no round-trip needed again.
    rec.isBinaryDiff = !!data.is_binary;
    rec.isTooLarge = !!data.too_large;
    rec.hunksData = null;
    reconcileSection(rec);
    return;
  }

  // Cache the hunks so we can rebuild on re-approach without asking again.
  rec.isBinaryDiff = false;
  rec.isTooLarge = false;
  rec.hunksData = data;
  rec.rendered = false;
  reconcileSection(rec);
}

// Render the binary / too-large placeholder directly into the container.
function renderPlaceholder(rec) {
  const container = rec.diffContainerEl;
  if (!container) return;
  container.textContent = "";
  const ph = document.createElement("div");
  ph.className = "review-placeholder";
  ph.textContent = rec.isBinaryDiff
    ? "Binary file not shown"
    : "File too large or complex to display";
  container.appendChild(ph);
  container.style.height = "auto";
  rec.rendered = false;
}

// ---------------------------------------------------------------------------
// Build the unified-diff rows for a section from its cached hunks.
//
// Syntax coloring: we drop all the hunks' content lines into one throwaway
// Monaco model and colorize each line synchronously via colorizeModelLine
// (tokenize-only — no diff algorithm, no editor instance), then dispose the
// model. Word-level emphasis is overlaid from each line's `spans`.
// ---------------------------------------------------------------------------
function renderHunks(rec) {
  const container = rec.diffContainerEl;
  if (!container) return;
  container.textContent = "";
  container.style.height = "auto";

  const data = rec.hunksData;
  if (!data || !data.hunks || data.hunks.length === 0) {
    const empty = document.createElement("div");
    empty.className = "review-placeholder";
    empty.textContent = "No textual changes.";
    container.appendChild(empty);
    rec.rendered = true;
    rec.lastHeight = container.offsetHeight;
    return;
  }

  const language = data.language || "plaintext";
  // One throwaway model holding every content line, for sync syntax coloring.
  const allContent = [];
  data.hunks.forEach(function (h) {
    h.lines.forEach(function (l) {
      allContent.push(l.content);
    });
  });
  let model = null;
  try {
    model = monaco.editor.createModel(allContent.join("\n"), language);
  } catch (e) {
    model = null;
  }

  const frag = document.createDocumentFragment();
  let lineNo = 0; // 1-based index into the throwaway model
  data.hunks.forEach(function (h) {
    const hh = document.createElement("div");
    hh.className = "review-hunk-header";
    hh.textContent =
      h.header || "@@ -" + h.old_start + " +" + h.new_start + " @@";
    frag.appendChild(hh);
    h.lines.forEach(function (l) {
      lineNo += 1;
      frag.appendChild(buildRow(l, model, lineNo));
    });
  });
  container.appendChild(frag);

  if (model) {
    try {
      model.dispose();
    } catch (e) {
      /* ignore */
    }
  }

  if (data.truncated) {
    const t = document.createElement("div");
    t.className = "review-placeholder review-truncated";
    t.textContent = "Diff truncated — file has more changes than shown.";
    container.appendChild(t);
  }

  rec.rendered = true;
  rec.lastHeight = container.offsetHeight;
}

// Build a single unified-diff row: old gutter, new gutter, +/- marker, content.
function buildRow(line, model, lineNo) {
  const kind = line.kind || "context";
  const row = document.createElement("div");
  row.className = "review-row review-row-" + kind;

  const gOld = document.createElement("span");
  gOld.className = "review-gutter review-gutter-old";
  gOld.textContent = line.old_lineno != null ? String(line.old_lineno) : "";
  const gNew = document.createElement("span");
  gNew.className = "review-gutter review-gutter-new";
  gNew.textContent = line.new_lineno != null ? String(line.new_lineno) : "";

  const marker = document.createElement("span");
  marker.className = "review-line-marker";
  marker.textContent = kind === "added" ? "+" : kind === "removed" ? "-" : " ";

  const content = document.createElement("span");
  content.className = "review-line-content";
  let html = null;
  if (model) {
    try {
      html = monaco.editor.colorizeModelLine(model, lineNo);
    } catch (e) {
      html = null;
    }
  }
  if (html != null) {
    content.appendChild(applyWordSpans(html, line.spans));
  } else if (line.spans && line.spans.length) {
    content.appendChild(highlightTextNode(line.content, 0, line.spans));
  } else {
    content.textContent = line.content;
  }
  // Keep empty lines at full row height.
  if (content.textContent.length === 0) {
    content.appendChild(document.createTextNode("​"));
  }

  row.appendChild(gOld);
  row.appendChild(gNew);
  row.appendChild(marker);
  row.appendChild(content);
  return row;
}

// Wrap the colorized line HTML, overlaying word-diff emphasis on the changed
// UTF-16 ranges. Returns a <span> whose children are the final content nodes.
function applyWordSpans(html, spans) {
  const wrapper = document.createElement("span");
  wrapper.innerHTML = html;
  if (!spans || spans.length === 0) return wrapper;

  // Collect text nodes first — splitting them while walking is unsafe.
  const textNodes = [];
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT, null);
  let n;
  while ((n = walker.nextNode())) textNodes.push(n);

  let offset = 0;
  textNodes.forEach(function (tn) {
    const text = tn.nodeValue;
    const frag = highlightTextNode(text, offset, spans);
    offset += text.length;
    if (tn.parentNode) tn.parentNode.replaceChild(frag, tn);
  });
  return wrapper;
}

// Split `text` (whose first char is at global UTF-16 `globalStart`) into a
// fragment where ranges intersecting `spans` are wrapped in <span class=word>.
function highlightTextNode(text, globalStart, spans) {
  const frag = document.createDocumentFragment();
  const len = text.length;
  let pos = 0;
  for (let i = 0; i < spans.length && pos < len; i++) {
    const s = spans[i];
    const localStart = Math.max(pos, s.start - globalStart);
    const localEnd = Math.min(len, s.end - globalStart);
    if (localEnd <= 0 || localStart >= len || localEnd <= localStart) continue;
    if (localStart > pos) {
      frag.appendChild(document.createTextNode(text.slice(pos, localStart)));
    }
    const mark = document.createElement("span");
    mark.className = "review-word";
    mark.textContent = text.slice(localStart, localEnd);
    frag.appendChild(mark);
    pos = localEnd;
  }
  if (pos < len) frag.appendChild(document.createTextNode(text.slice(pos)));
  return frag;
}

// ---------------------------------------------------------------------------
// Clear a section's rows.
//
// keepSpacer: when true (virtualized-out), leave a spacer div of the last-known
// height so the page scroll position does not jump. When false (collapsed), the
// body is hidden anyway so the container is just reset.
// ---------------------------------------------------------------------------
function clearSection(rec, keepSpacer) {
  if (!rec || !rec.diffContainerEl) return;
  rec.rendered = false;
  rec.diffContainerEl.textContent = "";
  if (keepSpacer && rec.lastHeight > 0) {
    const spacer = document.createElement("div");
    spacer.className = "review-spacer";
    spacer.style.height = rec.lastHeight + "px";
    rec.diffContainerEl.appendChild(spacer);
    rec.diffContainerEl.style.height = rec.lastHeight + "px";
  } else {
    rec.diffContainerEl.style.height = "auto";
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
