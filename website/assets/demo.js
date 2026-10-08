// Impulse homepage: the interactive demo window.
//
// A scripted imitation of an Impulse window on the trailhead example project
// (the one the docs screenshots use, see demo-files.js): terminals with
// command blocks and the input bar, an editor tab, Review, the command
// palette, the app's built-in themes (themes.js, generated from the app's
// theme files) and a simulated coding agent. Nothing here runs a real shell
// or calls any service.
(() => {
  "use strict";
  const win = document.querySelector("[data-demo]");
  if (!win) return;

  const FILES = window.DEMO_FILES || {};
  const THEMES = window.IMPULSE_THEMES || {};
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = (sel, el = win) => el.querySelector(sel);
  const $$ = (sel, el = win) => [...el.querySelectorAll(sel)];
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const h = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };
  const CANCEL = Symbol("cancel");

  const HOME = "/Users/you";
  const REPO = "~/Code/trailhead";
  const BRANCH = "feature/forecast-cache";

  const ICON = {
    term: '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m5 7 5 5-5 5M12 17h7"/></svg>',
    file: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>',
    review: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="6" cy="6" r="2.5"/><circle cx="18" cy="18" r="2.5"/><path d="M6 8.5V14a4 4 0 0 0 4 4h5.5M18 15.5V6"/></svg>',
    x: '<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18"/></svg>',
    copy: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></svg>',
    rerun: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/></svg>',
    send: '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 2 11 13M22 2l-7 20-4-9-9-4z"/></svg>',
    chev: '<svg class="chev" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="m9 6 6 6-6 6"/></svg>',
    folder: '<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" opacity=".85"><path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    doc: '<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M9 13h6M9 17h4"/></svg>',
  };

  // ── State ─────────────────────────────────────────────────────────────

  const S = {
    tabs: [],
    active: null,
    nextId: 1,
    fixed: false, // the agent has fixed the failing test
    base: Object.fromEntries(Object.entries(FILES).map(([p, f]) => [p, f.base])),
    staged: new Set(["src/lib/cache.ts"]),
    comments: [],
    history: [],
    agent: null,
    theme: "tokyo-night",
    tookOver: false,
    open: new Set(["src", "src/lib"]),
    commits: [
      ["9da0f3e", "", "Document the API"],
      ["290dcdb", "|", "Cache forecasts per trail"],
      ["6b31fc3", "|", "TTL cache for forecast lookups"],
      ["dfde538", "", "Handle missing elevation data", "tag: v0.2.0"],
      ["e72cf96", "", "Project actions for dev server and tests"],
      ["793da2d", "", "Forecast endpoint"],
      ["334682c", "", "Unit conversions with tests", "tag: v0.1.0"],
      ["70b7c90", "", "Add trail list endpoint"],
    ],
  };

  const current = (path) => {
    const f = FILES[path];
    if (!f) return null;
    if (S.fixed && f.fixed) return f.fixed;
    return f.work ?? f.base;
  };
  const sleep = (ms) => new Promise((r) => setTimeout(r, reduced ? Math.min(ms, 20) : ms));

  // ── Diffs ─────────────────────────────────────────────────────────────

  function diffLines(a, b) {
    const A = a.split("\n"), B = b.split("\n");
    const n = A.length, m = B.length;
    const L = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = A[i] === B[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
    const ops = [];
    let i = 0, j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && A[i] === B[j]) { ops.push({ t: " ", a: i + 1, b: j + 1, s: A[i] }); i++; j++; }
      else if (j < m && (i >= n || L[i][j + 1] >= L[i + 1][j])) { ops.push({ t: "+", b: j + 1, s: B[j] }); j++; }
      else { ops.push({ t: "-", a: i + 1, s: A[i] }); i++; }
    }
    return ops;
  }

  function hunks(ops, ctx = 3) {
    const out = [];
    let cur = null;
    ops.forEach((op, k) => {
      if (op.t === " ") return;
      const from = Math.max(0, k - ctx);
      if (cur && from <= cur.end + 1) cur.end = Math.min(ops.length - 1, k + ctx);
      else { cur = { start: from, end: Math.min(ops.length - 1, k + ctx) }; out.push(cur); }
    });
    return out.map(({ start, end }) => {
      const lines = ops.slice(start, end + 1);
      const a0 = lines.find((l) => l.a)?.a ?? 0, b0 = lines.find((l) => l.b)?.b ?? 0;
      const ac = lines.filter((l) => l.t !== "+").length, bc = lines.filter((l) => l.t !== "-").length;
      return { header: `@@ -${a0},${ac} +${b0},${bc} @@`, lines };
    });
  }

  function changes() {
    const list = [];
    for (const path of Object.keys(FILES)) {
      const now = current(path);
      if (now === S.base[path]) continue;
      const ops = diffLines(S.base[path], now);
      list.push({ path, ops, add: ops.filter((o) => o.t === "+").length, del: ops.filter((o) => o.t === "-").length, status: S.base[path] ? "M" : "U" });
    }
    return list;
  }

  function totals() {
    const c = changes();
    return { files: c.length, add: c.reduce((s, f) => s + f.add, 0), del: c.reduce((s, f) => s + f.del, 0) };
  }

  // ── Syntax highlighting for the editor and Review ─────────────────────

  const TS_KEYWORDS = new Set("import from export const let var function return if else for of in new class private public readonly type interface async await throw try catch typeof void extends implements get set static default while break continue as".split(" "));
  const TS_CONST = new Set("true false null undefined this".split(" "));

  function highlightTS(line) {
    const re = /(\/\/.*$|\/\*.*?\*\/|\/\*\*?.*$|^\s*\*.*$)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b\d[\d_.]*\b)|([A-Za-z_$][\w$]*)|(=>|[=!<>]=?=?|&&|\|\||\?\?|[+\-*/%?:])|([{}()[\];,.])/g;
    let out = "", last = 0, m;
    while ((m = re.exec(line))) {
      out += esc(line.slice(last, m.index));
      const [tok, com, str, num, word, op, punct] = m;
      if (com) out += `<span class="sx-c">${esc(tok)}</span>`;
      else if (str) out += `<span class="sx-s">${esc(tok)}</span>`;
      else if (num) out += `<span class="sx-n">${esc(tok)}</span>`;
      else if (word) {
        const next = line.slice(re.lastIndex).trimStart()[0];
        if (TS_KEYWORDS.has(word)) out += `<span class="sx-k">${tok}</span>`;
        else if (TS_CONST.has(word)) out += `<span class="sx-n">${tok}</span>`;
        else if (next === "(") out += `<span class="sx-f">${tok}</span>`;
        else if (/^[A-Z]/.test(word)) out += `<span class="sx-t">${tok}</span>`;
        else out += esc(tok);
      } else if (op) out += `<span class="sx-o">${esc(tok)}</span>`;
      else if (punct) out += `<span class="sx-p">${esc(tok)}</span>`;
      last = re.lastIndex;
    }
    return out + esc(line.slice(last));
  }

  function highlightJSON(line) {
    return esc(line)
      .replace(/(&quot;[^&]*?&quot;)(\s*:)/g, '<span class="sx-a">$1</span>$2')
      .replace(/(:\s*)(&quot;.*?&quot;)/g, '$1<span class="sx-s">$2</span>')
      .replace(/(\[|,\s*)(&quot;.*?&quot;)/g, '$1<span class="sx-s">$2</span>')
      .replace(/\b(true|false|null|\d+)\b/g, '<span class="sx-n">$1</span>');
  }

  function highlightMD(line) {
    if (/^#/.test(line)) return `<span class="sx-k">${esc(line)}</span>`;
    if (/^```/.test(line)) return `<span class="sx-c">${esc(line)}</span>`;
    return esc(line).replace(/(#.*)$/, '<span class="sx-c">$1</span>');
  }

  const highlighter = (path) => (path.endsWith(".json") ? highlightJSON : path.endsWith(".md") ? highlightMD : highlightTS);

  // ── Shell input highlighting and completion ───────────────────────────

  const KNOWN = new Set(["git", "npm", "node", "ls", "cd", "cat", "pwd", "echo", "clear", "help", "impulse", "claude", "codex", "gemini", "aider", "opencode", "whoami", "date", "uname", "sw_vers", "history", "exit", "vim", "nvim", "nano", "code", "open", "sudo", "rm", "brew", "theme", "top", "htop", "ssh", "man", "less", "which", "hostname", "true", "false", "mkdir", "touch", "npx"]);
  const SUGGEST = ["git status -sb", "git log --oneline --graph", "git diff", "git diff --stat", "git branch", "git add src/server.ts", "git commit -m \"Cache stats on /health\"", "npm test", "npm run lint", "npm run dev", "claude", "codex", "ls -la", "ls src", "cat src/lib/cache.ts", "cat package.json", "impulse open src/lib/cache.ts", "impulse review", "impulse notify \"Build done\"", "help", "clear", "theme dracula", "cd src", "echo $SHELL"];

  function highlightShell(text) {
    const re = /(\s+)|("[^"]*"?|'[^']*'?)|(\|\||&&|[|;&])|(\$\w+)|([^\s|;&]+)/g;
    let out = "", atCmd = true, m;
    while ((m = re.exec(text))) {
      const [tok, ws, str, op, v, word] = m;
      if (ws) out += tok;
      else if (str) { out += `<span class="hl-str">${esc(tok)}</span>`; atCmd = false; }
      else if (op) { out += `<span class="hl-op">${esc(tok)}</span>`; atCmd = true; }
      else if (v) { out += `<span class="hl-var">${esc(tok)}</span>`; atCmd = false; }
      else if (word) {
        if (atCmd) {
          const complete = re.lastIndex < text.length;
          const ok = KNOWN.has(word) || (!complete && [...KNOWN].some((k) => k.startsWith(word)));
          out += `<span class="${ok ? "hl-cmd" : "hl-bad"}">${esc(tok)}</span>`;
          atCmd = false;
        } else if (word.startsWith("-")) out += `<span class="hl-opt">${esc(tok)}</span>`;
        else if (word.includes("/") || /\.\w+$/.test(word)) out += `<span class="hl-path">${esc(tok)}</span>`;
        else out += esc(tok);
      }
    }
    return out;
  }

  function suggestion(value) {
    if (!value.trim()) return "";
    const pool = [...S.history].reverse().concat(SUGGEST);
    const hit = pool.find((c) => c.startsWith(value) && c.length > value.length);
    return hit ? hit.slice(value.length) : "";
  }

  function splitArgs(line) {
    const args = [];
    line.replace(/"([^"]*)"|'([^']*)'|(\S+)/g, (_, a, b, c) => args.push(a ?? b ?? c));
    return args;
  }

  // ── Tabs ──────────────────────────────────────────────────────────────

  const tabsEl = $("[data-tabs]");
  const panesEl = $("[data-panes]");

  function addTab(tab, activate = true) {
    tab.id = S.nextId++;
    S.tabs.push(tab);
    panesEl.appendChild(tab.el);
    if (activate) activateTab(tab.id);
    else renderTabs();
    return tab;
  }

  function tabTitle(tab) {
    if (tab.kind === "terminal") return tab.term.shortCwd();
    if (tab.kind === "editor") return tab.path.split("/").pop();
    return "Review · trailhead";
  }

  function renderTabs() {
    tabsEl.innerHTML = "";
    for (const tab of S.tabs) {
      const icon = tab.kind === "terminal" ? ICON.term : tab.kind === "editor" ? ICON.file : ICON.review;
      const agent = S.agent && S.agent.term.tab === tab ? `<span class="adot ${S.agent.state}" title="${esc(S.agent.name)}: ${S.agent.state}"></span>` : "";
      const el = h(`<div class="tab" role="tab" aria-selected="${tab.id === S.active}" tabindex="-1">
        <span class="tab-ico">${icon}</span>${agent}<span class="tab-title">${esc(tabTitle(tab))}</span>
        <span class="tab-x" role="button" aria-label="Close tab" title="Close tab">${ICON.x}</span></div>`);
      el.addEventListener("mousedown", (e) => {
        if (e.target.closest(".tab-x")) return;
        e.preventDefault();
        activateTab(tab.id);
      });
      el.querySelector(".tab-x").addEventListener("click", (e) => { e.stopPropagation(); closeTab(tab.id); });
      tabsEl.appendChild(el);
    }
  }

  function activeTab() { return S.tabs.find((t) => t.id === S.active); }

  function activateTab(id, focus = true) {
    S.active = id;
    for (const tab of S.tabs) tab.el.classList.toggle("active", tab.id === id);
    renderTabs();
    const tab = activeTab();
    if (tab?.kind === "terminal") { if (focus) tab.term.focus(); tab.term.scrollToEnd(); }
    if (tab?.kind === "editor" && focus) tab.el.querySelector(".ed").focus({ preventScroll: true });
    if (tab?.kind === "review") { tab.render(); if (focus) win.focus({ preventScroll: true }); }
    updateChrome();
  }

  function closeTab(id) {
    const i = S.tabs.findIndex((t) => t.id === id);
    if (i < 0) return;
    const [tab] = S.tabs.splice(i, 1);
    if (S.agent && S.agent.term.tab === tab) endAgent();
    tab.term?.cancelRun();
    tab.el.remove();
    if (!S.tabs.length) { newTerminal(); return; }
    if (S.active === id) activateTab(S.tabs[Math.min(i, S.tabs.length - 1)].id);
    else renderTabs();
    toast({ title: `Closed “${tabTitle(tab)}”`, sub: "In Impulse, ⇧⌘T or ⌘Z brings it back.", ms: 2600 });
  }

  function firstTerminal() { return activeTab()?.kind === "terminal" ? activeTab() : S.tabs.find((t) => t.kind === "terminal"); }
  function terminalTab() {
    const t = firstTerminal();
    if (t) { if (S.active !== t.id) activateTab(t.id); return t; }
    return newTerminal();
  }

  // ── Terminal ──────────────────────────────────────────────────────────

  class Terminal {
    constructor() {
      this.cwd = "";
      this.busy = false;
      this.cancelled = false;
      this.last = null;
      this.histIndex = null;
      this.waiter = null;
      this.pane = h(`<div class="pane" role="tabpanel">
        <div class="term-grid" aria-live="off"></div>
        <div class="ib">
          <div class="ib-chips"></div>
          <div class="ib-editor"><span class="ib-chev">›</span><div class="ib-field"><div class="ib-mirror" aria-hidden="true"></div>
            <input type="text" spellcheck="false" autocomplete="off" autocapitalize="off" aria-label="Command input" placeholder="Run a command…"></div></div>
        </div></div>`);
      this.grid = $(".term-grid", this.pane);
      this.input = $("input", this.pane);
      this.mirror = $(".ib-mirror", this.pane);
      this.chips = $(".ib-chips", this.pane);
      this.input.addEventListener("input", () => { this.histIndex = null; this.paint(); });
      this.input.addEventListener("keydown", (e) => this.onKey(e));
      this.input.addEventListener("scroll", () => (this.mirror.scrollLeft = this.input.scrollLeft));
      this.pane.addEventListener("mouseup", (e) => {
        if (e.target.closest("button, .t-link, .blk-tools") || getSelection().toString()) return;
        this.focus();
      });
      this.renderChips();
      this.paint();
    }

    shortCwd() {
      const parts = ["~", "C", "trailhead", ...this.cwd.split("/").filter(Boolean)];
      return parts.map((p, i) => (i === 0 || i === parts.length - 1 ? p : p[0])).join("/");
    }
    longCwd() { return REPO + (this.cwd ? "/" + this.cwd : ""); }
    focus() { this.input.focus({ preventScroll: true }); }
    scrollToEnd() { this.grid.scrollTop = this.grid.scrollHeight; }
    nearEnd() { return this.grid.scrollHeight - this.grid.scrollTop - this.grid.clientHeight < 40; }

    paint() {
      const v = this.input.value;
      const ghost = this.agentMode ? "" : suggestion(v);
      this.ghost = ghost;
      this.mirror.innerHTML = (this.agentMode ? esc(v) : highlightShell(v)) + (ghost ? `<span class="ib-ghost">${esc(ghost)}</span>` : "");
      this.mirror.scrollLeft = this.input.scrollLeft;
    }

    setInput(v) { this.input.value = v; this.paint(); this.input.setSelectionRange(v.length, v.length); }

    renderChips() {
      const t = totals();
      const status = this.last == null ? "" : this.last === 0 ? '<span class="chip"><span class="ok">✓</span></span>' : `<span class="chip"><span class="bad">✗ ${this.last}</span></span>`;
      this.chips.innerHTML = `<span class="chip">fish</span><span class="chip">${esc(this.longCwd())}</span><span class="chip">⑂ ${BRANCH}</span>` +
        (t.files ? `<span class="chip">${t.files} · <span class="add">+${t.add}</span> <span class="del">−${t.del}</span></span>` : "") + status;
      this.updatePlaceholder();
    }

    updatePlaceholder() {
      if (this.agentMode) this.input.placeholder = S.agent?.state === "working" ? `${S.agent.name} is working… (⌃C to stop)` : `Reply to ${S.agent?.name ?? "the agent"}… (/exit to quit)`;
      else if (this.busy) this.input.placeholder = "Running… ⌃C stops it";
      else this.input.placeholder = this.coach ? "Your turn: try claude, git log or help" : "Run a command…";
    }

    onKey(e) {
      const v = this.input.value;
      if (e.key === "Enter") {
        e.preventDefault();
        if (this.agentMode) { if (this.waiter && S.agent?.state !== "working") { const w = this.waiter; this.waiter = null; this.setInput(""); w(v); } return; }
        if (this.busy) return;
        this.setInput("");
        this.run(v);
      } else if ((e.key === "Tab" || (e.key === "ArrowRight" && this.input.selectionStart === v.length)) && this.ghost) {
        e.preventDefault();
        this.setInput(v + this.ghost);
      } else if (e.key === "Tab") {
        e.preventDefault();
      } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !this.agentMode && S.history.length) {
        e.preventDefault();
        const n = S.history.length;
        if (this.histIndex == null) this.histIndex = n;
        this.histIndex = Math.max(0, Math.min(n, this.histIndex + (e.key === "ArrowUp" ? -1 : 1)));
        const idx = this.histIndex;
        this.setInput(idx >= n ? "" : S.history[idx]);
        this.histIndex = idx;
      } else if (e.ctrlKey && e.key.toLowerCase() === "c") {
        e.preventDefault();
        if (this.agentMode) { this.waiter?.("/exit"); this.waiter = null; this.cancelled = true; }
        else if (this.busy) this.cancelRun();
        else this.setInput("");
      } else if (e.ctrlKey && e.key.toLowerCase() === "l") {
        e.preventDefault();
        if (!this.busy) this.clear();
      }
    }

    cancelRun() { if (this.busy) { this.cancelled = true; this.wake?.(); } }

    clear() { this.grid.querySelectorAll(".blk, .term-welcome").forEach((b) => b.remove()); }

    block(line) {
      const blk = h(`<div class="blk running">
        <div class="blk-head"><span class="cwd">${esc(this.longCwd())}</span><span class="br">${BRANCH}</span><span class="blk-status"><span class="spinner"></span></span></div>
        <div class="blk-cmd"><span class="chev">❯</span> ${highlightShell(line)}</div>
        <div class="blk-out"></div>
        <div class="blk-tools"><button type="button" data-b="copy" title="Copy output">${ICON.copy}</button><button type="button" data-b="rerun" title="Rerun command">${ICON.rerun}</button><button type="button" data-b="send" title="Send to agent">${ICON.send}</button></div>
      </div>`);
      blk.dataset.cmd = line;
      blk.addEventListener("click", (e) => {
        const b = e.target.closest("[data-b]");
        if (!b) {
          if (!e.target.closest(".t-link") && !getSelection().toString()) blk.classList.toggle("selected");
          return;
        }
        if (b.dataset.b === "copy") {
          navigator.clipboard?.writeText($(".blk-out", blk).innerText).catch(() => {});
          toast({ title: "Copied the block’s output", ms: 1800 });
        } else if (b.dataset.b === "rerun") {
          if (!this.busy) this.run(line);
        } else if (b.dataset.b === "send") {
          if (S.agent) toast({ title: `Sent the block to ${S.agent.name}`, sub: `❯ ${line}`, ms: 2400 });
          else toast({ title: "No agent is running", sub: "Run claude in a terminal first.", ms: 2600 });
        }
      });
      const stick = this.nearEnd();
      this.grid.appendChild(blk);
      if (stick) this.scrollToEnd();
      const out = $(".blk-out", blk);
      return {
        el: blk,
        print: (html = "", animate = true) => {
          const stick2 = this.nearEnd();
          const line = h(`<div class="line${animate && !reduced ? " in" : ""}"></div>`);
          line.innerHTML = html;
          out.appendChild(line);
          if (stick2) this.scrollToEnd();
          return line;
        },
        finish: (code, ms) => {
          blk.classList.remove("running");
          if (code) blk.classList.add("fail");
          $(".blk-status", blk).innerHTML = (code ? `<span class="bad">✗ ${code}</span>` : '<span class="ok">✓</span>') + ` · ${fmtDuration(ms)}`;
        },
      };
    }

    async run(line, { instant = false } = {}) {
      line = line.trim();
      if (!line) return;
      if (S.history[S.history.length - 1] !== line) S.history.push(line);
      this.histIndex = null;
      if (/^(clear|cls)$/.test(line)) { this.clear(); return; }
      const blk = this.block(line);
      this.busy = true;
      this.cancelled = false;
      this.updatePlaceholder();
      const t0 = performance.now();
      const io = {
        term: this,
        print: (html, animate) => blk.print(html, !instant && animate !== false),
        sleep: async (ms) => {
          if (this.cancelled) throw CANCEL;
          if (!instant) await new Promise((r) => { this.wake = r; setTimeout(r, reduced ? Math.min(ms, 20) : ms); });
          if (this.cancelled) throw CANCEL;
        },
        forever: () => new Promise((_, reject) => { this.wake = () => reject(CANCEL); }),
        duration: null,
      };
      let code = 0;
      try {
        for (const part of splitChain(line)) {
          code = await execute(part.cmd, io);
          if (code && part.op === "&&") break;
        }
      } catch (err) {
        if (err !== CANCEL) { console.error(err); io.print(`<span class="t-red">${esc(String(err))}</span>`); code = 1; }
        else { io.print('<span class="t-dim">^C</span>'); code = 130; }
      }
      const ms = io.duration ?? Math.round(performance.now() - t0);
      blk.finish(code, ms);
      this.busy = false;
      this.wake = null;
      this.last = code;
      this.renderChips();
      if (this.nearEnd()) this.scrollToEnd();
      renderTabs();
      updateChrome();
      return code;
    }
  }

  function splitChain(line) {
    const parts = [];
    let op = null;
    for (const piece of line.split(/(&&|;)/)) {
      if (piece === "&&" || piece === ";") { op = piece; continue; }
      if (piece.trim()) parts.push({ cmd: piece.split("|")[0].trim(), op });
    }
    return parts;
  }

  function fmtDuration(ms) {
    if (ms < 1000) return `${Math.max(1, ms)}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
  }

  function newTerminal(activate = true) {
    const term = new Terminal();
    const tab = { kind: "terminal", term, el: term.pane };
    term.tab = tab;
    return addTab(tab, activate);
  }

  // ── Commands ──────────────────────────────────────────────────────────

  const DIRS = new Set([""]);
  Object.keys(FILES).forEach((p) => { const parts = p.split("/"); for (let i = 1; i < parts.length; i++) DIRS.add(parts.slice(0, i).join("/")); });

  function resolvePath(term, arg = "") {
    if (arg === "~" || arg === HOME) return null;
    let parts = arg.startsWith("~/Code/trailhead") ? arg.slice(17).split("/") : arg.startsWith("/") ? null : [...term.cwd.split("/"), ...arg.split("/")];
    if (!parts) return null;
    const out = [];
    for (const p of parts) {
      if (!p || p === ".") continue;
      if (p === "..") { if (!out.length) return null; out.pop(); } else out.push(p);
    }
    return out.join("/");
  }

  function listDir(dir) {
    const prefix = dir ? dir + "/" : "";
    const names = new Map();
    for (const p of Object.keys(FILES)) {
      if (!p.startsWith(prefix)) continue;
      const rest = p.slice(prefix.length);
      const [first, ...more] = rest.split("/");
      names.set(first, more.length ? "dir" : "file");
    }
    return [...names].sort((a, b) => a[0].localeCompare(b[0]));
  }

  const COMMANDS = {
    help: async ({ print }) => {
      [
        '<span class="t-bold">This is a scripted demo</span> of an Impulse terminal, in the trailhead example project.',
        "",
        '  <span class="t-cyan">git</span> status · log · diff · add · commit · branch     <span class="t-cyan">npm</span> test · run lint · run dev',
        '  <span class="t-cyan">ls</span> · <span class="t-cyan">cd</span> · <span class="t-cyan">cat</span> · <span class="t-cyan">pwd</span> · <span class="t-cyan">echo</span> · <span class="t-cyan">clear</span>                   <span class="t-cyan">impulse</span> open &lt;file&gt; · review · notify',
        '  <span class="t-cyan">claude</span> or <span class="t-cyan">codex</span>  a simulated coding agent        <span class="t-cyan">theme</span> &lt;name&gt;  switch the demo’s theme',
        "",
        '<span class="t-dim">Tab or → accepts the gray suggestion, ↑ and ↓ walk history, ⌃C stops a command.</span>',
        '<span class="t-dim">⇧⌘P opens the command palette, ⌘P finds a file, ⌘B toggles the sidebar, ⇧⌘G opens Review.</span>',
      ].forEach((l) => print(l, false));
      return 0;
    },

    ls: async ({ print, term }, args) => {
      const long = args.some((a) => /^-\w*l/.test(a));
      const target = args.find((a) => !a.startsWith("-"));
      const dir = target == null ? term.cwd : resolvePath(term, target);
      if (dir == null || !DIRS.has(dir)) {
        if (dir != null && FILES[dir]) { print(esc(target)); return 0; }
        print(`ls: ${esc(target)}: No such file or directory`); return 1;
      }
      const entries = listDir(dir);
      if (!long) { print(entries.map(([n, k]) => (k === "dir" ? `<span class="t-blue t-bold">${n}/</span>` : esc(n))).join("   ")); return 0; }
      print(`total ${entries.length * 8}`, false);
      for (const [n, k] of entries) {
        const path = (dir ? dir + "/" : "") + n;
        const size = k === "dir" ? 96 + 32 * listDir(path).length : (current(path) || "").length;
        print(`${k === "dir" ? "drwxr-xr-x" : "-rw-r--r--"}  1 you  staff  ${String(size).padStart(5)} Oct  6 14:02 ${k === "dir" ? `<span class="t-blue t-bold">${n}</span>` : esc(n)}`, false);
      }
      return 0;
    },

    cd: async ({ print, term }, args) => {
      const arg = args[0] ?? "";
      if (arg === "") { term.prev = term.cwd; term.cwd = ""; }
      else if (arg === "-") { [term.cwd, term.prev] = [term.prev ?? "", term.cwd]; }
      else {
        const dir = arg === "~" ? null : resolvePath(term, arg);
        if (dir == null || !DIRS.has(dir)) {
          if (arg === "~" || arg === "..") print('<span class="t-dim">This demo stays inside the trailhead project.</span>');
          else print(`cd: The directory “${esc(arg)}” does not exist`);
          return 1;
        }
        term.prev = term.cwd;
        term.cwd = dir;
      }
      renderTabs();
      return 0;
    },

    pwd: async ({ print, term }) => { print(`${HOME}/Code/trailhead${term.cwd ? "/" + term.cwd : ""}`); return 0; },

    cat: async ({ print, sleep, term }, args) => {
      if (!args.length) { print("cat: give it a file, like src/lib/cache.ts"); return 1; }
      for (const a of args) {
        const p = resolvePath(term, a);
        const text = p != null && current(p);
        if (text == null || text === false) { print(`cat: ${esc(a)}: No such file or directory`); return 1; }
        for (const l of text.replace(/\n$/, "").split("\n")) { print(esc(l), false); }
        await sleep(10);
      }
      return 0;
    },

    echo: async ({ print }, args) => {
      const vars = { $SHELL: "/opt/homebrew/bin/fish", $HOME: HOME, $USER: "you", $EDITOR: "impulse edit", $TERM_PROGRAM: "Impulse", $PWD: `${HOME}/Code/trailhead` };
      print(esc(args.map((a) => a.replace(/\$\w+/g, (v) => vars[v] ?? "")).join(" ")));
      return 0;
    },

    whoami: async ({ print }) => { print("you"); return 0; },
    hostname: async ({ print }) => { print("MacBook-Pro.local"); return 0; },
    date: async ({ print }) => { print(new Date().toString().replace(/ GMT.*/, "")); return 0; },
    uname: async ({ print }, args) => { print(args.includes("-a") ? "Darwin MacBook-Pro.local 25.0.0 Darwin Kernel Version 25.0.0 arm64" : "Darwin"); return 0; },
    sw_vers: async ({ print }) => { ["ProductName:		macOS", "ProductVersion:		26.0", "BuildVersion:		25A354"].forEach((l) => print(l, false)); return 0; },
    true: async () => 0,
    false: async () => 1,
    which: async ({ print }, args) => {
      const paths = { git: "/usr/bin/git", npm: "/opt/homebrew/bin/npm", node: "/opt/homebrew/bin/node", impulse: "/Applications/Impulse.app/Contents/Resources/bin/impulse", claude: "/Users/you/.local/bin/claude", fish: "/opt/homebrew/bin/fish" };
      for (const a of args) { if (!paths[a]) return 1; print(paths[a]); }
      return 0;
    },
    history: async ({ print }) => { S.history.forEach((l, i) => print(`${String(i + 1).padStart(4)}  ${esc(l)}`, false)); return 0; },

    exit: async ({ print, sleep, term }) => {
      print('<span class="t-dim">Closing the tab.</span>');
      await sleep(400);
      setTimeout(() => closeTab(term.tab.id), 0);
      return 0;
    },

    git: async (io, args) => {
      const { print, sleep } = io;
      const sub = args[0];
      const list = changes();
      if (sub === "status" || sub === "st") {
        const short = args.some((a) => /^-\w*s/.test(a));
        if (short) {
          print(`<span class="t-green">## ${BRANCH}</span>...<span class="t-red">origin/${BRANCH}</span> [ahead <span class="t-green">2</span>]`);
          for (const f of list) {
            const staged = S.staged.has(f.path);
            if (f.status === "U") print(`<span class="t-red">??</span> ${f.path}`);
            else print(staged ? `<span class="t-green">M</span>  ${f.path}` : ` <span class="t-red">M</span> ${f.path}`);
          }
          return 0;
        }
        print(`On branch ${BRANCH}`);
        print(`Your branch is ahead of 'origin/${BRANCH}' by 2 commits.`);
        const st = list.filter((f) => S.staged.has(f.path)), un = list.filter((f) => !S.staged.has(f.path));
        if (st.length) { print(""); print("Changes to be committed:"); st.forEach((f) => print(`        <span class="t-green">modified:   ${f.path}</span>`)); }
        if (un.length) { print(""); print("Changes not staged for commit:"); un.forEach((f) => print(`        <span class="t-red">modified:   ${f.path}</span>`)); }
        if (!list.length) print("nothing to commit, working tree clean");
        return 0;
      }
      if (sub === "log") {
        for (let i = 0; i < S.commits.length; i++) {
          const [sha, lane, msg, ref] = S.commits[i];
          const head = i === 0 ? ` (<span class="t-cyan t-bold">HEAD -&gt; </span><span class="t-green t-bold">${BRANCH}</span>)` : "";
          const tag = ref ? ` (<span class="t-yellow t-bold">${ref}</span>)` : "";
          const g = lane ? '<span class="t-red">|</span> * ' : "* ";
          print(`${g}<span class="t-yellow">${sha}</span>${head}${tag} ${esc(msg)}`);
          if (i === 2 && S.commits[i][1]) print('<span class="t-red">|</span>/');
          if (i % 2) await sleep(25);
        }
        return 0;
      }
      if (sub === "diff") {
        const stat = args.includes("--stat");
        const staged = args.includes("--staged") || args.includes("--cached");
        const files = list.filter((f) => (staged ? S.staged.has(f.path) : !S.staged.has(f.path)));
        if (stat) {
          files.forEach((f) => print(` ${f.path.padEnd(22)} | ${String(f.add + f.del).padStart(2)} <span class="t-green">${"+".repeat(Math.min(f.add, 20))}</span><span class="t-red">${"-".repeat(Math.min(f.del, 20))}</span>`));
          if (files.length) print(` ${files.length} file${files.length > 1 ? "s" : ""} changed, ${files.reduce((s, f) => s + f.add, 0)} insertions(+), ${files.reduce((s, f) => s + f.del, 0)} deletions(-)`);
          return 0;
        }
        for (const f of files) {
          print(`<span class="t-bold">diff --git a/${f.path} b/${f.path}</span>`, false);
          print(`<span class="t-bold">--- a/${f.path}</span>`, false);
          print(`<span class="t-bold">+++ b/${f.path}</span>`, false);
          for (const hk of hunks(f.ops)) {
            print(`<span class="t-cyan">${hk.header}</span>`, false);
            for (const l of hk.lines) print(l.t === "+" ? `<span class="t-green">+${esc(l.s)}</span>` : l.t === "-" ? `<span class="t-red">-${esc(l.s)}</span>` : ` ${esc(l.s)}`, false);
          }
          await sleep(30);
        }
        if (!files.length && !staged && S.staged.size) print('<span class="t-dim">(Nothing unstaged. Try git diff --staged, or open Review with ⇧⌘G.)</span>');
        return 0;
      }
      if (sub === "add") {
        const targets = args.slice(1);
        if (!targets.length) { print("Nothing specified, nothing added."); return 0; }
        for (const t of targets) {
          if (t === "." || t === "-A" || t === "--all") list.forEach((f) => S.staged.add(f.path));
          else {
            const p = resolvePath(io.term, t);
            if (!list.some((f) => f.path === p)) { print(`fatal: pathspec '${esc(t)}' did not match any files`); return 128; }
            S.staged.add(p);
          }
        }
        refreshAll();
        return 0;
      }
      if (sub === "restore" && args[1] === "--staged") { args.slice(2).forEach((t) => S.staged.delete(resolvePath(io.term, t))); refreshAll(); return 0; }
      if (sub === "commit") {
        const mi = args.indexOf("-m");
        const msg = mi >= 0 ? args[mi + 1] : null;
        const staged = list.filter((f) => S.staged.has(f.path));
        if (!staged.length) { print("no changes added to commit (use \"git add\")"); return 1; }
        if (!msg) { print('<span class="t-dim">In Impulse this opens the message in an editor tab (impulse edit). Here, use -m "message".</span>'); return 1; }
        await sleep(160);
        const sha = Math.random().toString(16).slice(2, 9);
        staged.forEach((f) => { S.base[f.path] = current(f.path); S.staged.delete(f.path); });
        S.commits.unshift([sha, "", msg]);
        print(`[${BRANCH} ${sha}] ${esc(msg)}`);
        print(` ${staged.length} file${staged.length > 1 ? "s" : ""} changed, ${staged.reduce((s, f) => s + f.add, 0)} insertions(+), ${staged.reduce((s, f) => s + f.del, 0)} deletions(-)`);
        refreshAll();
        return 0;
      }
      if (sub === "push") {
        print("Enumerating objects: 9, done.");
        await sleep(350);
        print("Writing objects: 100% (5/5), 912 bytes | 912.00 KiB/s, done.");
        await sleep(300);
        print("To github.com:example/trailhead.git");
        print(`   ${S.commits[1][0]}..${S.commits[0][0]}  ${BRANCH} -&gt; ${BRANCH}`);
        return 0;
      }
      if (sub === "pull" || sub === "fetch") { await sleep(500); if (sub === "pull") print("Already up to date."); return 0; }
      if (sub === "branch") { ["  fix-elevation", `* <span class="t-green">${BRANCH}</span>`, "  main"].forEach((l) => print(l, false)); return 0; }
      if (sub === "checkout" || sub === "switch") { print('<span class="t-dim">Switching branches is ⌃⌘B in Impulse. This demo stays on its branch.</span>'); return 0; }
      if (!sub || sub === "--help" || sub === "help") { print("usage: git [-v | --version] [-C &lt;path&gt;] &lt;command&gt; [&lt;args&gt;]"); return sub ? 0 : 1; }
      if (sub === "--version" || sub === "-v") { print("git version 2.51.0"); return 0; }
      print(`git: '${esc(sub)}' is not a git command. See 'git --help'.`);
      return 1;
    },

    npm: async (io, args) => {
      const { print, sleep } = io;
      let script = args[0] === "run" || args[0] === "run-script" ? args[1] : args[0];
      if (script === "t") script = "test";
      if (args[0] === "-v" || args[0] === "--version") { print("11.6.0"); return 0; }
      if (script === "install" || script === "i" || script === "ci") { await sleep(500); print("up to date, audited 1 package in 214ms"); print(""); print("found <span class=\"t-green t-bold\">0</span> vulnerabilities"); return 0; }
      const scripts = { test: "node --test --test-reporter=./scripts/test-reporter.mjs test/", lint: "node --check src/server.ts", dev: "node --watch src/server.ts", start: "node src/server.ts" };
      if (!script) { print("Usage: npm &lt;command&gt;  (try npm test)"); return 1; }
      if (!scripts[script]) { print(`<span class="t-red">npm error</span> Missing script: "${esc(script)}"`); return 1; }
      print(`<span class="t-dim">&gt; trailhead@0.2.0 ${script}</span>`, false);
      print(`<span class="t-dim">&gt; ${scripts[script]}</span>`, false);
      print("", false);
      if (script === "test") return runTests(io);
      if (script === "lint") { await sleep(650); return 0; }
      await sleep(400);
      print('trailhead listening on <span class="t-link" data-port>http://localhost:3000</span>');
      toast({ title: "Port 3000 is listening", sub: "Impulse shows it in the status bar, ⌘-click to open.", ms: 3200 });
      $('[data-status-right]').textContent = ":3000 · fish";
      try { await io.forever(); } finally { $('[data-status-right]').textContent = "fish"; }
      return 0;
    },

    node: async ({ print }, args) => {
      if (args[0] === "-v" || args[0] === "--version") { print("v24.9.0"); return 0; }
      print('<span class="t-dim">Try npm test or npm run dev: this demo has no Node REPL.</span>');
      return 1;
    },
    npx: async ({ print }) => { print('<span class="t-dim">This demo can’t download packages.</span>'); return 1; },

    impulse: async ({ print, term }, args) => {
      const sub = args[0];
      if (sub === "open" || sub === "edit") {
        if (!args[1]) { print(`usage: impulse ${sub} &lt;file&gt;[:line[:column]]`); return 2; }
        const [file, line] = args[1].split(":");
        const p = resolvePath(term, file);
        if (p == null || !FILES[p]) { print(`impulse: no such file: ${esc(args[1])}`); return 1; }
        openFile(p, Number(line) || 1);
        if (sub === "edit") print('<span class="t-dim">(impulse edit waits until the tab closes; the demo doesn’t wait.)</span>');
        return 0;
      }
      if (sub === "review") { openReview(); return 0; }
      if (sub === "notify") { toast({ title: args[1] || "Impulse", sub: args.slice(2).join(" ") || "From impulse notify", ms: 3000 }); return 0; }
      if (sub === "status") { print("trailhead · " + BRANCH + " · " + totals().files + " changed files"); return 0; }
      if (sub === "split") { print('<span class="t-dim">In Impulse this splits the current pane. The demo keeps one pane per tab.</span>'); return 0; }
      [
        "Usage: impulse &lt;command&gt;",
        "",
        "  open &lt;file&gt;[:line]   Open a file in an editor tab",
        "  edit &lt;file&gt;         Open a file and wait for its tab to close ($EDITOR)",
        "  review [scope]      Open Review",
        "  split [right|down]  Split the current pane",
        "  tab [command]       Open a new terminal tab",
        "  notify &lt;title&gt;      Post a notification",
        "  status &lt;state&gt;      Report agent status (hooks)",
      ].forEach((l) => print(l, false));
      return sub ? 1 : 0;
    },

    theme: async ({ print }, args) => {
      const ids = Object.keys(THEMES);
      if (!args.length) { print(ids.map((id) => (id === S.theme ? `<span class="t-green">${id}</span>` : id)).join("  ")); return 0; }
      const want = args.join("-").toLowerCase();
      const id = ids.find((i) => i === want) || ids.find((i) => i.startsWith(want)) || ids.find((i) => THEMES[i].name.toLowerCase().replace(/\s+/g, "-") === want);
      if (!id) { print(`theme: no theme named “${esc(args.join(" "))}”. Try theme, without a name, for the list.`); return 1; }
      applyTheme(id);
      print(`Theme: <span class="t-bold">${esc(THEMES[id].name)}</span>`);
      return 0;
    },

    sudo: async ({ print }) => { print("Sorry, this demo has no root."); return 1; },
    rm: async ({ print }) => { print('<span class="t-dim">Nothing gets deleted in a demo. (In Impulse, destructive git commands take a snapshot first so they can be undone.)</span>'); return 1; },
    mkdir: async ({ print }) => { print('<span class="t-dim">The demo’s project is read-only.</span>'); return 1; },
    touch: async ({ print }) => { print('<span class="t-dim">The demo’s project is read-only.</span>'); return 1; },
    brew: async ({ print }) => { print("Impulse comes as a disk image from GitHub Releases: use the Download button above."); return 1; },
  };

  for (const name of ["vim", "nvim", "nano", "code", "open", "less"]) {
    COMMANDS[name] = async (io, args) => {
      const p = args[0] && resolvePath(io.term, args[0]);
      if (p && FILES[p]) {
        if (name !== "code" && name !== "open") io.print(`<span class="t-dim">In Impulse, ${name} runs right here in the terminal. The demo opens an editor tab instead.</span>`);
        openFile(p, 1);
        return 0;
      }
      io.print(`<span class="t-dim">Full-screen programs like ${name} take over the terminal in Impulse. Try impulse open src/lib/cache.ts.</span>`);
      return 1;
    };
  }
  for (const name of ["top", "htop", "ssh", "man"]) {
    COMMANDS[name] = async ({ print }) => { print(`<span class="t-dim">${name} works in Impulse (it takes the keyboard while it runs), but not in this demo.</span>`); return 1; };
  }
  const AGENTS = { claude: "Claude Code", codex: "Codex", gemini: "Gemini CLI", aider: "Aider", opencode: "opencode" };
  for (const [cmd, name] of Object.entries(AGENTS)) COMMANDS[cmd] = (io) => agentSession(cmd, name, io);

  async function execute(line, io) {
    const [name, ...args] = splitArgs(line);
    const fn = COMMANDS[name];
    if (!fn) { io.print(`fish: Unknown command: <span class="t-red">${esc(name)}</span>`); return 127; }
    return (await fn(io, args)) ?? 0;
  }

  async function runTests(io) {
    const { print, sleep } = io;
    const tests = [
      ["returns what was stored", 0.33],
      ["forgets expired entries", 0.23],
      ["size leaves out expired entries", 0.41, true],
      ["kilometres to miles", 0.48],
      ["metres to feet", 0.06],
      ["celsius to fahrenheit", 0.06],
    ];
    let fail = 0;
    for (const [name, ms, broken] of tests) {
      await sleep(110 + Math.random() * 120);
      if (broken && !S.fixed) { fail++; print(`<span class="t-red">✖ ${name}</span> <span class="t-dim">(${ms}ms)</span>`); }
      else print(`<span class="t-green">✔ ${name}</span> <span class="t-dim">(${ms}ms)</span>`);
    }
    await sleep(120);
    print(`<span class="t-blue">ℹ</span> tests ${tests.length}`, false);
    print(`<span class="t-blue">ℹ</span> pass ${tests.length - fail}`, false);
    print(`<span class="t-blue">ℹ</span> fail ${fail}`, false);
    if (fail) {
      print("", false);
      print('<span class="t-red">✖ failing tests:</span>', false);
      print("", false);
      print('<span class="t-red">✖ size leaves out expired entries</span>', false);
      print("  AssertionError [ERR_ASSERTION]: Expected values to be strictly equal:", false);
      print("", false);
      print('  <span class="t-green">1</span> !== <span class="t-red">0</span>', false);
      print("", false);
      print(`      at <span class="t-link" data-open="test/cache.test.ts:28">test/cache.test.ts:28:10</span>`, false);
      io.duration = 1240 + Math.round(Math.random() * 300);
      return 1;
    }
    io.duration = 980 + Math.round(Math.random() * 300);
    return 0;
  }

  // Clicking a file:line in output opens it, like ⌘-click in Impulse.
  panesEl.addEventListener("click", (e) => {
    const link = e.target.closest(".t-link");
    if (!link) return;
    if (link.dataset.open) { const [p, l] = link.dataset.open.split(":"); openFile(p, Number(l)); }
    else if (link.hasAttribute("data-port")) toast({ title: "Would open localhost:3000", sub: "In Impulse, ⌘-click opens links and ports.", ms: 2400 });
  });

  // ── The simulated agent ───────────────────────────────────────────────

  function setAgentState(state) {
    if (!S.agent) return;
    S.agent.state = state;
    S.agent.term.updatePlaceholder();
    renderTabs();
    updateChrome();
  }

  function endAgent() {
    if (!S.agent) return;
    const term = S.agent.term;
    term.agentMode = false;
    S.agent = null;
    term.updatePlaceholder();
    term.paint();
    renderTabs();
    updateChrome();
  }

  async function agentSession(cmd, name, io) {
    const { print, sleep, term } = io;
    if (S.agent) { print(`<span class="t-dim">${esc(S.agent.name)} is already running in another tab. The demo runs one agent at a time.</span>`); return 1; }
    S.agent = { name, cmd, term, state: "waiting" };
    term.agentMode = true;
    term.paint();
    print(`<span class="agent-box"><span class="t-magenta">✻</span> <span class="t-bold">${esc(name)}</span>  <span class="t-dim">a simulated session · type /exit to leave</span></span>`, false);
    print('<span class="t-dim">What should we work on? (Try: fix the failing test)</span>');
    setAgentState("waiting");
    let turns = 0;
    try {
      for (;;) {
        const text = (await new Promise((r) => (term.waiter = r))).trim();
        if (!text) continue;
        if (text === "/exit" || text === "exit" || text === "/quit") break;
        print(`<span class="t-magenta">›</span> ${esc(text)}`, false);
        setAgentState("working");
        turns++;
        const out = (l) => print(l);
        if (!S.fixed && turns <= 3) {
          await sleep(700); out('<span class="t-magenta">●</span> Read <span class="t-bold">src/lib/cache.ts</span>');
          await sleep(500); out('<span class="t-magenta">●</span> Read <span class="t-bold">test/cache.test.ts</span>');
          await sleep(600); out('<span class="t-magenta">●</span> Bash <span class="t-dim">npm test</span>  <span class="t-red">✖ size leaves out expired entries</span>');
          await sleep(900); out('<span class="t-magenta">●</span> Update <span class="t-bold">src/lib/cache.ts</span>  <span class="t-green">+4</span> <span class="t-red">−1</span>');
          S.fixed = true;
          refreshAll();
          $("[data-changes]").classList.remove("bump"); void $("[data-changes]").offsetWidth; $("[data-changes]").classList.add("bump");
          await sleep(500); out('  <span class="t-dim">size counted entries that had expired but that get() hadn’t removed yet.</span>');
          await sleep(500); out('<span class="t-magenta">●</span> Bash <span class="t-dim">npm test</span>  <span class="t-green">✔ 6 tests pass</span>');
          await sleep(400); out("The size getter now counts only live entries, and all 6 tests pass.");
          out('<span class="t-dim">Impulse checkpointed this turn: Review ▸ Last agent turn shows exactly what changed.</span>');
          setAgentState("done");
          toast({
            title: `${name} finished`, sub: "trailhead · Review the turn, or reply in the terminal",
            action: { label: "Review", run: () => openReview() }, ms: 7000,
          });
        } else {
          await sleep(900);
          out(S.fixed ? "Nothing else to fix: the tests pass. (This agent is scripted and knows one trick. Type /exit to leave.)" : "This agent is scripted. Type /exit to leave.");
          setAgentState("done");
        }
      }
    } finally {
      const quitting = S.agent?.term === term;
      if (quitting) endAgent();
    }
    print('<span class="t-dim">Session ended.</span>', false);
    return 0;
  }

  // ── Editor tabs ───────────────────────────────────────────────────────

  function openFile(path, line = 1, focus = true) {
    let tab = S.tabs.find((t) => t.kind === "editor" && t.path === path);
    if (!tab) {
      const el = h(`<div class="pane" role="tabpanel"><div class="ed-bar"></div><div class="ed" tabindex="0" aria-label="Editor"></div></div>`);
      tab = { kind: "editor", path, el, line };
      tab.render = () => renderEditor(tab);
      $(".ed", el).addEventListener("click", (e) => {
        const row = e.target.closest(".ed-line");
        if (row) { tab.line = Number(row.dataset.n); renderEditor(tab, false); }
      });
      $(".ed", el).addEventListener("keydown", (e) => {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const max = current(path).split("\n").length;
          tab.line = Math.max(1, Math.min(max, tab.line + (e.key === "ArrowDown" ? 1 : -1)));
          renderEditor(tab, true);
        }
      });
      addTab(tab, false);
    }
    tab.line = line;
    renderEditor(tab, true);
    activateTab(tab.id, focus);
    selectFileInTree(path);
  }

  function renderEditor(tab, scroll) {
    const text = current(tab.path) ?? "";
    const lines = text.replace(/\n$/, "").split("\n");
    const marks = {};
    if (text !== S.base[tab.path]) {
      const ops = diffLines(S.base[tab.path], text);
      ops.forEach((op, i) => { if (op.t === "+") marks[op.b] = ops[i - 1]?.t === "-" || ops.slice(Math.max(0, i - 4), i).some((o) => o.t === "-") ? "m" : "a"; });
    }
    const hl = highlighter(tab.path);
    const crumbs = tab.path.split("/").map((p, i, all) => (i === all.length - 1 ? `<b>${esc(p)}</b>` : esc(p))).join(" › ");
    const status = Object.keys(marks).length ? ' <span style="color:var(--w-mod)">M</span>' : "";
    $(".ed-bar", tab.el).innerHTML = `${ICON.file} ${crumbs}${status}`;
    const blame = (n) => (marks[n] ? "You · Uncommitted changes" : n < 4 ? "maya · 3 weeks ago · Unit conversions with tests" : "maya · 4 days ago · TTL cache for forecast lookups");
    $(".ed", tab.el).innerHTML = lines.map((l, i) => {
      const n = i + 1, cur = n === tab.line;
      return `<div class="ed-line${cur ? " cur" : ""}" data-n="${n}"><span class="ed-num">${n}</span><span class="ed-mark ${marks[n] || ""}"></span><span class="ed-code">${hl(l) || " "}${cur ? `<span class="ed-blame">${esc(blame(n))}</span>` : ""}</span></div>`;
    }).join("");
    if (scroll) {
      const row = $(`.ed-line[data-n="${tab.line}"]`, tab.el);
      const ed = $(".ed", tab.el);
      if (row) requestAnimationFrame(() => {
        const top = row.offsetTop - ed.clientHeight / 3;
        if (row.offsetTop < ed.scrollTop || row.offsetTop > ed.scrollTop + ed.clientHeight - 30) ed.scrollTop = Math.max(0, top);
      });
    }
  }

  // ── Review ────────────────────────────────────────────────────────────

  function openReview() {
    let tab = S.tabs.find((t) => t.kind === "review");
    if (!tab) {
      const el = h(`<div class="pane" role="tabpanel"><div class="rv-head"></div><div class="rv-body"><div class="rv-files"></div><div class="rv-diff"></div></div></div>`);
      tab = { kind: "review", el, composer: null };
      tab.render = () => renderReview(tab);
      el.addEventListener("click", (e) => onReviewClick(tab, e));
      addTab(tab, false);
    }
    activateTab(tab.id);
  }

  function renderReview(tab) {
    const list = changes();
    const t = totals();
    const n = S.comments.length;
    $(".rv-head", tab.el).innerHTML = `${ICON.review}<b>Uncommitted changes</b><span>${t.files} file${t.files === 1 ? "" : "s"}</span><span class="add">+${t.add}</span><span class="del">−${t.del}</span>
      <span class="rv-spacer"></span><span style="color:var(--w-comment)">${n} comment${n === 1 ? "" : "s"}</span>
      <button type="button" class="rv-send" data-rv="send" ${n ? "" : "disabled"}>Send to agent</button>`;
    let dir = null;
    $(".rv-files", tab.el).innerHTML = list.map((f) => {
      const d = f.path.includes("/") ? f.path.slice(0, f.path.lastIndexOf("/")) : "";
      const head = d !== dir ? `<div class="rv-dir">${esc((dir = d) || "/")}</div>` : "";
      return `${head}<button type="button" class="rv-file" data-rv="file" data-path="${f.path}"><span class="rv-check${S.staged.has(f.path) ? " on" : ""}" data-rv="stage" data-path="${f.path}" title="Stage file"></span><span class="st ${f.status}">${f.status}</span>${esc(f.path.split("/").pop())}<span class="counts"><span class="add">+${f.add}</span> <span class="del">−${f.del}</span></span></button>`;
    }).join("") || '<div class="rv-dir">No changes</div>';
    const diff = $(".rv-diff", tab.el);
    const scroll = diff.scrollTop;
    if (!list.length) { diff.innerHTML = '<div class="rv-empty">Nothing to review: the working tree is clean.</div>'; return; }
    diff.innerHTML = list.map((f) => {
      const body = hunks(f.ops).map((hk) => `<div class="rv-hunk">${hk.header}</div>` + hk.lines.map((l) => {
        const cls = l.t === "+" ? "a" : l.t === "-" ? "d" : "";
        const line = l.b ?? l.a;
        const comments = l.t !== "-" ? S.comments.filter((c) => c.path === f.path && c.line === l.b).map((c) => `<div class="rv-comment"><div class="who">line ${c.line}</div>${esc(c.text)}</div>`).join("") : "";
        const composer = tab.composer && tab.composer.path === f.path && tab.composer.line === l.b && l.t !== "-" ? `<div class="rv-composer"><textarea placeholder="Comment on line ${l.b}… (⌘↩ to save)"></textarea><div class="row"><button type="button" data-rv="cancel">Cancel</button><button type="button" class="primary" data-rv="save">Comment</button></div></div>` : "";
        return `<div class="rv-line ${cls}" data-path="${f.path}" data-line="${l.b ?? ""}"><span class="n" data-rv="comment" title="${l.t === "-" ? "" : "Comment on this line"}">${l.a ?? ""}</span><span class="n" data-rv="comment">${l.b ?? ""}</span><span class="sign">${l.t === " " ? "" : l.t === "-" ? "−" : "+"}</span><span class="code">${highlighter(f.path)(l.s) || " "}</span></div>${comments}${composer}`;
      }).join("")).join("");
      return `<div class="rv-fhead" id="rv-${f.path.replace(/\W/g, "-")}"><span class="st">${f.status}</span>${esc(f.path)}<span style="margin-left:auto"><span class="add">+${f.add}</span> <span class="del">−${f.del}</span></span></div>${body}`;
    }).join("");
    diff.scrollTop = scroll;
    const ta = $(".rv-composer textarea", diff);
    if (ta) {
      ta.focus({ preventScroll: true });
      ta.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); saveComment(tab, ta.value); }
        if (e.key === "Escape") { e.stopPropagation(); tab.composer = null; renderReview(tab); }
      });
    }
  }

  function saveComment(tab, text) {
    if (text.trim()) S.comments.push({ ...tab.composer, text: text.trim() });
    tab.composer = null;
    renderReview(tab);
  }

  function onReviewClick(tab, e) {
    const el = e.target.closest("[data-rv]");
    if (!el) return;
    const act = el.dataset.rv;
    if (act === "stage") {
      e.stopPropagation();
      const p = el.dataset.path;
      S.staged.has(p) ? S.staged.delete(p) : S.staged.add(p);
      refreshAll();
    } else if (act === "file") {
      $(`#rv-${el.dataset.path.replace(/\W/g, "-")}`, tab.el)?.scrollIntoView({ block: "start", behavior: reduced ? "auto" : "smooth" });
    } else if (act === "comment") {
      const row = el.closest(".rv-line");
      if (!row.dataset.line) return;
      tab.composer = { path: row.dataset.path, line: Number(row.dataset.line) };
      renderReview(tab);
    } else if (act === "cancel") { tab.composer = null; renderReview(tab); }
    else if (act === "save") saveComment(tab, $(".rv-composer textarea", tab.el).value);
    else if (act === "send") {
      const n = S.comments.length;
      if (!S.agent) { toast({ title: "No agent is running", sub: "Run claude in a terminal, then send your comments to it.", ms: 3000 }); return; }
      S.comments = [];
      renderReview(tab);
      const term = S.agent.term;
      toast({ title: `Sent ${n} comment${n === 1 ? "" : "s"} to ${S.agent.name}`, action: { label: "Show", run: () => activateTab(term.tab.id) }, ms: 3000 });
      if (term.waiter && S.agent.state !== "working") { const w = term.waiter; term.waiter = null; w(`Address ${n} review comment${n === 1 ? "" : "s"}`); }
    }
  }

  // ── File tree ─────────────────────────────────────────────────────────

  const filesEl = $("[data-files]");
  function fileIcon(name) {
    if (name.endsWith(".ts")) return '<span class="ficon ts">TS</span>';
    if (name.endsWith(".json")) return '<span class="ficon json">{}</span>';
    if (name.endsWith(".md")) return '<span class="ficon md">ⓘ</span>';
    return `<span class="ficon">${ICON.file}</span>`;
  }

  function renderTree() {
    const changed = new Map(changes().map((f) => [f.path, f.status]));
    const rows = [];
    const walk = (dir, depth) => {
      const entries = listDir(dir).sort((a, b) => (a[1] === b[1] ? a[0].localeCompare(b[0]) : a[1] === "dir" ? -1 : 1));
      for (const [name, kind] of entries) {
        const path = (dir ? dir + "/" : "") + name;
        if (kind === "dir") {
          const open = S.open.has(path);
          const dirty = [...changed.keys()].some((p) => p.startsWith(path + "/"));
          rows.push(`<button type="button" class="fi${open ? " open" : ""}${dirty ? " modified" : ""}" role="treeitem" aria-expanded="${open}" data-dir="${path}" style="--depth:${depth}">${ICON.chev}<span class="ficon dir">${ICON.folder}</span><span class="fname">${name}</span>${dirty ? '<span class="badge">•</span>' : ""}</button>`);
          if (open) walk(path, depth + 1);
        } else {
          const st = changed.get(path);
          const cls = st === "M" ? " modified" : st === "U" ? " untracked" : "";
          const sel = activeTab()?.kind === "editor" && activeTab().path === path ? " selected" : "";
          rows.push(`<button type="button" class="fi${cls}${sel}" role="treeitem" data-file="${path}" style="--depth:${depth}"><span style="width:10px"></span>${fileIcon(name)}<span class="fname">${name}</span>${st ? `<span class="badge">${st}</span>` : ""}</button>`);
        }
      }
    };
    walk("", 0);
    filesEl.innerHTML = rows.join("");
  }

  filesEl.addEventListener("click", (e) => {
    const row = e.target.closest(".fi");
    if (!row) return;
    if (row.dataset.dir) { const d = row.dataset.dir; S.open.has(d) ? S.open.delete(d) : S.open.add(d); renderTree(); }
    else openFile(row.dataset.file, 1);
  });

  function selectFileInTree(path) {
    const parts = path.split("/");
    for (let i = 1; i < parts.length; i++) S.open.add(parts.slice(0, i).join("/"));
    renderTree();
  }

  // ── Chrome: changes pill, agents button, status bar ───────────────────

  function updateChrome() {
    const t = totals();
    const pill = $("[data-changes]");
    pill.hidden = !t.files;
    pill.innerHTML = `${ICON.doc}${t.files} <span class="add">+${t.add}</span> <span class="del">−${t.del}</span>`;
    $("[data-status-changes]").innerHTML = t.files ? `${ICON.doc} ${t.files} <span class="add">+${t.add}</span> <span class="del">−${t.del}</span>` : "";
    $("[data-ws-counts]").innerHTML = t.files ? `<i class="add">+${t.add}</i> <i class="del">−${t.del}</i>` : "";
    const term = activeTab()?.term;
    $("[data-status-cwd]").textContent = term ? term.longCwd() : REPO;
    const agents = $("[data-agents]");
    if (S.agent) {
      agents.hidden = false;
      const st = S.agent.state;
      agents.innerHTML = st === "working" ? `<span class="adot working"></span>1 working` : st === "waiting" ? `<span class="adot waiting"></span><span class="waiting-txt">1 waiting</span>` : `<span class="adot done"></span>1 finished`;
      agents.title = `${S.agent.name}: ${st} (⇧⌘U)`;
    } else agents.hidden = true;
  }

  $("[data-agents]").addEventListener("click", () => S.agent && activateTab(S.agent.term.tab.id));

  function refreshAll() {
    for (const tab of S.tabs) {
      if (tab.kind === "editor") renderEditor(tab, false);
      if (tab.kind === "review" && tab.id === S.active) renderReview(tab);
      if (tab.kind === "terminal") tab.term.renderChips();
    }
    renderTree();
    updateChrome();
  }

  // ── Toasts ────────────────────────────────────────────────────────────

  const toasts = $("[data-toasts]");
  function toast({ title, sub, action, ms = 3000 }) {
    const el = h(`<div class="toast" role="status"><div class="t-body"><div class="t-title">${esc(title)}</div>${sub ? `<div class="t-sub">${esc(sub)}</div>` : ""}</div>${action ? `<button type="button">${esc(action.label)}</button>` : ""}</div>`);
    const close = () => { el.classList.add("out"); setTimeout(() => el.remove(), 220); };
    if (action) el.querySelector("button").addEventListener("click", () => { action.run(); close(); });
    toasts.appendChild(el);
    while (toasts.children.length > 3) toasts.firstElementChild.remove();
    setTimeout(close, ms);
  }

  // ── Command palette ───────────────────────────────────────────────────

  const overlay = $("[data-overlay]");
  const palInput = $("[data-pal-input]");
  const palList = $("[data-pal-list]");
  const palFoot = $("[data-pal-foot]");
  const P = { mode: "files", items: [], sel: 0, themeBefore: null };

  const PAL_COMMANDS = [
    { label: "New Terminal Tab", key: "⌘T", run: () => newTerminal() },
    { label: "Close Tab", key: "⌘W", run: () => closeTab(S.active) },
    { label: "Toggle Sidebar", key: "⌘B", run: () => toggleSidebar() },
    { label: "Review Changes", key: "⇧⌘G", run: () => openReview() },
    { label: "Go to File…", key: "⌘P", run: () => openPalette("") , keep: true },
    { label: "Change Theme…", key: "", run: () => openPalette("theme"), keep: true },
    { label: "Run Project Action: test", sub: "npm test", run: () => runInTerminal("npm test") },
    { label: "Run Project Action: lint", sub: "npm run lint", run: () => runInTerminal("npm run lint") },
    { label: "Run Project Action: dev", sub: "npm run dev", run: () => runInTerminal("npm run dev") },
    { label: "Start Claude Code", sub: "claude in a new terminal", run: () => { newTerminal(); runInTerminal("claude"); } },
    { label: "Next Agent Needing You", key: "⇧⌘U", run: () => S.agent ? activateTab(S.agent.term.tab.id) : toast({ title: "No agent needs you", ms: 1800 }) },
    { label: "Clear Terminal", key: "⌃L", run: () => firstTerminal()?.term.clear() },
    { label: "Open the Documentation", sub: "impulse-terminal.app/docs", run: () => { location.href = "docs/"; } },
  ];

  function fuzzy(query, text) {
    if (!query) return { score: 0, idx: [] };
    const q = query.toLowerCase(), t = text.toLowerCase();
    let ti = 0, score = 0, prev = -2;
    const idx = [];
    for (const ch of q) {
      const at = t.indexOf(ch, ti);
      if (at < 0) return null;
      score += at === prev + 1 ? 6 : 1;
      if (at === 0 || /[\s/._-]/.test(t[at - 1])) score += 4;
      idx.push(at);
      prev = at;
      ti = at + 1;
    }
    return { score: score - t.length * 0.05, idx };
  }

  const mark = (text, idx) => [...text].map((c, i) => (idx.includes(i) ? `<mark>${esc(c)}</mark>` : esc(c))).join("");

  function palItems() {
    const raw = palInput.value;
    if (P.mode === "theme") {
      return Object.entries(THEMES).map(([id, t]) => ({ id, label: t.name, theme: t, m: fuzzy(raw, t.name) })).filter((i) => i.m).sort((a, b) => (raw ? b.m.score - a.m.score : 0));
    }
    if (raw.startsWith(">")) {
      const q = raw.slice(1).trim();
      return PAL_COMMANDS.map((c) => ({ ...c, m: fuzzy(q, c.label) })).filter((i) => i.m).sort((a, b) => (q ? b.m.score - a.m.score : 0));
    }
    return Object.keys(FILES).map((p) => ({ label: p.split("/").pop(), sub: p, path: p, m: fuzzy(raw, p) })).filter((i) => i.m).sort((a, b) => (raw ? b.m.score - a.m.score : 0));
  }

  function renderPalette() {
    P.items = palItems();
    P.sel = Math.min(P.sel, Math.max(0, P.items.length - 1));
    palList.innerHTML = P.items.map((it, i) => {
      const lblIdx = it.path ? it.m.idx.map((x) => x - (it.path.length - it.label.length)).filter((x) => x >= 0) : it.m.idx;
      const right = it.theme ? `<span class="sw">${["bg", "accent", "blue", "magenta", "green"].map((k) => `<i style="background:${it.theme.palette[k]}"></i>`).join("")}</span>` : it.key ? `<span class="key">${it.key}</span>` : "";
      const sub = it.sub && it.sub !== it.label ? `<span class="sub">${esc(it.sub)}</span>` : "";
      return `<div class="dpal-item" role="option" data-i="${i}" aria-selected="${i === P.sel}"><span class="lbl">${mark(it.label, lblIdx)}</span>${sub}${right}</div>`;
    }).join("") || `<div class="dpal-empty">No matches</div>`;
    palList.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
    palFoot.innerHTML = P.mode === "theme" ? "<span>↑↓ preview</span><span>↩ choose</span><span>esc cancel</span>" : "<span>↑↓ move</span><span>↩ open</span><span><b>&gt;</b> commands</span><span>esc close</span>";
    if (P.mode === "theme" && P.items[P.sel]) applyTheme(P.items[P.sel].id, false);
  }

  function openPalette(mode) {
    tookOver();
    if (P.mode === "theme" && mode !== "theme" && P.themeBefore) P.themeBefore = null;
    P.mode = mode === "theme" ? "theme" : "files";
    if (P.mode === "theme") P.themeBefore = S.theme;
    overlay.hidden = false;
    palInput.value = mode === ">" ? ">" : "";
    palInput.placeholder = P.mode === "theme" ? "Choose a theme" : "Search files, or type > for commands";
    P.sel = P.mode === "theme" ? Math.max(0, Object.keys(THEMES).indexOf(S.theme)) : 0;
    renderPalette();
    palInput.focus({ preventScroll: true });
  }

  function closePalette(revert = true) {
    if (overlay.hidden) return;
    if (revert && P.mode === "theme" && P.themeBefore) applyTheme(P.themeBefore, false);
    P.themeBefore = null;
    overlay.hidden = true;
    const tab = activeTab();
    if (tab?.kind === "terminal") tab.term.focus();
    else win.focus({ preventScroll: true });
  }

  function choose(i) {
    const it = P.items[i];
    if (!it) return;
    if (P.mode === "theme") { P.themeBefore = null; applyTheme(it.id); closePalette(false); return; }
    if (it.path) { closePalette(false); openFile(it.path, 1); return; }
    if (it.keep) { it.run(); return; }
    closePalette(false);
    it.run();
  }

  palInput.addEventListener("input", () => { P.sel = 0; renderPalette(); });
  palInput.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!P.items.length) return;
      P.sel = (P.sel + (e.key === "ArrowDown" ? 1 : -1) + P.items.length) % P.items.length;
      renderPalette();
    } else if (e.key === "Enter") { e.preventDefault(); choose(P.sel); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closePalette(); }
  });
  palList.addEventListener("mousemove", (e) => {
    const row = e.target.closest(".dpal-item");
    if (row && Number(row.dataset.i) !== P.sel) { P.sel = Number(row.dataset.i); renderPalette(); }
  });
  palList.addEventListener("click", (e) => { const row = e.target.closest(".dpal-item"); if (row) choose(Number(row.dataset.i)); });
  overlay.addEventListener("mousedown", (e) => { if (e.target === overlay) { e.preventDefault(); closePalette(); } });

  function runInTerminal(cmd) {
    const tab = terminalTab();
    if (tab.term.busy || tab.term.agentMode) { const t = newTerminal(); t.term.run(cmd); return; }
    tab.term.run(cmd);
    tab.term.focus();
  }

  // ── Themes ────────────────────────────────────────────────────────────

  function applyTheme(id, remember = true) {
    const t = THEMES[id];
    if (!t) return;
    const p = t.palette, u = t.ui, s = t.syntax, x = t.terminal;
    const vars = {
      "--w-bg": p.bg, "--w-bg-dark": u.bg_dark, "--w-surface": u.bg_surface, "--w-hl": u.bg_highlight, "--w-border": u.border,
      "--w-fg": p.fg, "--w-muted": u.fg_muted, "--w-comment": u.fg_comment, "--w-accent": p.accent, "--w-sel": u.selection,
      "--w-red": p.red, "--w-orange": p.orange, "--w-yellow": p.yellow, "--w-green": p.green, "--w-cyan": p.cyan, "--w-blue": p.blue, "--w-magenta": p.magenta,
      "--w-add": u.git_added || p.green, "--w-mod": u.git_modified || p.yellow, "--w-del": u.git_deleted || p.red,
      "--s-keyword": s.keyword, "--s-function": s.function, "--s-type": s.type, "--s-string": s.string, "--s-number": s.number, "--s-comment": s.comment,
      "--s-operator": s.operator, "--s-variable": s.variable, "--s-delimiter": s.delimiter, "--s-constant": s.constant, "--s-attribute": s.attribute, "--s-tag": s.tag,
      "--t-red": x.red, "--t-green": x.green, "--t-yellow": x.yellow, "--t-blue": x.blue, "--t-magenta": x.magenta, "--t-cyan": x.cyan, "--t-white": x.white, "--t-dim": x.bright_black,
    };
    for (const [k, v] of Object.entries(vars)) if (v) win.style.setProperty(k, v);
    win.dataset.variant = t.variant;
    if (remember) {
      S.theme = id;
      try { localStorage.setItem("impulse-demo-theme", id); } catch { /* storage may be unavailable */ }
    }
    $$(".theme-chip", document).forEach((c) => c.setAttribute("aria-checked", String(c.dataset.theme === id)));
  }

  function buildThemeStrip() {
    const strip = document.querySelector("[data-theme-strip]");
    if (!strip) return;
    const ids = Object.keys(THEMES).sort((a, b) => (a === "tokyo-night" ? -1 : b === "tokyo-night" ? 1 : THEMES[a].name.localeCompare(THEMES[b].name)));
    strip.innerHTML = ids.map((id) => {
      const t = THEMES[id];
      return `<button type="button" class="theme-chip" role="radio" aria-checked="false" data-theme="${id}" style="background:${t.palette.bg};color:${t.palette.fg}"><span class="sw">${["accent", "blue", "magenta", "green", "red"].map((k) => `<i style="background:${t.palette[k]}"></i>`).join("")}</span>${esc(t.name)}</button>`;
    }).join("");
    strip.addEventListener("click", (e) => {
      const chip = e.target.closest(".theme-chip");
      if (!chip) return;
      tookOver();
      applyTheme(chip.dataset.theme);
    });
  }

  // ── Window keys and buttons ───────────────────────────────────────────

  function toggleSidebar() { win.classList.toggle("no-sidebar"); }

  win.addEventListener("keydown", (e) => {
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (mod && e.shiftKey && k === "p") { e.preventDefault(); e.stopPropagation(); overlay.hidden ? openPalette(">") : closePalette(); }
    else if (e.metaKey && !e.shiftKey && k === "p") { e.preventDefault(); openPalette(""); }
    else if (mod && !e.shiftKey && k === "b") { e.preventDefault(); toggleSidebar(); }
    else if (mod && e.shiftKey && k === "g") { e.preventDefault(); openReview(); }
    else if (mod && e.shiftKey && k === "u") { e.preventDefault(); if (S.agent) activateTab(S.agent.term.tab.id); }
    else if (e.key === "Escape" && !overlay.hidden) { e.preventDefault(); closePalette(); }
  });

  document.addEventListener("click", (e) => {
    const act = e.target.closest("[data-act]");
    if (act) {
      tookOver();
      const a = act.dataset.act;
      if (a === "sidebar") toggleSidebar();
      else if (a === "new-tab") newTerminal();
      else if (a === "palette") { if (!win.contains(act)) scrollDemoIntoView(); openPalette(">"); }
      else if (a === "review") { if (!win.contains(act)) scrollDemoIntoView(); openReview(); }
      else if (a === "task") toast({ title: "fix-elevation is a task", sub: "A branch in its own worktree, open as its own workspace. The demo stays in trailhead.", ms: 3600 });
      return;
    }
    const tryBtn = e.target.closest("[data-try]");
    if (tryBtn) {
      tookOver();
      scrollDemoIntoView();
      runInTerminal(tryBtn.dataset.try);
    }
  });

  function scrollDemoIntoView() {
    const r = win.getBoundingClientRect();
    if (r.top < 60 || r.bottom > innerHeight) win.scrollIntoView({ block: "center", behavior: reduced ? "auto" : "smooth" });
  }

  // ── Autoplay ──────────────────────────────────────────────────────────

  let typing = null;
  function tookOver() {
    if (S.tookOver) return;
    S.tookOver = true;
    if (typing) { typing.term.setInput(""); typing = null; }
  }
  win.addEventListener("pointerdown", () => tookOver(), true);
  win.addEventListener("keydown", () => tookOver(), true);

  async function typeAndRun(term, cmd) {
    typing = { term };
    for (let i = 1; i <= cmd.length; i++) {
      if (S.tookOver) return false;
      term.setInput(cmd.slice(0, i));
      await sleep(45 + Math.random() * 70);
    }
    await sleep(380);
    if (S.tookOver) return false;
    typing = null;
    term.setInput("");
    await term.run(cmd);
    return !S.tookOver;
  }

  async function autoplay(term) {
    await sleep(600);
    for (const cmd of ["git status -sb", "npm test"]) {
      if (!(await typeAndRun(term, cmd))) return;
      await sleep(900);
    }
    if (S.tookOver) return;
    term.coach = true;
    term.updatePlaceholder();
    const ed = $(".ib-editor", term.pane);
    ed.classList.add("coach");
    toast({ title: "Your turn", sub: "Type a command, try claude to fix that test, or press ⇧⌘P.", ms: 6000 });
  }

  // ── Start ─────────────────────────────────────────────────────────────

  buildThemeStrip();
  let saved = null;
  try { saved = localStorage.getItem("impulse-demo-theme"); } catch { /* ignore */ }
  applyTheme(saved && THEMES[saved] ? saved : "tokyo-night");

  const main = newTerminal();
  openFile("src/lib/cache.ts", 5, false);
  activateTab(main.id, false);
  main.term.run("git log --oneline --graph", { instant: true });
  renderTree();
  updateChrome();

  if ("IntersectionObserver" in window && !reduced) {
    const io = new IntersectionObserver(([en]) => {
      if (en.isIntersecting) { io.disconnect(); if (!S.tookOver) autoplay(main.term); }
    }, { threshold: 0.45 });
    io.observe(win);
  } else {
    (async () => {
      await main.term.run("git status -sb", { instant: true });
      await main.term.run("npm test", { instant: true });
      main.term.coach = true;
      main.term.updatePlaceholder();
    })();
  }
})();
