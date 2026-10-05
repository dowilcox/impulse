"use strict";

// ===========================================================================
// Review — multi-file change review renderer (protocol v2).
//
// Host -> JS: window.__applyReviewCommand(cmd) with cmd.type in
//   Configure, SetFiles, SetFileDiff, DiffError, SetViewed, Focus, SetTheme,
//   SetBusy.
// JS -> Host: messageHandlers.impulseReview events
//   Ready, RequestDiff, HunkAction, FileAction, ToggleViewed, OpenFile,
//   AddComment, EditComment, DeleteComment, CopyPath.
//
// The DOM persists across updates: SetFiles reconciles cards by path, and a
// SetFileDiff whose content hash and comments are unchanged doesn't re-render,
// so expansion, scroll position and line selections survive refreshes. Only
// cards near the viewport keep their rows (IntersectionObserver), so very
// large reviews stay cheap.
// ===========================================================================

function post(msg) {
  const json = JSON.stringify(msg);
  if (
    window.webkit &&
    window.webkit.messageHandlers &&
    window.webkit.messageHandlers.impulseReview
  ) {
    window.webkit.messageHandlers.impulseReview.postMessage(json);
  } else {
    console.log("IMPULSE_REVIEW_EVENT:" + json);
  }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
const state = {
  ready: false,
  pending: [],
  caps: { stage: false, unstage: false, revert: false },
  options: { layout: "unified", ignoreWhitespace: false, contextLines: 3 },
  scopeTitle: "",
  generation: 0,
  order: [], // paths in display order
  emptyMessage: "No changes.",
  filter: "",
  focus: null, // { path, hunk } keyboard focus
  currentPath: null, // file at the top of the viewport (navigator highlight)
};

// path -> record
//  { item, card, header, body, nav, expanded, near, diff, renderedKey,
//    requestedGen, loadedGen, busy, selection: { hunk, lines:Set, anchor } }
const files = new Map();
let observer = null;

const $ = (sel) => document.querySelector(sel);
const mainEl = () => $("#main");
const filesEl = () => $("#files");

// ---------------------------------------------------------------------------
// Monaco (only used to colorize code; no editors)
// ---------------------------------------------------------------------------
require.config({ paths: { vs: "./vs" } });
window.MonacoEnvironment = {
  getWorker: function () {
    const base = document.baseURI.substring(
      0,
      document.baseURI.lastIndexOf("/") + 1,
    );
    const blob = new Blob(
      [
        "self.MonacoEnvironment={baseUrl:" +
          JSON.stringify(base) +
          "};importScripts(" +
          JSON.stringify(base + "vs/base/worker/workerMain.js") +
          ");",
      ],
      { type: "application/javascript" },
    );
    const url = URL.createObjectURL(blob);
    const worker = new Worker(url);
    URL.revokeObjectURL(url);
    return worker;
  },
};

require(["vs/editor/editor.main"], function () {
  try {
    monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions({
      noSemanticValidation: true,
      noSyntaxValidation: true,
    });
    monaco.languages.typescript.javascriptDefaults.setDiagnosticsOptions({
      noSemanticValidation: true,
      noSyntaxValidation: true,
    });
  } catch (e) {
    /* ignore */
  }
  state.ready = true;
  observer = new IntersectionObserver(onIntersect, {
    root: mainEl(),
    rootMargin: "800px 0px",
    threshold: 0,
  });
  const queued = state.pending;
  state.pending = [];
  queued.forEach(dispatch);
  post({ type: "Ready" });
});

window.__applyReviewCommand = function (cmd) {
  if (typeof cmd === "string") {
    try {
      cmd = JSON.parse(cmd);
    } catch (e) {
      return;
    }
  }
  if (!cmd) return;
  if (!state.ready) {
    state.pending.push(cmd);
    return;
  }
  dispatch(cmd);
};

function dispatch(cmd) {
  try {
    switch (cmd.type) {
      case "Configure":
        onConfigure(cmd);
        break;
      case "SetFiles":
        onSetFiles(cmd);
        break;
      case "SetFileDiff":
        onSetFileDiff(cmd.diff);
        break;
      case "DiffError":
        onDiffError(cmd.path, cmd.message);
        break;
      case "SetViewed":
        setViewed(cmd.path, cmd.viewed, false);
        break;
      case "Focus":
        focusFile(cmd.path, true);
        break;
      case "SetTheme":
        onSetTheme(cmd);
        break;
      case "SetBusy":
        onSetBusy(cmd.path, cmd.busy);
        break;
    }
  } catch (e) {
    console.error("review command failed", cmd.type, e);
  }
}

// ---------------------------------------------------------------------------
// Configure / theme
// ---------------------------------------------------------------------------
function onConfigure(cmd) {
  const layoutChanged =
    cmd.options && cmd.options.layout !== state.options.layout;
  const whitespaceChanged =
    cmd.options &&
    cmd.options.ignoreWhitespace !== state.options.ignoreWhitespace;
  state.caps = cmd.capabilities || state.caps;
  state.options = cmd.options || state.options;
  const scopeChanged = cmd.scopeTitle !== state.scopeTitle;
  state.scopeTitle = cmd.scopeTitle || "";
  if (scopeChanged || whitespaceChanged) {
    // A different comparison: start over.
    files.forEach(
      (rec) => observer && rec.card && observer.unobserve(rec.card),
    );
    files.clear();
    state.order = [];
    state.focus = null;
    filesEl().textContent = "";
    $("#nav-list").textContent = "";
    return;
  }
  // Same scope: refresh header actions and re-render rows in the new layout.
  files.forEach((rec) => {
    renderHeader(rec);
    if (layoutChanged) {
      rec.renderedKey = null;
      reconcile(rec);
    } else if (rec.diff) {
      // Hunk header buttons depend on capabilities.
      rec.renderedKey = null;
      reconcile(rec);
    }
  });
}

function onSetTheme(cmd) {
  const theme = cmd.theme;
  if (theme) {
    monaco.editor.defineTheme("impulse-review", {
      base: theme.base || "vs-dark",
      inherit: theme.inherit !== false,
      rules: (theme.rules || []).map(function (r) {
        const rule = { token: r.token };
        if (r.foreground) rule.foreground = r.foreground;
        if (r.font_style || r.fontStyle)
          rule.fontStyle = r.font_style || r.fontStyle;
        return rule;
      }),
      colors: theme.colors || {},
    });
    monaco.editor.setTheme("impulse-review");
  }
  const root = document.documentElement.style;
  Object.entries(cmd.chrome || {}).forEach(([k, v]) => {
    if (
      typeof v === "string" &&
      /^(#[0-9a-fA-F]{3,8}|rgba?\([0-9.,\s]+\))$/.test(v)
    ) {
      root.setProperty(k, v);
    }
  });
  // Re-colorize visible rows with the new token colors.
  files.forEach((rec) => {
    rec.renderedKey = null;
    reconcile(rec);
  });
}

// ---------------------------------------------------------------------------
// File list
// ---------------------------------------------------------------------------
function onSetFiles(cmd) {
  state.generation = cmd.generation;
  state.emptyMessage = cmd.emptyMessage || "No changes.";
  const incoming = cmd.files || [];
  const seen = new Set();
  const container = filesEl();

  // Remove the loading/empty placeholder.
  const empty = container.querySelector(".empty");
  if (empty) empty.remove();

  incoming.forEach((item) => {
    seen.add(item.path);
    let rec = files.get(item.path);
    if (!rec) {
      rec = createRecord(item);
      files.set(item.path, rec);
      observer.observe(rec.card);
    } else {
      const statsChanged =
        rec.item.added !== item.added ||
        rec.item.removed !== item.removed ||
        rec.item.status !== item.status;
      rec.item = item;
      renderHeader(rec);
      if (statsChanged) rec.renderedKey = null;
    }
    // Every new generation re-requests diffs lazily (only what's visible).
    rec.stale = true;
  });

  // Remove files that are gone.
  files.forEach((rec, path) => {
    if (!seen.has(path)) {
      observer.unobserve(rec.card);
      rec.card.remove();
      files.delete(path);
      if (state.focus && state.focus.path === path) state.focus = null;
    }
  });

  // Order cards like the incoming list.
  state.order = incoming.map((f) => f.path);
  state.order.forEach((path) => container.appendChild(files.get(path).card));

  if (state.order.length === 0) {
    const div = document.createElement("div");
    div.className = "empty";
    div.textContent = state.emptyMessage;
    container.appendChild(div);
  }

  renderNav();
  files.forEach(reconcile);
}

function createRecord(item) {
  const card = document.createElement("section");
  card.className = "card";
  card.dataset.path = item.path;

  const header = document.createElement("div");
  header.className = "card-header";
  card.appendChild(header);

  const body = document.createElement("div");
  body.className = "card-body";
  card.appendChild(body);

  const rec = {
    item,
    card,
    header,
    body,
    nav: null,
    // Viewed files start collapsed; everything else starts expanded.
    expanded: !item.viewed,
    near: false,
    diff: null,
    renderedKey: null,
    stale: true,
    requestedGen: -1,
    busy: false,
    selection: null,
    composer: null, // { hunk, side, line, endLine, el }
  };
  renderHeader(rec);
  return rec;
}

function renderHeader(rec) {
  const item = rec.item;
  const h = rec.header;
  h.textContent = "";
  rec.card.classList.toggle("collapsed", !rec.expanded);

  const chev = svg('<path d="M4 6l4 4 4-4"/>', "chev");
  h.appendChild(chev);

  const status = el("span", "status status-" + item.status, item.status);
  h.appendChild(status);

  const path = el("span", "path");
  if (item.oldPath) path.appendChild(el("span", "old", item.oldPath));
  const slash = item.path.lastIndexOf("/");
  if (slash >= 0)
    path.appendChild(el("span", "dir", item.path.slice(0, slash + 1)));
  path.appendChild(document.createTextNode(item.path.slice(slash + 1)));
  path.title = item.path;
  h.appendChild(path);

  if (item.changedSinceViewed)
    h.appendChild(el("span", "changed-badge", "changed since viewed"));
  if (item.binary) h.appendChild(el("span", "stat", "binary"));
  else h.appendChild(statEl(item.added, item.removed));

  const viewed = el("button", "viewed-toggle");
  viewed.appendChild(el("span", "check" + (item.viewed ? " on" : "")));
  viewed.appendChild(document.createTextNode("Viewed"));
  viewed.title = "Mark as viewed (v)";
  viewed.addEventListener("click", (e) => {
    e.stopPropagation();
    setViewed(item.path, !rec.item.viewed, true);
  });
  h.appendChild(viewed);

  if (state.caps.stage) {
    h.appendChild(
      actionButton(
        "Stage",
        "Stage file",
        () => fileAction(rec, "stage"),
        "primary",
      ),
    );
  }
  if (state.caps.unstage) {
    h.appendChild(
      actionButton(
        "Unstage",
        "Unstage file",
        () => fileAction(rec, "unstage"),
        "primary",
      ),
    );
  }
  if (state.caps.revert) {
    h.appendChild(
      actionButton(
        "Revert",
        "Discard changes to this file",
        () => fileAction(rec, "revert"),
        "danger",
      ),
    );
  }
  if (state.caps.stage && item.status !== "D") {
    h.appendChild(
      actionButton("Edit Diff", "Edit the file beside its staged version", () =>
        post({ type: "OpenFile", path: item.path, diff: true }),
      ),
    );
  }
  h.appendChild(
    actionButton("Open", "Open file (o)", () =>
      post({ type: "OpenFile", path: item.path, line: firstChangedLine(rec) }),
    ),
  );

  h.onclick = (e) => {
    if (e.target.closest("button")) return;
    toggleExpanded(rec);
  };
  h.oncontextmenu = (e) => {
    e.preventDefault();
    post({ type: "CopyPath", path: item.path });
  };
}

function statEl(added, removed) {
  const s = el("span", "stat");
  if (added != null) s.appendChild(el("span", "a", "+" + added));
  if (removed != null && removed > 0)
    s.appendChild(el("span", "r", "−" + removed));
  return s;
}

function actionButton(label, title, onClick, kind) {
  const b = el("button", "act" + (kind ? " " + kind : ""), label);
  b.title = title;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  return b;
}

function fileAction(rec, action) {
  post({ type: "FileAction", action, path: rec.item.path });
}

function toggleExpanded(rec, value) {
  rec.expanded = value === undefined ? !rec.expanded : value;
  rec.card.classList.toggle("collapsed", !rec.expanded);
  reconcile(rec);
}

function setViewed(path, viewed, fromUser) {
  const rec = files.get(path);
  if (!rec) return;
  rec.item = Object.assign({}, rec.item, {
    viewed,
    changedSinceViewed: fromUser
      ? false
      : rec.item.changedSinceViewed && !viewed,
  });
  if (fromUser) post({ type: "ToggleViewed", path, viewed });
  // Viewing collapses; un-viewing (e.g. it changed) expands.
  toggleExpanded(rec, !viewed);
  renderHeader(rec);
  renderNav();
}

// ---------------------------------------------------------------------------
// Navigator
// ---------------------------------------------------------------------------
function renderNav() {
  const list = $("#nav-list");
  list.textContent = "";
  const filter = state.filter.toLowerCase();
  let lastDir = null;
  let viewedCount = 0;
  state.order.forEach((path) => {
    const rec = files.get(path);
    if (rec.item.viewed) viewedCount++;
    if (filter && !path.toLowerCase().includes(filter)) {
      rec.nav = null;
      return;
    }
    const slash = path.lastIndexOf("/");
    const dir = slash >= 0 ? path.slice(0, slash) : "";
    if (dir !== lastDir) {
      if (dir) {
        const d = el("div", "nav-dir", dir);
        d.title = dir;
        list.appendChild(d);
      }
      lastDir = dir;
    }
    const row = el(
      "div",
      "nav-item" +
        (rec.item.viewed ? " viewed" : "") +
        (path === state.currentPath ? " current" : ""),
    );
    const check = el("span", "check" + (rec.item.viewed ? " on" : ""));
    check.title = "Viewed";
    check.addEventListener("click", (e) => {
      e.stopPropagation();
      setViewed(path, !rec.item.viewed, true);
    });
    row.appendChild(check);
    row.appendChild(
      el("span", "status status-" + rec.item.status, rec.item.status),
    );
    row.appendChild(el("span", "name", path.slice(slash + 1)));
    if (rec.item.changedSinceViewed) row.appendChild(el("span", "changed"));
    if (rec.item.commentCount > 0)
      row.appendChild(el("span", "comments", "💬" + rec.item.commentCount));
    if (!rec.item.binary)
      row.appendChild(statEl(rec.item.added, rec.item.removed));
    row.title = path;
    row.addEventListener("click", () => focusFile(path, true));
    rec.nav = row;
    list.appendChild(row);
  });
  const total = state.order.length;
  $("#nav-progress > i").style.width = total
    ? (100 * viewedCount) / total + "%"
    : "0";
}

$("#nav-filter").addEventListener("input", (e) => {
  state.filter = e.target.value || "";
  renderNav();
});

function focusFile(path, scroll) {
  const rec = files.get(path);
  if (!rec) return;
  if (!rec.expanded) toggleExpanded(rec, true);
  state.focus = { path, hunk: 0 };
  markFocus();
  if (scroll) rec.card.scrollIntoView({ block: "start" });
  setCurrent(path);
}

function setCurrent(path) {
  if (state.currentPath === path) return;
  const prev = state.currentPath && files.get(state.currentPath);
  if (prev && prev.nav) prev.nav.classList.remove("current");
  state.currentPath = path;
  const rec = files.get(path);
  if (rec && rec.nav) {
    rec.nav.classList.add("current");
    rec.nav.scrollIntoView({ block: "nearest" });
  }
}

mainEl().addEventListener(
  "scroll",
  () => {
    // The first card whose bottom is below the top edge is "current".
    const top = mainEl().getBoundingClientRect().top + 4;
    for (const path of state.order) {
      const rect = files.get(path).card.getBoundingClientRect();
      if (rect.bottom > top) {
        setCurrent(path);
        break;
      }
    }
  },
  { passive: true },
);

// ---------------------------------------------------------------------------
// Diff loading + virtualization
// ---------------------------------------------------------------------------
function onIntersect(entries) {
  entries.forEach((entry) => {
    const rec = files.get(entry.target.dataset.path);
    if (!rec) return;
    rec.near = entry.isIntersecting;
    reconcile(rec);
  });
}

// Single funnel: bring a card's DOM in line with (expanded, near, diff).
function reconcile(rec) {
  if (!rec.expanded) {
    clearBody(rec, false);
    return;
  }
  if (!rec.near) {
    if (rec.renderedKey) clearBody(rec, true);
    return;
  }
  if (rec.stale && rec.requestedGen !== state.generation) {
    rec.requestedGen = state.generation;
    post({ type: "RequestDiff", path: rec.item.path });
    if (!rec.diff) {
      rec.body.textContent = "";
      rec.body.appendChild(el("div", "placeholder", "Loading…"));
      rec.body.style.height = "";
      return;
    }
  }
  if (rec.diff) renderBody(rec);
}

function onSetFileDiff(diff) {
  const rec = files.get(diff.path);
  if (!rec) return;
  rec.stale = false;
  const prevHash = rec.diff && rec.diff.diffHash;
  rec.diff = diff;
  if (prevHash !== diff.diffHash && rec.selection) rec.selection = null;
  if (rec.expanded && rec.near) renderBody(rec);
}

function onDiffError(path, message) {
  const rec = files.get(path);
  if (!rec) return;
  rec.stale = false;
  rec.body.textContent = "";
  rec.body.appendChild(el("div", "placeholder error", message));
}

function onSetBusy(path, busy) {
  const rec = files.get(path);
  if (!rec) return;
  rec.busy = busy;
  rec.card.classList.toggle("busy", busy);
}

function clearBody(rec, keepHeight) {
  const height = rec.body.offsetHeight;
  rec.body.textContent = "";
  rec.renderedKey = null;
  rec.body.style.height = keepHeight && height > 0 ? height + "px" : "";
}

// ---------------------------------------------------------------------------
// Rendering a file's hunks
// ---------------------------------------------------------------------------
function renderKey(rec) {
  const d = rec.diff;
  return [
    d.diffHash,
    state.options.layout,
    JSON.stringify(d.comments || []),
    rec.selection
      ? rec.selection.hunk + ":" + Array.from(rec.selection.lines).join(",")
      : "",
    state.caps.stage,
    state.caps.unstage,
    state.caps.revert,
    rec.composer ? rec.composer.hunk + ":" + rec.composer.line : "",
  ].join("|");
}

function renderBody(rec) {
  const key = renderKey(rec);
  if (key === rec.renderedKey) return;
  rec.renderedKey = key;
  const diff = rec.diff;
  const body = rec.body;
  const keepComposerText =
    rec.composer && rec.composer.el
      ? rec.composer.el.querySelector("textarea").value
      : null;
  body.textContent = "";
  body.style.height = "";

  if (diff.binary) {
    body.appendChild(el("div", "placeholder", "Binary file — no diff shown"));
    return;
  }
  if (diff.tooLarge) {
    body.appendChild(
      el(
        "div",
        "placeholder",
        "File too large to display — open it in the editor",
      ),
    );
    return;
  }
  if (!diff.hunks || diff.hunks.length === 0) {
    body.appendChild(el("div", "placeholder", "No textual changes"));
    return;
  }

  // Outdated comments float at the top of the card.
  const outdated = (diff.comments || []).filter((c) => c.outdated);
  if (outdated.length) {
    const box = el("div", "outdated-box");
    box.appendChild(
      el("div", "title", "Outdated comments — their lines changed or aren't in this diff"),
    );
    outdated.forEach((c) => box.appendChild(commentEl(rec, c)));
    body.appendChild(box);
  }

  const colorizers = makeColorizers(diff);
  diff.hunks.forEach((hunk, index) =>
    body.appendChild(renderHunk(rec, hunk, index, colorizers)),
  );
  colorizers.dispose();

  if (diff.truncated) {
    body.appendChild(
      el(
        "div",
        "placeholder",
        "Diff truncated — the file has more changes than shown.",
      ),
    );
  }

  if (keepComposerText != null && rec.composer && rec.composer.el) {
    rec.composer.el.querySelector("textarea").value = keepComposerText;
  }
  markFocus();
}

function renderHunk(rec, hunk, index, colorizers) {
  const wrap = el("div", "hunk");
  wrap.dataset.index = String(index);

  const header = el("div", "hunk-header");
  header.appendChild(el("span", "label", hunk.header));
  const actions = el("div", "hunk-actions");
  const sel =
    rec.selection && rec.selection.hunk === index
      ? rec.selection.lines.size
      : 0;
  const what = sel ? sel + " line" + (sel === 1 ? "" : "s") : "hunk";
  if (state.caps.stage)
    actions.appendChild(
      actionButton(
        "Stage " + what,
        "Stage (s / ⌘Y)",
        () => hunkAction(rec, index, "stage"),
        "primary",
      ),
    );
  if (state.caps.unstage)
    actions.appendChild(
      actionButton(
        "Unstage " + what,
        "Unstage (u / ⌘⇧Y)",
        () => hunkAction(rec, index, "unstage"),
        "primary",
      ),
    );
  if (state.caps.revert)
    actions.appendChild(
      actionButton(
        "Revert " + what,
        "Revert in working tree (x / ⌘⌥Z)",
        () => hunkAction(rec, index, "revert"),
        "danger",
      ),
    );
  actions.appendChild(
    actionButton("Comment", "Comment (c)", () =>
      openComposerForHunk(rec, index),
    ),
  );
  header.appendChild(actions);
  header.addEventListener("click", (e) => {
    if (e.target.closest("button")) return;
    state.focus = { path: rec.item.path, hunk: index };
    markFocus();
  });
  wrap.appendChild(header);

  const rows = el(
    "div",
    "rows" + (state.options.layout === "split" ? " split" : ""),
  );
  const commentsByEnd = groupCommentsByEnd(rec.diff.comments || []);
  if (state.options.layout === "split") {
    renderSplitRows(rec, hunk, index, rows, colorizers, commentsByEnd);
  } else {
    hunk.lines.forEach((line, li) => {
      rows.appendChild(unifiedRow(rec, line, index, li, colorizers));
      appendLineExtras(rec, rows, line, index, li, commentsByEnd);
    });
  }
  wrap.appendChild(rows);
  return wrap;
}

function unifiedRow(rec, line, hunkIndex, lineIndex, colorizers) {
  const row = el("div", "row " + line.kind);
  if (isSelected(rec, hunkIndex, lineIndex)) row.classList.add("selected");
  const gOld = el("span", "gutter", line.old != null ? String(line.old) : "");
  const gNew = el("span", "gutter", line.new != null ? String(line.new) : "");
  if (line.kind !== "context") {
    gOld.classList.add("selectable");
    gNew.classList.add("selectable");
    gOld.title = gNew.title = "Select line (shift-click for a range)";
    const select = (e) => toggleLine(rec, hunkIndex, lineIndex, e.shiftKey);
    gOld.addEventListener("click", select);
    gNew.addEventListener("click", select);
  }
  const plus = el("button", "add-comment", "+");
  plus.title = "Comment on this line";
  plus.addEventListener("click", (e) => {
    e.stopPropagation();
    openComposer(rec, hunkIndex, lineIndex);
  });
  gOld.appendChild(plus);
  row.appendChild(gOld);
  row.appendChild(gNew);
  row.appendChild(
    el(
      "span",
      "marker",
      line.kind === "added" ? "+" : line.kind === "removed" ? "−" : " ",
    ),
  );
  row.appendChild(codeEl(line, colorizers));
  return row;
}

// Split view: context on both sides; removed/added runs paired line by line.
function renderSplitRows(
  rec,
  hunk,
  hunkIndex,
  rows,
  colorizers,
  commentsByEnd,
) {
  const lines = hunk.lines;
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind === "context") {
      rows.appendChild(
        splitRow(rec, hunkIndex, [i, lines[i]], [i, lines[i]], colorizers),
      );
      appendLineExtras(rec, rows, lines[i], hunkIndex, i, commentsByEnd);
      i++;
      continue;
    }
    const removed = [];
    const added = [];
    while (i < lines.length && lines[i].kind === "removed")
      removed.push([i, lines[i++]]);
    while (i < lines.length && lines[i].kind === "added")
      added.push([i, lines[i++]]);
    const count = Math.max(removed.length, added.length);
    for (let k = 0; k < count; k++) {
      rows.appendChild(
        splitRow(
          rec,
          hunkIndex,
          removed[k] || null,
          added[k] || null,
          colorizers,
        ),
      );
      if (removed[k])
        appendLineExtras(
          rec,
          rows,
          removed[k][1],
          hunkIndex,
          removed[k][0],
          commentsByEnd,
        );
      if (added[k])
        appendLineExtras(
          rec,
          rows,
          added[k][1],
          hunkIndex,
          added[k][0],
          commentsByEnd,
        );
    }
  }
}

