#!/usr/bin/env python3
"""build-site.py — build impulse-terminal.app (the homepage and the docs).

The website is static and generated: the homepage and 404 page come from
website/*.html, the documentation pages from docs/*.md, the changelog page
from CHANGELOG.md, and the interactive demo's themes from the app's own theme
files. Nothing generated is committed; GitHub Actions
(.github/workflows/website.yml) runs this script on every push to main that
touches its inputs and publishes the result to GitHub Pages.

The docs sidebar follows docs/README.md: each `## Section` with a bullet list
of links to pages becomes a group, in the same order. A page that README
doesn't list still builds, under "More". Links between pages
(`git.md#merge-conflicts`), to screenshots (`images/x.png`) and to other
files in the repository are rewritten for the site, and every one of them is
checked: a missing page, anchor or image is reported, and with --check the
build fails.

Usage:
  python3 scripts/build-site.py              # build into website/_site
  python3 scripts/build-site.py --check      # also fail on broken links (CI)
  python3 scripts/build-site.py --serve      # build, serve on :8000 and
                                             # rebuild + reload on every change
  python3 scripts/build-site.py --out DIR    # build somewhere else

Needs Python 3.9+ and `pip install -r website/requirements.txt`.
"""

from __future__ import annotations

import argparse
import html
import http.server
import json
import os
import posixpath
import re
import shutil
import sys
import threading
import time
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

try:
    import tomllib
except ImportError:  # Python < 3.11 (macOS's /usr/bin/python3)
    import tomli as tomllib

try:
    from markdown_it import MarkdownIt
    from markdown_it.token import Token
except ImportError:
    sys.exit("build-site.py needs markdown-it-py: pip install -r website/requirements.txt")

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
WEBSITE = ROOT / "website"
TEMPLATES = WEBSITE / "templates"
THEMES = ROOT / "impulse-macos/Sources/ImpulseKit/Resources/Themes"
DEFAULT_OUT = WEBSITE / "_site"

REPO_URL = "https://github.com/dowilcox/impulse"
# The site lives on www; Cloudflare redirects the bare impulse-terminal.app to it.
SITE_URL = "https://www.impulse-terminal.app"
DOMAIN = "www.impulse-terminal.app"
# Written into every build; the output folder is only ever emptied when it
# holds this file, so --out can't wipe an unrelated folder.
MARKER = ".impulse-site"


# ── Markdown ────────────────────────────────────────────────────────────


def github_slug(text: str) -> str:
    """The anchor GitHub gives a heading, so links written for GitHub work."""
    text = text.strip().lower()
    out = []
    for ch in text:
        if ch in " -":
            out.append("-")
        elif ch == "_" or unicodedata.category(ch)[0] in "LN":
            out.append(ch)
    return "".join(out)


def inline_text(token: Token) -> str:
    """Plain text of an inline token (what GitHub slugs and what search indexes)."""
    parts = []
    for child in token.children or []:
        if child.type in ("text", "code_inline"):
            parts.append(child.content)
        elif child.type in ("softbreak", "hardbreak"):
            parts.append(" ")
        elif child.type == "image":
            parts.append(child.content)
    return "".join(parts)


@dataclass
class Page:
    source: Path  # the markdown file
    out: str  # output path relative to the site root, e.g. "docs/git/index.html"
    url: str  # site-root-relative URL, e.g. "docs/git/"
    title: str = ""
    description: str = ""
    html: str = ""
    toc: list[tuple[int, str, str]] = field(default_factory=list)  # (level, id, text)
    ids: set[str] = field(default_factory=set)
    links: list[tuple[str, str]] = field(default_factory=list)  # (kind, target)
    sections: list[dict] = field(default_factory=list)  # search index entries


def make_markdown() -> MarkdownIt:
    md = MarkdownIt("commonmark", {"html": False, "typographer": False})
    md.enable(["table", "strikethrough"])
    return md


