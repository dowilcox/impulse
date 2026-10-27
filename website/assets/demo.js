// Impulse homepage: the interactive demo window.
//
// A scripted imitation of an Impulse window on the trailhead example project
// (the one the docs screenshots use, see demo-files.js), drawn the way the app
// draws itself: the chrome's colors are derived from the theme as the app's
// ChromePalette derives them, the editor tokenizes like Monaco's TypeScript,
// JSON and Markdown tokenizers and colors tokens with the app's Monaco theme
// (themes.js, written by scripts/build-site.py from the app's theme files),
// and the icons are the app's own Lucide and Material icons (demo-icons.js).
// It has terminals with command blocks, an editor, Review, the command
// palette, tasks (worktree workspaces, New Task, overlap checks, Finish Task)
// and simulated coding agents. Nothing here runs a real shell or calls any
// service.
(() => {
  "use strict";
  const win = document.querySelector("[data-demo]");
  if (!win) return;

  const FILES = window.DEMO_FILES || {};
  const THEMES = window.IMPULSE_THEMES || {};
  const ICONS = window.IMPULSE_ICONS || { lucide: {}, material: {}, files: {}, folders: {} };
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = (sel, el = win) => el.querySelector(sel);
  const $$ = (sel, el = win) => [...el.querySelectorAll(sel)];
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const h = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };
  const sleep = (ms) => new Promise((r) => setTimeout(r, reduced ? Math.min(ms, 20) : ms));
  const CANCEL = Symbol("cancel");

  const HOME = "/Users/you";
  const SHELL = "zsh";
  const REPO = "trailhead";
  const AUTHORS = { maya: "Maya Chen", sam: "Sam Ortiz", priya: "Priya Raman" };
  // Files the main checkout's two unpushed commits changed (they count as its
  // changes when Impulse looks for workspaces that change the same files).
  const UNPUSHED = ["src/lib/cache.ts", "src/forecast.ts", "test/cache.test.ts"];

  // ── Icons: the app's Lucide icons, and Material icons for files ───────

  // Name icons literally (icon("x")): the build inlines only the ones it finds.
  function icon(name, size = 14, stroke = 2) {
    return `<svg class="ico" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS.lucide[name] || ""}</svg>`;
  }
  // The Finish tab's icon is the SF Symbol flag.checkered; this draws one like it.
  const FLAG = '<svg class="ico" width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 1.5v13" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M3.7 2.2h9.6v6.6H3.7z" fill="none" stroke="currentColor" stroke-width="1.2"/><path d="M3.7 2.2h3.2v2.2H3.7zM10.1 2.2h3.2v2.2h-3.2zM6.9 4.4h3.2v2.2H6.9zM3.7 6.6h3.2v2.2H3.7zM10.1 6.6h3.2v2.2h-3.2z" fill="currentColor"/></svg>';
  const material = (svg) => `<span class="fico" aria-hidden="true">${ICONS.material[svg] || ""}</span>`;
  const fileIcon = (name) => material(ICONS.files[name] || ICONS.files[""]);
  const folderIcon = (name, open) => material((ICONS.folders[name] || ICONS.folders[""] || [])[open ? 1 : 0]);
  const ring = (size = 10) => `<span class="ring" style="width:${size}px;height:${size}px" aria-hidden="true"></span>`;
  function agentGlyph(state, size = 13) {
    if (state === "working") return ring(size - 1);
    const badge = state === "waiting" ? '<span class="badge-dot"></span>' : state === "done" ? `<span class="badge-check">${icon("check", Math.round(size * 0.6), 3.5)}</span>` : "";
    return `<span class="glyph">${icon("bot", size)}${badge}</span>`;
  }
  const stat = (add, del) => `<span class="stat"><span class="add">+${add}</span><span class="del">−${del}</span></span>`;
  const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

  // A label cut in the middle to fit, as the app truncates branch names ("feat…cache").
  const measure = document.createElement("canvas").getContext("2d");
  function fitMiddle(text, px, font) {
    measure.font = font;
    if (measure.measureText(text).width <= px) return text;
    let keep = text.length - 1;
    while (keep > 2) {
      const head = Math.ceil(keep / 2), tail = Math.floor(keep / 2);
      const cut = text.slice(0, head) + "…" + text.slice(text.length - tail);
      if (measure.measureText(cut).width <= px) return cut;
      keep--;
    }
    return text.slice(0, 1) + "…";
  }

  // ── Themes: the colors the app derives from a theme ──────────────────

  const rgb = (c) => { const x = c.replace("#", ""); return [0, 2, 4].map((i) => parseInt(x.slice(i, i + 2), 16)); };
  // ChromePalette.mix: a linear blend in sRGB.
  const mix = (a, b, t) => "#" + rgb(a).map((v, i) => Math.round(v + (rgb(b)[i] - v) * t).toString(16).padStart(2, "0")).join("");
  const alpha = (c, a) => `rgba(${rgb(c).join(", ")}, ${a})`;
  function readableOn(c) {
    const lin = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    const [r, g, b] = rgb(c);
    return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) > 0.4 ? "#141419" : "#ffffff";
  }
  // Rules from Monaco's own vs / vs-dark themes that win over the app's for
  // tokens the demo's tokenizers make (the app's themes inherit them).
  const BASE_RULES = {
    dark: { "string.key.json": "#9CDCFE", "string.value.json": "#CE9178", "keyword.json": "#CE9178" },
    light: { "string.key.json": "#A31515", "string.value.json": "#0451A5", "keyword.json": "#0451A5" },
  };
  let themeRules = new Map();
  let themeColors = {};
  const tokenIds = new Map();

  // Monaco's theme matching: the most specific rule wins (a.b.c, then a.b, then a).
  function tokenStyle(token) {
    const parts = token.split(".");
    for (let n = parts.length; n > 0; n--) {
      const rule = themeRules.get(parts.slice(0, n).join("."));
      if (rule) return rule;
    }
    return [themeColors.fg];
  }
  function tokenVar(token) {
    let id = tokenIds.get(token);
    if (id == null) {
      id = tokenIds.size;
      tokenIds.set(token, id);
      win.style.setProperty(`--tk${id}`, tokenStyle(token)[0]);
    }
    const style = tokenStyle(token)[1];
    return `color:var(--tk${id})${style === "italic" ? ";font-style:italic" : style === "bold" ? ";font-weight:700" : ""}`;
  }

  function applyTheme(id, remember = true) {
    const t = THEMES[id];
    if (!t) return;
    const c = t.colors, x = t.terminal || {}, light = t.variant === "light";
    themeColors = c;
    themeRules = new Map(Object.entries(BASE_RULES[light ? "light" : "dark"]).map(([k, v]) => [k, [v]]));
    for (const [token, fg, style] of t.monaco.rules) themeRules.set(token, [fg, style]);
    const vars = {
      "--bg": c.bg, "--fg": c.fg, "--text2": c.fg_muted, "--text3": c.fg_comment, "--accent": c.accent,
      "--chrome": mix(c.bg, "#000000", light ? 0.035 : 0.22), "--raised": mix(c.bg, c.fg, light ? 0.06 : 0.07),
      "--overlay": mix(c.bg, c.fg, light ? 0.02 : 0.05), "--hairline": mix(c.bg, c.fg, 0.12), "--hairline-strong": mix(c.bg, c.fg, 0.2),
      "--hover": alpha(c.fg, light ? 0.06 : 0.07), "--pressed": alpha(c.fg, light ? 0.1 : 0.12),
      "--selection": alpha(c.accent, light ? 0.14 : 0.18), "--selection-strong": alpha(c.accent, light ? 0.24 : 0.3), "--on-accent": readableOn(c.accent),
      "--t-bg-dark": c.bg_dark, "--t-border": c.border, "--t-hl": c.bg_highlight, "--t-sel": c.selection, "--cursor": c.cursor,
      "--red": c.red, "--orange": c.orange, "--yellow": c.yellow, "--green": c.green, "--cyan": c.cyan, "--blue": c.blue, "--magenta": c.magenta,
      "--git-add": c.git_added, "--git-mod": c.git_modified, "--git-del": c.git_deleted, "--git-ignored": c.git_ignored || c.fg_muted,
      "--br1": light ? "#0431FA" : "#FFD700", "--br2": light ? "#319331" : "#DA70D6", "--br3": light ? "#7B3814" : "#179FFF",
      "--word": light ? "rgba(87, 87, 87, .25)" : "rgba(87, 87, 87, .72)",
      "--ansi-red": x.red || c.red, "--ansi-green": x.green || c.green, "--ansi-yellow": x.yellow || c.yellow, "--ansi-blue": x.blue || c.blue,
      "--ansi-magenta": x.magenta || c.magenta, "--ansi-cyan": x.cyan || c.cyan, "--ansi-white": x.white || c.fg_muted, "--ansi-dim": x.bright_black || c.fg_comment,
    };
    // Review's highlight.js tokens take the theme's syntax colors (SyntaxHighlighter).
    for (const k of ["keyword", "type", "function", "string", "number", "constant", "comment", "variable", "attribute", "tag", "regexp", "operator", "delimiter", "link"]) vars[`--sx-${k}`] = c[`syntax_${k}`];
    for (const [k, v] of Object.entries(vars)) win.style.setProperty(k, v);
    for (const [token, tid] of tokenIds) win.style.setProperty(`--tk${tid}`, tokenStyle(token)[0]);
    win.dataset.variant = t.variant;
    if (remember) {
      S.theme = id;
      try { localStorage.setItem("impulse-demo-theme", id); } catch { /* storage may be unavailable */ }
    }
    $$(".theme-chip", document).forEach((chip) => chip.setAttribute("aria-checked", String(chip.dataset.theme === id)));
    if (S.ready) refreshAll();
  }

  // ── State: workspaces, each a checkout of trailhead ───────────────────

  const BASE = Object.fromEntries(Object.entries(FILES).filter(([, f]) => !f.ignored).map(([p, f]) => [p, f.base]));
  const pick = (key) => Object.fromEntries(Object.entries(FILES).filter(([, f]) => f[key] != null).map(([p, f]) => [p, f[key]]));

  const S = {
    workspaces: [],
    ws: null,
    tabs: [],
    nextId: 1,
    history: [],
    comments: [],
    agents: [],
    theme: "nord",
    tookOver: false,
    ready: false,
    open: new Set(["src", "src/lib"]),
    overlapsSeen: new Set(),
    landing: null,
    lastWord: null,
  };

  // Commits as `git log --oneline --graph` shows them on feature/forecast-cache.
  const MAIN_LOG = [
    ["9da0f3e", "", "Document the API"],
    ["dfde538", "", "Handle missing elevation data", "tag: v0.2.0"],
    ["e72cf96", "", "Project actions for dev server and tests"],
    ["793da2d", "", "Forecast endpoint"],
    ["334682c", "", "Unit conversions with tests", "tag: v0.1.0"],
    ["70b7c90", "", "Add trail list endpoint"],
  ];
  const FEATURE_LOG = [["290dcdb", "|", "Cache forecasts per trail"], ["6b31fc3", "|", "TTL cache for forecast lookups"]];

  function makeWorkspace(o) {
    return {
      id: o.name, name: o.name, branch: o.branch, main: !!o.main,
      root: o.main ? `~/Code/${REPO}` : `~/Code/${REPO}.worktrees/${o.folder || o.name}`,
      base: o.base || null, ahead: o.ahead || 0, slot: o.slot || 0,
      head: { ...BASE, ...(o.head || {}) }, index: { ...(o.index || {}) }, work: { ...(o.work || {}) },
      commits: o.commits || [...MAIN_LOG], active: null, attention: 0, fixed: false, turn: null, fresh: !!o.fresh,
    };
  }

  const wsById = (id) => S.workspaces.find((w) => w.id === id);
  const curWs = () => wsById(S.ws);
  const current = (ws, path) => (path in ws.work ? ws.work[path] : path in ws.index ? ws.index[path] : ws.head[path]);
  const indexText = (ws, path) => (path in ws.index ? ws.index[path] : ws.head[path]);
  const paths = (ws) => [...new Set([...Object.keys(FILES), ...Object.keys(ws.work), ...Object.keys(ws.index)])];
  const isIgnored = (path) => !!FILES[path]?.ignored;

  // ── Diffs ─────────────────────────────────────────────────────────────

  const splitLines = (text) => (text == null || text === "" ? [] : text.replace(/\n$/, "").split("\n"));
  function diffLines(a, b) {
    const A = splitLines(a), B = splitLines(b);
    const n = A.length, m = B.length;
    const L = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = A[i] === B[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
    const ops = [];
    let i = 0, j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && A[i] === B[j]) { ops.push({ t: " ", a: i + 1, b: j + 1, s: A[i] }); i++; j++; }
      else if (i < n && (j >= m || L[i + 1][j] >= L[i][j + 1])) { ops.push({ t: "-", a: i + 1, s: A[i] }); i++; }
      else { ops.push({ t: "+", b: j + 1, s: B[j] }); j++; }
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

  // Runs of changed lines: what the editor's change marks and the conflict check use.
  function changeRuns(ops) {
    const runs = [];
    let run = null, aPos = 0, bPos = 0;
    for (const op of ops) {
      if (op.t === " ") { aPos = op.a; bPos = op.b; run = null; continue; }
      if (!run) { run = { aStart: aPos, aEnd: aPos, bStart: bPos, added: [], removed: 0 }; runs.push(run); }
      if (op.t === "-") { run.removed++; run.aEnd = op.a; aPos = op.a; } else { run.added.push(op.b); bPos = op.b; }
    }
    return runs;
  }

  // What a scope of Review (and git status, the tree, the counts) compares.
  function diffSet(ws, scope = "uncommitted") {
    const list = [];
    for (const path of paths(ws)) {
      if (isIgnored(path)) continue;
      let before, after;
      if (scope === "turn") { if (!ws.turn || !(path in ws.turn.after)) continue; before = ws.turn.before[path]; after = ws.turn.after[path]; }
      else if (scope === "base") { before = BASE[path]; after = current(ws, path); }
      else { before = ws.head[path]; after = current(ws, path); }
      if (before === after) continue;
      const ops = diffLines(before, after);
      const status = before == null ? (path in ws.index && scope === "uncommitted" ? "A" : "U") : after == null ? "D" : "M";
      list.push({ path, ops, before, after, add: ops.filter((o) => o.t === "+").length, del: ops.filter((o) => o.t === "-").length, status });
    }
    return list;
  }
  const changes = (ws = curWs()) => diffSet(ws);
  function totals(ws = curWs()) {
    const c = changes(ws);
    return { files: c.length, add: c.reduce((s, f) => s + f.add, 0), del: c.reduce((s, f) => s + f.del, 0) };
  }
  const staged = (ws, path) => path in ws.index && ws.index[path] !== ws.head[path];

  // ── Tokenizers: Monaco's Monarch definitions, run the way Monaco runs them ──
  //
  // A small Monarch interpreter: rules are [regex, action, next] or
  // { include }; an action is a token, one token per regex group, or a
  // function of the matched text. Tokens get the language's postfix
  // ("keyword.ts") and are colored by tokenStyle, as Monaco colors them.

  function monarch(def) {
    const expand = (name) => def.states[name].flatMap((rule) => (rule.include ? expand(rule.include) : [{ re: new RegExp(rule[0].source, "y"), action: rule[1], next: rule[2] }]));
    return { ...def, rules: Object.fromEntries(Object.keys(def.states).map((name) => [name, expand(name)])) };
  }

  const TS_KEYWORDS = new Set("abstract any as asserts bigint boolean break case catch class continue const constructor debugger declare default delete do else enum export extends false finally for from function get if implements import in infer instanceof interface is keyof let module namespace never new null number object out package private protected public override readonly require global return satisfies set static string super switch symbol this throw true try type typeof undefined unique unknown var void while with yield async await of".split(" "));
  const TS_OPERATORS = new Set("<= >= == != === !== => + - ** * / % ++ -- << </ >> >>> & | ^ ! ~ && || ?? ? : = += -= *= **= /= %= <<= >>= >>>= &= |= ^= @".split(" "));
  const BRACKETS = { "(": "delimiter.parenthesis", ")": "delimiter.parenthesis", "[": "delimiter.square", "]": "delimiter.square", "<": "delimiter.angle", ">": "delimiter.angle" };
  const ESCAPES = /\\(?:[abfnrtv\\"']|x[0-9A-Fa-f]{1,4}|u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8})/;
  const tsDef = (postfix) => monarch({
    postfix, defaultToken: "invalid",
    states: {
      root: [[/[{}]/, "delimiter.bracket"], { include: "common" }],
      common: [
        [/#?[a-z_$][\w$]*/, (m) => (TS_KEYWORDS.has(m) ? "keyword" : "identifier")],
        [/[A-Z][\w$]*/, "type.identifier"],
        { include: "whitespace" },
        [/\/(?=([^\\/]|\\.)+\/([dgimsuy]*)(\s*)(\.|;|,|\)|\]|\}|$))/, "regexp", "@regexp"],
        [/[()[\]]/, (m) => BRACKETS[m]],
        [/[<>](?![=><!~?:&|+\-*/^%]+)/, (m) => BRACKETS[m]],
        [/!(?=([^=]|$))/, "delimiter"],
        [/[=><!~?:&|+\-*/^%]+/, (m) => (TS_OPERATORS.has(m) ? "delimiter" : "")],
        [/\d+(_+\d+)*[eE]([-+]?\d+(_+\d+)*)?/, "number.float"],
        [/\d+(_+\d+)*\.\d+(_+\d+)*([eE][-+]?\d+(_+\d+)*)?/, "number.float"],
        [/0[xX][0-9a-fA-F]+(_+[0-9a-fA-F]+)*n?/, "number.hex"],
        [/0[oO]?[0-7]+(_+[0-7]+)*n?/, "number.octal"],
        [/0[bB][01]+(_+[01]+)*n?/, "number.binary"],
        [/\d+(_+\d+)*n?/, "number"],
        [/[;,.]/, "delimiter"],
        [/"([^"\\]|\\.)*$/, "string.invalid"],
        [/'([^'\\]|\\.)*$/, "string.invalid"],
        [/"/, "string", "@string_double"],
        [/'/, "string", "@string_single"],
        [/`/, "string", "@string_backtick"],
      ],
      whitespace: [[/[ \t\r\n]+/, ""], [/\/\*\*(?!\/)/, "comment.doc", "@jsdoc"], [/\/\*/, "comment", "@comment"], [/\/\/.*$/, "comment"]],
      comment: [[/[^/*]+/, "comment"], [/\*\//, "comment", "@pop"], [/[/*]/, "comment"]],
      jsdoc: [[/[^/*]+/, "comment.doc"], [/\*\//, "comment.doc", "@pop"], [/[/*]/, "comment.doc"]],
      regexp: [[/\\./, "regexp.escape"], [/[()[\]{}$^|\-*+?.]/, "regexp.escape.control"], [/\/[dgimsuy]*/, "regexp", "@pop"], [/[^\\/()[\]{}$^|\-*+?.]+/, "regexp"]],
      string_double: [[/[^\\"]+/, "string"], [ESCAPES, "string.escape"], [/\\./, "string.escape.invalid"], [/"/, "string", "@pop"]],
      string_single: [[/[^\\']+/, "string"], [ESCAPES, "string.escape"], [/\\./, "string.escape.invalid"], [/'/, "string", "@pop"]],
      string_backtick: [[/\$\{/, "delimiter.bracket", "@bracketCounting"], [/[^\\`$]+/, "string"], [ESCAPES, "string.escape"], [/\\./, "string.escape.invalid"], [/`/, "string", "@pop"]],
      bracketCounting: [[/\{/, "delimiter.bracket", "@bracketCounting"], [/\}/, "delimiter.bracket", "@pop"], { include: "common" }],
    },
  });
  // The Monaco-based app (through 0.36) registered its own JSON tokenizer.
  const JSON_LANG = monarch({
    postfix: ".json", defaultToken: "",
    states: {
      root: [
        [/\/\/.*$/, "comment"], [/\/\*/, "comment", "@comment"], [/\s+/, ""],
        [/"(?:[^"\\]|\\.)*"(?=\s*:)/, "string.key"], [/"/, "string", "@string"],
        [/-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/, "number"], [/\b(?:true|false|null)\b/, "keyword.constant"],
        [/[{}[\]]/, "delimiter.bracket"], [/[,:]/, "delimiter"],
      ],
      string: [[/\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4})/, "string.escape"], [/\\./, "string.escape.invalid"], [/[^"\\]+/, "string"], [/"/, "string", "@pop"]],
      comment: [[/[^/*]+/, "comment"], [/\*\//, "comment", "@pop"], [/./, "comment"]],
    },
  });
  const MD_ESC = /\\[\\`*_[\]{}()#+\-.!]/;
  const MD_LANG = monarch({
    postfix: ".md", defaultToken: "",
    states: {
      root: [
        [/^(\s{0,3})(#+)((?:[^\\#]|\\.)+)((?:#+)?)/, ["white", "keyword", "keyword", "keyword"]],
        [/^\s*(=+|-+)\s*$/, "keyword"], [/^\s*((\*[ ]?)+)\s*$/, "meta.separator"], [/^\s*>+/, "comment"],
        [/^\s*([*\-+:]|\d+\.)\s/, "keyword"], [/^(\t|[ ]{4})[^ ].*$/, "string"],
        [/^\s*~~~\s*((?:\w|[/\-#])+)?\s*$/, "string", "@codeblock"],
        [/^\s*```\s*((?:\w|[/\-#])+).*$/, "string", "@codeblockgh"],
        [/^\s*```\s*$/, "string", "@codeblock"],
        { include: "linecontent" },
      ],
      codeblock: [[/^\s*~~~\s*$/, "string", "@pop"], [/^\s*```\s*$/, "string", "@pop"], [/.*$/, "variable.source"]],
      // Fenced code with a language (```sh) is tokenized as that language; for a shell, comments stand out.
      codeblockgh: [[/```\s*$/, "string", "@pop"], [/#.*$/, "comment.sh"], [/[^`#]+/, ""]],
      linecontent: [
        [/&\w+;/, "string.escape"], [MD_ESC, "escape"],
        [/\b__([^\\_]|\\.|_(?!_))+__\b/, "strong"], [/\*\*([^\\*]|\\.|\*(?!\*))+\*\*/, "strong"],
        [/\b_[^_]+_\b/, "emphasis"], [/\*([^\\*]|\\.)+\*/, "emphasis"], [/`([^\\`]|\\.)+`/, "variable"],
        [/\{+[^}]+\}+/, "string.target"],
        [/(!?\[)((?:[^\]\\]|\\.)*)(\]\([^)]+\))/, ["string.link", "", "string.link"]],
        [/(!?\[)((?:[^\]\\]|\\.)*)(\])/, "string.link"],
      ],
    },
  });
  const LANGS = { ts: { def: tsDef(".ts"), name: "typescript" }, mjs: { def: tsDef(".js"), name: "javascript" }, js: { def: tsDef(".js"), name: "javascript" }, json: { def: JSON_LANG, name: "json" }, md: { def: MD_LANG, name: "markdown" } };
  const langOf = (path) => LANGS[path.split(".").pop()] || { def: null, name: "plaintext" };

  function tokenizeLine(lang, line, stack) {
    const out = [];
    const push = (text, kind) => {
      if (!text) return;
      const k = kind && kind !== "white" ? kind + (kind.includes(".sh") ? "" : lang.postfix) : "";
      const last = out[out.length - 1];
      if (last && last.k === k) last.t += text; else out.push({ t: text, k });
    };
    let pos = 0;
    while (pos < line.length) {
      let matched = false;
      for (const rule of lang.rules[stack[stack.length - 1]]) {
        rule.re.lastIndex = pos;
        const m = rule.re.exec(line);
        if (!m || !m[0]) continue;
        const action = typeof rule.action === "function" ? rule.action(m[0]) : rule.action;
        if (Array.isArray(action)) action.forEach((kind, i) => push(m[i + 1], kind));
        else push(m[0], action);
        pos += m[0].length;
        if (rule.next === "@pop") { if (stack.length > 1) stack.pop(); }
        else if (rule.next) stack.push(rule.next.slice(1));
        matched = true;
        break;
      }
      if (!matched) { push(line[pos], lang.defaultToken); pos++; }
    }
    return out;
  }

  // Bracket pair colorization: (), [] and {} outside strings and comments take
  // a color by nesting depth; the pairs are kept for the bracket pair guides.
  function colorBrackets(lines) {
    const stack = [], pairs = [];
    const colored = lines.map((tokens, li) => {
      const out = [];
      let col = 0;
      for (const tok of tokens) {
        if (/(^|\.)(comment|string|regexp)(\.|$)/.test(tok.k)) { out.push(tok); col += tok.t.length; continue; }
        let buf = "";
        for (const ch of tok.t) {
          const o = "([{".indexOf(ch), c = ")]}".indexOf(ch);
          if (o >= 0 || c >= 0) {
            if (buf) out.push({ t: buf, k: tok.k });
            buf = "";
            if (o >= 0) { stack.push({ type: o, line: li, col, level: stack.length }); out.push({ t: ch, k: tok.k, br: stack.length - 1 }); }
            else if (stack.length && stack[stack.length - 1].type === c) { const open = stack.pop(); out.push({ t: ch, k: tok.k, br: open.level }); pairs.push({ ...open, closeLine: li, closeCol: col }); }
            else out.push({ t: ch, k: tok.k, br: -1 });
          } else buf += ch;
          col++;
        }
        if (buf) out.push({ t: buf, k: tok.k });
      }
      return out;
    });
    return { lines: colored, pairs };
  }

  const tokenCache = new Map();
  function tokenize(path, text) {
    const key = path + "\0" + text;
    if (tokenCache.has(key)) return tokenCache.get(key);
    const lang = langOf(path).def;
    const lines = splitLines(text);
    const stack = ["root"];
    const tokens = lines.map((line) => (lang ? tokenizeLine(lang, line, stack) : [{ t: line, k: "" }]));
    const result = lang && lang !== MD_LANG ? colorBrackets(tokens) : { lines: tokens, pairs: [] };
    if (tokenCache.size > 60) tokenCache.clear();
    tokenCache.set(key, result);
    return result;
  }
  const bracketColor = (level) => (level < 0 ? "rgba(255, 18, 18, .8)" : `var(--br${(level % 3) + 1})`);
  function tokensHTML(tokens, word) {
    return tokens.map((tok) => {
      const style = tok.br != null ? `color:${bracketColor(tok.br)}` : tok.k ? tokenVar(tok.k) : "";
      const cls = word && tok.t === word && !tok.br && /^(identifier|keyword|type\.identifier)/.test(tok.k) ? ' class="wh"' : "";
      return style || cls ? `<span${cls}${style ? ` style="${style}"` : ""}>${esc(tok.t)}</span>` : esc(tok.t);
    }).join("");
  }

  // ── Shell input highlighting and completion ───────────────────────────

  const KNOWN = new Set(["git", "npm", "node", "ls", "cd", "cat", "pwd", "echo", "clear", "help", "impulse", "claude", "codex", "gemini", "aider", "opencode", "whoami", "date", "uname", "sw_vers", "history", "exit", "vim", "nvim", "nano", "code", "open", "sudo", "rm", "brew", "theme", "top", "htop", "ssh", "man", "less", "which", "hostname", "true", "false", "mkdir", "touch", "npx"]);
  const SUGGEST = ["git status -sb", "git log --oneline --graph", "git diff", "git diff --stat", "git branch", "git add src/server.ts", "git commit -m \"Cache stats on /health\"", "npm test", "npm run lint", "npm run dev", "claude", "codex", "ls -la", "ls src", "cat src/lib/cache.ts", "cat package.json", "impulse open src/lib/cache.ts", "impulse review", "impulse tasks", "impulse notify \"Build done\"", "help", "clear", "theme dracula", "cd src", "echo $SHELL"];

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

  // ── Tabs (each belongs to a workspace) ────────────────────────────────

  const tabsEl = $("[data-tabs]");
  const panesEl = $("[data-panes]");
  // Review's tab icon is the SF Symbol arrow.triangle.branch; this draws one like it.
  const BRANCH_ARROWS = '<svg class="ico" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 15V8.5L3.5 4M8 8.5 12.5 4"/><path d="M3 7V3.5h3.5M13 7V3.5H9.5"/></svg>';

  const wsTabs = (ws = curWs()) => S.tabs.filter((t) => t.ws === ws.id);
  const activeTab = () => S.tabs.find((t) => t.id === curWs().active);

  function addTab(tab, activate = true) {
    tab.id = S.nextId++;
    S.tabs.push(tab);
    panesEl.appendChild(tab.el);
    tab.el.id = `pane-${tab.id}`;
    if (activate) activateTab(tab.id);
    else renderTabs();
    return tab;
  }

  function tabTitle(tab) {
    if (tab.kind === "terminal") return SHELL;
    if (tab.kind === "editor") return tab.path.split("/").pop();
    if (tab.kind === "finish") return `Finish ${wsById(tab.ws).name}`;
    return `Review · ${wsById(tab.ws).name}`;
  }

  function tabIcon(tab, selected) {
    const agent = tab.kind === "terminal" && agentOf(tab.term);
    if (agent) return agentGlyph(agent.state, 13);
    if (tab.kind === "terminal") return `<span class="tab-ico term">${icon("terminal", 13)}</span>`;
    if (tab.kind === "editor") return `<span class="tab-ico">${fileIcon(tab.path.split("/").pop())}</span>`;
    if (tab.kind === "finish") return `<span class="tab-ico">${FLAG}</span>`;
    return `<span class="tab-ico"${selected ? ' style="color:var(--fg)"' : ""}>${BRANCH_ARROWS}</span>`;
  }

  function renderTabs() {
    const ws = curWs();
    tabsEl.innerHTML = "";
    for (const tab of wsTabs(ws)) {
      const selected = tab.id === ws.active;
      const el = h(`<div class="tab" role="tab" id="tab-${tab.id}" aria-controls="pane-${tab.id}" aria-selected="${selected}" tabindex="${selected ? 0 : -1}" title="${esc(tabTitle(tab))}">
        ${tabIcon(tab, selected)}<span class="tab-title">${esc(tabTitle(tab))}</span>${tab.attention && !selected ? '<span class="att" title="Needs attention"></span>' : ""}
        <span class="tab-x" role="button" tabindex="-1" aria-label="Close ${esc(tabTitle(tab))}" title="Close Tab">${icon("x", 11, 2.2)}</span></div>`);
      el.addEventListener("mousedown", (e) => { if (e.target.closest(".tab-x")) return; e.preventDefault(); activateTab(tab.id); });
      el.addEventListener("keydown", (e) => {
        const list = wsTabs(), i = list.indexOf(tab);
        if (e.key === "ArrowRight" || e.key === "ArrowLeft") { e.preventDefault(); const next = list[(i + (e.key === "ArrowRight" ? 1 : -1) + list.length) % list.length]; activateTab(next.id); $(`#tab-${next.id}`)?.focus(); }
        else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); activateTab(tab.id); }
        else if (e.key === "Delete" || e.key === "Backspace") { e.preventDefault(); closeTab(tab.id); }
      });
      el.querySelector(".tab-x").addEventListener("click", (e) => { e.stopPropagation(); closeTab(tab.id); });
      tabsEl.appendChild(el);
    }
  }

  // The last pointer was a finger: keep focus (and the on-screen keyboard) where it is.
  let touch = false, keyboardOpened = false;
  addEventListener("pointerdown", (e) => { touch = e.pointerType === "touch"; keyboardOpened = false; }, true);
  addEventListener("keydown", () => (keyboardOpened = true), true);

  function activateTab(id, focus = true) {
    focus = focus && !touch;
    const tab = S.tabs.find((t) => t.id === id);
    if (!tab) return;
    const ws = wsById(tab.ws);
    ws.active = id;
    if (ws.id !== S.ws) { selectWorkspace(ws.id, false); return activateTab(id, focus); }
    for (const t of S.tabs) t.el.classList.toggle("active", t.id === id);
    if (tab.kind === "terminal") {
      const agent = agentOf(tab.term);
      if (agent && agent.state === "done") ws.attention = 0;
      tab.attention = false;
      if (focus) tab.term.focus();
      tab.term.scrollToEnd();
    }
    if (tab.kind === "editor") { renderEditor(tab); if (focus) $(".ed", tab.el).focus({ preventScroll: true }); }
    if (tab.kind === "review" || tab.kind === "finish") { tab.render(); if (focus) win.focus({ preventScroll: true }); }
    renderTabs();
    renderTree();
    updateChrome();
  }

  function closeTab(id) {
    const tab = S.tabs.find((t) => t.id === id);
    if (!tab) return;
    const ws = wsById(tab.ws);
    const list = wsTabs(ws), i = list.indexOf(tab);
    S.tabs.splice(S.tabs.indexOf(tab), 1);
    const agent = tab.term && agentOf(tab.term);
    if (agent) agent.quit();
    tab.term?.cancelRun();
    tab.el.remove();
    const rest = wsTabs(ws);
    if (!rest.length) { newTerminal(ws); return; }
    if (ws.active === id) activateTab(rest[Math.min(i, rest.length - 1)].id);
    else renderTabs();
    toast({ title: `Closed “${tabTitle(tab)}”`, sub: "In Impulse, ⇧⌘T or ⌘Z brings it back.", ms: 2600 });
  }

  function terminalTab(ws = curWs()) {
    const t = activeTab()?.kind === "terminal" && activeTab().ws === ws.id ? activeTab() : wsTabs(ws).find((x) => x.kind === "terminal" && !agentOf(x.term));
    if (t) { if (ws.active !== t.id) activateTab(t.id); return t; }
    return newTerminal(ws);
  }

  // ── Terminal ──────────────────────────────────────────────────────────

  class Terminal {
    constructor(ws) {
      this.ws = ws;
      this.cwd = "";
      this.busy = false;
      this.cancelled = false;
      this.last = null;
      this.lastMs = null;
      this.histIndex = null;
      this.pane = h(`<div class="pane" role="tabpanel">
        <div class="term-grid" aria-live="off"></div>
        <input class="term-keys" type="text" tabindex="-1" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" enterkeyhint="send" aria-label="Type to the agent">
        <div class="belt" hidden></div>
        <div class="ib">
          <div class="ib-chips"></div>
          <div class="ib-editor"><span class="ib-chev">${icon("chevron-right", 13, 2.4)}</span><div class="ib-field"><div class="ib-mirror" aria-hidden="true"></div>
            <input type="text" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" enterkeyhint="go" aria-label="Command input" placeholder="Run a command…"></div></div>
        </div></div>`);
      this.grid = $(".term-grid", this.pane);
      this.ib = $(".ib", this.pane);
      this.belt = $(".belt", this.pane);
      this.input = $(".ib input", this.pane);
      this.keys = $(".term-keys", this.pane);
      this.mirror = $(".ib-mirror", this.pane);
      this.chips = $(".ib-chips", this.pane);
      this.input.addEventListener("input", () => { this.histIndex = null; this.paint(); });
      this.input.addEventListener("keydown", (e) => this.onKey(e));
      this.input.addEventListener("scroll", () => (this.mirror.scrollLeft = this.input.scrollLeft));
      this.keys.addEventListener("input", () => agentOf(this)?.typed(this.keys.value));
      this.keys.addEventListener("keydown", (e) => agentOf(this)?.key(e));
      this.pane.addEventListener("pointerup", (e) => {
        if (e.target.closest("button, .t-link, .blk-tools, .belt") || getSelection().toString()) return;
        if (e.pointerType === "touch" && !e.target.closest(".ib") && !agentOf(this)) return;
        this.focus();
      });
      this.renderChips();
      this.paint();
    }

    get root() { return this.ws.root; }
    longCwd() { return this.ws.root + (this.cwd ? "/" + this.cwd : ""); }
    fullCwd() { return this.longCwd().replace(/^~/, HOME); }
    focus() { (agentOf(this) ? this.keys : this.input).focus({ preventScroll: true }); }
    scrollToEnd() { this.grid.scrollTop = this.grid.scrollHeight; }
    nearEnd() { return this.grid.scrollHeight - this.grid.scrollTop - this.grid.clientHeight < 40; }

    paint() {
      const v = this.input.value;
      const ghost = suggestion(v);
      this.ghost = ghost;
      this.mirror.innerHTML = highlightShell(v) + (ghost ? `<span class="ib-ghost">${esc(ghost)}</span>` : "");
      this.mirror.scrollLeft = this.input.scrollLeft;
    }

    setInput(v) { this.input.value = v; this.paint(); this.input.setSelectionRange(v.length, v.length); }

    // The context chips above the input: shell, folder, branch, changes, last exit.
    renderChips() {
      const t = totals(this.ws);
      const status = this.busy || this.last == null ? "" : this.last === 0
        ? `<span class="chip"><span class="ok">${icon("circle-check", 11, 2.4)}</span>${fmtDuration(this.lastMs)}</span>`
        : `<span class="chip"><span class="bad">${icon("circle-x", 11, 2.4)}</span>exit ${this.last} · ${fmtDuration(this.lastMs)}</span>`;
      this.chips.innerHTML = `<span class="chip">${icon("terminal", 11)}${SHELL}</span><span class="chip">${icon("folder", 11)}${esc(this.longCwd())}</span><span class="chip">${icon("git-branch", 11)}${esc(this.ws.branch)}${icon("chevrons-up-down", 9)}</span>` +
        (t.files ? `<span class="chip">${icon("file-diff", 11)}${t.files} · ${stat(t.add, t.del)}</span>` : "") + status +
        `<button class="ib-hist" type="button" title="Command History (⌃R)" aria-label="Command History">${icon("history", 12)}</button>`;
      this.updatePlaceholder();
    }

    updatePlaceholder() {
      this.input.placeholder = this.busy ? "Running… ⌃C stops it" : this.coach ? "Your turn: try claude" : "Run a command…";
    }

    onKey(e) {
      const v = this.input.value;
      if (e.key === "Enter") {
        e.preventDefault();
        if (this.busy) return;
        this.setInput("");
        this.run(v);
      } else if ((e.key === "Tab" || (e.key === "ArrowRight" && this.input.selectionStart === v.length)) && this.ghost) {
        e.preventDefault();
        this.setInput(v + this.ghost);
      } else if (e.key === "Tab") {
        e.preventDefault();
      } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && S.history.length) {
        e.preventDefault();
        const n = S.history.length;
        if (this.histIndex == null) this.histIndex = n;
        this.histIndex = Math.max(0, Math.min(n, this.histIndex + (e.key === "ArrowUp" ? -1 : 1)));
        const idx = this.histIndex;
        this.setInput(idx >= n ? "" : S.history[idx]);
        this.histIndex = idx;
      } else if (e.ctrlKey && e.key.toLowerCase() === "c") {
        e.preventDefault();
        if (this.busy) this.cancelRun(); else this.setInput("");
      } else if (e.ctrlKey && e.key.toLowerCase() === "l") {
        e.preventDefault();
        if (!this.busy) this.clear();
      }
    }

    cancelRun() { if (this.busy) { this.cancelled = true; this.wake?.(); } }
    clear() { this.grid.querySelectorAll(".blk, .term-welcome").forEach((b) => b.remove()); }

    // A command block: the prompt (folder and branch, then ❯ and the command), its output, and how it ended.
    block(line) {
      const blk = h(`<div class="blk running">
        <div class="blk-head"><span class="cwd">${esc(this.longCwd())}</span><span class="br">${esc(this.ws.branch)}</span><span class="blk-status"><span class="spinner"></span></span></div>
        <div class="blk-cmd"><span class="chev">❯</span> ${highlightShell(line)}</div>
        <div class="blk-out"></div>
        <div class="blk-tools"><button type="button" data-b="copy" title="Copy Output" aria-label="Copy output">${icon("copy", 12)}</button><button type="button" data-b="rerun" title="Run Again" aria-label="Run again">${icon("rotate-ccw", 12)}</button><button type="button" data-b="send" title="Send to Agent" aria-label="Send to agent">${icon("message-square-plus", 12)}</button></div>
      </div>`);
      blk.addEventListener("click", (e) => {
        const b = e.target.closest("[data-b]");
        if (!b) {
          if (!e.target.closest(".t-link") && !getSelection().toString() && !blk.classList.contains("running")) blk.classList.toggle("selected");
          return;
        }
        if (b.dataset.b === "copy") {
          navigator.clipboard?.writeText($(".blk-out", blk).innerText).catch(() => {});
          toast({ title: "Copied the block’s output", ms: 1800, kind: "success" });
        } else if (b.dataset.b === "rerun") {
          if (!this.busy) this.run(line);
        } else if (b.dataset.b === "send") {
          const agent = S.agents.find((a) => a.ws === this.ws) || S.agents[0];
          if (agent) { agent.insert(`Look at the output of \`${line}\`. `); toast({ title: `Sent to ${agent.name}. Review it in its prompt, then press Return.`, action: { label: "Show", run: () => activateTab(agent.term.tab.id) }, ms: 3000 }); }
          else toast({ title: "No agent is running", sub: "Run claude in a terminal first.", ms: 2600 });
        }
      });
      const stick = this.nearEnd();
      this.grid.appendChild(blk);
      if (stick) this.scrollToEnd();
      const out = $(".blk-out", blk);
      return {
        el: blk,
        out,
        print: (html = "", animate = true) => {
          const stick2 = this.nearEnd();
          const row = h(`<div class="line${animate && !reduced ? " in" : ""}"></div>`);
          row.innerHTML = html;
          out.appendChild(row);
          if (stick2) this.scrollToEnd();
          return row;
        },
        finish: (code, ms) => {
          blk.classList.remove("running");
          if (code) blk.classList.add("fail");
          $(".blk-status", blk).textContent = `${code ? `✕ ${code}` : "✓"} · ${fmtDuration(ms)}`;
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
      this.renderChips();
      const t0 = performance.now();
      const io = {
        term: this,
        ws: this.ws,
        blk,
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
      this.lastMs = ms;
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

  // TerminalRenderer.formatBlockDuration: 102ms, 1.2s, 1m 4s.
  function fmtDuration(ms) {
    if (ms == null) return "";
    if (ms < 1000) return `${Math.max(1, Math.round(ms))}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
  }

  function newTerminal(ws = curWs(), activate = true) {
    const term = new Terminal(ws);
    const tab = { kind: "terminal", ws: ws.id, term, el: term.pane };
    term.tab = tab;
    return addTab(tab, activate);
  }

  // ── Commands ──────────────────────────────────────────────────────────

  const DIRS = new Set([""]);
  Object.keys(FILES).forEach((p) => { const parts = p.split("/"); for (let i = 1; i < parts.length; i++) DIRS.add(parts.slice(0, i).join("/")); });

  function resolvePath(term, arg = "") {
    if (arg === "~" || arg === HOME) return null;
    const root = term.root;
    const parts = arg.startsWith(root + "/") ? arg.slice(root.length + 1).split("/") : arg.startsWith("/") || arg.startsWith("~") ? null : [...term.cwd.split("/"), ...arg.split("/")];
    if (!parts) return null;
    const out = [];
    for (const p of parts) {
      if (!p || p === ".") continue;
      if (p === "..") { if (!out.length) return null; out.pop(); } else out.push(p);
    }
    return out.join("/");
  }

  function listDir(dir, ws = curWs()) {
    const prefix = dir ? dir + "/" : "";
    const names = new Map();
    for (const p of paths(ws)) {
      if (!p.startsWith(prefix) || current(ws, p) == null) continue;
      const [first, ...more] = p.slice(prefix.length).split("/");
      names.set(first, more.length ? "dir" : "file");
    }
    return [...names].sort((a, b) => a[0].localeCompare(b[0]));
  }

  const shortSha = () => Math.random().toString(16).slice(2, 9);

  const COMMANDS = {
    help: async ({ print }) => {
      [
        '<span class="t-bold">This is a scripted demo</span> of an Impulse terminal, in the trailhead example project.',
        "",
        '  <span class="t-cyan">git</span> status · log · diff · add · commit · branch · worktree list     <span class="t-cyan">npm</span> test · run lint · run dev',
        '  <span class="t-cyan">ls</span> · <span class="t-cyan">cd</span> · <span class="t-cyan">cat</span> · <span class="t-cyan">pwd</span> · <span class="t-cyan">echo</span> · <span class="t-cyan">clear</span>      <span class="t-cyan">impulse</span> open &lt;file&gt; · review · tasks · notify',
        '  <span class="t-cyan">claude</span> or <span class="t-cyan">codex</span>  a simulated coding agent        <span class="t-cyan">theme</span> &lt;name&gt;  switch the demo’s theme',
        "",
        '<span class="t-dim">Tab or → accepts the gray suggestion, ↑ and ↓ walk history, ⌃C stops a command.</span>',
        '<span class="t-dim">⇧⌘P opens the command palette, ⌘P finds a file, ⌥⌘N starts a task, ⇧⌘G opens Review.</span>',
      ].forEach((l) => print(l, false));
      return 0;
    },

    ls: async ({ print, term, ws }, args) => {
      const long = args.some((a) => /^-\w*l/.test(a));
      const target = args.find((a) => !a.startsWith("-"));
      const dir = target == null ? term.cwd : resolvePath(term, target);
      if (dir == null || !DIRS.has(dir)) {
        if (dir != null && current(ws, dir) != null) { print(esc(target)); return 0; }
        print(`ls: ${esc(target)}: No such file or directory`); return 1;
      }
      const entries = listDir(dir, ws);
      if (!long) { print(entries.map(([n, k]) => (k === "dir" ? `<span class="t-blue t-bold">${n}</span>` : esc(n))).join("   ")); return 0; }
      print(`total ${entries.length * 8}`, false);
      for (const [n, k] of entries) {
        const path = (dir ? dir + "/" : "") + n;
        const size = k === "dir" ? 96 + 32 * listDir(path, ws).length : (current(ws, path) || "").length;
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
          else print(`cd: no such file or directory: ${esc(arg)}`);
          return 1;
        }
        term.prev = term.cwd;
        term.cwd = dir;
      }
      return 0;
    },

    pwd: async ({ print, term }) => { print(esc(term.fullCwd())); return 0; },

    cat: async ({ print, sleep, term, ws }, args) => {
      if (!args.length) { print("cat: give it a file, like src/lib/cache.ts"); return 1; }
      for (const a of args) {
        const p = resolvePath(term, a);
        const text = p != null ? current(ws, p) ?? FILES[p]?.base : null;
        if (text == null) { print(`cat: ${esc(a)}: No such file or directory`); return 1; }
        for (const l of splitLines(text)) print(esc(l), false);
        await sleep(10);
      }
      return 0;
    },

    echo: async ({ print, term }, args) => {
      const vars = { $SHELL: "/bin/zsh", $HOME: HOME, $USER: "you", $EDITOR: "impulse edit", $TERM_PROGRAM: "Impulse", $PWD: term.fullCwd(), $IMPULSE_TASK: term.ws.main ? "" : term.ws.name };
      print(esc(args.map((a) => a.replace(/\$\w+/g, (v) => vars[v] ?? "")).join(" ")));
      return 0;
    },

    whoami: async ({ print }) => { print("you"); return 0; },
    hostname: async ({ print }) => { print("MacBook-Pro.local"); return 0; },
    date: async ({ print }) => { print(new Date().toString().replace(/ GMT.*/, "")); return 0; },
    uname: async ({ print }, args) => { print(args.includes("-a") ? "Darwin MacBook-Pro.local 26.0.0 Darwin Kernel Version 26.0.0 arm64" : "Darwin"); return 0; },
    sw_vers: async ({ print }) => { ["ProductName:		macOS", "ProductVersion:		26.0", "BuildVersion:		26A305"].forEach((l) => print(l, false)); return 0; },
    true: async () => 0,
    false: async () => 1,
    which: async ({ print }, args) => {
      const where = { git: "/usr/bin/git", npm: "/opt/homebrew/bin/npm", node: "/opt/homebrew/bin/node", impulse: "/Applications/Impulse.app/Contents/Resources/bin/impulse", claude: "/Users/you/.local/bin/claude", zsh: "/bin/zsh" };
      for (const a of args) { if (!where[a]) return 1; print(where[a]); }
      return 0;
    },
    history: async ({ print }) => { S.history.forEach((l, i) => print(`${String(i + 1).padStart(5)}  ${esc(l)}`, false)); return 0; },

    exit: async ({ print, sleep, term }) => {
      print('<span class="t-dim">Closing the tab.</span>');
      await sleep(400);
      setTimeout(() => closeTab(term.tab.id), 0);
      return 0;
    },

    git: async (io, args) => {
      const { print, sleep, ws, term } = io;
      const sub = args[0];
      const list = changes(ws);
      if (sub === "status" || sub === "st") {
        const short = args.some((a) => /^-\w*s/.test(a));
        const tracking = ws.main ? `origin/${ws.branch}` : null;
        if (short) {
          print(`<span class="t-green">## ${esc(ws.branch)}</span>${tracking ? `...<span class="t-red">${tracking}</span>${ws.ahead ? ` [ahead <span class="t-green">${ws.ahead}</span>]` : ""}` : ""}`);
          for (const f of list) {
            if (f.status === "U") { print(`<span class="t-red">??</span> ${f.path}`); continue; }
            const st = staged(ws, f.path), dirty = current(ws, f.path) !== indexText(ws, f.path);
            print(`${st ? `<span class="t-green">${f.status === "A" ? "A" : "M"}</span>` : " "}${dirty ? '<span class="t-red">M</span>' : " "} ${f.path}`);
          }
          return 0;
        }
        print(`On branch ${esc(ws.branch)}`);
        if (tracking) print(ws.ahead ? `Your branch is ahead of '${tracking}' by ${plural(ws.ahead, "commit")}.` : `Your branch is up to date with '${tracking}'.`);
        const st = list.filter((f) => staged(ws, f.path)), un = list.filter((f) => f.status !== "U" && current(ws, f.path) !== indexText(ws, f.path)), ut = list.filter((f) => f.status === "U");
        if (st.length) { print(""); print("Changes to be committed:"); st.forEach((f) => print(`        <span class="t-green">${f.status === "A" ? "new file" : "modified"}:   ${f.path}</span>`)); }
        if (un.length) { print(""); print("Changes not staged for commit:"); un.forEach((f) => print(`        <span class="t-red">modified:   ${f.path}</span>`)); }
        if (ut.length) { print(""); print("Untracked files:"); ut.forEach((f) => print(`        <span class="t-red">${f.path}</span>`)); }
        if (!list.length) print("nothing to commit, working tree clean");
        return 0;
      }
      if (sub === "log") {
        const log = ws.main ? [...ws.commits.slice(0, ws.commits.length - MAIN_LOG.length), ...FEATURE_LOG, ...MAIN_LOG] : ws.commits;
        for (let i = 0; i < log.length; i++) {
          const [sha, lane, msg, ref] = log[i];
          const head = i === 0 ? ` (<span class="t-cyan t-bold">HEAD -&gt; </span><span class="t-green t-bold">${esc(ws.branch)}</span>)` : "";
          const tag = ref ? ` (<span class="t-yellow t-bold">${ref}</span>)` : "";
          print(`${lane ? '<span class="t-red">|</span> * ' : "* "}<span class="t-yellow">${sha}</span>${head}${tag} ${esc(msg)}`);
          if (lane && !log[i + 1]?.[1]) print('<span class="t-red">|</span>/');
          if (i % 2) await sleep(25);
        }
        return 0;
      }
      if (sub === "diff") {
        const stat = args.includes("--stat");
        const cached = args.includes("--staged") || args.includes("--cached");
        const files = list.map((f) => {
          const before = cached ? ws.head[f.path] : indexText(ws, f.path), after = cached ? indexText(ws, f.path) : current(ws, f.path);
          const ops = diffLines(before, after);
          return { ...f, ops, add: ops.filter((o) => o.t === "+").length, del: ops.filter((o) => o.t === "-").length, same: before === after || f.status === "U" };
        }).filter((f) => !f.same);
        if (stat) {
          files.forEach((f) => print(` ${f.path.padEnd(22)} | ${String(f.add + f.del).padStart(2)} <span class="t-green">${"+".repeat(Math.min(f.add, 20))}</span><span class="t-red">${"-".repeat(Math.min(f.del, 20))}</span>`));
          if (files.length) print(` ${plural(files.length, "file")} changed, ${files.reduce((s, f) => s + f.add, 0)} insertions(+), ${files.reduce((s, f) => s + f.del, 0)} deletions(-)`);
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
        if (!files.length && !cached && list.some((f) => staged(ws, f.path))) print('<span class="t-dim">(Nothing unstaged. Try git diff --staged, or open Review with ⇧⌘G.)</span>');
        return 0;
      }
      if (sub === "add") {
        const targets = args.slice(1);
        if (!targets.length) { print("Nothing specified, nothing added."); return 0; }
        for (const t of targets) {
          if (t === "." || t === "-A" || t === "--all") list.forEach((f) => (ws.index[f.path] = current(ws, f.path)));
          else {
            const p = resolvePath(term, t);
            if (!list.some((f) => f.path === p)) { print(`fatal: pathspec '${esc(t)}' did not match any files`); return 128; }
            ws.index[p] = current(ws, p);
          }
        }
        refreshAll();
        return 0;
      }
      if (sub === "restore" && args[1] === "--staged") { args.slice(2).forEach((t) => delete ws.index[resolvePath(term, t)]); refreshAll(); return 0; }
      if (sub === "commit") {
        const flags = args.filter((a) => /^-[a-z]+$/.test(a)).join("");
        const mi = args.findIndex((a) => /^-[a-z]*m$/.test(a));
        const msg = mi >= 0 ? args[mi + 1] : null;
        if (flags.includes("a")) list.filter((f) => f.status !== "U").forEach((f) => (ws.index[f.path] = current(ws, f.path)));
        const toCommit = list.filter((f) => staged(ws, f.path));
        if (!toCommit.length) { print('no changes added to commit (use "git add" and/or "git commit -a")'); return 1; }
        if (!msg) { print('<span class="t-dim">In Impulse this opens the message in an editor tab (impulse edit). Here, use -m "message".</span>'); return 1; }
        await sleep(160);
        const sha = commit(ws, msg);
        print(`[${esc(ws.branch)} ${sha}] ${esc(msg)}`);
        print(` ${plural(toCommit.length, "file")} changed, ${toCommit.reduce((s, f) => s + f.add, 0)} insertions(+), ${toCommit.reduce((s, f) => s + f.del, 0)} deletions(-)`);
        return 0;
      }
      if (sub === "push") {
        print("Enumerating objects: 9, done.");
        await sleep(350);
        print("Writing objects: 100% (5/5), 912 bytes | 912.00 KiB/s, done.");
        await sleep(300);
        print("To github.com:example/trailhead.git");
        print(`   ${ws.commits[1]?.[0] ?? "9da0f3e"}..${ws.commits[0][0]}  ${esc(ws.branch)} -&gt; ${esc(ws.branch)}`);
        if (ws.main) ws.ahead = 0;
        refreshAll();
        return 0;
      }
      if (sub === "pull" || sub === "fetch") { await sleep(500); if (sub === "pull") print("Already up to date."); return 0; }
      if (sub === "branch") {
        const names = [...new Set(["main", ...S.workspaces.map((w) => w.branch)])].sort();
        names.forEach((b) => print(b === ws.branch ? `* <span class="t-green">${esc(b)}</span>` : S.workspaces.some((w) => w.branch === b) ? `+ <span class="t-cyan">${esc(b)}</span>` : `  ${esc(b)}`, false));
        return 0;
      }
      if (sub === "worktree") {
        S.workspaces.forEach((w) => print(`${(w.root.replace(/^~/, HOME)).padEnd(52)} ${w.commits[0][0]} [${esc(w.branch)}]`, false));
        return 0;
      }
      if (sub === "checkout" || sub === "switch") { print('<span class="t-dim">Switching branches is ⌃⌘B in Impulse. For a second branch at the same time, start a task (⌥⌘N).</span>'); return 0; }
      if (sub === "merge") { print('<span class="t-dim">Finish Task (in the palette) lands a task’s branch for you: it syncs, runs the check, merges and cleans up.</span>'); return 0; }
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
      if (script === "ci") { await sleep(700); print("added 1 package, and audited 2 packages in 412ms"); print(""); print('found <span class="t-green t-bold">0</span> vulnerabilities'); return 0; }
      if (script === "install" || script === "i") { await sleep(500); print("up to date, audited 2 packages in 214ms"); print(""); print('found <span class="t-green t-bold">0</span> vulnerabilities'); return 0; }
      const scripts = { test: "node --test --test-reporter=./scripts/test-reporter.mjs test/*.test.ts", lint: "node --check src/server.ts", dev: "node --watch src/server.ts", start: "node src/server.ts" };
      if (!script) { print("Usage: npm &lt;command&gt;  (try npm test)"); return 1; }
      if (!scripts[script]) { print(`<span class="t-red">npm error</span> Missing script: "${esc(script)}"`); return 1; }
      print(`<span class="t-dim">&gt; trailhead@0.2.0 ${script}</span>`, false);
      print(`<span class="t-dim">&gt; ${scripts[script]}</span>`, false);
      print("", false);
      if (script === "test") return runTests(io);
      if (script === "lint") { await sleep(650); return 0; }
      await sleep(400);
      const port = 3000 + io.ws.slot * 100;
      print(`trailhead listening on <span class="t-link" data-port>http://localhost:${port}</span>`);
      io.ws.port = port;
      refreshAll();
      try { await io.forever(); } finally { io.ws.port = null; refreshAll(); }
      return 0;
    },

    node: async ({ print }, args) => {
      if (args[0] === "-v" || args[0] === "--version") { print("v24.9.0"); return 0; }
      print('<span class="t-dim">Try npm test or npm run dev: this demo has no Node REPL.</span>');
      return 1;
    },
    npx: async ({ print }) => { print('<span class="t-dim">This demo can’t download packages.</span>'); return 1; },

    impulse: async ({ print, term, ws }, args) => {
      const sub = args[0];
      if (sub === "open" || sub === "edit") {
        if (!args[1]) { print(`usage: impulse ${sub} &lt;file&gt;[:line[:column]]`); return 2; }
        const [file, line] = args[1].split(":");
        const p = resolvePath(term, file);
        if (p == null || current(ws, p) == null) { print(`impulse: no such file: ${esc(args[1])}`); return 1; }
        openFile(p, Number(line) || 1);
        if (sub === "edit") print('<span class="t-dim">(impulse edit waits until the tab closes; the demo doesn’t wait.)</span>');
        return 0;
      }
      if (sub === "review") { openReview(ws); return 0; }
      if (sub === "notify") { toast({ title: args[1] || "Impulse", sub: args.slice(2).join(" ") || undefined, ms: 3000 }); return 0; }
      if (sub === "tasks" && args[1] === "new") { openNewTask({ title: args.slice(2).filter((a) => !a.startsWith("-")).join(" ") }); return 0; }
      if (sub === "tasks") { taskListing(ws).split("\n").forEach((l) => print(esc(l), false)); return 0; }
      if (sub === "split") { print('<span class="t-dim">In Impulse this splits the current pane. The demo keeps one pane per tab.</span>'); return 0; }
      [
        "Usage: impulse &lt;command&gt;",
        "",
        "  open &lt;file&gt;[:line]   Open a file in an editor tab",
        "  edit &lt;file&gt;         Open a file and wait for its tab to close ($EDITOR)",
        "  review [scope]      Open Review",
        "  tasks               List the repository's workspaces and what's happening in them",
        "  split [right|down]  Split the current pane",
        "  notify &lt;title&gt;      Post a notification",
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
      if (p && current(io.ws, p) != null) {
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
  const AGENT_NAMES = { claude: "Claude Code", codex: "Codex", gemini: "Gemini CLI", aider: "Aider", opencode: "opencode" };
  for (const [cmd, name] of Object.entries(AGENT_NAMES)) COMMANDS[cmd] = (io) => agentSession(cmd, name, io);

  async function execute(line, io) {
    const [name, ...args] = splitArgs(line);
    const fn = COMMANDS[name];
    if (!fn) { io.print(`zsh: command not found: <span class="t-red">${esc(name)}</span>`); return 127; }
    return (await fn(io, args)) ?? 0;
  }

  // Tests as scripts/test-reporter.mjs prints them. The cache tests only exist
  // where the cache does (not on main).
  async function runTests(io) {
    const { print, sleep, ws } = io;
    const cache = ws.base !== "origin/main";
    const broken = cache && current(ws, "src/lib/cache.ts") !== FILES["src/lib/cache.ts"].fixed;
    const tests = [
      ...(cache ? [["returns what was stored", 0.3], ["forgets expired entries", 0.1], ["size leaves out expired entries", 0.4, true]] : []),
      ["kilometres to miles", 0.3], ["metres to feet", 0.1], ["celsius to fahrenheit", 0.0],
    ];
    let fail = 0;
    for (const [name, ms, fragile] of tests) {
      await sleep(110 + Math.random() * 120);
      if (fragile && broken) {
        fail++;
        print(`<span class="t-red">✖ ${name}</span>`);
        print('  <span class="t-link" data-open="test/cache.test.ts:28">test/cache.test.ts:28:10</span>');
        print("  expected 1 live entry, got 2");
      } else print(`<span class="t-green">✔</span> ${name} <span class="t-dim">(${ms}ms)</span>`);
    }
    await sleep(120);
    print(`<span class="t-dim">tests ${tests.length} · pass ${tests.length - fail} · fail ${fail}</span>`, false);
    io.duration = (fail ? 1240 : 980) + Math.round(Math.random() * 300);
    return fail ? 1 : 0;
  }

  // Clicking a file:line in output opens it, like ⌘-click in Impulse.
  panesEl.addEventListener("click", (e) => {
    const link = e.target.closest(".t-link");
    if (!link) return;
    if (link.dataset.open) { const [p, l] = link.dataset.open.split(":"); openFile(p, Number(l)); }
    else if (link.hasAttribute("data-port")) toast({ title: `Would open ${link.textContent.replace("http://", "")}`, sub: "In Impulse, ⌘-click opens links and ports.", ms: 2400 });
    else if (link.dataset.review) openReview(curWs(), link.dataset.review);
  });

  // `impulse tasks` (ImpulseKit's TaskListing).
  function taskListing(me) {
    const lines = [`${REPO}: ${plural(S.workspaces.length, "workspace")}`];
    for (const ws of S.workspaces) {
      lines.push("", `${ws.name}${ws.main ? " (main checkout)" : " (task)"}${ws === me ? " ← you" : ""}`);
      lines.push(`  branch ${ws.branch}${ws.base ? ` from ${ws.base}` : ""}${ws.main ? `, ${ws.ahead} ahead, 0 behind` : ""} at ${ws.commits[0][0]}`);
      lines.push(`  ${ws.root.replace(/^~/, HOME)}`);
      const agent = S.agents.find((a) => a.ws === ws);
      if (agent) lines.push(`  ${agent.name}: ${{ working: "working", waiting: "waiting for input", done: "finished" }[agent.state] || "idle"}`);
      const n = changes(ws).length;
      if (n) lines.push(`  ${plural(n, "uncommitted file")}`);
      if (ws !== me) { const shared = sharedFiles(ws, me); if (shared.length) lines.push(`  also changes files you change: ${shared.join(", ")}`); }
    }
    return lines.join("\n");
  }

  function commit(ws, msg) {
    const sha = shortSha();
    for (const [p, text] of Object.entries(ws.index)) { ws.head[p] = text; delete ws.index[p]; }
    for (const p of Object.keys(ws.work)) if (ws.work[p] === ws.head[p]) delete ws.work[p];
    ws.commits.unshift([sha, "", msg]);
    if (ws.main) ws.ahead++;
    refreshAll();
    return sha;
  }

  // ── The simulated agents ──────────────────────────────────────────────

  const agentOf = (term) => S.agents.find((a) => a.term === term);

  class Agent {
    constructor(cmd, name, io) {
      Object.assign(this, { cmd, name, term: io.term, ws: io.ws, blk: io.blk, state: "idle", since: Date.now(), turns: [], buffer: "", queued: null });
    }
    setState(state) {
      this.state = state;
      this.since = Date.now();
      // A finished agent you aren't looking at flags its tab and workspace row.
      if (state === "done" && (curWs() !== this.ws || activeTab() !== this.term.tab)) { this.term.tab.attention = true; if (curWs() !== this.ws) this.ws.attention = 1; }
      renderBelt(this);
      // Impulse checkpoints every turn: point at the way to review it.
      if (state === "done" && this.reviewable) { this.reviewable = false; const b = $("[data-belt='turn']", this.term.belt); b?.classList.add("bump"); }
      renderTabs();
      renderWorkspaces();
      updateChrome();
    }
    showPrompt() {
      this.promptEl?.remove();
      this.promptEl = this.blk.print('<span class="t-cyan">&gt;</span> <span class="typed"></span><span class="caret"></span>', false);
      this.promptEl.classList.add("agent-prompt");
      this.typed(this.term.keys.value);
    }
    typed(text) {
      this.buffer = text;
      if (this.promptEl) $(".typed", this.promptEl).textContent = text;
      this.term.scrollToEnd();
    }
    insert(text) { this.term.keys.value += text; this.typed(this.term.keys.value); if (curWs() === this.ws) this.term.focus(); }
    key(e) {
      if (e.key === "Enter") {
        e.preventDefault();
        const text = this.buffer.trim();
        this.term.keys.value = "";
        this.typed("");
        if (text) this.submit(text);
      } else if (e.ctrlKey && e.key.toLowerCase() === "c") { e.preventDefault(); this.quit(); }
    }
    async submit(text) {
      if (text === "/exit" || text === "exit" || text === "/quit") { this.quit(); return; }
      if (this.state === "working") { this.queued = text; return; }
      this.promptEl?.remove();
      this.promptEl = null;
      this.blk.print(`<span class="t-cyan">&gt;</span> ${esc(text)}`, false);
      this.blk.print("", false);
      this.setState("working");
      await agentTurn(this, text);
      this.blk.print("", false);
      this.showPrompt();
      this.setState("done");
      if (this.queued) { const next = this.queued; this.queued = null; this.submit(next); }
    }
    quit() { this.promptEl?.remove(); this.promptEl = null; this.exit?.(); }
  }

  async function agentSession(cmd, name, io) {
    const { term, print } = io;
    if (agentOf(term)) return 1;
    const agent = new Agent(cmd, name, io);
    S.agents.push(agent);
    term.ib.hidden = true;
    term.belt.hidden = false;
    print(`<span class="agent-box"><span class="t-bold">${esc(name)}</span>  <span class="t-dim">(simulated)</span><br><span class="t-dim">${esc(term.longCwd())}</span></span>`, false);
    const seed = io.ws.seed;
    if (seed) { io.ws.seed = null; await seed(agent); }
    else print(`<span class="t-dim">${io.ws.main ? "Try: fix the failing test" : "Try: add a difficulty filter to the trail search"}</span>`, false);
    agent.showPrompt();
    agent.setState(seed ? "done" : "idle");
    if (seed) agent.since = Date.now() - 3 * 60e3;
    if (curWs() === io.ws && activeTab() === term.tab) term.focus();
    await new Promise((resolve) => (agent.exit = resolve));
    endAgent(agent);
    print('<span class="t-dim">Session ended.</span>', false);
    return 0;
  }

  function endAgent(agent) {
    const i = S.agents.indexOf(agent);
    if (i < 0) return;
    S.agents.splice(i, 1);
    agent.term.ib.hidden = false;
    agent.term.belt.hidden = true;
    if (curWs() === agent.ws && activeTab() === agent.term.tab) agent.term.focus();
    renderTabs();
    renderWorkspaces();
    updateChrome();
  }

  const TURN_STATUS = { working: (s) => `Working · ${s}`, waiting: () => "Needs your input", done: (s) => `Finished ${s} ago`, idle: () => "Idle" };
  const elapsed = (ms) => { const s = Math.max(0, Math.floor(ms / 1000)); return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m` : `${Math.floor(s / 3600)}h ${Math.floor(s / 60) % 60}m`; };
  function renderBelt(agent) {
    const belt = agent.term.belt;
    belt.innerHTML = `${agentGlyph(agent.state, 12)}<span class="who">${esc(agent.name)}</span><span class="what">${TURN_STATUS[agent.state](elapsed(Date.now() - agent.since))}</span><span class="grow"></span>
      <button type="button" class="raised" data-belt="compose">${icon("message-square-plus", 11)}Compose<span class="k">⌘I</span></button>
      <button type="button" class="raised" data-belt="turn"${agent.turns.length ? "" : " disabled style=\"opacity:.5\""}>${icon("file-diff", 11)}Review Turn</button>
      <button type="button" data-belt="turns" class="hide-sm">${icon("history", 11)}Turns</button>`;
    belt.onclick = (e) => {
      const b = e.target.closest("[data-belt]");
      if (!b) return;
      if (b.dataset.belt === "compose") { agent.term.focus(); toast({ title: "Compose opens a message box for the agent", sub: "For long prompts. Here, type straight into the terminal.", ms: 3200 }); }
      else if (b.dataset.belt === "turn" && agent.turns.length) openReview(agent.ws, "turn");
      else if (b.dataset.belt === "turns") showMenu(b, agent.turns.length ? agent.turns.map((t) => [`Review Turn ${t.n} · ${t.time}`, () => openReview(agent.ws, "turn")]) : [["No turns yet", null]]);
    };
  }
  setInterval(() => { for (const a of S.agents) { const what = $(".what", a.term.belt); if (what) what.textContent = TURN_STATUS[a.state](elapsed(Date.now() - a.since)); } }, 1000);

  // One scripted turn. Impulse checkpoints every turn, so Review can show it.
  async function agentTurn(agent, text) {
    const ws = agent.ws, out = (l) => agent.blk.print(l), wait = (ms) => sleep(ms);
    const before = Object.fromEntries(paths(ws).map((p) => [p, current(ws, p)]));
    const edit = (p, next) => { ws.work[p] = next; refreshAll(); bumpChanges(); };
    const cacheBroken = ws.base !== "origin/main" && current(ws, "src/lib/cache.ts") !== FILES["src/lib/cache.ts"].fixed;
    const wantsFix = /test|fail|fix|bug/i.test(text), wantsFilter = /difficult|filter|trail|search/i.test(text);
    const uncommitted = changes(ws).filter((f) => f.status !== "U");
    let message = null;
    if (/commit/i.test(text) && uncommitted.length) {
      const msg = ws.message || "Changes from the last turn";
      await wait(600); out(`<span class="t-green">●</span> Bash <span class="t-dim">git commit -am "${esc(msg)}"</span>`);
      uncommitted.forEach((f) => (ws.index[f.path] = current(ws, f.path)));
      const sha = commit(ws, msg);
      await wait(400); out(`  <span class="t-dim">⎿ [${esc(ws.branch)} ${sha}] ${esc(msg)}</span>`);
      await wait(300); out(`Committed ${plural(uncommitted.length, "file")} on ${esc(ws.branch)}.`);
    } else if (cacheBroken && (wantsFix || (ws.main && !wantsFilter))) {
      await wait(700); out('<span class="t-green">●</span> Read <span class="t-dim">src/lib/cache.ts (39 lines)</span>');
      await wait(500); out('<span class="t-green">●</span> Read <span class="t-dim">test/cache.test.ts (26 lines)</span>');
      await wait(600); out('<span class="t-green">●</span> Bash <span class="t-dim">npm test</span>  <span class="t-red">✖ size leaves out expired entries</span>');
      await wait(900); out('<span class="t-green">●</span> Update <span class="t-dim">src/lib/cache.ts</span>');
      edit("src/lib/cache.ts", FILES["src/lib/cache.ts"].fixed);
      await wait(300); out("  <span class=\"t-dim\">⎿ size counted entries that had expired but that get() hadn’t removed yet</span>");
      await wait(600); out('<span class="t-green">●</span> Bash <span class="t-dim">npm test</span>  <span class="t-green">✔ 6 tests pass</span>');
      await wait(400); out("Done. The size getter now counts only live entries, and all 6 tests pass.");
      message = "Count only live entries in size";
    } else if (current(ws, "src/trails.ts") === BASE["src/trails.ts"] && current(ws, "src/server.ts") === BASE["src/server.ts"]) {
      await wait(700); out('<span class="t-green">●</span> Read <span class="t-dim">src/trails.ts (31 lines)</span>');
      await wait(500); out('<span class="t-green">●</span> Read <span class="t-dim">src/server.ts (30 lines)</span>');
      await wait(900); out('<span class="t-green">●</span> Update <span class="t-dim">src/trails.ts</span>');
      edit("src/trails.ts", FILES["src/trails.ts"].difficulty);
      await wait(300); out('  <span class="t-dim">⎿ Added difficulty(trail): easy under 300 m of climbing, hard over 700 m</span>');
      await wait(800); out('<span class="t-green">●</span> Update <span class="t-dim">src/server.ts</span>');
      edit("src/server.ts", FILES["src/server.ts"].difficulty);
      await wait(300); out('  <span class="t-dim">⎿ GET /trails?difficulty=hard filters the search</span>');
      await wait(600); out(`<span class="t-green">●</span> Bash <span class="t-dim">npm test</span>  ${cacheBroken ? '<span class="t-red">✖ size leaves out expired entries</span> <span class="t-dim">(not mine: it fails without this change too)</span>' : '<span class="t-green">✔ tests pass</span>'}`);
      await wait(400); out("Done. GET /trails?difficulty=hard now returns only the trails with over 700 m of climbing.");
      message = "Filter trails by difficulty";
    } else {
      await wait(900);
      out("Nothing else to do here. (This agent is scripted and knows two tricks: fixing the failing test and adding a difficulty filter. Type /exit to leave.)");
    }
    const after = Object.fromEntries(paths(ws).map((p) => [p, current(ws, p)]));
    const touched = Object.keys(after).filter((p) => after[p] !== before[p]);
    if (message) ws.message = message;
    if (touched.length) {
      const n = agent.turns.length + 1;
      const time = new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
      agent.turns.push({ n, time });
      ws.turn = { n, time, agent: agent.name, before: Object.fromEntries(touched.map((p) => [p, before[p]])), after: Object.fromEntries(touched.map((p) => [p, after[p]])) };
      agent.reviewable = true;
    }
  }

  function bumpChanges() {
    const pill = $("[data-changes]");
    pill.classList.remove("bump");
    void pill.offsetWidth;
    pill.classList.add("bump");
  }

  // ── Editor tabs (Monaco as the app sets it up) ────────────────────────

  // Who last changed each line of the committed files (inline blame).
  const BLAME = {
    "src/lib/cache.ts": [[1, 99, "maya", 4, "TTL cache for forecast lookups"]],
    "test/cache.test.ts": [[1, 99, "maya", 4, "TTL cache for forecast lookups"]],
    "src/forecast.ts": [[3, 3, "maya", 3, "Cache forecasts per trail"], [14, 15, "maya", 3, "Cache forecasts per trail"], [20, 21, "maya", 3, "Cache forecasts per trail"], [32, 32, "maya", 3, "Cache forecasts per trail"], [1, 99, "sam", 49, "Forecast endpoint"]],
    "src/trails.ts": [[7, 8, "maya", 45, "Handle missing elevation data"], [16, 16, "maya", 45, "Handle missing elevation data"], [1, 99, "maya", 62, "Add trail list endpoint"]],
    "src/server.ts": [[3, 3, "sam", 49, "Forecast endpoint"], [15, 21, "sam", 49, "Forecast endpoint"], [1, 99, "maya", 62, "Add trail list endpoint"]],
    "src/lib/units.ts": [[1, 99, "priya", 58, "Unit conversions with tests"]],
    "test/units.test.ts": [[1, 99, "priya", 58, "Unit conversions with tests"]],
    "scripts/test-reporter.mjs": [[1, 99, "priya", 58, "Unit conversions with tests"]],
    "README.md": [[1, 99, "sam", 9, "Document the API"]],
    "package.json": [[1, 99, "maya", 60, "Project actions for dev server and tests"]],
    "tsconfig.json": [[1, 99, "maya", 62, "Add trail list endpoint"]],
  };
  // relativeTime in the app's editor.js.
  function relativeTime(days) {
    const secs = days * 86400;
    for (const [unit, size] of [["year", 31536000], ["month", 2592000], ["week", 604800], ["day", 86400]]) {
      const n = Math.floor(secs / size);
      if (n >= 1) return `${n} ${unit}${n === 1 ? "" : "s"} ago`;
    }
    return "just now";
  }
  // Blame describes the saved, committed file: none on lines changed since.
  function blameFor(ws, path, line) {
    const head = ws.head[path], now = current(ws, path);
    if (head == null || head !== BASE[path]) return null;
    let headLine = null;
    for (const op of diffLines(head, now)) if (op.b === line) { headLine = op.t === " " ? op.a : null; break; }
    if (headLine == null) return null;
    const hit = (BLAME[path] || []).find(([from, to]) => headLine >= from && headLine <= to);
    return hit ? `${AUTHORS[hit[2]]}, ${relativeTime(hit[3])} · ${hit[4]}` : null;
  }

  const indentOf = (line) => (line.match(/^ */) || [""])[0].length;
  const wordAt = (line, col) => {
    const isW = (c) => /[\w$]/.test(c || "");
    let a = col, b = col;
    if (!isW(line[col]) && isW(line[col - 1])) a = b = col - 1;
    if (!isW(line[a])) return null;
    while (isW(line[a - 1])) a--;
    while (isW(line[b + 1])) b++;
    return line.slice(a, b + 1);
  };

  function openFile(path, line = 1, focus = true, ws = curWs()) {
    let tab = S.tabs.find((t) => t.kind === "editor" && t.ws === ws.id && t.path === path);
    if (!tab) {
      const el = h(`<div class="pane" role="tabpanel"><div class="ed-wrap"><div class="ed" tabindex="0" role="textbox" aria-readonly="true" aria-multiline="true" aria-label="${esc(path)}"><div class="ed-lines"></div></div><div class="ed-ruler" aria-hidden="true"></div></div></div>`);
      tab = { kind: "editor", ws: ws.id, path, el, line, col: 0 };
      const ed = $(".ed", el);
      ed.addEventListener("mousedown", (e) => {
        const row = e.target.closest(".ed-line");
        if (!row) return;
        const code = $(".ed-code", row), ch = charWidth(ed);
        tab.line = Number(row.dataset.n);
        tab.col = Math.max(0, Math.round((e.clientX - code.getBoundingClientRect().left) / ch));
        renderEditor(tab);
      });
      ed.addEventListener("keydown", (e) => {
        const lines = splitLines(current(wsById(tab.ws), tab.path));
        const moves = { ArrowDown: [1, 0], ArrowUp: [-1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1], PageDown: [12, 0], PageUp: [-12, 0] };
        if (moves[e.key]) {
          e.preventDefault();
          tab.line = Math.max(1, Math.min(lines.length, tab.line + moves[e.key][0]));
          tab.col = Math.max(0, Math.min((lines[tab.line - 1] || "").length, tab.col + moves[e.key][1]));
          renderEditor(tab, true);
        } else if (e.key === "Home" || e.key === "End") {
          e.preventDefault();
          tab.col = e.key === "Home" ? indentOf(lines[tab.line - 1] || "") : (lines[tab.line - 1] || "").length;
          renderEditor(tab, true);
        }
      });
      ed.addEventListener("scroll", () => drawRuler(tab));
      addTab(tab, false);
    }
    tab.line = line;
    tab.col = 0;
    activateTab(tab.id, focus);
    renderEditor(tab, true);
  }

  function charWidth(ed) {
    measure.font = getComputedStyle(ed).font;
    return measure.measureText("0").width;
  }

  function renderEditor(tab, reveal = false) {
    const ws = wsById(tab.ws);
    const text = current(ws, tab.path) ?? "";
    const src = splitLines(text);
    const { lines: toks, pairs } = tokenize(tab.path, text);
    tab.line = Math.max(1, Math.min(tab.line, src.length || 1));
    tab.col = Math.min(tab.col, (src[tab.line - 1] || "").length);
    // Change marks against the index, like the app's gutter (added, changed, removed).
    const marks = {};
    const base = indexText(ws, tab.path);
    if (base !== text) {
      for (const run of changeRuns(diffLines(base, text))) {
        if (!run.added.length) marks[Math.max(run.bStart, 1)] = marks[Math.max(run.bStart, 1)] || "d";
        else run.added.forEach((n) => (marks[n] = run.removed ? "m" : "a"));
      }
    }
    // Bracket pair guides, in the color of their brackets.
    const guides = {};
    for (const p of pairs) {
      if (p.closeLine - p.line < 2) continue;
      const col = Math.min(indentOf(src[p.line]), indentOf(src[p.closeLine]));
      for (let l = p.line + 1; l < p.closeLine; l++) (guides[l + 1] ||= []).push([col, p.level]);
    }
    const word = wordAt(src[tab.line - 1] || "", tab.col);
    tab.word = word;
    const html = src.map((line, i) => {
      const n = i + 1, cur = n === tab.line;
      const g = (guides[n] || []).map(([col, level]) => `<i class="ed-guide" style="left:${col}ch;background:color-mix(in srgb, ${bracketColor(level)} 30%, transparent)"></i>`).join("");
      const caret = cur ? `<i class="ed-caret" style="left:${tab.col}ch"></i>` : "";
      return `<div class="ed-line${cur ? " cur" : ""}" data-n="${n}"><span class="ed-num">${n}</span><span class="ed-deco">${marks[n] ? `<i class="${marks[n]}"></i>` : ""}</span><span class="ed-code">${g}${caret}${tokensHTML(toks[i] || [], word) || " "}</span></div>`;
    }).join("");
    $(".ed-lines", tab.el).innerHTML = html;
    tab.marks = marks;
    tab.lineCount = src.length;
    tab.occurrences = word ? src.map((l, i) => (new RegExp(`(^|[^\\w$])${word.replace(/\$/g, "\\$")}([^\\w$]|$)`).test(l) ? i + 1 : 0)).filter(Boolean) : [];
    const ed = $(".ed", tab.el);
    if (reveal) requestAnimationFrame(() => {
      const row = $(`.ed-line[data-n="${tab.line}"]`, tab.el);
      if (row && (row.offsetTop < ed.scrollTop || row.offsetTop > ed.scrollTop + ed.clientHeight - 40)) ed.scrollTop = Math.max(0, row.offsetTop - ed.clientHeight / 3);
      drawRuler(tab);
    });
    else requestAnimationFrame(() => drawRuler(tab));
    // Inline blame, after the cursor rests on a line for a moment.
    clearTimeout(tab.blameTimer);
    tab.blameTimer = setTimeout(() => {
      const blame = blameFor(ws, tab.path, tab.line);
      const code = $(".ed-line.cur .ed-code", tab.el);
      if (blame && code && !$(".ed-blame", code)) code.insertAdjacentHTML("beforeend", `<span class="ed-blame">    ${esc(blame)}</span>`);
    }, reduced ? 0 : 450);
    if (activeTab() === tab) updateStatus();
  }

  // The overview ruler on the right edge: change marks, the word's other
  // occurrences, and where the view is.
  function drawRuler(tab) {
    const ed = $(".ed", tab.el), ruler = $(".ed-ruler", tab.el);
    if (!ed || !ruler || !ed.clientHeight) return;
    const lh = parseFloat(getComputedStyle(ed).lineHeight) || 20;
    const content = Math.max(ed.scrollHeight, ed.clientHeight), H = ruler.clientHeight || ed.clientHeight;
    const y = (n) => ((n - 1) * lh / content) * H, size = Math.max(2, (lh / content) * H);
    let html = "";
    for (const [n, kind] of Object.entries(tab.marks || {})) html += `<i class="${kind}" style="top:${y(+n)}px;height:${size}px"></i>`;
    for (const n of tab.occurrences || []) html += `<i class="w" style="top:${y(n)}px;height:${size}px;left:5px"></i>`;
    if (ed.scrollHeight > ed.clientHeight + 2) html += `<b style="top:${(ed.scrollTop / content) * H}px;height:${(ed.clientHeight / content) * H}px"></b>`;
    ruler.innerHTML = html;
  }

  // ── Review ────────────────────────────────────────────────────────────

  // Review colors code with highlight.js, mapped to the theme's syntax colors
  // the way the app's SyntaxHighlighter maps it; loaded the first time Review opens.
  let hljsState = window.hljs ? "ready" : null;
  function loadHighlighter() {
    if (hljsState) return;
    hljsState = "loading";
    const s = document.createElement("script");
    s.src = "assets/vendor/highlight.min.js";
    s.onload = () => { hljsState = "ready"; S.tabs.filter((t) => t.kind === "review").forEach((t) => t.render()); };
    s.onerror = () => (hljsState = "failed");
    document.head.appendChild(s);
  }
  const HLJS_TOKEN = {
    "hljs-keyword": "keyword", "hljs-meta": "keyword", "hljs-bullet": "keyword", "hljs-selector-pseudo": "keyword",
    "hljs-built_in": "type", "hljs-type": "type", "hljs-string": "string", "hljs-char": "string", "hljs-template-tag": "string", "hljs-template-string": "string",
    "hljs-number": "number", "hljs-literal": "constant", "hljs-symbol": "constant", "hljs-selector-id": "constant", "hljs-selector-class": "constant",
    "hljs-comment": "comment", "hljs-quote": "comment", "hljs-doctag": "comment",
    "hljs-variable": "variable", "hljs-template-variable": "variable", "hljs-params": "variable", "hljs-subst": "variable",
    "hljs-attr": "attribute", "hljs-attribute": "attribute", "hljs-property": "attribute", "hljs-selector-attr": "attribute",
    "hljs-tag": "tag", "hljs-name": "tag", "hljs-selector-tag": "tag", "hljs-regexp": "regexp", "hljs-operator": "operator",
    "hljs-punctuation": "delimiter", "hljs-link": "link", "hljs-section": "function",
  };
  const HLJS_LANG = { typescript: "typescript", javascript: "javascript", json: "json", markdown: "markdown" };
  // One side of a diff, highlighted as a whole file and split into lines of [text, token].
  function highlightSide(path, text) {
    const lang = HLJS_LANG[langOf(path).name];
    if (text == null) return [];
    if (hljsState !== "ready" || !lang || !window.hljs?.getLanguage(lang)) return null;
    const html = window.hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
    const lines = [[]], stack = [];
    const ent = { amp: "&", lt: "<", gt: ">", quot: '"', "#x27": "'", "#39": "'", apos: "'" };
    for (const m of html.matchAll(/<span class="([^"]*)">|<\/span>|([^<]+)/g)) {
      if (m[1] != null) {
        const [first, ...mods] = m[1].split(" ");
        stack.push(first === "hljs-title" ? (mods.some((x) => x.startsWith("class")) ? "type" : "function") : HLJS_TOKEN[first] || null);
      } else if (m[0] === "</span>") stack.pop();
      else {
        const token = [...stack].reverse().find(Boolean) || null;
        m[2].replace(/&(#?\w+);/g, (_, e) => ent[e] ?? `&${e};`).split("\n").forEach((part, i) => { if (i) lines.push([]); if (part) lines[lines.length - 1].push([part, token]); });
      }
    }
    return lines;
  }
  const spansHTML = (spans) => spans.map(([t, k]) => (k ? `<span style="color:var(--sx-${k})">${esc(t)}</span>` : esc(t))).join("");

  function openReview(ws = curWs(), scope = "uncommitted") {
    let tab = S.tabs.find((t) => t.kind === "review" && t.ws === ws.id);
    if (!tab) {
      const el = h(`<div class="pane" role="tabpanel"><div class="rv-head"></div><div class="rv-body"><div class="rv-nav"><label class="rv-filter">${icon("search", 12)}<input type="text" placeholder="Filter files (T)" aria-label="Filter files" style="border:0;outline:0;background:none;color:inherit;font:inherit;width:100%"></label><div class="rv-files"></div></div><div class="rv-diff"></div></div></div>`);
      tab = { kind: "review", ws: ws.id, el, composer: null, scope, viewed: new Set(), filter: "" };
      tab.render = () => renderReview(tab);
      el.addEventListener("click", (e) => onReviewClick(tab, e));
      $(".rv-filter input", el).addEventListener("input", (e) => { tab.filter = e.target.value; renderReview(tab); });
      addTab(tab, false);
    }
    tab.scope = scope;
    loadHighlighter();
    activateTab(tab.id);
  }

  const SCOPE_LABEL = { uncommitted: () => "Uncommitted changes", turn: (ws) => `Agent turn at ${ws.turn?.time}`, base: (ws) => `vs ${ws.base || "origin/main"}` };

  function renderReview(tab) {
    const ws = wsById(tab.ws);
    if (tab.scope === "turn" && !ws.turn) tab.scope = "uncommitted";
    const list = diffSet(ws, tab.scope);
    const t = { files: list.length, add: list.reduce((s, f) => s + f.add, 0), del: list.reduce((s, f) => s + f.del, 0) };
    const viewed = list.filter((f) => tab.viewed.has(f.path)).length;
    const comments = S.comments.filter((c) => c.ws === ws.id && list.some((f) => f.path === c.path));
    $(".rv-head", tab.el).innerHTML = `<button type="button" class="rv-scope" data-rv="scope" aria-haspopup="menu">${icon("file-diff", 13)}${esc(SCOPE_LABEL[tab.scope](ws))}${icon("chevron-down", 11)}</button>
      <span class="muted">${plural(t.files, "file")}</span>${stat(t.add, t.del)}<span class="grow"></span>
      <span class="stat" style="color:${viewed && viewed === t.files ? "var(--green)" : "var(--text2)"}">${viewed}/${t.files} viewed</span>
      <span class="rv-seg" role="group" aria-label="Layout"><button type="button" aria-pressed="true">Unified</button><button type="button" aria-pressed="false" data-rv="split">Split</button></span>
      <button type="button" class="rv-icon" data-rv="comments" title="Comments" aria-label="Comments: ${comments.length}" aria-haspopup="menu">${icon("message-square", 13)}${comments.length ? `<span class="stat" style="color:var(--accent)">${comments.length}</span>` : ""}</button>
      <button type="button" class="rv-icon hide-sm" data-rv="changes" title="Show Changes panel to commit" aria-label="Show Changes panel to commit">${icon("git-branch", 13)}</button>
      <button type="button" class="rv-icon hide-sm" data-rv="refresh" title="Refresh" aria-label="Refresh">${icon("refresh-cw", 13)}</button>`;
    let dir = null;
    const shown = list.filter((f) => !tab.filter || f.path.toLowerCase().includes(tab.filter.toLowerCase()));
    $(".rv-files", tab.el).innerHTML = shown.map((f) => {
      const d = f.path.includes("/") ? f.path.slice(0, f.path.lastIndexOf("/")) : "";
      const head = d !== dir ? `<div class="rv-dir">${esc((dir = d) || "/")}</div>` : "";
      const n = comments.filter((c) => c.path === f.path).length;
      return `${head}<div class="rv-file" role="button" tabindex="0" data-rv="file" data-path="${f.path}"><span class="rv-check${tab.viewed.has(f.path) ? " on" : ""}" role="checkbox" aria-checked="${tab.viewed.has(f.path)}" aria-label="Viewed" data-rv="viewed" data-path="${f.path}">${tab.viewed.has(f.path) ? icon("check", 10, 3) : ""}</span><span class="st ${f.status}">${f.status}</span>${esc(f.path.split("/").pop())}<span class="counts">${n ? `<span class="stat" style="color:var(--text2)">${icon("message-square", 10)}${n}</span>` : ""}${stat(f.add, f.del)}</span></div>`;
    }).join("") || '<div class="rv-dir">No changes</div>';
    const diff = $(".rv-diff", tab.el);
    const scroll = diff.scrollTop;
    if (!list.length) { diff.innerHTML = `<div class="rv-empty">${tab.scope === "uncommitted" ? "Nothing to review: the working tree is clean." : "No changes in this scope."}</div>`; return; }
    diff.innerHTML = list.map((f) => {
      const oldHl = highlightSide(f.path, f.before), newHl = highlightSide(f.path, f.after);
      const oldTok = oldHl ? null : tokenize(f.path, f.before ?? "").lines, newTok = newHl ? null : tokenize(f.path, f.after ?? "").lines;
      const code = (l) => (l.t === "-" ? (oldHl ? spansHTML(oldHl[l.a - 1] || []) : tokensHTML(oldTok[l.a - 1] || [])) : newHl ? spansHTML(newHl[l.b - 1] || []) : tokensHTML(newTok[l.b - 1] || []));
      const collapsed = tab.viewed.has(f.path);
      const body = collapsed ? "" : hunks(f.ops).map((hk) => `<div class="rv-hunk"><span>${hk.header}</span><span class="grow"></span><button type="button" data-rv="comment-hunk" data-path="${f.path}" data-line="${hk.lines.find((l) => l.t !== " ")?.b ?? hk.lines[0].b}">Comment</button></div>` + hk.lines.map((l) => {
        const cls = l.t === "+" ? "a" : l.t === "-" ? "d" : "";
        const cs = l.t !== "-" ? S.comments.filter((c) => c.ws === ws.id && c.path === f.path && c.line === l.b).map((c) => `<div class="rv-comment"><div class="who">line ${c.line}<span class="grow" style="flex:1"></span><button type="button" class="btn2 ghost" data-rv="delete-comment" data-id="${c.id}">Delete</button></div>${esc(c.text)}</div>`).join("") : "";
        const composer = tab.composer && tab.composer.path === f.path && tab.composer.line === l.b && l.t !== "-" ? `<div class="rv-composer"><textarea aria-label="Comment on line ${l.b}" placeholder="Comment on line ${l.b}… (⌘↩ to save)"></textarea><div class="row"><button type="button" class="btn2" data-rv="cancel">Cancel</button><button type="button" class="btn2 primary" data-rv="save">Comment</button></div></div>` : "";
        return `<div class="rv-line ${cls}" data-path="${f.path}" data-line="${l.b ?? ""}"><span class="n" data-rv="comment" title="${l.t === "-" ? "" : "Comment on this line"}">${l.a ?? ""}</span><span class="n" data-rv="comment">${l.b ?? ""}</span><span class="sign">${l.t === " " ? "" : l.t === "-" ? "−" : "+"}</span><span class="code">${code(l) || " "}</span></div>${cs}${composer}`;
      }).join("")).join("");
      const slash = f.path.lastIndexOf("/");
      return `<div class="rv-filecard" id="rv-${tab.id}-${f.path.replace(/\W/g, "-")}"><div class="rv-fhead"><span class="ico" style="color:var(--text3)">${collapsed ? icon("chevron-right", 12) : icon("chevron-down", 12)}</span><span class="st ${f.status}">${f.status}</span><span class="path"><span class="dir">${esc(f.path.slice(0, slash + 1))}</span>${esc(f.path.slice(slash + 1))}</span><span class="grow"></span>${stat(f.add, f.del)}<label><span class="rv-check${collapsed ? " on" : ""}" role="checkbox" aria-checked="${collapsed}" data-rv="viewed" data-path="${f.path}">${collapsed ? icon("check", 10, 3) : ""}</span>Viewed</label><button type="button" class="open" data-rv="open" data-path="${f.path}">Open</button></div>${body}</div>`;
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

  let commentId = 1;
  function saveComment(tab, text) {
    if (text.trim()) S.comments.push({ ...tab.composer, ws: tab.ws, id: commentId++, text: text.trim() });
    tab.composer = null;
    renderReview(tab);
  }

  function onReviewClick(tab, e) {
    const el = e.target.closest("[data-rv]");
    if (!el) return;
    const ws = wsById(tab.ws), act = el.dataset.rv;
    if (act === "viewed") {
      e.stopPropagation();
      const p = el.dataset.path;
      tab.viewed.has(p) ? tab.viewed.delete(p) : tab.viewed.add(p);
      renderReview(tab);
    } else if (act === "file") {
      $(`#rv-${tab.id}-${el.dataset.path.replace(/\W/g, "-")}`, tab.el)?.scrollIntoView({ block: "start", behavior: reduced ? "auto" : "smooth" });
    } else if (act === "comment" || act === "comment-hunk") {
      const row = act === "comment" ? el.closest(".rv-line") : el;
      if (!row.dataset.line) return;
      tab.composer = { path: row.dataset.path, line: Number(row.dataset.line) };
      renderReview(tab);
    } else if (act === "cancel") { tab.composer = null; renderReview(tab); }
    else if (act === "save") saveComment(tab, $(".rv-composer textarea", tab.el).value);
    else if (act === "delete-comment") { S.comments = S.comments.filter((c) => c.id !== Number(el.dataset.id)); renderReview(tab); }
    else if (act === "open") openFile(el.dataset.path, 1, true, ws);
    else if (act === "refresh") renderReview(tab);
    else if (act === "split") toast({ title: "Split shows the old and new file side by side", sub: "This demo shows the unified layout only.", ms: 2600 });
    else if (act === "changes") toast({ title: "The Changes panel is where you commit", sub: "⌃⇧G in Impulse. Here, use git commit in a terminal, or ask the agent to commit.", ms: 3600 });
    else if (act === "scope") {
      showMenu(el, [
        ["All uncommitted changes", () => { tab.scope = "uncommitted"; renderReview(tab); }],
        ...(ws.turn ? [[`Last agent turn (${ws.turn.agent})`, () => { tab.scope = "turn"; renderReview(tab); }]] : []),
        [`Compare with ${ws.main ? "origin/main" : ws.base}`, () => { tab.scope = "base"; renderReview(tab); }],
      ]);
    } else if (act === "comments") {
      const mine = S.comments.filter((c) => c.ws === ws.id);
      const agents = S.agents;
      showMenu(el, [
        ...(agents.length ? agents.map((a) => [`Send Comments to ${a.name} · ${a.ws.name}`, mine.length ? () => sendComments(tab, a, mine) : null]) : [["Send Comments to an Agent (none running)", null]]),
        ["Copy Comments as Prompt", mine.length ? () => { navigator.clipboard?.writeText(commentsPrompt(mine)).catch(() => {}); toast({ title: `Copied ${plural(mine.length, "comment")} as a prompt`, kind: "success", ms: 2000 }); } : null],
        ["Delete All Comments in Repository…", S.comments.length ? () => { S.comments = []; renderReview(tab); } : null],
      ]);
    }
  }

  const commentsPrompt = (list) => "Please address these review comments:\n\n" + list.map((c) => `${c.path}:${c.line}\n${c.text}`).join("\n\n");
  // Sending types the prompt into the agent; it doesn't press Return.
  function sendComments(tab, agent, list) {
    agent.insert(`Please address my ${plural(list.length, "review comment")} on ${[...new Set(list.map((c) => c.path))].join(", ")}`);
    toast({ title: `Sent to ${agent.name}. Review it in its prompt, then press Return.`, action: { label: "Show", run: () => activateTab(agent.term.tab.id) }, ms: 4000 });
  }

  // ── Finish Task (TaskFinish.swift) ────────────────────────────────────

  const STEPS = ["commit", "sync", "check", "land", "update", "cleanUp"];
  const IGNORED_IN_TASKS = "\x2eenv, config/dev.local.json";
  function finishModel(ws) {
    const main = S.workspaces.find((w) => w.main);
    const baseRef = ws.base || "origin/main", base = baseRef.replace(/^origin\//, "");
    return { name: ws.name, branch: ws.branch, baseRef, base, mainName: main.name };
  }
  function finishTitle(tab, step) {
    const m = finishModel(wsById(tab.ws)), review = (S.landing || tab.landing) === "review";
    return { commit: "Commit", sync: `Sync with ${m.baseRef}`, check: "Check", land: review ? "Push for review" : `Merge into ${m.baseRef} and push`, update: `Update ${m.mainName}`, cleanUp: "Clean up" }[step];
  }
  function finishPlan(tab, step) {
    const m = finishModel(wsById(tab.ws)), review = (S.landing || tab.landing) === "review";
    return {
      commit: `Everything in ${m.name} is committed, and no agent there is working.`,
      sync: `${m.baseRef}'s new commits are merged into ${m.branch}, here in the task (never a rebase).`,
      check: "Runs npm test in a terminal tab; it has to pass.",
      land: review ? `Pushes ${m.branch} to origin. Your git host takes it from there.` : `A merge commit, made in a throwaway folder and pushed as ${m.base}. ${m.mainName} isn't touched.`,
      update: review ? `After ${m.branch} is merged.` : `Fast-forwards ${m.mainName} when it's on ${m.base} with nothing uncommitted.`,
      cleanUp: review ? `After ${m.branch} is merged, Impulse offers to clean up.` : "Archives the task (its archive script runs) and deletes its branch.",
    }[step];
  }

  function openFinish(ws = curWs()) {
    if (ws.main) { toast({ title: "Finish Task works in a task's workspace.", sub: "Open a task in the sidebar first, like fix-elevation.", ms: 3200 }); return; }
    let tab = S.tabs.find((t) => t.kind === "finish" && t.ws === ws.id);
    if (!tab) {
      const el = h('<div class="pane" role="tabpanel"><div class="fin"></div></div>');
      tab = { kind: "finish", ws: ws.id, el, statuses: {}, landing: "merge", remember: true, deleteBranch: true, running: false };
      tab.render = () => renderFinish(tab);
      el.addEventListener("click", (e) => onFinishClick(tab, e));
      addTab(tab, false);
    }
    activateTab(tab.id);
  }

  function renderFinish(tab) {
    const ws = wsById(tab.ws);
    if (!ws) return;
    const m = finishModel(ws);
    const started = Object.keys(tab.statuses).length > 0;
    const stopped = STEPS.some((s) => tab.statuses[s]?.kind === "failed");
    const statusIcon = (st) => !st ? icon("circle", 15) : st.kind === "running" ? ring(13) : st.kind === "done" ? icon("circle-check", 15) : st.kind === "failed" ? icon("circle-x", 15) : st.kind === "skipped" ? icon("minus", 15) : icon("circle-alert", 15);
    const landing = S.landing ? "" : `<div class="fin-q"><h4>How does work land in ${esc(m.mainName)}?</h4>
      ${["merge", "review"].map((k) => `<button type="button" class="fin-opt" role="radio" aria-checked="${tab.landing === k}" data-fin="landing" data-value="${k}"${tab.running ? " disabled" : ""}><span class="ico">${tab.landing === k ? icon("circle-dot", 13) : icon("circle", 13)}</span><span><b>${k === "merge" ? `Merge into ${esc(m.baseRef)} and push` : "Push for review"}</b><span>${k === "merge" ? `Impulse merges the branch with a merge commit and pushes ${esc(m.base)}. Nobody reviews it on the server first.` : "Impulse pushes the branch; you open a merge request on your git host and it's merged there."}</span></span></button>`).join("")}
      <label class="fin-check small"><input type="checkbox" data-fin="remember"${tab.remember ? " checked" : ""}> Remember for ${esc(m.mainName)} (on this Mac; Project Setup can change it)</label></div>`;
    const branchOpt = (S.landing || tab.landing) === "merge" ? `<label class="fin-check"><input type="checkbox" data-fin="delete"${tab.deleteBranch ? " checked" : ""}${tab.running ? " disabled" : ""}> Delete ${esc(m.branch)} once it's merged</label>` : "";
    $(".fin", tab.el).innerHTML = `<div class="fin-head"><div><h3>Finish ${esc(m.name)}</h3><div class="sub">${esc(m.branch)} → ${esc(m.baseRef)}</div></div><span class="grow"></span>
      <button type="button" class="btn2" data-fin="close">Close</button>
      <button type="button" class="btn2 primary" data-fin="run"${tab.running || tab.finished ? " disabled" : ""}>${icon("git-merge", 12)}${started && stopped ? "Continue" : "Finish"}</button></div>
      <div class="fin-body">${landing}${branchOpt}
        <div class="fin-steps">${STEPS.map((s) => {
          const st = tab.statuses[s];
          const acts = st?.remedy === "changes" ? '<div class="acts"><button type="button" class="btn2" data-fin="changes">Show Changes</button></div>' : st?.remedy === "check" ? '<div class="acts"><button type="button" class="btn2" data-fin="check-tab">Show Its Terminal</button></div>' : "";
          return `<div class="fin-step ${st?.kind || "pending"}"><span class="ico">${statusIcon(st)}</span><div><b>${esc(finishTitle(tab, s))}</b><p>${esc(st?.text || finishPlan(tab, s))}</p>${acts}</div></div>`;
        }).join("")}</div>
      </div>`;
  }

  function onFinishClick(tab, e) {
    const el = e.target.closest("[data-fin]");
    if (!el) return;
    const act = el.dataset.fin;
    if (act === "landing") { tab.landing = el.dataset.value; renderFinish(tab); }
    else if (act === "remember") tab.remember = el.checked;
    else if (act === "delete") tab.deleteBranch = el.checked;
    else if (act === "close") closeTab(tab.id);
    else if (act === "run") runFinish(tab);
    else if (act === "changes") openReview(wsById(tab.ws));
    else if (act === "check-tab" && tab.checkTab) activateTab(tab.checkTab.id);
  }

  async function runFinish(tab) {
    const ws = wsById(tab.ws), m = finishModel(ws), main = S.workspaces.find((w) => w.main);
    if (!S.landing && tab.remember) S.landing = tab.landing;
    const landing = S.landing || tab.landing;
    tab.running = true;
    const set = (step, kind, text, remedy) => { tab.statuses[step] = { kind, text, remedy }; renderFinish(tab); };
    const stop = (step, text, remedy) => { tab.running = false; set(step, "failed", text, remedy); };
    for (const step of STEPS) {
      if (tab.statuses[step]?.kind === "done" || tab.statuses[step]?.kind === "skipped") continue;
      if (step === "commit") {
        set(step, "running", `Looking at ${m.name}…`);
        await sleep(500);
        const n = changes(ws).length;
        if (n) return stop(step, `${plural(n, "uncommitted file")}. Commit (or stash) ${n === 1 ? "it" : "them"}, then Continue.`, "changes");
        set(step, "done", "Everything is committed.");
      } else if (step === "sync") {
        set(step, "running", "Fetching origin…");
        await sleep(800);
        if (m.baseRef === "origin/main" && !ws.synced) { ws.synced = true; ws.commits.unshift([shortSha(), "", `Merge remote-tracking branch '${m.baseRef}' into ${m.branch}`]); set(step, "done", `Merged 1 new commit from ${m.baseRef} into ${m.branch}.`); }
        else set(step, "done", `Up to date with ${m.baseRef}.`);
      } else if (step === "check") {
        set(step, "running", "Running npm test in a terminal tab…");
        tab.checkTab = newTerminal(ws, false);
        renderTabs();
        const code = await tab.checkTab.term.run("npm test");
        if (code) return stop(step, `Failed (exit status ${code}). See its terminal tab; fix it, commit, then Continue.`, "check");
        set(step, "done", `Passed in ${Math.max(1, Math.round((tab.checkTab.term.lastMs || 1000) / 1000))}s.`);
      } else if (step === "land") {
        if (landing === "review") {
          set(step, "running", `Pushing ${m.branch} to origin…`);
          await sleep(900);
          set(step, "done", `Pushed ${m.branch} to origin. Open a merge request for it on your git host.`);
          set("update", "skipped", `After ${m.branch} is merged into ${m.baseRef}.`);
          set("cleanUp", "skipped", `Once ${m.branch} is merged, Impulse offers to clean up.`);
          break;
        }
        set(step, "running", `Merging into ${m.baseRef} and pushing…`);
        await sleep(1100);
        tab.sha = shortSha();
        set(step, "done", `Merged as ${tab.sha} and pushed to ${m.baseRef}.`);
      } else if (step === "update") {
        set(step, "running", `Updating ${m.mainName}…`);
        await sleep(500);
        const dirty = changes(main).length;
        if (main.branch !== m.base) set(step, "skipped", `${m.mainName} is on ${main.branch}, so it was left alone.`);
        else if (dirty) set(step, "skipped", `${m.mainName} has ${plural(dirty, "uncommitted file")}, so it wasn't updated. Its row shows how far behind it is; click that to pull when you're ready.`);
        else set(step, "done", `Fast-forwarded ${m.mainName} to ${m.baseRef}.`);
      } else if (step === "cleanUp") {
        set(step, "running", `Looking at what's left in ${m.name}…`);
        await sleep(400);
        const ok = await confirmSheet(`Archive ${m.name}?`, `The task's folder is removed. Ignored files in it (${IGNORED_IN_TASKS}) are deleted, and Restore Task can't bring them back.`, "Archive");
        if (!ok) return stop(step, "The task wasn't archived, so its ignored files are still there. Continue when you're ready.");
        set(step, "running", "Closing the task's workspace…");
        await sleep(600);
        tab.finished = true;
        const snapshot = removeWorkspace(ws);
        toast({ title: `Finished ${m.branch}: merged into ${m.baseRef} as ${tab.sha}.`, kind: "success", action: { label: "Restore Task", run: () => restoreWorkspace(snapshot) }, ms: 15000 });
        return;
      }
    }
    tab.running = false;
    tab.finished = true;
    renderFinish(tab);
  }

  // ── File tree ─────────────────────────────────────────────────────────

  const filesEl = $("[data-files]");

  // Letters as the app's tree shows them; a folder takes what's inside it.
  function treeStatus(ws) {
    const st = new Map();
    for (const f of changes(ws)) st.set(f.path, f.status === "U" ? "?" : f.status === "A" ? "A" : "M");
    for (const p of Object.keys(FILES)) if (isIgnored(p)) st.set(p, "!");
    return st;
  }

  function renderTree() {
    const ws = curWs();
    const st = treeStatus(ws);
    const activePath = activeTab()?.kind === "editor" ? activeTab().path : null;
    const all = [...new Set([...paths(ws).filter((p) => current(ws, p) != null), ...Object.keys(FILES).filter(isIgnored)])];
    const rows = [];
    const walk = (dir, depth) => {
      const prefix = dir ? dir + "/" : "";
      const entries = new Map();
      for (const p of all) {
        if (!p.startsWith(prefix)) continue;
        const [first, ...more] = p.slice(prefix.length).split("/");
        entries.set(first, more.length ? "dir" : "file");
      }
      const sorted = [...entries].sort((a, b) => (a[1] === b[1] ? a[0].localeCompare(b[0]) : a[1] === "dir" ? -1 : 1));
      for (const [name, kind] of sorted) {
        const path = prefix + name;
        const indent = depth ? `<span class="indent" style="width:${depth * 16}px"></span>` : "";
        if (kind === "dir") {
          const open = S.open.has(path);
          const inside = [...st].filter(([p]) => p.startsWith(path + "/")).map(([, s]) => s);
          const dirSt = inside.includes("?") ? "?" : inside.includes("A") ? "A" : inside.includes("M") ? "M" : "";
          rows.push(`<button type="button" class="fi" role="treeitem" aria-level="${depth + 1}" aria-expanded="${open}" data-dir="${path}"${dirSt ? ` data-st="${dirSt}"` : ""}>${indent}<span class="chev">${open ? icon("chevron-down", 10, 2.2) : icon("chevron-right", 10, 2.2)}</span>${folderIcon(name, open)}<span class="fname">${esc(name)}</span>${dirSt ? `<span class="st">${dirSt}</span>` : ""}</button>`);
          if (open) walk(path, depth + 1);
        } else {
          const s = st.get(path) || "";
          rows.push(`<button type="button" class="fi${path === activePath ? " active" : ""}" role="treeitem" aria-level="${depth + 1}" data-file="${path}"${s ? ` data-st="${s}"` : ""}${path === activePath ? ' aria-selected="true"' : ""}>${indent}<span class="chev"></span>${fileIcon(name)}<span class="fname">${esc(name)}</span>${s && s !== "!" ? `<span class="st">${s}</span>` : ""}</button>`);
        }
      }
    };
    walk("", 0);
    filesEl.innerHTML = rows.join("");
  }

  filesEl.addEventListener("click", (e) => {
    const row = e.target.closest(".fi");
    if (!row) return;
    tookOver();
    if (row.dataset.dir) { const d = row.dataset.dir; S.open.has(d) ? S.open.delete(d) : S.open.add(d); renderTree(); }
    else if (isIgnored(row.dataset.file)) openFile(row.dataset.file, 1);
    else openFile(row.dataset.file, 1);
  });

  // ── Workspaces in the sidebar, and where they change the same files ──

  // What a workspace changes: the main checkout's uncommitted files and
  // unpushed commits; a task's uncommitted files and commits since its base.
  function changedFiles(ws) {
    const set = new Set(changes(ws).map((f) => f.path));
    if (ws.main) { if (ws.ahead) UNPUSHED.forEach((p) => set.add(p)); }
    else for (const p of Object.keys(ws.head)) if (ws.head[p] !== BASE[p]) set.add(p);
    return set;
  }
  function sharedFiles(a, b) {
    const B = changedFiles(b);
    return [...changedFiles(a)].filter((p) => B.has(p)).sort();
  }
  const overlapPairs = (ws) => S.workspaces.filter((o) => o !== ws).map((other) => ({ other, files: sharedFiles(ws, other) })).filter((p) => p.files.length);
  const overlapCount = (ws) => new Set(overlapPairs(ws).flatMap((p) => p.files)).size;
  // Check for Conflicts asks git whether merging the two would conflict:
  // changes to the same or adjacent lines of a file do.
  function conflicting(a, b, files) {
    return files.filter((p) => {
      const ra = changeRuns(diffLines(BASE[p], current(a, p))), rb = changeRuns(diffLines(BASE[p], current(b, p)));
      return ra.some((x) => rb.some((y) => x.aStart <= y.aEnd && y.aStart <= x.aEnd));
    });
  }
  // Impulse says so once when two workspaces start to change the same files.
  function checkOverlaps(notify = true) {
    for (let i = 0; i < S.workspaces.length; i++) for (let j = i + 1; j < S.workspaces.length; j++) {
      const a = S.workspaces[i], b = S.workspaces[j], key = [a.id, b.id].sort().join("|");
      const n = sharedFiles(a, b).length;
      if (!n) { S.overlapsSeen.delete(key); continue; }
      if (S.overlapsSeen.has(key)) continue;
      S.overlapsSeen.add(key);
      if (notify) toast({ title: `${b.name} and ${a.name} now change the same ${n === 1 ? "file" : `${n} files`}`, kind: "warning", action: { label: "Show", run: () => showOverlap(b) }, ms: 6000 });
    }
  }

  const wsEl = $("[data-workspaces]");
  function renderWorkspaces() {
    const grouped = S.workspaces.length > 1;
    let html = grouped ? `<div class="ws-group">${icon("git-fork", 11)}<span>${REPO}</span></div>` : "";
    for (const ws of S.workspaces) {
      const t = totals(ws), mine = S.agents.filter((a) => a.ws === ws);
      const working = mine.filter((a) => a.state === "working").length, waiting = mine.filter((a) => a.state === "done" || a.state === "waiting").length;
      const shared = overlapCount(ws), n = wsTabs(ws).length;
      const help = overlapPairs(ws).map((p) => `Shares ${plural(p.files.length, "file")} with ${p.other.name}`).join("\n");
      html += `<div class="ws-row${grouped ? " indented" : ""}${ws.id === S.ws ? " active" : ""}${ws.fresh ? " fresh" : ""}" role="button" tabindex="0" data-ws="${ws.id}" aria-current="${ws.id === S.ws}" title="${esc(`${ws.root.replace(/^~/, HOME)}\n${ws.branch}${ws.slot ? `\nTask slot ${ws.slot} · from ${ws.base} · PORT ${3000 + ws.slot * 100}` : ""}`)}">
        <span class="chev"></span><span class="lead">${icon("folder-git-2", 13)}</span><span class="name">${esc(ws.name)}</span>${ws.branch !== ws.name ? `<span class="branch" data-full="${esc(ws.branch)}">${esc(ws.branch)}</span>` : ""}
        <span class="trail">${shared ? `<button type="button" class="overlap" data-overlap="${ws.id}" title="${esc(help)}" aria-label="${esc(help)}" aria-haspopup="dialog">${icon("triangle-alert", 10)}${shared}</button>` : ""}${ws.port ? `<span class="port">:${ws.port}</span>` : ""}${working ? ring(10) : ""}${waiting ? `<span class="waiting" title="${plural(waiting, "agent")} waiting for you">${icon("bot", 10)}${waiting}</span>` : ""}${t.files ? stat(t.add, t.del) : ""}${ws.attention ? `<span class="badge">${ws.attention}</span>` : n > 1 ? `<span class="n">${n}</span>` : ""}</span>
        <button type="button" class="plus" data-wsplus="${ws.id}" title="New workspace" aria-label="New workspace" aria-haspopup="menu">${icon("plus", 12)}</button></div>`;
      ws.fresh = false;
    }
    wsEl.innerHTML = html;
    // Branches are cut in the middle to fit, and left out when there's no room.
    requestAnimationFrame(() => $$(".ws-row .branch", wsEl).forEach((b) => {
      if (b.scrollWidth <= b.clientWidth + 1) return;
      if (b.clientWidth < 44) b.remove();
      else b.textContent = fitMiddle(b.dataset.full, b.clientWidth, getComputedStyle(b).font);
    }));
  }

  wsEl.addEventListener("click", (e) => {
    tookOver();
    const chip = e.target.closest("[data-overlap]");
    if (chip) { e.stopPropagation(); showOverlap(wsById(chip.dataset.overlap), chip); return; }
    const plus = e.target.closest("[data-wsplus]");
    if (plus) { e.stopPropagation(); showMenu(plus, [[`New Task from ${REPO}…`, () => openNewTask()], ["Open Folder as Workspace…", () => toast({ title: "Opens any folder as another workspace", sub: "This demo has only the trailhead repository.", ms: 2600 })]]); return; }
    const row = e.target.closest("[data-ws]");
    if (row) selectWorkspace(row.dataset.ws);
  });
  wsEl.addEventListener("keydown", (e) => {
    const row = e.target.closest("[data-ws]");
    if (row && e.target === row && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); selectWorkspace(row.dataset.ws); }
  });

  function selectWorkspace(id, activate = true) {
    const ws = wsById(id);
    if (!ws) return;
    S.ws = id;
    closePopover();
    if (!wsTabs(ws).length) newTerminal(ws, false);
    if (!ws.active || !S.tabs.some((t) => t.id === ws.active)) ws.active = wsTabs(ws)[0].id;
    for (const t of S.tabs) t.el.classList.toggle("active", t.id === ws.active);
    renderWorkspaces();
    if (activate) activateTab(ws.active);
  }

  // The list the chip opens: each workspace it overlaps with and the files they share.
  const pop = $("[data-popover]");
  function showOverlap(ws, anchor) {
    if (ws.id !== S.ws) selectWorkspace(ws.id);
    anchor = anchor && anchor.isConnected ? anchor : $(`[data-overlap="${ws.id}"]`);
    const pairs = overlapPairs(ws);
    if (!pairs.length) return;
    pop.innerHTML = pairs.map((p) => `<section><h4>Also changed in ${esc(p.other.name)}</h4><div class="files-list">${p.files.map((f) => `<button type="button" data-pop="open" data-path="${f}" title="Open ${f} in this workspace">${esc(f)}</button>`).join("")}</div>
      <div class="row"><button type="button" class="btn2" data-pop="check" data-other="${p.other.id}">Check for Conflicts</button>${p.other.main || ws.main ? '<button type="button" class="btn2 ghost" data-pop="move">Move to New Task…</button>' : ""}</div></section>`).join("");
    pop.hidden = false;
    const r = (anchor || wsEl).getBoundingClientRect(), w = win.getBoundingClientRect();
    pop.style.left = `${Math.max(8, Math.min(r.right - w.left + 10, w.width - 312))}px`;
    pop.style.top = `${Math.max(46, r.top - w.top - 12)}px`;
    pop.dataset.ws = ws.id;
    if (keyboardOpened) $("button", pop)?.focus({ preventScroll: true });
  }
  function closePopover() { if (pop) pop.hidden = true; }
  pop.addEventListener("click", async (e) => {
    const b = e.target.closest("[data-pop]");
    if (!b) return;
    const ws = wsById(pop.dataset.ws);
    if (b.dataset.pop === "open") { closePopover(); openFile(b.dataset.path, 1, true, ws); }
    else if (b.dataset.pop === "move") { closePopover(); openNewTask({ move: true }); }
    else if (b.dataset.pop === "check") {
      const other = wsById(b.dataset.other), section = b.closest("section");
      const files = sharedFiles(ws, other);
      b.outerHTML = `<span class="result">${ring(10)} Checking…</span>`;
      await sleep(700);
      const bad = conflicting(ws, other, files);
      $(".result", section).outerHTML = `<span class="result ${bad.length ? "bad" : "ok"}">${bad.length ? `${bad.length} would conflict.` : "These would merge without conflicts."}</span>`;
      bad.forEach((f) => { const fb = $(`[data-path="${f}"]`, section); if (fb) { fb.classList.add("conflict"); fb.insertAdjacentHTML("afterbegin", icon("triangle-alert", 10)); } });
    }
  });

  function removeWorkspace(ws) {
    const snapshot = { ws };
    for (const tab of S.tabs.filter((t) => t.ws === ws.id)) { const a = tab.term && agentOf(tab.term); if (a) endAgent(a); tab.term?.cancelRun(); tab.el.remove(); }
    S.tabs = S.tabs.filter((t) => t.ws !== ws.id);
    S.workspaces = S.workspaces.filter((w) => w !== ws);
    if (S.ws === ws.id) selectWorkspace(S.workspaces[0].id);
    checkOverlaps(false);
    refreshAll();
    return snapshot;
  }
  function restoreWorkspace({ ws }) {
    S.workspaces.push(Object.assign(ws, { active: null, fresh: true, attention: 0 }));
    selectWorkspace(ws.id);
    checkOverlaps(false);
    refreshAll();
  }

  // ── Chrome: titlebar, status bar ──────────────────────────────────────

  function updateChrome() {
    const ws = curWs();
    const t = totals(ws);
    const crumb = $("[data-crumb]");
    crumb.innerHTML = `<button type="button" class="ws-name" data-act="workspaces" title="Switch workspace (⌃⌘O)">${esc(ws.name)}</button><button type="button" class="tb-branch" title="Switch branch (⌃⌘B)" data-act="branch">${icon("git-branch", 11)}<span data-full="${esc(ws.branch)}">${esc(ws.branch)}</span>${ws.ahead ? `<span class="sync">↑${ws.ahead}</span>` : ""}</button>`;
    requestAnimationFrame(() => {
      const label = $(".tb-branch span[data-full]", crumb), lead = $(".tb-lead");
      if (!label || !lead || !lead.clientWidth) return;
      const room = lead.getBoundingClientRect().right - 8 - label.getBoundingClientRect().left - ($(".sync", crumb)?.offsetWidth || 0) - 4;
      if (room < 36) $(".tb-branch", crumb).hidden = true;
      else label.textContent = fitMiddle(ws.branch, room, getComputedStyle(label).font);
    });
    $("[data-act='sidebar']").setAttribute("aria-pressed", String(!win.classList.contains("no-sidebar")));
    const pill = $("[data-changes]");
    pill.hidden = !t.files;
    pill.innerHTML = `<span class="vh">Review changes:</span>${icon("file-diff", 12)}<span class="n">${t.files}</span>${stat(t.add, t.del)}`;
    const agents = $("[data-agents]");
    const waiting = S.agents.filter((a) => a.state === "done" || a.state === "waiting").length, working = S.agents.filter((a) => a.state === "working").length;
    agents.hidden = !S.agents.length;
    agents.innerHTML = `<span class="vh">Agents: ${working} working${waiting ? "," : ""}</span><span class="${waiting ? "bell" : ""}">${waiting ? icon("bell-dot", 13) : icon("bot", 13)}</span>${working ? `${ring(10)}<span class="n" aria-hidden="true">${working}</span>` : ""}${waiting ? `<span class="waiting">${waiting}<span class="wl"> waiting</span></span>` : ""}`;
    renderWorkspaces();
    updateStatus();
  }

  function updateStatus() {
    const ws = curWs(), t = totals(ws), tab = activeTab();
    const editor = tab?.kind === "editor";
    let left = `<button type="button" class="si" data-act="branch" title="Switch branch (⌃⌘B)">${icon("git-branch", 11.5)}${esc(ws.branch)}</button>`;
    if (t.files) left += `<button type="button" class="si" data-act="review" title="Review changes (⇧⌘G)">${icon("file-diff", 11.5)}${t.files} ${stat(t.add, t.del)}</button>`;
    if (ws.port) left += `<button type="button" class="si" data-act="port" title="Listening on ${ws.port}: open in browser">${icon("globe", 11.5)}:${ws.port}</button>`;
    if (!editor) left += `<span class="si hide-sm" title="${esc(ws.root)}">${icon("folder", 11.5)}${esc(tab?.term ? tab.term.longCwd() : ws.root)}</span>`;
    let right = "";
    if (editor) right = `<span class="si hide-sm">Ln ${tab.line}, Col ${tab.col + 1}</span><span class="si hide-sm">Spaces: 2</span><span class="si hide-sm">UTF-8</span><span class="si">${icon("code", 11.5)}${langOf(tab.path).name}</span>`;
    else if (tab?.kind === "terminal") right = `<span class="si">${icon("terminal", 11.5)}${SHELL}</span>`;
    $("[data-status]").innerHTML = `${left}<span class="grow"></span>${right}`;
  }

  $("[data-agents]").addEventListener("click", (e) => {
    const items = S.agents.map((a) => [`${a.name} · ${a.ws.name} — ${TURN_STATUS[a.state](elapsed(Date.now() - a.since))}`, () => activateTab(a.term.tab.id)]);
    showMenu(e.currentTarget, items.length ? items : [["No agents", null]], true);
  });

  function refreshAll() {
    if (!S.ready) return;
    const ws = curWs();
    for (const tab of S.tabs) {
      if (tab.kind === "terminal") { tab.term.renderChips(); const a = agentOf(tab.term); if (a) renderBelt(a); }
      else if (tab.id === ws.active) tab.kind === "editor" ? renderEditor(tab) : tab.render();
    }
    checkOverlaps();
    renderTree();
    renderTabs();
    updateChrome();
  }

  // ── Toasts, menus and confirmations ──────────────────────────────────

  const toasts = $("[data-toasts]");
  function toast({ title, sub, action, ms = 3000, kind = "info" }) {
    const ico = { info: icon("info", 14), success: icon("circle-check", 14), warning: icon("triangle-alert", 14) }[kind];
    const el = h(`<div class="toast ${kind}" role="status"><span class="t-ico">${ico}</span><div class="t-body"><div class="t-title">${esc(title)}</div>${sub ? `<div class="t-sub">${esc(sub)}</div>` : ""}</div>${action ? `<button type="button" class="btn2">${esc(action.label)}</button>` : ""}<button type="button" class="tb-icon" data-x title="Dismiss" aria-label="Dismiss">${icon("x", 11)}</button></div>`);
    const close = () => { el.classList.add("out"); setTimeout(() => el.remove(), 220); };
    if (action) el.querySelector(".btn2").addEventListener("click", () => { action.run(); close(); });
    el.querySelector("[data-x]").addEventListener("click", close);
    toasts.appendChild(el);
    while (toasts.children.length > 3) toasts.firstElementChild.remove();
    setTimeout(close, ms);
  }

  const menu = $("[data-menu]");
  function showMenu(anchor, items, alignRight = false) {
    menu.innerHTML = items.map(([label, run], i) => `<button type="button" role="menuitem" data-i="${i}"${run ? "" : ' disabled style="opacity:.45;cursor:default"'}>${esc(label)}</button>`).join("");
    menu.hidden = false;
    menu.setAttribute("role", "menu");
    const r = anchor.getBoundingClientRect(), w = win.getBoundingClientRect();
    const width = menu.offsetWidth;
    menu.style.left = `${Math.max(8, Math.min(alignRight ? r.right - w.left - width : r.left - w.left, w.width - width - 8))}px`;
    menu.style.top = `${Math.min(r.bottom - w.top + 4, w.height - menu.offsetHeight - 8)}px`;
    menu.onclick = (e) => { const b = e.target.closest("[data-i]"); if (!b || b.disabled) return; menu.hidden = true; items[Number(b.dataset.i)][1]?.(); };
    $("button:not([disabled])", menu)?.focus({ preventScroll: true });
  }
  menu.addEventListener("keydown", (e) => {
    const all = $$("button:not([disabled])", menu), i = all.indexOf(document.activeElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); all[(i + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length]?.focus(); }
  });

  const sheetWrap = $("[data-sheet]");
  function openSheet(html, onKey) {
    sheetWrap.innerHTML = html;
    sheetWrap.hidden = false;
    const dialog = $(".sheet", sheetWrap);
    dialog.addEventListener("keydown", (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onKey?.("Escape"); return; }
      if (e.key === "Tab") {
        const f = $$("input:not([disabled]), select, button:not([disabled])", dialog);
        if (e.shiftKey && document.activeElement === f[0]) { e.preventDefault(); f[f.length - 1].focus(); }
        else if (!e.shiftKey && document.activeElement === f[f.length - 1]) { e.preventDefault(); f[0].focus(); }
      }
    });
    return dialog;
  }
  function closeSheet() { sheetWrap.hidden = true; sheetWrap.innerHTML = ""; restoreFocus(); }
  function confirmSheet(title, message, ok) {
    return new Promise((resolve) => {
      const done = (v) => { closeSheet(); resolve(v); };
      const d = openSheet(`<div class="sheet" role="alertdialog" aria-modal="true" aria-labelledby="cf-title" style="width:min(380px, calc(100% - 24px))"><h3 id="cf-title">${esc(title)}</h3><p class="lede">${esc(message)}</p><div class="acts"><button type="button" class="btn2" data-cf="no">Cancel</button><button type="button" class="btn2 primary" data-cf="yes">${esc(ok)}</button></div></div>`, () => done(false));
      d.addEventListener("click", (e) => { const b = e.target.closest("[data-cf]"); if (b) done(b.dataset.cf === "yes"); });
      $("[data-cf='yes']", d).focus();
    });
  }
  function restoreFocus() {
    const tab = activeTab();
    if (touch) return;
    if (tab?.kind === "terminal") tab.term.focus();
    else if (tab?.kind === "editor") $(".ed", tab.el).focus({ preventScroll: true });
    else win.focus({ preventScroll: true });
  }

  // ── New Task (the sheet, and what Create Task does) ───────────────────

  // How the app makes a branch name from a title (tasks.md).
  function branchName(title) {
    let s = title.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().replace(/[^a-z0-9/]+/g, "-").replace(/^-+/, "").slice(0, 48).replace(/[-/]+$/, "");
    if (!s) s = "task";
    const taken = new Set(["main", "feature/forecast-cache", ...S.workspaces.map((w) => w.branch), ...(S.removedBranches || [])]);
    let name = s, n = 2;
    while (taken.has(name)) name = `${s}-${n++}`;
    return name;
  }

  const nextSlot = () => Math.max(0, ...S.workspaces.map((w) => w.slot)) + 1;

  function openNewTask({ title = "", move = false } = {}) {
    tookOver();
    closePalette(false);
    const main = S.workspaces.find((w) => w.main);
    const dirty = changes(main).length;
    const local = main.branch, tracked = `origin/${main.branch}`;
    const agents = [["", "Just a terminal"], ["claude", "Claude Code"], ["codex", "Codex"], ["gemini", "Gemini CLI"]];
    const d = openSheet(`<div class="sheet" role="dialog" aria-modal="true" aria-labelledby="nt-title">
      <h3 id="nt-title">New Task</h3>
      <p class="lede">A branch checked out in its own folder, opened as a workspace — work on it (or let an agent) without touching your current checkout.</p>
      <div class="grid">
        <label class="l" for="nt-task">Task</label><input id="nt-task" type="text" autocomplete="off" spellcheck="false" value="${esc(title)}">
        <label class="l" for="nt-from">From</label><div><input id="nt-from" class="from" type="text" autocomplete="off" spellcheck="false" value="${move ? local : tracked}"><div class="note" data-nt="unpushed" style="font-size:11px;color:var(--text3);margin-top:4px"></div></div>
        <label class="l" for="nt-start">Start</label><div><select id="nt-start">${agents.map(([v, n]) => `<option value="${v}"${v === "claude" ? " selected" : ""}>${n}</option>`).join("")}</select></div>
        ${dirty ? `<div class="move"><label><input type="checkbox" id="nt-move"${move ? " checked" : ""}> <span>Move ${esc(main.name)}'s ${plural(dirty, "uncommitted file")} into this task</span></label><div class="note" data-nt="move-note"></div><button type="button" class="link" data-nt="start-from">Start from ${esc(local)}</button></div>` : ""}
      </div>
      <dl class="preview"><dt>Branch</dt><dd data-nt="branch">—</dd><dt>Folder</dt><dd data-nt="folder">—</dd><dt>Copies</dt><dd>${IGNORED_IN_TASKS}</dd><dt>Values</dt><dd>PORT ${3000 + nextSlot() * 100}</dd></dl>
      <div class="acts"><button type="button" class="btn2" data-nt="cancel">Cancel</button><button type="button" class="btn2 primary" data-nt="create" disabled>${icon("git-branch-plus", 13)}Create Task</button></div>
    </div>`, () => closeSheet());
    const f = { task: $("#nt-task", d), from: $("#nt-from", d), start: $("#nt-start", d), move: $("#nt-move", d) };
    const update = () => {
      const branch = f.task.value.trim() ? branchName(f.task.value) : null;
      $("[data-nt='branch']", d).textContent = branch || "—";
      $("[data-nt='folder']", d).textContent = branch ? `~/Code/${REPO}.worktrees/${branch.replace(/\//g, "-")}` : "—";
      $("[data-nt='create']", d).disabled = !branch;
      const from = f.from.value.trim();
      $("[data-nt='unpushed']", d).textContent = from === tracked && main.ahead ? `${local} has ${plural(main.ahead, "unpushed commit")} that aren't included; type ${local} to include them.` : "";
      if (f.move) {
        const onHead = from === local;
        f.move.disabled = !onHead;
        if (!onHead) f.move.checked = false;
        f.move.closest("label").classList.toggle("off", !onHead);
        $("[data-nt='move-note']", d).textContent = onHead ? "" : `The changes were made on ${main.name}'s current commit, so From has to be that commit.`;
        $("[data-nt='start-from']", d).hidden = onHead;
      }
    };
    d.addEventListener("input", update);
    d.addEventListener("click", (e) => {
      const b = e.target.closest("[data-nt]");
      if (!b) return;
      if (b.dataset.nt === "cancel") closeSheet();
      else if (b.dataset.nt === "start-from") { f.from.value = local; update(); f.move.focus(); }
      else if (b.dataset.nt === "create" && !b.disabled) createTask({ title: f.task.value, from: f.from.value.trim() || "HEAD", start: f.start.value, move: !!f.move?.checked });
    });
    f.task.addEventListener("keydown", (e) => { if (e.key === "Enter" && f.task.value.trim()) { e.preventDefault(); createTask({ title: f.task.value, from: f.from.value.trim() || "HEAD", start: f.start.value, move: !!f.move?.checked }); } });
    update();
    f.task.focus();
    if (!touch) f.task.select();
  }

  function createTask({ title, from, start, move }) {
    closeSheet();
    const main = S.workspaces.find((w) => w.main);
    const branch = branchName(title), folder = branch.replace(/\//g, "-");
    const slot = nextSlot();
    const ws = makeWorkspace({ name: folder, folder, branch, base: from, slot, fresh: true, commits: from === "origin/main" ? [...MAIN_LOG] : [["6b31fc3", "", "TTL cache for forecast lookups"], ...MAIN_LOG] });
    let moved = null;
    if (move) {
      moved = { work: { ...main.work }, index: { ...main.index }, files: changes(main).map((f) => f.path) };
      for (const p of moved.files) ws.work[p] = current(main, p);
      main.work = {};
      main.index = {};
    }
    S.workspaces.push(ws);
    const tab = newTerminal(ws, false);
    selectWorkspace(ws.id);
    tab.term.run(start ? `npm ci && ${start}` : "npm ci");
    if (moved) toast({ title: `Started task ${branch} with ${plural(moved.files.length, "file")} from ${main.name}.`, kind: "success", action: { label: "Undo", run: () => { main.work = moved.work; main.index = moved.index; removeWorkspace(ws); } }, ms: 9000 });
    else toast({ title: `Started task ${branch}.`, kind: "success", ms: 3200 });
    refreshAll();
  }

  // ── Command palette ───────────────────────────────────────────────────

  const overlay = $("[data-overlay]");
  const palInput = $("[data-pal-input]");
  const palList = $("[data-pal-list]");
  const palFoot = $("[data-pal-foot]");
  const palIcon = $("[data-pal-icon]");
  const P = { items: [], sel: 0, recent: [] };

  const MODES = {
    "": { icon: icon("file", 15), placeholder: "Go to file…  (type > for commands, ? for help)", enter: "open" },
    ">": { icon: icon("command", 15), placeholder: "Run a command…", enter: "run" },
    ":": { icon: icon("corner-down-left", 15), placeholder: "Go to line, e.g. 42 or 42:7", enter: "open" },
    "%": { icon: icon("search", 15), placeholder: "Search text in project…", enter: "open" },
    "a:": { icon: icon("play", 15), placeholder: "Run a project action…", enter: "open" },
    "w:": { icon: icon("folder-git-2", 15), placeholder: "Switch to workspace or open a folder…", enter: "open" },
    "b:": { icon: icon("git-branch", 15), placeholder: "Switch to branch…", enter: "switch" },
    "h:": { icon: icon("history", 15), placeholder: "Search history…  @here @repo @failed @today", enter: "insert" },
    "?": { icon: icon("info", 15), placeholder: "Palette modes", enter: "open" },
  };
  function modeOf(raw) {
    for (const p of ["a:", "w:", "b:", "h:", ">", ":", "%", "?"]) if (raw.startsWith(p)) return [p, raw.slice(p.length).trimStart()];
    return ["", raw];
  }

  const PAL_COMMANDS = () => [
    { label: "New Tab", sub: "Tabs", glyph: icon("plus", 15), key: "⌘T", words: ["terminal", "shell"], run: () => newTerminal() },
    { label: "Close Tab", sub: "Tabs", glyph: icon("x", 15), key: "⌘W", run: () => closeTab(curWs().active) },
    { label: "Toggle Sidebar", sub: "Navigation", glyph: icon("panel-left", 15), key: "⌘B", words: ["files"], run: () => toggleSidebar() },
    { label: "Review Changes", sub: "Navigation", glyph: icon("file-diff", 15), key: "⇧⌘G", words: ["git", "diff"], run: () => openReview() },
    { label: "Go to File…", sub: "Navigation", glyph: icon("file", 15), key: "⌘P", keep: true, run: () => openPalette("") },
    { label: "New Task…", sub: "Workspaces", glyph: icon("git-branch-plus", 15), key: "⌥⌘N", words: ["worktree", "branch"], run: () => openNewTask() },
    { label: "Finish Task…", sub: "Workspaces", glyph: icon("git-merge", 15), words: ["worktree", "merge", "land"], run: () => openFinish() },
    { label: "Switch Workspace…", sub: "Workspaces", glyph: icon("folder-git-2", 15), key: "⌃⌘O", keep: true, run: () => openPalette("w:") },
    { label: "Run Project Action…", sub: "Workspaces", glyph: icon("play", 15), key: "⌃⌘R", keep: true, run: () => openPalette("a:") },
    { label: "Next Agent Needing You", sub: "Agents", glyph: icon("bot", 15), key: "⇧⌘U", run: () => nextAgent() },
    { label: "Review Last Agent Turn", sub: "Agents", glyph: icon("file-diff", 15), key: "⇧⌘I", run: () => reviewLastTurn() },
  ];
  // The project's actions (.impulse/project.toml).
  const ACTIONS = [["dev", "npm run dev"], ["test", "npm test"], ["lint", "npm run lint"]];

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
  const mark = (text, idx = []) => [...text].map((c, i) => (idx.includes(i) ? `<mark>${esc(c)}</mark>` : esc(c))).join("");
  const rank = (list, q, key = (x) => x.label) => (q ? list.map((x) => ({ ...x, m: fuzzy(q, key(x)) })).filter((x) => x.m).sort((a, b) => b.m.score - a.m.score) : list);

  function palItems() {
    const [mode, q] = modeOf(palInput.value);
    const ws = curWs();
    if (mode === ">") {
      const all = PAL_COMMANDS();
      if (!q) return [...all].sort((a, b) => (P.recent.indexOf(b.label) - P.recent.indexOf(a.label)) || a.label.localeCompare(b.label));
      const byTitle = rank(all, q);
      const rest = all.filter((c) => !byTitle.includes(c) && !byTitle.some((x) => x.label === c.label) && fuzzy(q, [c.sub, ...(c.words || [])].join(" "))).map((c) => ({ ...c, m: { idx: [] } }));
      return [...byTitle, ...rest];
    }
    if (mode === "a:") return rank(ACTIONS.map(([name, cmd]) => ({ label: name, sub: cmd, glyph: icon("play", 15), run: () => runInTerminal(cmd) })), q);
    if (mode === "w:") return rank(S.workspaces.map((w) => ({ label: w.name, sub: w.main ? `${w.branch} · main checkout` : `${w.branch} · task`, glyph: icon("folder-git-2", 15), run: () => selectWorkspace(w.id) })), q);
    if (mode === "b:") return rank([...new Set(["main", ...S.workspaces.map((w) => w.branch)])].sort().map((b) => ({ label: b, sub: b === ws.branch ? "current" : S.workspaces.find((w) => w.branch === b) ? "in a task" : "", glyph: icon("git-branch", 15), run: () => { const owner = S.workspaces.find((w) => w.branch === b); if (owner) selectWorkspace(owner.id); else toast({ title: `In Impulse this switches ${ws.name} to ${b}`, sub: "The demo stays on its branches.", ms: 2600 }); } })), q);
    if (mode === "h:") return rank([...S.history].reverse().map((c) => ({ label: c, glyph: icon("history", 15), run: () => { const t = terminalTab(); t.term.setInput(c); t.term.focus(); } })), q);
    if (mode === "?") return rank([["", "Files"], [">", "Commands"], [":", "Go to line"], ["%", "Text in files"], ["a:", "Project actions"], ["w:", "Workspaces"], ["b:", "Branches"], ["h:", "Command history"]].map(([p, label]) => ({ label, sub: p || "(no prefix)", glyph: MODES[p].icon, keep: true, run: () => openPalette(p) })), q);
    if (mode === ":") {
      const tab = activeTab();
      const [line, col] = q.split(":").map(Number);
      if (tab?.kind !== "editor") return [];
      return line ? [{ label: `Go to line ${line}${col ? `, column ${col}` : ""}`, sub: tab.path, glyph: icon("corner-down-left", 15), run: () => { tab.line = line; tab.col = Math.max(0, (col || 1) - 1); renderEditor(tab, true); $(".ed", tab.el).focus({ preventScroll: true }); } }] : [];
    }
    if (mode === "%") {
      if (q.length < 2) return [];
      const hits = [];
      for (const p of paths(ws)) splitLines(current(ws, p) || "").forEach((l, i) => { const at = l.toLowerCase().indexOf(q.toLowerCase()); if (at >= 0 && hits.length < 60) hits.push({ label: l.trim(), sub: `${p}:${i + 1}`, glyph: fileIcon(p.split("/").pop()), m: { idx: [] }, run: () => openFile(p, i + 1) }); });
      return hits;
    }
    const open = wsTabs(ws).filter((t) => t.kind === "editor").map((t) => t.path);
    const files = paths(ws).filter((p) => current(ws, p) != null && !isIgnored(p));
    const pool = q ? files : open.length ? open : files;
    return rank(pool.map((p) => ({ label: p.split("/").pop(), sub: p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "", path: p, glyph: fileIcon(p.split("/").pop()), run: () => openFile(p, 1) })), q, (x) => x.path).map((x) => ({ ...x, m: x.m && { idx: x.m.idx.map((i) => i - (x.path.length - x.label.length)).filter((i) => i >= 0) } }));
  }

  function renderPalette() {
    const [mode] = modeOf(palInput.value);
    palIcon.innerHTML = MODES[mode].icon;
    palInput.placeholder = MODES[mode].placeholder;
    P.items = palItems();
    P.sel = Math.min(P.sel, Math.max(0, P.items.length - 1));
    palList.innerHTML = P.items.map((it, i) => `<div class="dpal-item" role="option" id="pal-${i}" data-i="${i}" aria-selected="${i === P.sel}"><span class="gl">${it.glyph || ""}</span><span class="lbl">${mark(it.label, it.m?.idx)}</span>${it.sub ? `<span class="sub">${esc(it.sub)}</span>` : ""}${it.key ? `<kbd>${it.key}</kbd>` : ""}</div>`).join("") || `<div class="dpal-empty">${mode === ":" && activeTab()?.kind !== "editor" ? "Open a file first." : "No matches"}</div>`;
    palInput.setAttribute("aria-activedescendant", P.items.length ? `pal-${P.sel}` : "");
    palList.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
    palFoot.innerHTML = `<span class="h"><kbd>↑↓</kbd>navigate</span><span class="h"><kbd>⏎</kbd>${MODES[mode].enter}</span><span class="h"><kbd>esc</kbd>close</span><span class="grow"></span><span class="modes">&gt; commands   : line   % text   b: branches   w: workspaces   h: history   ? more</span>`;
  }

  function openPalette(prefix = ">") {
    tookOver();
    closePopover();
    menu.hidden = true;
    overlay.hidden = false;
    palInput.value = prefix;
    P.sel = 0;
    renderPalette();
    palInput.focus({ preventScroll: true });
  }

  function closePalette(focus = true) {
    if (overlay.hidden) return;
    overlay.hidden = true;
    if (focus) restoreFocus();
  }

  function choose(i) {
    const it = P.items[i];
    if (!it) return;
    if (it.keep) { it.run(); return; }
    const [mode] = modeOf(palInput.value);
    if (mode === ">") P.recent = [...P.recent.filter((l) => l !== it.label), it.label];
    closePalette(false);
    it.run();
  }

  palInput.addEventListener("input", () => { P.sel = 0; renderPalette(); });
  palInput.addEventListener("keydown", (e) => {
    const step = e.key === "ArrowDown" || (e.ctrlKey && e.key === "n") ? 1 : e.key === "ArrowUp" || (e.ctrlKey && e.key === "p") ? -1 : 0;
    if (step) {
      e.preventDefault();
      if (!P.items.length) return;
      P.sel = (P.sel + step + P.items.length) % P.items.length;
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
    if (tab.term.busy || agentOf(tab.term)) { const t = newTerminal(); t.term.run(cmd); return; }
    tab.term.run(cmd);
    if (!touch) tab.term.focus();
  }
  function nextAgent() {
    const a = S.agents.find((x) => x.state === "waiting") || S.agents.find((x) => x.state === "done") || S.agents[0];
    if (a) activateTab(a.term.tab.id); else toast({ title: "No agent needs you", ms: 1800 });
  }
  function reviewLastTurn() {
    const ws = curWs().turn ? curWs() : S.workspaces.find((w) => w.turn);
    if (ws) openReview(ws, "turn"); else toast({ title: "No agent turn to review yet", sub: "Run claude in a terminal and give it something to do.", ms: 2600 });
  }

  // ── Themes ────────────────────────────────────────────────────────────

  function buildThemeStrip() {
    const strip = document.querySelector("[data-theme-strip]");
    if (!strip) return;
    const ids = Object.keys(THEMES).sort((a, b) => (a === "nord" ? -1 : b === "nord" ? 1 : THEMES[a].name.localeCompare(THEMES[b].name)));
    strip.innerHTML = ids.map((id) => {
      const c = THEMES[id].colors;
      return `<button type="button" class="theme-chip" role="radio" aria-checked="false" data-theme="${id}" style="background:${c.bg};color:${c.fg}"><span class="sw">${["accent", "blue", "magenta", "green", "red"].map((k) => `<i style="background:${c[k]}"></i>`).join("")}</span>${esc(THEMES[id].name)}${id === "nord" ? '<span style="opacity:.6;font-weight:400"> · default</span>' : ""}</button>`;
    }).join("");
    strip.addEventListener("click", (e) => {
      const chip = e.target.closest(".theme-chip");
      if (!chip) return;
      tookOver();
      applyTheme(chip.dataset.theme);
    });
  }

  // ── Window keys and buttons ───────────────────────────────────────────

  function toggleSidebar() { win.classList.toggle("no-sidebar"); updateChrome(); }

  win.addEventListener("keydown", (e) => {
    const mod = e.metaKey || e.ctrlKey, k = e.key.toLowerCase();
    let handled = true;
    if (mod && e.shiftKey && k === "p") overlay.hidden ? openPalette(">") : closePalette();
    else if (e.metaKey && !e.shiftKey && !e.altKey && !e.ctrlKey && k === "p") openPalette("");
    else if (mod && !e.shiftKey && !e.altKey && k === "b") toggleSidebar();
    else if (mod && e.shiftKey && k === "g") openReview();
    else if (mod && e.shiftKey && k === "u") nextAgent();
    else if (mod && e.shiftKey && k === "i") reviewLastTurn();
    else if (mod && e.altKey && e.code === "KeyN") openNewTask();
    else if (e.ctrlKey && e.metaKey && e.code === "KeyO") openPalette("w:");
    else if (e.ctrlKey && e.metaKey && e.code === "KeyR") openPalette("a:");
    else if (e.key === "Escape" && (!menu.hidden || !pop.hidden)) { menu.hidden = true; closePopover(); restoreFocus(); }
    else if (e.key === "Escape" && !overlay.hidden) closePalette();
    else handled = false;
    if (handled) { e.preventDefault(); e.stopPropagation(); }
  });

  document.addEventListener("mousedown", (e) => {
    if (!menu.hidden && !menu.contains(e.target)) menu.hidden = true;
    if (!pop.hidden && !pop.contains(e.target) && !e.target.closest("[data-overlap]")) closePopover();
  });

  document.addEventListener("click", (e) => {
    const act = e.target.closest("[data-act]");
    if (act) {
      tookOver();
      const a = act.dataset.act;
      const outside = !win.contains(act);
      if (outside) scrollDemoIntoView();
      if (a === "sidebar") toggleSidebar();
      else if (a === "new-tab") newTerminal();
      else if (a === "palette") openPalette(">");
      else if (a === "review") openReview();
      else if (a === "new-task") openNewTask();
      else if (a === "finish") openFinish(curWs().main ? S.workspaces.find((w) => !w.main) || curWs() : curWs());
      else if (a === "workspaces") openPalette("w:");
      else if (a === "branch") toast({ title: "Switch branch (⌃⌘B)", sub: `Lists ${curWs().name}'s branches. For a second branch at once, start a task instead (⌥⌘N).`, ms: 3200 });
      else if (a === "port") toast({ title: `Would open localhost:${curWs().port}`, sub: "In Impulse, the status bar opens listening ports in your browser.", ms: 2400 });
      return;
    }
    const dock = e.target.closest("[data-dock]");
    if (dock) {
      tookOver();
      const d = dock.dataset.dock;
      if (d === "changes") toast({ title: "The Changes panel", sub: "Stage and commit from the sidebar. Not in this demo: try Review (⇧⌘G).", ms: 3000 });
      else if (d === "search") openPalette("%");
      else if (d === "collapse") { S.open.clear(); renderTree(); }
      else if (d === "refresh") renderTree();
      else toast({ title: "The demo’s project is read-only", ms: 2000 });
      return;
    }
    const tryBtn = e.target.closest("[data-try]");
    if (tryBtn) {
      tookOver();
      scrollDemoIntoView();
      if (curWs().id !== "trailhead" && wsById("trailhead")) selectWorkspace("trailhead");
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
    $(".ib-editor", term.pane).classList.add("coach");
    toast({ title: "Your turn", sub: "Type a command, try claude to fix that test, or open the fix-elevation task in the sidebar.", ms: 7000 });
  }

  // ── Start ─────────────────────────────────────────────────────────────

  // index.html names its icons with data-icon (the build inlines those too).
  $$("[data-icon]", document).forEach((el) => (el.outerHTML = icon(el.dataset.icon, Number(el.dataset.size) || 14)));
  buildThemeStrip();
  let saved = null;
  try { saved = localStorage.getItem("impulse-demo-theme"); } catch { /* ignore */ }

  const trailhead = makeWorkspace({ name: REPO, branch: "feature/forecast-cache", main: true, ahead: 2, work: pick("work"), index: { "src/lib/cache.ts": FILES["src/lib/cache.ts"].work }, commits: [...MAIN_LOG] });
  delete trailhead.work["src/lib/cache.ts"];
  const elevation = makeWorkspace({ name: "fix-elevation", branch: "fix-elevation", base: "origin/main", slot: 1, head: pick("fixElevation"), commits: [["7c2e1f4", "", "Fall back to a nearby trailhead elevation"], ...MAIN_LOG] });
  S.workspaces.push(trailhead, elevation);
  S.ws = trailhead.id;
  applyTheme(saved && THEMES[saved] ? saved : "nord");

  // fix-elevation: Claude Code finished its turn there and committed it.
  elevation.seed = async (agent) => {
    const out = (l) => agent.blk.print(l, false);
    out('<span class="t-cyan">&gt;</span> Some trails have no trailhead elevation, so the forecast page shows "undefined m". Fall back to the nearest known trailhead in the same region, and commit it.');
    out("");
    out('<span class="t-green">●</span> Read <span class="t-dim">src/trails.ts (31 lines)</span>');
    out('<span class="t-green">●</span> Update <span class="t-dim">src/trails.ts</span>');
    out("  <span class=\"t-dim\">⎿ Added trailheadElevation(trail): the trail's own elevation, else the first known one in the same region</span>");
    out('<span class="t-green">●</span> Update <span class="t-dim">src/server.ts</span>');
    out("  <span class=\"t-dim\">⎿ GET /trails/:id fills in trailheadElevationM</span>");
    out('<span class="t-green">●</span> Bash <span class="t-dim">git commit -am "Fall back to a nearby trailhead elevation"</span>');
    out("");
    out("Done. Trails without an elevation now borrow one from their region; Annette Lake gets 205 m from Mount Si's trailhead.");
    const time = new Date(Date.now() - 3 * 60e3).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    agent.turns.push({ n: 1, time });
    elevation.turn = { n: 1, time, agent: agent.name, before: { "src/trails.ts": BASE["src/trails.ts"], "src/server.ts": BASE["src/server.ts"] }, after: { "src/trails.ts": FILES["src/trails.ts"].fixElevation, "src/server.ts": FILES["src/server.ts"].fixElevation } };
    out("");
  };
  const elevationTerm = newTerminal(elevation, false);
  elevationTerm.term.run("claude", { instant: true });

  const main = newTerminal(trailhead, false);
  openFile("src/forecast.ts", 1, false, trailhead);
  activateTab(main.id, false);
  main.term.run("git log --oneline --graph", { instant: true });
  checkOverlaps(false);
  S.ready = true;
  refreshAll();

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