function splitRow(rec, hunkIndex, left, right, colorizers) {
  const row = el("div", "row");
  const side = (entry, which) => {
    if (!entry) {
      row.appendChild(el("span", "gutter cell blank"));
      row.appendChild(el("span", "marker cell blank"));
      row.appendChild(el("span", "code cell blank"));
      return;
    }
    const [index, line] = entry;
    const kind = line.kind === "context" ? "" : " " + line.kind;
    const selected = isSelected(rec, hunkIndex, index) ? " selected" : "";
    const num = which === "old" ? line.old : line.new;
    const g = el(
      "span",
      "gutter cell" + kind + selected,
      num != null ? String(num) : "",
    );
    if (line.kind !== "context") {
      g.classList.add("selectable");
      g.addEventListener("click", (e) =>
        toggleLine(rec, hunkIndex, index, e.shiftKey),
      );
    }
    const plus = el("button", "add-comment", "+");
    plus.addEventListener("click", (e) => {
      e.stopPropagation();
      openComposer(rec, hunkIndex, index);
    });
    g.appendChild(plus);
    row.appendChild(g);
    row.appendChild(
      el(
        "span",
        "marker cell" + kind + selected,
        line.kind === "added" ? "+" : line.kind === "removed" ? "−" : " ",
      ),
    );
    const code = codeEl(line, colorizers);
    code.className += " cell" + kind + selected;
    row.appendChild(code);
  };
  side(left && left[1].kind !== "added" ? left : null, "old");
  side(right && right[1].kind !== "removed" ? right : null, "new");
  return row;
}

