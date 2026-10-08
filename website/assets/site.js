// Impulse website: search (styled as the app's command palette), the docs
// sidebar on small screens, "on this page" highlighting, code copy buttons,
// screenshot zoom and code highlighting. Shared by the homepage and the docs.
(() => {
  "use strict";
  const root = window.IMPULSE_ROOT || "";
  const $ = (sel, el = document) => el.querySelector(sel);
  const $$ = (sel, el = document) => [...el.querySelectorAll(sel)];

  // Mark the current section in the top bar.
  const section = document.body.dataset.section;
  if (section) $$(`.topnav [data-section="${section}"]`).forEach((a) => a.setAttribute("aria-current", "page"));

  // ── Search ────────────────────────────────────────────────────────────

  const backdrop = $("[data-search-backdrop]");
  const input = $("[data-search-input]");
  const results = $("[data-search-results]");
  let index = null;
  let selected = 0;
  let lastFocus = null;

  function loadIndex() {
    if (index || window.IMPULSE_SEARCH) return Promise.resolve((index = window.IMPULSE_SEARCH));
    return new Promise((resolve) => {
      const s = document.createElement("script");
      s.src = root + "assets/search-index.js";
      s.onload = () => resolve((index = window.IMPULSE_SEARCH || []));
      s.onerror = () => resolve((index = []));
      document.head.appendChild(s);
    });
  }

  function openSearch(initial = "") {
    if (!backdrop) return;
    lastFocus = document.activeElement;
    backdrop.setAttribute("data-open", "");
    input.value = initial;
    input.focus();
    loadIndex().then(render);
  }

  function closeSearch() {
    if (!backdrop?.hasAttribute("data-open")) return;
    backdrop.removeAttribute("data-open");
    lastFocus?.focus?.();
  }

  const escapeHTML = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const escapeRE = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

  function highlight(text, terms) {
    if (!terms.length) return escapeHTML(text);
    const re = new RegExp(`(${terms.map(escapeRE).join("|")})`, "gi");
    return text.split(re).map((part, i) => (i % 2 ? `<mark>${escapeHTML(part)}</mark>` : escapeHTML(part))).join("");
  }

  function snippet(text, terms) {
    const lower = text.toLowerCase();
    let at = -1;
    for (const t of terms) {
      const i = lower.indexOf(t);
      if (i >= 0 && (at < 0 || i < at)) at = i;
    }
    if (at < 0) return text.slice(0, 150);
    const start = Math.max(0, at - 50);
    return (start ? "…" : "") + text.slice(start, start + 170);
  }

  function search(query) {
    query = query.trim();
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length || !index) return [];
    const scored = [];
    for (const entry of index) {
      const h = entry.h.toLowerCase();
      const p = entry.p.toLowerCase();
      const t = entry.t.toLowerCase();
      const phrase = query.toLowerCase();
      let score = h === phrase ? 80 : h.startsWith(phrase) ? 40 : h.includes(phrase) ? 20 : 0;
      let all = true;
      for (const term of terms) {
        const wordStart = new RegExp(`(^|[^a-z0-9])${escapeRE(term)}`);
        let s = 0;
        if (h === term) s += 60;
        if (wordStart.test(h)) s += 30;
        else if (h.includes(term)) s += 16;
        if (wordStart.test(p)) s += 8;
        if (t.includes(term)) s += 4 + Math.min(4, t.split(term).length - 1);
        if (!s) { all = false; break; }
        score += s;
      }
      if (all) scored.push({ entry, score: score - (entry.u.includes("#") ? 0 : 2) - (entry.u.startsWith("changelog") ? 6 : 0) });
    }
    scored.sort((a, b) => b.score - a.score);
    return scored.slice(0, 30).map((s) => s.entry);
  }

  function render() {
    const query = input.value.trim();
    const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
    const hits = search(query);
    selected = 0;
    results.innerHTML = hits
      .map((e, i) => {
        const same = e.h === e.p;
        return `<a class="palette-item" role="option" href="${root}${e.u}" aria-selected="${i === 0}">
          <span class="pi-title">${highlight(e.h, terms)}${same ? "" : `<span class="pi-page">${escapeHTML(e.p)}</span>`}</span>
          <span class="pi-text">${highlight(snippet(e.t, terms), terms)}</span></a>`;
      })
      .join("");
    results.dataset.empty = query ? `No results for “${query}”.` : "Type to search every page of the docs.";
  }

  function move(delta) {
    const items = $$(".palette-item", results);
    if (!items.length) return;
    items[selected]?.setAttribute("aria-selected", "false");
    selected = (selected + delta + items.length) % items.length;
    items[selected].setAttribute("aria-selected", "true");
    items[selected].scrollIntoView({ block: "nearest" });
  }

  if (backdrop) {
    $$("[data-search-open]").forEach((b) => b.addEventListener("click", () => openSearch()));
    input.addEventListener("input", render);
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); move(1); }
      else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); move(-1); }
      else if (e.key === "Enter") {
        const item = $$(".palette-item", results)[selected];
        if (item) { e.preventDefault(); closeSearch(); location.href = item.href; }
      } else if (e.key === "Escape") { e.preventDefault(); closeSearch(); }
    });
    backdrop.addEventListener("mousedown", (e) => { if (e.target === backdrop) closeSearch(); });
    results.addEventListener("click", () => closeSearch());
    document.addEventListener("keydown", (e) => {
      const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName) || e.target.isContentEditable;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k" && !e.shiftKey && !e.defaultPrevented) {
        e.preventDefault();
        backdrop.hasAttribute("data-open") ? closeSearch() : openSearch();
      } else if (e.key === "/" && !typing && !e.defaultPrevented) {
        e.preventDefault();
        openSearch();
      }
    });
  }

  // ── Docs ──────────────────────────────────────────────────────────────

  // Small screens: the sidebar slides over the page.
  const navToggle = $("[data-nav-toggle]");
  if (navToggle && $(".docs-sidebar")) {
    navToggle.addEventListener("click", () => document.body.toggleAttribute("data-nav-open"));
    $(".docs-sidebar").addEventListener("click", (e) => { if (e.target.closest("a")) document.body.removeAttribute("data-nav-open"); });
    $("[aria-current]", $(".docs-sidebar"))?.scrollIntoView({ block: "center" });
  } else if (navToggle) {
    navToggle.remove();
  }
  const main = $("[data-docs-main]");
  if (main && !$(".toc", main)) main.classList.add("no-toc");

  // On this page: highlight the section being read.
  const tocLinks = $$(".toc a");
  if (tocLinks.length && "IntersectionObserver" in window) {
    const byId = new Map(tocLinks.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
    const headings = [...byId.keys()].map((id) => document.getElementById(id)).filter(Boolean);
    const visible = new Set();
    const update = () => {
      let current = headings.find((h) => visible.has(h));
      if (!current) {
        current = headings.filter((h) => h.getBoundingClientRect().top < 120).pop() || headings[0];
      }
      tocLinks.forEach((a) => a.classList.toggle("active", byId.get(current?.id) === a));
    };
    const io = new IntersectionObserver((entries) => {
      entries.forEach((en) => (en.isIntersecting ? visible.add(en.target) : visible.delete(en.target)));
      update();
    }, { rootMargin: "-60px 0px -65% 0px" });
    headings.forEach((h) => io.observe(h));
  }

  // Code blocks: highlighting and copy buttons.
  const blocks = $$(".codeblock");
  if (blocks.length) {
    blocks.forEach((block) => {
      const button = $(".copy", block);
      button?.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText($("code", block).innerText.replace(/\n$/, ""));
          button.textContent = "Copied";
        } catch {
          button.textContent = "Press ⌘C";
        }
        setTimeout(() => (button.textContent = "Copy"), 1600);
      });
    });
    const s = document.createElement("script");
    s.src = root + "assets/vendor/highlight.min.js";
    s.onload = () => {
      window.hljs.configure({ ignoreUnescapedHTML: true });
      $$(".codeblock code").forEach((code) => {
        if (!code.classList.contains("language-plaintext")) {
          try { window.hljs.highlightElement(code); } catch { /* unknown language: leave it plain */ }
        }
      });
    };
    document.head.appendChild(s);
  }

  // Screenshots open full size over the page.
  const lightbox = $("[data-lightbox]");
  if (lightbox) {
    const img = $("img", lightbox);
    document.addEventListener("click", (e) => {
      const shot = e.target.closest(".shot");
      if (!shot || e.metaKey || e.ctrlKey) return;
      e.preventDefault();
      img.src = shot.href;
      img.alt = $("img", shot)?.alt || "";
      lightbox.setAttribute("data-open", "");
    });
    lightbox.addEventListener("click", () => lightbox.removeAttribute("data-open"));
    document.addEventListener("keydown", (e) => { if (e.key === "Escape") lightbox.removeAttribute("data-open"); });
  }
})();