class Renderer:
    """Renders one markdown file for the site, recording anchors and links."""

    def __init__(self, site: "Site"):
        self.site = site
        self.md = make_markdown()

    def render(self, page: Page) -> None:
        text = page.source.read_text(encoding="utf-8")
        tokens = self.md.parse(text)
        self._headings(page, tokens)
        self._rewrite_links(page, tokens)
        self._index(page, tokens)
        page.html = self._render_tokens(tokens)

    # Headings: GitHub ids (deduplicated with -1, -2…), a title, a TOC.
    def _headings(self, page: Page, tokens: list[Token]) -> None:
        seen: dict[str, int] = {}
        for i, tok in enumerate(tokens):
            if tok.type != "heading_open":
                continue
            text = inline_text(tokens[i + 1])
            slug = github_slug(text)
            if slug in seen:
                seen[slug] += 1
                slug = f"{slug}-{seen[slug]}"
            else:
                seen[slug] = 0
            tok.attrSet("id", slug)
            page.ids.add(slug)
            level = int(tok.tag[1])
            if level == 1 and not page.title:
                page.title = text
            elif level in (2, 3):
                page.toc.append((level, slug, text))
        for i, tok in enumerate(tokens):
            if tok.type == "paragraph_open" and not page.description:
                inline = tokens[i + 1]
                if not any(c.type == "image" for c in inline.children or []):
                    page.description = re.sub(r"\s+", " ", inline_text(inline)).strip()

    def _rewrite_links(self, page: Page, tokens: list[Token]) -> None:
        for tok in tokens:
            for child in tok.children or []:
                if child.type == "link_open":
                    href = child.attrGet("href") or ""
                    new, external = self.site.resolve_link(page, href)
                    child.attrSet("href", new)
                    if external:
                        child.attrSet("rel", "noopener")
                elif child.type == "image":
                    src = child.attrGet("src") or ""
                    child.attrSet("src", self.site.resolve_image(page, src))

    # Search: one entry per h2/h3 section, text truncated.
    def _index(self, page: Page, tokens: list[Token]) -> None:
        current = {"h": page.title, "id": "", "t": []}
        sections = [current]
        for i, tok in enumerate(tokens):
            if tok.type == "heading_open" and tok.tag in ("h2", "h3"):
                current = {"h": inline_text(tokens[i + 1]), "id": tok.attrGet("id"), "t": []}
                sections.append(current)
            elif tok.type == "inline" and tokens[i - 1].type != "heading_open":
                current["t"].append(inline_text(tok))
            elif tok.type in ("fence", "code_block"):
                current["t"].append(tok.content)
        for s in sections:
            body = re.sub(r"\s+", " ", " ".join(s["t"])).strip()
            if not body and not s["id"]:
                continue
            url = page.url + (f"#{s['id']}" if s["id"] else "")
            page.sections.append({"p": page.title, "h": s["h"], "u": url, "t": body[:600]})

    def _render_tokens(self, tokens: list[Token]) -> str:
        r = self.md.renderer
        rules = r.rules

        def heading_open(tokens, idx, options, env):
            tok = tokens[idx]
            slug = tok.attrGet("id")
            if tok.tag == "h1":
                return f'<h1 id="{slug}">'
            return (
                f'<{tok.tag} id="{slug}">'
                f'<a class="anchor" href="#{slug}" aria-label="Link to this section">#</a>'
            )

        def fence(tokens, idx, options, env):
            tok = tokens[idx]
            lang = (tok.info or "").strip().split(" ")[0]
            label = {"sh": "shell", "bash": "shell", "text": "", "": ""}.get(lang, lang)
            hl = {"sh": "bash", "fish": "bash", "text": "plaintext", "": "plaintext"}.get(lang, lang)
            code = html.escape(tok.content)
            return (
                '<div class="codeblock">'
                f'<div class="codeblock-bar"><span>{html.escape(label)}</span>'
                '<button class="copy" type="button" aria-label="Copy code">Copy</button></div>'
                f'<pre><code class="language-{html.escape(hl)}">{code}</code></pre></div>\n'
            )

        def table_open(tokens, idx, options, env):
            return '<div class="table-wrap"><table>\n'

        def table_close(tokens, idx, options, env):
            return "</table></div>\n"

        def image(tokens, idx, options, env):
            tok = tokens[idx]
            alt = html.escape(inline_text(tok) if tok.children else tok.content)
            src = html.escape(tok.attrGet("src") or "")
            return (
                f'<a class="shot" href="{src}"><img src="{src}" alt="{alt}" '
                'loading="lazy" decoding="async"></a>'
            )

        rules["heading_open"] = heading_open
        rules["fence"] = fence
        rules["table_open"] = table_open
        rules["table_close"] = table_close
        rules["image"] = image
        return r.render(tokens, self.md.options, {})