// Word highlights help when a line changed a little; when most of it
// changed they just box every token, so drop them.
function usefulSpans(line) {
  const spans = line.spans || [];
  if (!spans.length || !line.text.length) return spans;
  const covered = spans.reduce((sum, s) => sum + Math.max(0, s[1] - s[0]), 0);
  return covered / line.text.length > 0.6 ? [] : spans;
}

function codeEl(line, colorizers) {
  const code = el("span", "code");
  const html = colorizers.html(line);
  line = Object.assign({}, line, { spans: usefulSpans(line) });
  if (html != null) {
    code.appendChild(applySpans(html, line.spans));
  } else if (line.spans && line.spans.length) {
    code.appendChild(highlightText(line.text, 0, line.spans));
  } else {
    code.textContent = line.text;
  }
  if (!code.textContent) code.appendChild(document.createTextNode("​"));
  return code;
}

// Colorize old-side and new-side lines with separate models so tokenizer
// state (multi-line strings/comments) doesn't bleed between the two sides.
function makeColorizers(diff) {
  const language = diff.language || "plaintext";
  const oldLines = [];
  const newLines = [];
  const indexOld = new Map();
  const indexNew = new Map();
  diff.hunks.forEach((h) =>
    h.lines.forEach((l) => {
      if (l.kind !== "added") {
        oldLines.push(l.text);
        indexOld.set(l, oldLines.length);
      }
      if (l.kind !== "removed") {
        newLines.push(l.text);
        indexNew.set(l, newLines.length);
      }
    }),
  );
  let oldModel = null;
  let newModel = null;
  try {
    oldModel = monaco.editor.createModel(oldLines.join("\n"), language);
    newModel = monaco.editor.createModel(newLines.join("\n"), language);
  } catch (e) {
    /* plaintext fallback */
  }
  return {
    html(line) {
      try {
        if (line.kind === "removed")
          return oldModel
            ? monaco.editor.colorizeModelLine(oldModel, indexOld.get(line))
            : null;
        return newModel
          ? monaco.editor.colorizeModelLine(newModel, indexNew.get(line))
          : null;
      } catch (e) {
        return null;
      }
    },
    dispose() {
      if (oldModel) oldModel.dispose();
      if (newModel) newModel.dispose();
    },
  };
}

