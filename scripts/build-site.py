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
MONACO_FIXTURES = ROOT / "impulse-macos/Tests/ImpulseKitTests/Fixtures/themes"
LUCIDE = ROOT / "vendor/lucide/icons"
MATERIAL = ROOT / "assets/icons/material"
DEFAULT_OUT = WEBSITE / "_site"

REPO_URL = "https://github.com/dowilcox/impulse"
# The site lives on www; Cloudflare redirects the bare impulse-terminal.app to it.
SITE_URL = "https://www.impulse-terminal.app"
DOMAIN = "www.impulse-terminal.app"
# Written into every build; the output folder is only ever emptied when it
# holds this file, so --out can't wipe an unrelated folder.
MARKER = ".impulse-site"


# ── The app's themes and icons, for the demo ────────────────────────────


def _hsl(hex_color: str) -> tuple[float, float, float]:
    """rgbToHsl in ThemeStore.swift: (hue in degrees, saturation, lightness)."""
    h = hex_color.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))
    hi, lo = max(r, g, b), min(r, g, b)
    light = (hi + lo) / 2
    if hi == lo:
        return 0.0, 0.0, light
    d = hi - lo
    sat = d / (2 - hi - lo) if light > 0.5 else d / (hi + lo)
    if hi == r:
        hue = (g - b) / d + (6 if g < b else 0)
    elif hi == g:
        hue = (b - r) / d + 2
    else:
        hue = (r - g) / d + 4
    return hue * 60, sat, light


def _hex_from_hsl(hue: float, sat: float, light: float) -> str:
    """hslToRgb + rgbToHex in ThemeStore.swift (rounding half away from zero)."""
    def byte(v: float) -> int:
        return int(min(255, max(0, int(v * 255 + 0.5))))
    if sat == 0:
        return "#%02x%02x%02x" % ((byte(light),) * 3)
    q = light * (1 + sat) if light < 0.5 else light + sat - light * sat
    p = 2 * light - q

    def channel(t: float) -> float:
        t = t + 1 if t < 0 else t - 1 if t > 1 else t
        if t < 1 / 6:
            return p + (q - p) * 6 * t
        if t < 1 / 2:
            return q
        if t < 2 / 3:
            return p + (q - p) * (2 / 3 - t) * 6
        return p
    h = hue / 360
    return "#%02x%02x%02x" % (byte(channel(h + 1 / 3)), byte(channel(h)), byte(channel(h - 1 / 3)))


def _shift_lightness(hex_color: str, delta: float) -> str:
    hue, sat, light = _hsl(hex_color)
    return _hex_from_hsl(hue, sat, min(1.0, max(0.0, light + delta)))


def _mute(hex_color: str, saturation: float, lightness_target: float) -> str:
    hue, sat, light = _hsl(hex_color)
    return _hex_from_hsl(hue, sat * saturation, light + (lightness_target - light) * 0.3)


def resolve_theme(t: dict) -> dict:
    """ThemeStore.resolveTheme (ImpulseKit): a theme file with every default
    filled in, keyed like the app's theme fixtures."""
    p, ui, syn = t.get("palette", {}), t.get("ui", {}), t.get("syntax", {})
    light = t.get("variant") == "light"
    fg_muted = ui.get("fg_muted") or p.get("muted") or _mute(p["fg"], 0.6, 0.55)
    fg_comment = ui.get("fg_comment") or p.get("subtle") or _mute(p["fg"], 0.4, 0.60 if light else 0.40)
    r = {
        "is_light": light,
        "bg": p["bg"],
        "bg_dark": ui.get("bg_dark") or _shift_lightness(p["bg"], 0.04 if light else -0.05),
        "bg_highlight": ui.get("bg_highlight") or _shift_lightness(p["bg"], -0.05 if light else 0.08),
        "bg_surface": ui.get("bg_surface") or p.get("surface") or _shift_lightness(p["bg"], 0.08 if light else -0.10),
        "border": ui.get("border") or p.get("overlay") or _shift_lightness(p["bg"], -0.08 if light else 0.04),
        "fg": p["fg"],
        "fg_muted": fg_muted,
        "fg_comment": fg_comment,
        "accent": p["accent"],
        "selection": ui.get("selection") or p["accent"] + "40",
        "cursor": ui.get("cursor") or p["accent"],
        **{k: p[k] for k in ("red", "orange", "yellow", "green", "cyan", "blue", "magenta")},
        "git_added": ui.get("git_added") or p["green"],
        "git_modified": ui.get("git_modified") or p["yellow"],
        "git_deleted": ui.get("git_deleted") or p["red"],
        "git_renamed": ui.get("git_renamed") or p["blue"],
        "git_conflict": ui.get("git_conflict") or p["orange"],
        "git_ignored": ui.get("git_ignored") or fg_muted,
    }
    defaults = {"keyword": p["magenta"], "function": p["blue"], "type": p["yellow"], "string": p["green"],
                "number": p["orange"], "constant": p["orange"], "comment": fg_comment, "operator": p["cyan"],
                "tag": p["red"], "attribute": p["yellow"], "variable": p["fg"], "delimiter": fg_muted,
                "escape": p["orange"], "regexp": p["red"], "link": p["blue"]}
    for key, default in defaults.items():
        r["syntax_" + key] = syn.get(key) or default
    return r