# ── Site ────────────────────────────────────────────────────────────────


def rel(from_out: str, to: str) -> str:
    """Relative URL from the page written at `from_out` to site path `to`."""
    base = posixpath.dirname(from_out) or "."
    if to.endswith("/"):
        r = posixpath.relpath(to.rstrip("/") or ".", base)
        return "./" if r == "." else r + "/"
    return posixpath.relpath(to, base)


class Site:
    def __init__(self, out: Path, check: bool, live_reload: bool = False):
        self.out = out
        self.check = check
        self.live_reload = live_reload
        self.errors: list[str] = []
        self.version = (ROOT / "VERSION").read_text().strip()
        self.pages: dict[Path, Page] = {}
        self.nav: list[tuple[str, list[tuple[Path, str]]]] = []
        self.images: set[Path] = set()

    # Pages and their order ------------------------------------------------

    def collect(self) -> None:
        for path in sorted(DOCS.glob("*.md")):
            if path.name == "README.md":
                self.pages[path] = Page(path, "docs/index.html", "docs/")
            else:
                slug = path.stem
                self.pages[path] = Page(path, f"docs/{slug}/index.html", f"docs/{slug}/")
        changelog = ROOT / "CHANGELOG.md"
        self.pages[changelog] = Page(changelog, "changelog/index.html", "changelog/")
        self.nav = self._nav_from_readme()

    def _nav_from_readme(self) -> list[tuple[str, list[tuple[Path, str]]]]:
        tokens = make_markdown().parse((DOCS / "README.md").read_text(encoding="utf-8"))
        groups: list[tuple[str, list[tuple[Path, str]]]] = [("", [(DOCS / "README.md", "Overview")])]
        listed = {DOCS / "README.md"}
        heading = None
        in_list = 0
        for i, tok in enumerate(tokens):
            if tok.type == "heading_open" and tok.tag == "h2":
                heading = inline_text(tokens[i + 1])
            elif tok.type == "bullet_list_open":
                in_list += 1
            elif tok.type == "bullet_list_close":
                in_list -= 1
            elif tok.type == "inline" and in_list and heading:
                kids = tok.children or []
                if not kids or kids[0].type != "link_open":
                    continue
                href = (kids[0].attrGet("href") or "").split("#")[0]
                target = (DOCS / href).resolve()
                if target.suffix != ".md" or target not in self.pages or target in listed:
                    continue
                text = []
                for c in kids[1:]:
                    if c.type == "link_close":
                        break
                    text.append(c.content)
                if not groups or groups[-1][0] != heading:
                    groups.append((heading, []))
                groups[-1][1].append((target, "".join(text)))
                listed.add(target)
        rest = [p for p in self.pages if p.parent == DOCS and p not in listed]
        if rest:
            groups.append(("More", [(p, "") for p in rest]))
        return groups

    def ordered(self) -> list[Path]:
        return [p for _, items in self.nav for p, _ in items]

    # Links ---------------------------------------------------------------

    def resolve_link(self, page: Page, href: str) -> tuple[str, bool]:
        parts = urlsplit(href)
        if parts.scheme or href.startswith("//"):
            return href, True
        if not parts.path:  # same-page anchor
            if parts.fragment:
                page.links.append(("anchor", f"{page.source}#{parts.fragment}"))
            return href, False
        target = (page.source.parent / parts.path).resolve()
        frag = f"#{parts.fragment}" if parts.fragment else ""
        if target in self.pages:
            page.links.append(("anchor" if frag else "page", f"{target}{frag}"))
            return rel(page.out, self.pages[target].url) + frag, False
        if target.suffix in (".png", ".jpg", ".jpeg", ".gif", ".svg") and DOCS in target.parents:
            return self.resolve_image(page, href), False
        try:
            repo_path = target.relative_to(ROOT).as_posix()
        except ValueError:
            self.error(page, f"link outside the repository: {href}")
            return href, False
        if not target.exists():
            self.error(page, f"link to a missing file: {href}")
        kind = "tree" if target.is_dir() else "blob"
        return f"{REPO_URL}/{kind}/main/{repo_path}{frag}", True

    def resolve_image(self, page: Page, src: str) -> str:
        if urlsplit(src).scheme:
            return src
        target = (page.source.parent / src).resolve()
        if not target.is_file():
            self.error(page, f"missing image: {src}")
            return src
        if DOCS in target.parents:
            site_path = "docs/" + target.relative_to(DOCS).as_posix()
        else:
            site_path = "assets/repo/" + target.relative_to(ROOT).as_posix()
        self.images.add(target)
        return rel(page.out, site_path)

    def verify_links(self) -> None:
        for page in self.pages.values():
            for kind, target in page.links:
                path, _, frag = target.partition("#")
                dest = self.pages.get(Path(path))
                if dest is None:
                    self.error(page, f"link to a missing page: {target}")
                elif frag and frag not in dest.ids:
                    name = dest.source.relative_to(ROOT)
                    self.error(page, f"link to a missing anchor: {name}#{frag}")

    def error(self, page: Page, message: str) -> None:
        self.errors.append(f"{page.source.relative_to(ROOT)}: {message}")

    # Output ----------------------------------------------------------------

    def build(self) -> None:
        self.collect()
        renderer = Renderer(self)
        for page in self.pages.values():
            renderer.render(page)
        self.verify_links()

        self._prepare_out()
        template = (TEMPLATES / "doc.html").read_text(encoding="utf-8")
        order = self.ordered()
        for path, page in self.pages.items():
            self._write(page.out, self._doc_page(template, page, order))
        for src in sorted(WEBSITE.glob("*.html")):
            # The 404 page is served at whatever URL was missing, so it links
            # from the site root instead of relatively.
            extra = {"root": "/"} if src.name == "404.html" else {}
            self._write(src.name, self._fill(src.read_text(encoding="utf-8"), src.name, extra))
        self._copy_assets()
        self._write_search_index()
        self._write_themes()
        self._write("CNAME", DOMAIN + "\n")
        self._write(".nojekyll", "")
        self._write("sitemap.xml", self._sitemap())
        self._write("robots.txt", f"User-agent: *\nAllow: /\nSitemap: {SITE_URL}/sitemap.xml\n")
        self._write(MARKER, "built by scripts/build-site.py\n")

    def _prepare_out(self) -> None:
        if self.out.exists():
            if any(self.out.iterdir()) and not (self.out / MARKER).exists():
                sys.exit(f"{self.out} isn't empty and wasn't built by this script; not touching it.")
            # Empty it in place (a server may be serving the folder itself).
            for child in self.out.iterdir():
                if child.is_dir() and not child.is_symlink():
                    shutil.rmtree(child)
                else:
                    child.unlink()
        self.out.mkdir(parents=True, exist_ok=True)

    def _write(self, rel_path: str, content: str) -> None:
        dest = self.out / rel_path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(content, encoding="utf-8")

    def _fill(self, template: str, out: str, values: dict[str, str]) -> str:
        values = {
            "root": "../" * out.count("/"),
            "version": html.escape(self.version),
            "repo": REPO_URL,
            "site_url": SITE_URL,
            "year": time.strftime("%Y"),
            **values,
        }
        values.setdefault("home", values["root"] or "./")
        # Partials (website/templates/_name.html) go in first, so their own
        # {{placeholders}} are filled with the page's values.
        for partial in sorted(TEMPLATES.glob("_*.html")):
            template = template.replace("{{%s}}" % partial.stem[1:], partial.read_text(encoding="utf-8"))
        result = re.sub(r"\{\{(\w+)\}\}", lambda m: values.get(m.group(1), m.group(0)), template)
        if self.live_reload:
            result = result.replace("</body>", LIVE_RELOAD + "</body>")
        return result

    def _doc_page(self, template: str, page: Page, order: list[Path]) -> str:
        values = {
            "title": html.escape(page.title),
            "description": html.escape(page.description[:200]),
            "canonical": f"{SITE_URL}/{page.url}",
            "content": page.html,
            "nav": self._nav_html(page),
            "toc": self._toc_html(page),
            "pager": self._pager_html(page, order),
            "edit_url": f"{REPO_URL}/edit/main/{page.source.relative_to(ROOT).as_posix()}",
            "section": "changelog" if page.out.startswith("changelog") else "docs",
        }
        return self._fill(template, page.out, values)

    def _nav_html(self, page: Page) -> str:
        out = []
        for heading, items in self.nav:
            out.append('<div class="nav-group">')
            if heading:
                out.append(f'<div class="nav-heading">{html.escape(heading)}</div>')
            for path, label in items:
                target = self.pages[path]
                label = label or target.title
                current = ' aria-current="page"' if target is page else ""
                href = rel(page.out, target.url)
                out.append(f'<a class="nav-item" href="{href}"{current}>{html.escape(label)}</a>')
            out.append("</div>")
        return "\n".join(out)

    def _toc_html(self, page: Page) -> str:
        if len(page.toc) < 2:
            return ""
        items = "\n".join(
            f'<a class="toc-l{level}" href="#{slug}">{html.escape(text)}</a>'
            for level, slug, text in page.toc
        )
        return f'<nav class="toc" aria-label="On this page"><div class="toc-title">On this page</div>{items}</nav>'

    def _pager_html(self, page: Page, order: list[Path]) -> str:
        if page.source not in order:
            return ""
        i = order.index(page.source)
        links = []
        if i > 0:
            prev = self.pages[order[i - 1]]
            links.append(
                f'<a class="pager-prev" href="{rel(page.out, prev.url)}"><span>Previous</span>'
                f"{html.escape(prev.title)}</a>"
            )
        if i + 1 < len(order):
            nxt = self.pages[order[i + 1]]
            links.append(
                f'<a class="pager-next" href="{rel(page.out, nxt.url)}"><span>Next</span>'
                f"{html.escape(nxt.title)}</a>"
            )
        return f'<nav class="pager">{"".join(links)}</nav>'

    def _copy_assets(self) -> None:
        shutil.copytree(WEBSITE / "assets", self.out / "assets", dirs_exist_ok=True)
        fonts = self.out / "assets/fonts"
        fonts.mkdir(parents=True, exist_ok=True)
        for name in ("Inter-Regular", "Inter-Medium", "Inter-SemiBold", "Inter-Bold"):
            shutil.copy2(ROOT / f"vendor/fonts/inter/{name}.ttf", fonts)
        for name in ("JetBrainsMono-Regular", "JetBrainsMono-Bold"):
            shutil.copy2(ROOT / f"vendor/fonts/jetbrains-mono/{name}.ttf", fonts)
        shutil.copy2(ROOT / "vendor/fonts/inter/OFL.txt", fonts / "Inter-OFL.txt")
        shutil.copy2(ROOT / "vendor/fonts/jetbrains-mono/OFL.txt", fonts / "JetBrainsMono-OFL.txt")
        vendor = self.out / "assets/vendor"
        vendor.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / "vendor/highlight/highlight.min.js", vendor)
        shutil.copy2(ROOT / "vendor/highlight/LICENSE", vendor / "highlight-LICENSE")
        shutil.copy2(ROOT / "assets/impulse-logo.svg", self.out / "assets/impulse-logo.svg")
        shutil.copy2(ROOT / "assets/impulse-logo.svg", self.out / "favicon.svg")
        # Every screenshot (the homepage shows some the docs don't link).
        shutil.copytree(DOCS / "images", self.out / "docs/images", dirs_exist_ok=True)
        for image in self.images:
            if DOCS not in image.parents:
                dest = self.out / "assets/repo" / image.relative_to(ROOT)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(image, dest)

    def _write_search_index(self) -> None:
        entries = []
        for path in self.ordered() + [ROOT / "CHANGELOG.md"]:
            entries.extend(self.pages[path].sections)
        data = json.dumps(entries, ensure_ascii=False, separators=(",", ":"))
        self._write("assets/search-index.js", f"window.IMPULSE_SEARCH={data};\n")

    def _write_themes(self) -> None:
        themes = {}
        for path in sorted(THEMES.glob("*.toml")):
            with path.open("rb") as f:
                t = tomllib.load(f)
            themes[path.stem] = {
                "name": t.get("name", path.stem),
                "variant": t.get("variant", "dark"),
                "palette": t.get("palette", {}),
                "ui": {k: v for k, v in t.get("ui", {}).items() if isinstance(v, str)},
                "syntax": t.get("syntax", {}),
                "terminal": t.get("terminal", {}),
            }
        data = json.dumps(themes, separators=(",", ":"))
        self._write("assets/themes.js", f"window.IMPULSE_THEMES={data};\n")

    def _sitemap(self) -> str:
        urls = [f"{SITE_URL}/"] + [f"{SITE_URL}/{p.url}" for p in self.pages.values()]
        body = "\n".join(f"  <url><loc>{html.escape(u)}</loc></url>" for u in urls)
        return (
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
            f"{body}\n</urlset>\n"
        )