function applySpans(html, spans) {
  const wrapper = document.createElement("span");
  wrapper.innerHTML = html;
  if (!spans || spans.length === 0) return wrapper;
  const nodes = [];
  const walker = document.createTreeWalker(wrapper, NodeFilter.SHOW_TEXT, null);
  let n;
  while ((n = walker.nextNode())) nodes.push(n);
  let offset = 0;
  nodes.forEach((tn) => {
    const text = tn.nodeValue;
    const frag = highlightText(text, offset, spans);
    offset += text.length;
    if (tn.parentNode) tn.parentNode.replaceChild(frag, tn);
  });
  return wrapper;
}

function highlightText(text, globalStart, spans) {
  const frag = document.createDocumentFragment();
  let pos = 0;
  for (const span of spans) {
    const start = Math.max(pos, span[0] - globalStart);
    const end = Math.min(text.length, span[1] - globalStart);
    if (end <= 0 || start >= text.length || end <= start) continue;
    if (start > pos)
      frag.appendChild(document.createTextNode(text.slice(pos, start)));
    frag.appendChild(el("span", "word", text.slice(start, end)));
    pos = end;
  }
  if (pos < text.length)
    frag.appendChild(document.createTextNode(text.slice(pos)));
  return frag;
}

// ---------------------------------------------------------------------------
// Line selection
// ---------------------------------------------------------------------------
function isSelected(rec, hunkIndex, lineIndex) {
  return !!(
    rec.selection &&
    rec.selection.hunk === hunkIndex &&
    rec.selection.lines.has(lineIndex)
  );
}