# themeToMonaco (ImpulseKit/Themes/MonacoTheme.swift): Monaco token -> the
# resolved color it gets, in the app's rule order.
MONACO_RULES = [
    ("comment", "syntax_comment", "italic"), ("comment.doc", "syntax_comment", "italic"),
    *[(k, "syntax_keyword", None) for k in (
        "keyword", "keyword.control", "keyword.declaration", "keyword.type", "keyword.other", "keyword.flow",
        "keyword.block", "keyword.try", "keyword.catch", "keyword.choice", "keyword.modifier")],
    ("keyword.constant", "syntax_constant", None),
    *[(k, "syntax_number", None) for k in ("number", "number.hex", "number.float", "number.binary", "number.octal")],
    ("constant", "syntax_constant", None), ("string.escape", "syntax_escape", None),
    *[(k, "syntax_string", None) for k in ("string", "string.heredoc", "string.raw", "attribute.value")],
    ("string.key", "syntax_operator", None), ("string.link", "syntax_link", None),
    *[(k, "syntax_operator", None) for k in ("operator", "keyword.operator", "variable.predefined", "predefined")],
    ("type", "syntax_type", None), ("type.identifier", "syntax_type", None), ("class", "syntax_type", None),
    ("annotation", "syntax_attribute", None), ("namespace", "syntax_type", None), ("constructor", "syntax_type", None),
    ("attribute.name", "syntax_attribute", None),
    *[(k, "syntax_function", None) for k in ("function", "function.declaration", "function.call", "predefined.function")],
    ("string.escape.invalid", "syntax_tag", None), ("string.invalid", "syntax_tag", None),
    ("regexp", "syntax_regexp", None), ("tag", "syntax_tag", None), ("metatag", "syntax_tag", None),
    ("invalid", "syntax_tag", None), ("variable", "syntax_variable", None), ("emphasis", "syntax_variable", "italic"),
    ("delimiter", "syntax_delimiter", None), ("strong", "orange", "bold"),
]


def theme_to_monaco(r: dict) -> dict:
    """themeToMonaco, serialized the way the app's fixtures are."""
    def strip(c: str) -> str:
        return c.lstrip("#")
    rules = []
    for token, key, style in MONACO_RULES:
        rule = {"foreground": strip(r[key]), "token": token}
        if style:
            rule["font_style"] = style
        rules.append(rule)
    colors = {
        "editor.background": r["bg"], "editor.foreground": r["fg"],
        "editor.lineHighlightBackground": r["bg_highlight"], "editor.selectionBackground": r["selection"],
        "editorCursor.foreground": r["cursor"], "editorLineNumber.foreground": r["fg_comment"],
        "editorLineNumber.activeForeground": r["fg"], "editorWidget.background": r["bg_dark"],
        "editorSuggestWidget.background": r["bg_dark"], "editorSuggestWidget.selectedBackground": r["bg_highlight"],
        "editorHoverWidget.background": r["bg_dark"], "editorGutter.background": r["bg"],
        "minimap.background": r["bg_dark"], "scrollbarSlider.background": r["fg_comment"] + "40",
        "scrollbarSlider.hoverBackground": r["fg_comment"] + "80",
        "scrollbarSlider.activeBackground": r["fg_comment"] + "A0",
        "impulse.diffAddedColor": r["git_added"], "impulse.diffModifiedColor": r["git_modified"],
        "impulse.diffDeletedColor": r["git_deleted"],
    }
    return {"base": "vs" if r["is_light"] else "vs-dark", "colors": {k: "#" + strip(v) for k, v in colors.items()},
            "inherit": True, "rules": rules}


def material_file_icon(name: str, mapping: dict) -> str:
    """IconCache.fileIcon in FileIcons.swift: exact name, then extension, then file.svg."""
    lower = name.lower()
    candidates = [mapping["file_stems"].get(lower)]
    if "." in lower:
        candidates.append(mapping["file_suffixes"].get(lower.rsplit(".", 1)[1]))
    for icon in candidates:
        if icon in mapping["file_icons"]:
            return mapping["file_icons"][icon]
    return "file.svg"


