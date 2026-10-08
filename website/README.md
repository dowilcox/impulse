# impulse-terminal.app

The source of the Impulse website: the homepage with its interactive demo, the
documentation (built from `docs/*.md`) and the changelog (from `CHANGELOG.md`).
Nothing in the built site is committed; `scripts/build-site.py` generates it into
`website/_site/`.

```sh
pip install -r website/requirements.txt
python3 scripts/build-site.py --serve    # http://127.0.0.1:8000, rebuilds and reloads on every change
python3 scripts/build-site.py --check    # one build; fails on a broken link, anchor or image
```

## What's here

| Path                      | What it is                                                                                         |
| ------------------------- | -------------------------------------------------------------------------------------------------- |
| `index.html`              | The homepage. `{{placeholders}}` (version, repo URL, partials) are filled in by the build.          |
| `404.html`                | The page GitHub Pages serves for a missing URL.                                                    |
| `templates/doc.html`      | The layout of every docs page and the changelog.                                                   |
| `templates/_*.html`       | Partials: `_topbar.html` is `{{topbar}}`, `_search.html` is `{{search}}`.                           |
| `assets/site.css`, `site.js` | Shared by every page: colors (Tokyo Night, the app's default theme), the top bar, search, docs. |
| `assets/home.css`, `home.js` | The homepage and its animations.                                                                |
| `assets/demo.js`          | The interactive demo window: a scripted shell, editor, Review, palette and agent.                  |
| `assets/demo-files.js`    | The demo's example project, `trailhead` (the same one `scripts/docs/make_demo.py` builds).         |

The build adds the rest: the fonts and highlight.js from `vendor/`, the logo, the
docs screenshots, `assets/themes.js` (every built-in theme, read from
`impulse-macos/Sources/ImpulseKit/Resources/Themes/`, so the demo's theme picker
always matches the app), `assets/search-index.js`, `sitemap.xml` and `CNAME`.

## The docs

Pages come from `docs/*.md` and the sidebar follows `docs/README.md`: each
`## Section` with a list of links to pages is a group, in that order. To add a
page, write `docs/<name>.md` and link it from a section of `docs/README.md`; it
is published at `/docs/<name>/`. Links between pages, anchors and screenshots
are written the way GitHub renders them (`git.md#merge-conflicts`,
`images/x.png`) and work in both places. Links to other files in the
repository become links to them on GitHub.

## Publishing

`.github/workflows/website.yml` builds the site with `--check` on every push to
`main` that changes `docs/`, `website/`, `CHANGELOG.md`, `VERSION`, the themes
or the build script, and deploys it to GitHub Pages. Pull requests that touch
the docs get the same check without the deploy. It can also be run by hand
from the Actions tab.

One-time setup:

1. In the repository's **Settings ▸ Pages**, set **Source** to **GitHub Actions**.
2. In the same page, set **Custom domain** to `impulse-terminal.app` and, once the
   certificate is issued, turn on **Enforce HTTPS**.
3. In Cloudflare, remove the redirect to the repository and point the domain at
   GitHub Pages: a `CNAME` record for `impulse-terminal.app` (Cloudflare flattens it
   at the apex) and one for `www`, both with the target `dowilcox.github.io`, set to
   **DNS only** until GitHub has issued the certificate.