function toggleLine(rec, hunkIndex, lineIndex, extend) {
  const lines = rec.diff.hunks[hunkIndex].lines;
  if (!rec.selection || rec.selection.hunk !== hunkIndex) {
    rec.selection = { hunk: hunkIndex, lines: new Set(), anchor: lineIndex };
  }
  const sel = rec.selection;
  if (extend && sel.anchor != null) {
    const [a, b] = [
      Math.min(sel.anchor, lineIndex),
      Math.max(sel.anchor, lineIndex),
    ];
    for (let i = a; i <= b; i++)
      if (lines[i].kind !== "context") sel.lines.add(i);
  } else {
    if (sel.lines.has(lineIndex)) sel.lines.delete(lineIndex);
    else sel.lines.add(lineIndex);
    sel.anchor = lineIndex;
  }
  if (sel.lines.size === 0) rec.selection = null;
  state.focus = { path: rec.item.path, hunk: hunkIndex };
  renderBody(rec);
}

function hunkAction(rec, hunkIndex, action) {
  if (rec.busy || !rec.diff) return;
  const hunk = rec.diff.hunks[hunkIndex];
  if (!hunk) return;
  const lines =
    rec.selection && rec.selection.hunk === hunkIndex
      ? Array.from(rec.selection.lines).sort((a, b) => a - b)
      : null;
  rec.selection = null;
  post({
    type: "HunkAction",
    action,
    path: rec.item.path,
    hunkIndex,
    hunkId: hunk.id,
    lines,
  });
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------
function groupCommentsByEnd(comments) {
  const map = new Map();
  comments
    .filter((c) => !c.outdated)
    .forEach((c) => {
      const key = c.side + ":" + c.endLine;
      if (!map.has(key)) map.set(key, []);
      map.get(key).push(c);
    });
  return map;
}

function appendLineExtras(
  rec,
  rows,
  line,
  hunkIndex,
  lineIndex,
  commentsByEnd,
) {
  const keys = [];
  if (line.new != null && line.kind !== "removed") keys.push("new:" + line.new);
  if (line.old != null && line.kind === "removed") keys.push("old:" + line.old);
  keys.forEach((key) =>
    (commentsByEnd.get(key) || []).forEach((c) =>
      rows.appendChild(commentEl(rec, c)),
    ),
  );
  if (
    rec.composer &&
    rec.composer.hunk === hunkIndex &&
    rec.composer.lineIndex === lineIndex
  ) {
    rows.appendChild(composerEl(rec));
  }
}

function commentEl(rec, c) {
  const box = el("div", "comment" + (c.outdated ? " outdated" : ""));
  const meta = el("div", "meta");
  meta.appendChild(
    el(
      "span",
      "",
      (c.side === "old" ? "removed line " : "line ") +
        (c.endLine > c.line ? c.line + "–" + c.endLine : c.line),
    ),
  );
  if (c.author) {
    // Imported from a pull request review thread.
    meta.insertBefore(el("span", "author", "@" + c.author), meta.firstChild);
  }
  meta.appendChild(el("span", "spacer"));
  if (c.url) {
    const view = el("button", "act", "View on GitHub");
    view.addEventListener("click", () => post({ type: "OpenURL", url: c.url }));
    meta.appendChild(view);
  } else {
    const edit = el("button", "act", "Edit");
    edit.addEventListener("click", () => editCommentInline(box, c));
    meta.appendChild(edit);
  }
  const del = el("button", "act danger", c.url ? "Dismiss" : "Delete");
  del.addEventListener("click", () =>
    post({ type: "DeleteComment", id: c.id }),
  );
  meta.appendChild(del);
  box.appendChild(meta);
  // The thread's author is in the header already.
  const lead = c.author ? "@" + c.author + ": " : null;
  const text = lead && c.text.startsWith(lead) ? c.text.slice(lead.length) : c.text;
  box.appendChild(el("div", "body", text));
  return box;
}

function editCommentInline(box, c) {
  const body = box.querySelector(".body");
  const area = document.createElement("textarea");
  area.value = c.text;
  area.style.width = "100%";
  area.style.minHeight = "48px";
  area.style.background = "transparent";
  area.style.color = "var(--text)";
  area.style.border = "0";
  area.style.outline = "none";
  area.style.font = "inherit";
  body.replaceWith(area);
  area.focus();
  area.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      post({ type: "EditComment", id: c.id, text: area.value });
    } else if (e.key === "Escape") {
      e.preventDefault();
      area.replaceWith(body);
    }
  });
}