def material_folder_icons(name: str, mapping: dict) -> list[str]:
    """IconCache.directoryIcon: [collapsed, expanded] for a folder name."""
    normalized = name.lower().lstrip("._").rstrip("_")
    default = [mapping["directory_icons"]["collapsed"], mapping["directory_icons"]["expanded"]]
    return mapping["named_directories"].get(normalized, default)


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
        self._write_demo_icons()
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
            resolved = resolve_theme(t)
            monaco = theme_to_monaco(resolved)
            self._check_monaco_fixture(path.stem, monaco)
            themes[path.stem] = {
                "name": t.get("name", path.stem),
                "variant": t.get("variant", "dark"),
                # What the app resolves and hands Monaco (ThemeStore.resolveTheme
                # and themeToMonaco in ImpulseKit), so the demo's editor and
                # chrome use the same colors as the app in every theme.
                "colors": {k: v for k, v in resolved.items() if k != "is_light"},
                "terminal": t.get("terminal", {}),
                "monaco": {
                    "rules": [[r["token"], "#" + r["foreground"]] + ([r["font_style"]] if "font_style" in r else [])
                              for r in monaco["rules"]],
                    "colors": monaco["colors"],
                },
            }
        data = json.dumps(themes, separators=(",", ":"))
        self._write("assets/themes.js", f"window.IMPULSE_THEMES={data};\n")

    def _check_monaco_fixture(self, theme_id: str, monaco: dict) -> None:
        """The app's golden fixture for this theme's Monaco theme is the spec;
        a port that drifts from it fails the build."""
        fixture = MONACO_FIXTURES / f"{theme_id}.monaco.json"
        if not fixture.exists():
            return
        expected = json.loads(fixture.read_text(encoding="utf-8"))
        if expected != monaco:
            self.errors.append(f"{fixture.relative_to(ROOT)}: the site's Monaco theme for {theme_id} doesn't match it "
                               "(update theme_to_monaco in scripts/build-site.py)")

    def _write_demo_icons(self) -> None:
        """The icons the demo window draws, inlined: the Lucide icons the app's
        chrome uses (vendor/lucide, the ones demo.js names with icon("…")) and
        the Material file and folder icons the app's file tree would show for
        the demo project (assets/icons/material, chosen the way FileIcons.swift
        chooses them). index.html names icons with data-icon="…"."""
        demo_js = (WEBSITE / "assets/demo.js").read_text(encoding="utf-8")
        index = (WEBSITE / "index.html").read_text(encoding="utf-8")
        wanted = set(re.findall(r'\bicon\("([a-z0-9-]+)"', demo_js)) | set(re.findall(r'data-icon="([a-z0-9-]+)"', index))
        for line_no, line in enumerate(demo_js.splitlines(), 1):
            for call in re.finditer(r'(?<!function )\bicon\((?!")', line):
                if "el.dataset.icon" not in line[call.end():call.end() + 20]:
                    self.errors.append(f"website/assets/demo.js:{line_no}: icon() needs a literal name, or the build can't include it")
        lucide = {}
        for name in sorted(wanted):
            svg_path = LUCIDE / f"{name}.svg"
            if not svg_path.exists():
                self.errors.append(f"website/: the demo uses a Lucide icon ({name}) that vendor/lucide/icons doesn't have")
                continue
            svg = svg_path.read_text(encoding="utf-8")
            lucide[name] = re.sub(r"\s*\n\s*", "", re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S).group(1))

        mapping = json.loads((MATERIAL / "icon-mapping.json").read_text(encoding="utf-8"))
        files_js = (WEBSITE / "assets/demo-files.js").read_text(encoding="utf-8")
        names, dirs = set(), set()
        for path in json.loads(files_js[files_js.index("{"):files_js.rindex("}") + 1]):
            *folder_names, name = path.split("/")
            names.add(name)
            dirs.update(folder_names)
        files = {name: material_file_icon(name, mapping) for name in sorted(names)}
        folders = {name: material_folder_icons(name, mapping) for name in sorted(dirs)}
        # Any other folder, and any other file.
        folders[""] = [mapping["directory_icons"]["collapsed"], mapping["directory_icons"]["expanded"]]
        files[""] = "file.svg"
        used = set(files.values()) | {svg for pair in folders.values() for svg in pair}
        material = {svg: re.sub(r"\s*\n\s*", " ", (MATERIAL / svg).read_text(encoding="utf-8").strip())
                    for svg in sorted(used)}
        data = json.dumps({"lucide": lucide, "material": material, "files": files, "folders": folders},
                          separators=(",", ":"))
        self._write("assets/demo-icons.js", f"window.IMPULSE_ICONS={data};\n")

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
    paths += [MATERIAL / "icon-mapping.json"]
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
    status = f"{len(site.errors)} problem(s)" if site.errors else "all links OK"
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