# ── Serve and watch ─────────────────────────────────────────────────────

LIVE_RELOAD = """<script>
(() => { let v; setInterval(async () => {
  try { const r = await fetch('/__build', {cache: 'no-store'}); const t = await r.text();
        if (v && t !== v) location.reload(); v = t; } catch {}
}, 700); })();
</script>
"""


def inputs() -> list[Path]:
    paths = [ROOT / "VERSION", ROOT / "CHANGELOG.md", ROOT / "assets/impulse-logo.svg", Path(__file__)]
    paths += list(DOCS.rglob("*"))
    paths += [p for p in WEBSITE.rglob("*") if DEFAULT_OUT not in p.parents and p != DEFAULT_OUT]
    paths += list(THEMES.glob("*.toml"))
    return [p for p in paths if p.is_file()]


def snapshot() -> dict[Path, float]:
    return {p: p.stat().st_mtime for p in inputs()}


def build_once(out: Path, check: bool, live_reload: bool = False) -> bool:
    started = time.time()
    site = Site(out, check, live_reload)
    site.build()
    for e in site.errors:
        print(f"  ✗ {e}", file=sys.stderr)
    pages = len(site.pages)
    took = (time.time() - started) * 1000
    status = f"{len(site.errors)} broken link(s)" if site.errors else "all links OK"
    print(f"Built {pages} pages into {out} in {took:.0f} ms, {status}.")
    return not site.errors