function openComposerForHunk(rec, hunkIndex) {
  const lines = rec.diff.hunks[hunkIndex].lines;
  let target = -1;
  if (
    rec.selection &&
    rec.selection.hunk === hunkIndex &&
    rec.selection.lines.size
  ) {
    target = Math.max.apply(null, Array.from(rec.selection.lines));
  } else {
    for (let i = lines.length - 1; i >= 0; i--) {
      if (lines[i].kind !== "context") {
        target = i;
        break;
      }
    }
  }
  if (target < 0) target = lines.length - 1;
  openComposer(rec, hunkIndex, target);
}

function openComposer(rec, hunkIndex, lineIndex) {
  const hunk = rec.diff.hunks[hunkIndex];
  // A selection in this hunk widens the comment to the selected range.
  let indices = [lineIndex];
  if (
    rec.selection &&
    rec.selection.hunk === hunkIndex &&
    rec.selection.lines.has(lineIndex)
  ) {
    indices = Array.from(rec.selection.lines).sort((a, b) => a - b);
  }
  const anchorLine = hunk.lines[lineIndex];
  const side = anchorLine.kind === "removed" ? "old" : "new";
  const sideLines = indices
    .map((i) => hunk.lines[i])
    .filter((l) =>
      side === "old" ? l.kind === "removed" : l.kind !== "removed",
    );
  const numbers = sideLines
    .map((l) => (side === "old" ? l.old : l.new))
    .filter((n) => n != null);
  const line = numbers.length
    ? Math.min.apply(null, numbers)
    : side === "old"
      ? anchorLine.old
      : anchorLine.new;
  const endLine = numbers.length ? Math.max.apply(null, numbers) : line;
  const snippet = sideLines.map((l) => l.text).join("\n");
  rec.composer = {
    hunk: hunkIndex,
    lineIndex: indices[indices.length - 1],
    side,
    line,
    endLine,
    snippet,
    el: null,
  };
  renderBody(rec);
  const area =
    rec.composer &&
    rec.composer.el &&
    rec.composer.el.querySelector("textarea");
  if (area) area.focus();
}

function composerEl(rec) {
  const c = rec.composer;
  const box = el("div", "composer");
  const area = document.createElement("textarea");
  area.placeholder = "Leave a comment for the agent or yourself…";
  box.appendChild(area);
  const buttons = el("div", "buttons");
  buttons.appendChild(
    el(
      "span",
      "hint",
      (c.side === "old" ? "Removed line " : "Line ") +
        (c.endLine > c.line ? c.line + "–" + c.endLine : c.line) +
        " · ⌘↩ to save",
    ),
  );
  const cancel = actionButton("Cancel", "Cancel (Esc)", () =>
    closeComposer(rec),
  );
  const save = actionButton(
    "Comment",
    "Save (⌘↩)",
    () => saveComposer(rec, area.value),
    "primary",
  );
  buttons.appendChild(cancel);
  buttons.appendChild(save);
  box.appendChild(buttons);
  area.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      saveComposer(rec, area.value);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeComposer(rec);
    }
  });
  c.el = box;
  return box;
}

function saveComposer(rec, text) {
  const c = rec.composer;
  if (!c || !text.trim()) return closeComposer(rec);
  post({
    type: "AddComment",
    path: rec.item.path,
    side: c.side,
    line: c.line,
    endLine: c.endLine,
    text,
    snippet: c.snippet,
  });
  rec.composer = null;
  rec.selection = null;
}

function closeComposer(rec) {
  rec.composer = null;
  renderBody(rec);
}