def serve(out: Path, port: int) -> None:
    build_id = {"n": str(time.time())}

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=str(out), **kw)

        def do_GET(self):
            if self.path == "/__build":
                body = build_id["n"].encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            super().do_GET()

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"Serving http://127.0.0.1:{port}/  (rebuilds when docs/, website/ or the themes change; ⌃C stops)")
    seen = snapshot()
    try:
        while True:
            time.sleep(0.5)
            now = snapshot()
            if now != seen:
                seen = now
                try:
                    build_once(out, check=False, live_reload=True)
                except Exception as e:  # keep serving through a bad edit
                    print(f"  ✗ build failed: {e}", file=sys.stderr)
                build_id["n"] = str(time.time())
    except KeyboardInterrupt:
        server.shutdown()


def main() -> None:
    parser = argparse.ArgumentParser(description="Build the Impulse website and docs.")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT, help="output folder (default website/_site)")
    parser.add_argument("--check", action="store_true", help="exit 1 on broken links, anchors or images")
    parser.add_argument("--serve", action="store_true", help="serve the site and rebuild on changes")
    parser.add_argument("--port", type=int, default=8000)
    args = parser.parse_args()
    out = args.out.resolve()

    ok = build_once(out, args.check, live_reload=args.serve)
    if args.serve:
        serve(out, args.port)
    elif args.check and not ok:
        sys.exit(1)


if __name__ == "__main__":
    main()