function firstChangedLine(rec) {
  if (!rec.diff || !rec.diff.hunks.length) return null;
  const h = rec.diff.hunks[0];
  const line = h.lines.find((l) => l.kind !== "context") || h.lines[0];
  return line ? line.new || line.old || null : null;
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------
function visiblePaths() {
  return state.order.filter(
    (p) =>
      !state.filter || p.toLowerCase().includes(state.filter.toLowerCase()),
  );
}

function markFocus() {
  document
    .querySelectorAll(".hunk.focused")
    .forEach((h) => h.classList.remove("focused"));
  document
    .querySelectorAll(".card.focused-file")
    .forEach((c) => c.classList.remove("focused-file"));
  if (!state.focus) return;
  const rec = files.get(state.focus.path);
  if (!rec) return;
  rec.card.classList.add("focused-file");
  const hunk = rec.body.querySelector(
    '.hunk[data-index="' + state.focus.hunk + '"]',
  );
  if (hunk) hunk.classList.add("focused");
}

function moveHunk(delta) {
  const paths = visiblePaths();
  if (!paths.length) return;
  let { path, hunk } = state.focus || {
    path: state.currentPath || paths[0],
    hunk: -1,
  };
  let pi = Math.max(0, paths.indexOf(path));
  for (let guard = 0; guard < paths.length * 2 + 2; guard++) {
    const rec = files.get(paths[pi]);
    const count = rec && rec.expanded && rec.diff ? rec.diff.hunks.length : 0;
    const next = hunk + delta;
    if (next >= 0 && next < count) {
      state.focus = { path: paths[pi], hunk: next };
      markFocus();
      const el = rec.body.querySelector('.hunk[data-index="' + next + '"]');
      if (el) el.scrollIntoView({ block: "nearest" });
      setCurrent(paths[pi]);
      return;
    }
    pi += delta;
    if (pi < 0 || pi >= paths.length) return;
    const nextRec = files.get(paths[pi]);
    hunk =
      delta > 0 ? -1 : nextRec && nextRec.diff ? nextRec.diff.hunks.length : 0;
    if (nextRec && !nextRec.diff) {
      // Not loaded yet: jump to the file and let it load.
      focusFile(paths[pi], true);
      return;
    }
  }
}

function moveFile(delta, unviewedOnly) {
  const paths = visiblePaths();
  if (!paths.length) return;
  const current = state.focus ? state.focus.path : state.currentPath;
  let i = paths.indexOf(current);
  for (let step = 0; step < paths.length; step++) {
    i = i + delta;
    if (i < 0 || i >= paths.length) return;
    if (!unviewedOnly || !files.get(paths[i]).item.viewed) {
      focusFile(paths[i], true);
      return;
    }
  }
}

document.addEventListener("keydown", (e) => {
  const tag = (e.target && e.target.tagName) || "";
  if (tag === "TEXTAREA" || tag === "INPUT") {
    if (e.key === "Escape" && tag === "INPUT") e.target.blur();
    return;
  }
  const rec = state.focus && files.get(state.focus.path);
  const key = e.key;
  const cmd = e.metaKey;
  if (cmd && (key === "y" || key === "Y") && rec) {
    e.preventDefault();
    if (e.shiftKey) {
      if (state.caps.unstage) hunkAction(rec, state.focus.hunk, "unstage");
    } else if (state.caps.stage) {
      hunkAction(rec, state.focus.hunk, "stage");
    }
    return;
  }
  if (
    cmd &&
    e.altKey &&
    (key === "z" || key === "Ω" || e.code === "KeyZ") &&
    rec &&
    state.caps.revert
  ) {
    e.preventDefault();
    hunkAction(rec, state.focus.hunk, "revert");
    return;
  }
  if (cmd || e.ctrlKey || e.altKey) return;
  switch (key) {
    case "j":
      moveHunk(1);
      break;
    case "k":
      moveHunk(-1);
      break;
    case "n":
      moveFile(1, false);
      break;
    case "p":
      moveFile(-1, false);
      break;
    case "N":
      moveFile(1, true);
      break;
    case "s":
      if (rec && state.caps.stage) hunkAction(rec, state.focus.hunk, "stage");
      break;
    case "u":
      if (rec && state.caps.unstage)
        hunkAction(rec, state.focus.hunk, "unstage");
      break;
    case "x":
      if (rec && state.caps.revert) hunkAction(rec, state.focus.hunk, "revert");
      break;
    case "v":
      if (rec) setViewed(rec.item.path, !rec.item.viewed, true);
      else if (state.currentPath)
        setViewed(
          state.currentPath,
          !files.get(state.currentPath).item.viewed,
          true,
        );
      break;
    case "c":
      if (rec && rec.diff) openComposerForHunk(rec, state.focus.hunk);
      break;
    case "o": {
      const target = rec || files.get(state.currentPath);
      if (target) {
        let line = firstChangedLine(target);
        if (rec && rec.diff && rec.diff.hunks[state.focus.hunk]) {
          const h = rec.diff.hunks[state.focus.hunk];
          const l = h.lines.find((x) => x.kind !== "context") || h.lines[0];
          line = l.new || l.old || line;
        }
        post({ type: "OpenFile", path: target.item.path, line });
      }
      break;
    }
    case "Enter":
    case " ": {
      const target = rec || files.get(state.currentPath);
      if (target) toggleExpanded(target);
      break;
    }
    case "Escape":
      files.forEach((r) => {
        if (r.selection) {
          r.selection = null;
          renderBody(r);
        }
      });
      break;
    case "t":
    case "T":
    case "/":
      $("#nav-filter").focus();
      break;
    default:
      return;
  }
  e.preventDefault();
});

// ---------------------------------------------------------------------------
// DOM helpers
// ---------------------------------------------------------------------------
function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function svg(inner, className) {
  const wrap = document.createElement("span");
  wrap.innerHTML =
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" class="' +
    (className || "") +
    '">' +
    inner +
    "</svg>";
  return wrap.firstChild;
}
