#!/usr/bin/env python3
"""Build the mock world the documentation screenshots are taken in.

Creates, under <repo>/target/docs-demo:

  home/                       a stand-in home directory (the app runs with
                              CFFIXED_USER_HOME/HOME pointing here, so it
                              shows ~/Code/trailhead and starts with default
                              settings and an empty command history)
    .zshrc, .gitconfig        a plain zsh and git setup
    bin/claude, codex, gemini stand-in agent CLIs (see agent.sh)
    Code/trailhead            the demo repository (TypeScript trail API)
    Code/trailhead.worktrees/ two task worktrees
  remotes/trailhead.git       its "origin"

The repository, as the docs describe it:

  main                     ... Document the API, Return JSON 404s (origin/main
                           has one more: "Mention lint in the README")
  feature/forecast-cache   checked out; 3 commits off main, 2 not pushed (↑2);
                           staged src/lib/cache.ts, unstaged src/forecast.ts and
                           src/server.ts, untracked src/lib/metrics.ts and
                           test/metrics.test.ts (which fails), one stash
  units-refactor,          merged and stale; origin/trail-search is remote-only
  readme-badges
  tags v0.1.0, v0.2.0
  fix-elevation,           task worktrees in ~/Code/trailhead.worktrees/
  add-trail-photos

Everything is rebuilt from scratch on each run; commit dates are relative to
now so relative times ("3 days ago") read naturally.

usage: make_demo.py [conflict]
  conflict: leave the checkout in the middle of a merge with a conflict in
            src/lib/units.ts (and no other uncommitted work)
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DEMO = (REPO / "target" / "docs-demo").resolve()
HOME = DEMO / "home"
PROJECT = HOME / "Code" / "trailhead"
WORKTREES = HOME / "Code" / "trailhead.worktrees"
REMOTE = DEMO / "remotes" / "trailhead.git"

AUTHORS = {
    "maya": ("Maya Chen", "maya@trailhead.dev"),
    "sam": ("Sam Ortiz", "sam@trailhead.dev"),
    "priya": ("Priya Raman", "priya@trailhead.dev"),
}
NOW = int(time.time())
DAY = 86400


def env_for(author: str, days_ago: float) -> dict:
    name, email = AUTHORS[author]
    stamp = f"{NOW - int(days_ago * DAY)} -0700"
    env = dict(os.environ)
    env.update(
        HOME=str(HOME),
        GIT_CONFIG_GLOBAL=str(HOME / ".gitconfig"),
        GIT_CONFIG_NOSYSTEM="1",
        GIT_AUTHOR_NAME=name,
        GIT_AUTHOR_EMAIL=email,
        GIT_COMMITTER_NAME=name,
        GIT_COMMITTER_EMAIL=email,
        GIT_AUTHOR_DATE=stamp,
        GIT_COMMITTER_DATE=stamp,
    )
    return env


def git(*args, cwd=PROJECT, author="maya", days_ago=0.0, check=True) -> str:
    result = subprocess.run(
        ["git", *args], cwd=cwd, env=env_for(author, days_ago), text=True, capture_output=True
    )
    if check and result.returncode != 0:
        sys.exit(f"git {' '.join(args)} failed in {cwd}:\n{result.stderr}")
    return result.stdout.strip()


def write(path: Path, text: str, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text.lstrip("\n"))
    if mode is not None:
        path.chmod(mode)


def files(root: Path, mapping: dict[str, str]) -> None:
    for name, text in mapping.items():
        write(root / name, text)


def commit(message: str, author: str, days_ago: float, cwd: Path = PROJECT) -> None:
    git("add", "-A", cwd=cwd, author=author, days_ago=days_ago)
    git("commit", "-q", "-m", message, cwd=cwd, author=author, days_ago=days_ago)


def merge(branch: str, author: str, days_ago: float) -> None:
    git("merge", "-q", "--no-ff", "-m", f"Merge branch '{branch}'", branch, author=author, days_ago=days_ago)


# ---------------------------------------------------------------------------
# File contents, by stage

PACKAGE_JSON = """
{
  "name": "trailhead",
  "version": "%s",
  "description": "Trail info and mountain forecasts for hikers",
  "type": "module",
  "scripts": {
    "dev": "node --watch src/server.ts",
    "start": "node src/server.ts",
    "test": "node --test --test-reporter=./scripts/test-reporter.mjs test/",
    "lint": "node --check src/server.ts"
  },
  "engines": { "node": ">=24" }
}
"""

TSCONFIG = """
{
  "compilerOptions": {
    "target": "es2023",
    "module": "nodenext",
    "strict": true,
    "allowImportingTsExtensions": true,
    "noEmit": true
  },
  "include": ["src", "test"]
}
"""

GITIGNORE = """
node_modules/
dist/
.env
config/*.local.json
*.log
"""

LOCAL_CONFIG = """
{
  "logLevel": "debug",
  "forecastTimeoutMs": 4000
}
"""

README_V1 = """
# trailhead

Trail info for hikers: a small HTTP API over a list of trails.

```sh
npm run dev      # http://localhost:3000
npm test
```
"""

README_V2 = """
# trailhead

Trail info and mountain forecasts for hikers: a small HTTP API over a list
of trails, with the weather at each trailhead.

| Route | Returns |
| --- | --- |
| `GET /trails` | Every trail (`?q=` searches names and regions) |
| `GET /trails/:id` | One trail |
| `GET /trails/:id/forecast` | The forecast at the trailhead |
| `GET /health` | `{ "ok": true }` |

## Running it

```sh
npm install
npm run dev
```

The API listens on http://localhost:3000. Forecasts come from the weather
service in `FORECAST_API`; set `FORECAST_KEY` in `.env`.

## Tests

```sh
npm test
```
"""

README_BADGES = README_V1.replace(
    "# trailhead\n",
    "# trailhead\n\n![CI](https://img.shields.io/badge/ci-passing-brightgreen) "
    "![License](https://img.shields.io/badge/license-MIT-blue)\n",
)

TRAILS_V1 = """
export type Trail = {
  id: string;
  name: string;
  region: string;
  lengthKm: number;
  elevationGainM: number;
  trailheadElevationM: number;
};

export const trails: Trail[] = [
  { id: "mt-si", name: "Mount Si", region: "Snoqualmie", lengthKm: 12.9, elevationGainM: 960, trailheadElevationM: 205 },
  { id: "rattlesnake", name: "Rattlesnake Ledge", region: "Snoqualmie", lengthKm: 6.4, elevationGainM: 350, trailheadElevationM: 280 },
  { id: "lake-22", name: "Lake 22", region: "Mountain Loop", lengthKm: 8.7, elevationGainM: 410, trailheadElevationM: 340 },
  { id: "heather", name: "Heather Lake", region: "Mountain Loop", lengthKm: 7.2, elevationGainM: 310, trailheadElevationM: 450 },
  { id: "skyline", name: "Skyline Trail", region: "Mount Rainier", lengthKm: 8.9, elevationGainM: 520, trailheadElevationM: 1645 },
];

export function findTrail(id: string): Trail | undefined {
  return trails.find((trail) => trail.id === id);
}
"""

TRAILS_V2 = TRAILS_V1.replace(
    """export function findTrail""",
    """export function searchTrails(query: string): Trail[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return trails;
  return trails.filter(
    (trail) => trail.name.toLowerCase().includes(needle) || trail.region.toLowerCase().includes(needle),
  );
}

export function findTrail""",
)


# Some trails don't know their trailhead elevation yet.
def with_missing_elevation(text: str) -> str:
    return text.replace(
        "  trailheadElevationM: number;\n};",
        "  /** Unknown for a few community-submitted trails. */\n  trailheadElevationM?: number;\n};",
    ).replace(
        """  { id: "skyline", """,
        """  { id: "annette", name: "Annette Lake", region: "Snoqualmie", lengthKm: 11.9, elevationGainM: 520 },
  { id: "skyline", """,
    )


TRAILS_V3 = with_missing_elevation(TRAILS_V2)

# The fix-elevation task's change (made by the stand-in agent in screenshots).
TRAILS_FIXED = TRAILS_V3.replace(
    """export function findTrail(id: string): Trail | undefined {
  return trails.find((trail) => trail.id === id);
}""",
    """export function findTrail(id: string): Trail | undefined {
  return trails.find((trail) => trail.id === id);
}

/** The trailhead elevation, or the nearest known one in the same region. */
export function trailheadElevation(trail: Trail): number | undefined {
  if (trail.trailheadElevationM !== undefined) return trail.trailheadElevationM;
  const sameRegion = trails.filter(
    (other) => other.region === trail.region && other.trailheadElevationM !== undefined,
  );
  return sameRegion[0]?.trailheadElevationM;
}""",
)

UNITS = """
const KM_PER_MILE = 1.609344;
const M_PER_FOOT = 0.3048;

export function kilometresToMiles(km: number): number {
  return km / KM_PER_MILE;
}

export function metresToFeet(m: number): number {
  return m / M_PER_FOOT;
}

export function celsiusToFahrenheit(c: number): number {
  return (c * 9) / 5 + 32;
}

export function round(value: number, places = 1): number {
  const factor = 10 ** places;
  return Math.round(value * factor) / factor;
}
"""

UNITS_TEST = """
import { test } from "node:test";
import assert from "node:assert/strict";
import { kilometresToMiles, metresToFeet, celsiusToFahrenheit, round } from "../src/lib/units.ts";

test("kilometres to miles", () => {
  assert.equal(round(kilometresToMiles(12.9)), 8);
});

test("metres to feet", () => {
  assert.equal(round(metresToFeet(960), 0), 3150);
});

test("celsius to fahrenheit", () => {
  assert.equal(celsiusToFahrenheit(0), 32);
  assert.equal(celsiusToFahrenheit(100), 212);
});
"""

# Compact test output (and no absolute paths in screenshots).
TEST_REPORTER = r"""
// One line per test; failures add where and why.
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

export default async function* reporter(source) {
  let pass = 0;
  let fail = 0;
  for await (const event of source) {
    const data = event.data;
    if (event.type === "test:pass" && data.details?.type !== "suite") {
      pass++;
      yield `\x1b[32m✔\x1b[0m ${data.name} \x1b[2m(${data.details.duration_ms.toFixed(1)}ms)\x1b[0m\n`;
    } else if (event.type === "test:fail" && data.details?.type !== "suite") {
      fail++;
      const error = data.details.error?.cause ?? data.details.error;
      const file = data.file ? relative(process.cwd(), data.file.startsWith("file:") ? fileURLToPath(data.file) : data.file) : "";
      yield `\x1b[31m✖ ${data.name}\x1b[0m\n`;
      yield `  ${file}:${data.line}:${data.column}\n`;
      yield `  ${String(error?.message ?? error).split("\n")[0]}\n`;
    }
  }
  yield `\x1b[2mtests ${pass + fail} · pass ${pass} · fail ${fail}\x1b[0m\n`;
}
"""

FORECAST_V1 = """
import { findTrail } from "./trails.ts";
import { celsiusToFahrenheit, round } from "./lib/units.ts";

export type Forecast = {
  trailId: string;
  highC: number;
  lowC: number;
  precipitationChance: number;
  summary: string;
};

const API = process.env.FORECAST_API ?? "https://api.example-weather.dev/v2/forecast";

export async function forecastFor(trailId: string): Promise<Forecast | undefined> {
  const trail = findTrail(trailId);
  if (!trail) return undefined;
  const response = await fetch(`${API}?trail=${encodeURIComponent(trail.id)}`);
  if (!response.ok) throw new Error(`forecast service: ${response.status}`);
  const data = await response.json();
  return {
    trailId,
    highC: data.high,
    lowC: data.low,
    precipitationChance: data.pop,
    summary: `${round(celsiusToFahrenheit(data.high), 0)}°F, ${Math.round(data.pop * 100)}% rain`,
  };
}
"""

# "Cache forecast responses": a plain map.
CACHE_PLAIN = """
/** Values we've already computed, by key. */
export class ForecastCache<V> {
  private entries = new Map<string, V>();

  get(key: string): V | undefined {
    return this.entries.get(key);
  }

  set(key: string, value: V): void {
    this.entries.set(key, value);
  }

  get size(): number {
    return this.entries.size;
  }
}
"""

FORECAST_CACHED = (
    FORECAST_V1.replace(
        """import { celsiusToFahrenheit, round } from "./lib/units.ts";""",
        """import { celsiusToFahrenheit, round } from "./lib/units.ts";
import { ForecastCache } from "./lib/cache.ts";""",
    )
    .replace(
        """const API = process.env.FORECAST_API ?? "https://api.example-weather.dev/v2/forecast";""",
        """const API = process.env.FORECAST_API ?? "https://api.example-weather.dev/v2/forecast";
const cache = new ForecastCache<Forecast>();""",
    )
    .replace(
        """  if (!trail) return undefined;
  const response""",
        """  if (!trail) return undefined;
  const cached = cache.get(trail.id);
  if (cached) return cached;
  const response""",
    )
    .replace(
        """  return {
    trailId,""",
        """  const forecast: Forecast = {
    trailId,""",
    )
    .replace(
        """% rain`,
  };
}""",
        """% rain`,
  };
  cache.set(trail.id, forecast);
  return forecast;
}""",
    )
)

# "Add TTL to forecast cache".
CACHE_TTL = """
/** A map whose entries expire `ttlMs` after they're set. */
export class TTLCache<V> {
  private entries = new Map<string, { value: V; expires: number }>();
  private ttlMs: number;
  private now: () => number;

  constructor(ttlMs: number, now: () => number = Date.now) {
    this.ttlMs = ttlMs;
    this.now = now;
  }

  get(key: string): V | undefined {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    if (entry.expires <= this.now()) {
      this.entries.delete(key);
      return undefined;
    }
    return entry.value;
  }

  set(key: string, value: V): void {
    this.entries.set(key, { value, expires: this.now() + this.ttlMs });
  }

  get size(): number {
    return this.entries.size;
  }
}
"""

FORECAST_TTL = FORECAST_CACHED.replace(
    "import { ForecastCache } from", "import { TTLCache } from"
).replace(
    "const cache = new ForecastCache<Forecast>();",
    "const TTL_MS = 60_000;\nconst cache = new TTLCache<Forecast>(TTL_MS);",
)

# "Test cache expiry".
CACHE_TEST = """
import { test } from "node:test";
import assert from "node:assert/strict";
import { TTLCache } from "../src/lib/cache.ts";

test("returns what was stored", () => {
  const cache = new TTLCache<number>(1000);
  cache.set("a", 1);
  assert.equal(cache.get("a"), 1);
});

test("forgets expired entries", () => {
  let now = 0;
  const cache = new TTLCache<number>(1000, () => now);
  cache.set("a", 1);
  now = 1500;
  assert.equal(cache.get("a"), undefined);
});
"""

# Working tree, staged: hit and miss counts.
CACHE_STATS = CACHE_TTL.replace(
    """  private entries = new Map<string, { value: V; expires: number }>();
""",
    """  private entries = new Map<string, { value: V; expires: number }>();
  private hits = 0;
  private misses = 0;
""",
).replace(
    """    const entry = this.entries.get(key);
    if (!entry) return undefined;
    if (entry.expires <= this.now()) {
      this.entries.delete(key);
      return undefined;
    }
    return entry.value;""",
    """    const entry = this.entries.get(key);
    if (!entry || entry.expires <= this.now()) {
      if (entry) this.entries.delete(key);
      this.misses++;
      return undefined;
    }
    this.hits++;
    return entry.value;""",
).replace(
    """  get size(): number {
    return this.entries.size;
  }
}""",
    """  get size(): number {
    return this.entries.size;
  }

  /** Hit and miss counts since the cache was made. */
  stats(): { hits: number; misses: number; size: number } {
    return { hits: this.hits, misses: this.misses, size: this.size };
  }
}""",
)

# Working tree, unstaged: a configurable TTL, and the cache exported for /health.
FORECAST_WIP = FORECAST_TTL.replace(
    "const TTL_MS = 60_000;\nconst cache = new TTLCache<Forecast>(TTL_MS);",
    """// Ten minutes unless FORECAST_TTL_MS says otherwise.
const DEFAULT_TTL_MS = 10 * 60 * 1000;
const TTL_MS = Number(process.env.FORECAST_TTL_MS ?? DEFAULT_TTL_MS);
export const forecastCache = new TTLCache<Forecast>(TTL_MS);""",
).replace("cache.get(trail.id)", "forecastCache.get(trail.id)").replace(
    "cache.set(trail.id, forecast)", "forecastCache.set(trail.id, forecast)"
)

SERVER_V1 = """
import { createServer } from "node:http";
import { trails, findTrail } from "./trails.ts";

const port = Number(process.env.PORT ?? 3000);

function send(res, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

createServer((req, res) => {
  const url = new URL(req.url ?? "/", `http://${req.headers.host}`);
  const [, resource, id] = url.pathname.split("/");
  if (resource === "trails" && !id) return send(res, 200, trails);
  if (resource === "trails" && id) {
    const trail = findTrail(id);
    return trail ? send(res, 200, trail) : send(res, 404, { error: "no such trail" });
  }
  if (resource === "health") return send(res, 200, { ok: true });
  res.writeHead(404).end();
}).listen(port, () => console.log(`trailhead listening on http://localhost:${port}`));
"""

SERVER_V2 = SERVER_V1.replace(
    """import { trails, findTrail } from "./trails.ts";""",
    """import { findTrail, searchTrails } from "./trails.ts";
import { forecastFor } from "./forecast.ts";""",
).replace(
    """  const [, resource, id] = url.pathname.split("/");
  if (resource === "trails" && !id) return send(res, 200, trails);""",
    """  const [, resource, id, sub] = url.pathname.split("/");
  if (resource === "trails" && !id) return send(res, 200, searchTrails(url.searchParams.get("q") ?? ""));
  if (resource === "trails" && id && sub === "forecast") {
    forecastFor(id).then(
      (forecast) => (forecast ? send(res, 200, forecast) : send(res, 404, { error: "no such trail" })),
      (error) => send(res, 502, { error: String(error.message) }),
    );
    return;
  }""",
)

SERVER_V3 = SERVER_V2.replace(
    """  res.writeHead(404).end();""",
    """  send(res, 404, { error: `no route for ${url.pathname}` });""",
)

# Working tree, unstaged: cache stats on /health (on the feature branch,
# which forked before SERVER_V3).
SERVER_WIP = SERVER_V2.replace(
    """import { forecastFor } from "./forecast.ts";""",
    """import { forecastFor, forecastCache } from "./forecast.ts";""",
).replace(
    """  if (resource === "health") return send(res, 200, { ok: true });""",
    """  if (resource === "health") return send(res, 200, { ok: true, cache: forecastCache.stats() });""",
)

# Untracked, with a bug its new test catches.
METRICS = """
import { forecastCache } from "../forecast.ts";

/** Prometheus-style text for a /metrics endpoint. */
export function metrics(): string {
  const { hits, misses, size } = forecastCache.stats();
  return [
    `trailhead_forecast_cache_hits ${hits}`,
    `trailhead_forecast_cache_misses ${hits}`,
    `trailhead_forecast_cache_entries ${size}`,
  ].join("\\n");
}
"""

METRICS_TEST = """
import { test } from "node:test";
import assert from "node:assert/strict";
import { forecastCache } from "../src/forecast.ts";
import { metrics } from "../src/lib/metrics.ts";

test("counts cache hits and misses", () => {
  forecastCache.get("lake-22");
  forecastCache.get("heather");
  const line = metrics().split("\\n").find((l) => l.includes("misses"));
  assert.equal(line, "trailhead_forecast_cache_misses 2", `expected 2 misses, got "${line}"`);
});
"""

# Stashed: a unit conversion for later.
UNITS_STASHED = UNITS.replace(
    """export function celsiusToFahrenheit""",
    """export function feetToMetres(ft: number): number {
  return ft * M_PER_FOOT;
}

export function celsiusToFahrenheit""",
)

# The conflict variant: both sides changed celsiusToFahrenheit.
UNITS_OURS = UNITS.replace(
    "  return (c * 9) / 5 + 32;",
    "  // Whole degrees read better in summaries.\n  return Math.round((c * 9) / 5 + 32);",
)
UNITS_THEIRS = UNITS.replace("  return (c * 9) / 5 + 32;", "  return round((c * 9) / 5 + 32, 1);")

PROJECT_TOML = """
# What trailhead tells Impulse about itself (see Impulse's docs/project-config.md).

[[actions]]
name = "dev"
command = "npm run dev"
open = "right"

[[actions]]
name = "test"
command = "npm test"

[[actions]]
name = "lint"
command = "npm run lint"

[scripts]
# Runs in a new task worktree before its first command.
setup = "npm ci"

[worktrees]
# Untracked files to copy into new task worktrees (besides .worktreeinclude).
copy = ["config/*.local.json"]
"""

WORKTREEINCLUDE = """
# Local settings a fresh checkout doesn't have
.env
"""

DOT_ENV = """
PORT=3000
FORECAST_API=https://api.example-weather.dev/v2/forecast
FORECAST_KEY=demo-key-not-real
"""

PHOTOS = """
export type Photo = { trailId: string; url: string; caption: string; takenAt: string };

const photos: Photo[] = [];

export function photosFor(trailId: string): Photo[] {
  return photos.filter((photo) => photo.trailId === trailId);
}
"""

# ---------------------------------------------------------------------------
# Home directory

ZSHRC = r"""
# Demo shell for Impulse's documentation screenshots.
export PATH="$HOME/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export LANG=en_US.UTF-8
export CLICOLOR=1
export GIT_PAGER=cat
export HISTFILE="$HOME/.zsh_history"

# Two-line prompt: folder and branch, then ❯.
autoload -Uz vcs_info
zstyle ':vcs_info:git:*' formats '%b'
precmd() { vcs_info }
setopt PROMPT_SUBST
PROMPT=$'%F{blue}%~%f %F{8}${vcs_info_msg_0_}%f\n%F{magenta}❯%f '
"""

GITCONFIG = """
[user]
	name = Maya Chen
	email = maya@trailhead.dev
[init]
	defaultBranch = main
[advice]
	detachedHead = false
	skippedCherryPicks = false
[color]
	ui = auto
[pull]
	ff = only
"""

SETTINGS = """
{
  "check_for_updates": false,
  "restore_session": false,
  "workspace_trust": false
}
"""


def build_home() -> None:
    write(HOME / ".zshrc", ZSHRC)
    write(HOME / ".zprofile", "")
    write(HOME / ".gitconfig", GITCONFIG)
    agent = (HERE / "agent.sh").read_text()
    for name in ("claude", "codex", "gemini"):
        write(HOME / "bin" / name, agent, mode=0o755)
    # Other projects, for "recent folders".
    for name, about in (("trail-ui", "The trailhead web app"), ("trailhead-infra", "Deploy config for trailhead")):
        write(HOME / "Code" / name / "README.md", f"# {name}\n\n{about}.\n")
    for data in ("impulse", "impulse-dev"):
        write(HOME / "Library" / "Application Support" / data / "settings.json", SETTINGS)


def build_repo() -> None:
    PROJECT.mkdir(parents=True)
    git("init", "-q", "-b", "main")

    # v0.1.0: the trail list, project config, unit conversions (merged branch).
    files(PROJECT, {
        "package.json": PACKAGE_JSON % "0.1.0", "tsconfig.json": TSCONFIG, ".gitignore": GITIGNORE,
        "README.md": README_V1, "src/trails.ts": TRAILS_V1, "src/server.ts": SERVER_V1,
    })
    commit("Add trail list endpoint", "maya", 62)
    files(PROJECT, {".impulse/project.toml": PROJECT_TOML, ".worktreeinclude": WORKTREEINCLUDE})
    commit("Project actions for dev server and tests", "maya", 60)

    git("switch", "-q", "-c", "units-refactor", author="priya", days_ago=58)
    files(PROJECT, {"src/lib/units.ts": UNITS})
    commit("Unit conversions for distance, height and temperature", "priya", 58)
    files(PROJECT, {"test/units.test.ts": UNITS_TEST, "scripts/test-reporter.mjs": TEST_REPORTER})
    commit("Tests for unit conversions", "priya", 57)
    git("switch", "-q", "main")
    merge("units-refactor", "sam", 56)
    git("tag", "v0.1.0", author="sam", days_ago=56)

    # v0.2.0: forecasts and search (merged), elevation data.
    files(PROJECT, {"src/forecast.ts": FORECAST_V1})
    commit("Forecast endpoint", "sam", 49)
    git("switch", "-q", "-c", "trail-search", author="priya", days_ago=47)
    files(PROJECT, {"src/trails.ts": TRAILS_V2})
    commit("Search trails by name and region", "priya", 47)
    git("switch", "-q", "main")
    files(PROJECT, {"src/trails.ts": with_missing_elevation(TRAILS_V1)})
    commit("Handle missing elevation data", "maya", 45)
    git("merge", "-q", "--no-ff", "--no-commit", "trail-search", author="sam", days_ago=44, check=False)
    # Both sides touched trails.ts; the merged result is TRAILS_V3.
    files(PROJECT, {"src/trails.ts": TRAILS_V3, "src/server.ts": SERVER_V2, "package.json": PACKAGE_JSON % "0.2.0"})
    commit("Merge branch 'trail-search'", "sam", 44)
    git("tag", "-a", "v0.2.0", "-m", "Forecasts and trail search", author="sam", days_ago=44)

    git("switch", "-q", "-c", "readme-badges", author="sam", days_ago=38)
    files(PROJECT, {"README.md": README_BADGES})
    commit("Add CI and license badges to the README", "sam", 38)
    git("switch", "-q", "main")
    merge("readme-badges", "sam", 37)

    files(PROJECT, {"README.md": README_V2})
    commit("Document the API", "sam", 9)

    # feature/forecast-cache: three commits, the first pushed.
    git("switch", "-q", "-c", "feature/forecast-cache", author="maya", days_ago=6)
    files(PROJECT, {"src/lib/cache.ts": CACHE_PLAIN, "src/forecast.ts": FORECAST_CACHED})
    commit("Cache forecast responses", "maya", 6)
    pushed_feature = git("rev-parse", "HEAD")
    files(PROJECT, {"src/lib/cache.ts": CACHE_TTL, "src/forecast.ts": FORECAST_TTL})
    commit("Add TTL to forecast cache", "maya", 3)
    files(PROJECT, {"test/cache.test.ts": CACHE_TEST})
    commit("Test cache expiry", "maya", 2)

    # main moves on after the fork point.
    git("switch", "-q", "main")
    files(PROJECT, {"src/server.ts": SERVER_V3})
    commit("Return JSON 404s for unknown routes", "priya", 4)

    # origin: everything published, plus a commit main hasn't pulled. The
    # merged trail-search branch only survives there.
    REMOTE.parent.mkdir(parents=True)
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(REMOTE)], check=True, env=env_for("maya", 0))
    git("remote", "add", "origin", str(REMOTE))
    git("push", "-q", "origin", "main", "units-refactor", "trail-search", "--tags")
    git("push", "-q", "origin", f"{pushed_feature}:refs/heads/feature/forecast-cache")
    git("branch", "-q", "-D", "trail-search")
    other = DEMO / "remotes" / "clone"
    subprocess.run(["git", "clone", "-q", str(REMOTE), str(other)], check=True, env=env_for("sam", 1))
    write(other / "README.md", README_V2.replace("npm test\n```", "npm test\nnpm run lint\n```"))
    git("commit", "-q", "-am", "Mention lint in the README", cwd=other, author="sam", days_ago=1)
    git("push", "-q", "origin", "main", cwd=other, author="sam", days_ago=1)
    shutil.rmtree(other)
    git("fetch", "-q", "origin")
    git("remote", "set-head", "origin", "main")
    git("branch", "-q", "--set-upstream-to=origin/main", "main")
    git("branch", "-q", "--set-upstream-to=origin/feature/forecast-cache", "feature/forecast-cache")

    # The checkout: feature/forecast-cache with a stash and uncommitted work.
    git("switch", "-q", "feature/forecast-cache")
    files(PROJECT, {"src/lib/units.ts": UNITS_STASHED})
    git("stash", "push", "-q", "-m", "WIP: feet to metres", author="maya", days_ago=1)
    files(PROJECT, {"src/lib/cache.ts": CACHE_STATS})
    git("add", "src/lib/cache.ts")
    files(PROJECT, {
        "src/forecast.ts": FORECAST_WIP, "src/server.ts": SERVER_WIP,
        "src/lib/metrics.ts": METRICS, "test/metrics.test.ts": METRICS_TEST,
    })
    write(PROJECT / ".env", DOT_ENV)
    write(PROJECT / "config/dev.local.json", LOCAL_CONFIG)

    # Task worktrees beside the repository, as New Task makes them.
    for branch, days in (("fix-elevation", 0.2), ("add-trail-photos", 0.1)):
        git("worktree", "add", "-q", "-b", branch, str(WORKTREES / branch), "main", days_ago=days)
        write(WORKTREES / branch / ".env", DOT_ENV)
        write(WORKTREES / branch / "config/dev.local.json", LOCAL_CONFIG)
    write(WORKTREES / "add-trail-photos" / "src/photos.ts", PHOTOS)
    # What the stand-in agent writes when asked to fix the elevation fallback.
    write(DEMO / "agent" / "trails.fixed.ts", TRAILS_FIXED)


def make_conflict() -> None:
    """The checkout in the middle of merging a branch that changed the same
    lines of src/lib/units.ts."""
    git("stash", "push", "-q", "-u", "-m", "Before the merge", days_ago=0.05)
    git("switch", "-q", "-c", "units-precision", "main", author="priya", days_ago=1)
    files(PROJECT, {"src/lib/units.ts": UNITS_THEIRS})
    commit("Keep one decimal place in Fahrenheit", "priya", 1)
    git("switch", "-q", "feature/forecast-cache")
    files(PROJECT, {"src/lib/units.ts": UNITS_OURS})
    commit("Round temperatures to whole degrees", "maya", 0.5)
    git("merge", "units-precision", days_ago=0.01, check=False)


# ---------------------------------------------------------------------------
# Session files for the snapshot runs (see shots.py)

def path_of(where: str) -> str:
    """'trailhead' or 'trailhead/src/x.ts', or a task name like
    'fix-elevation' / 'fix-elevation/src/x.ts'."""
    head, _, rest = where.partition("/")
    root = PROJECT if head == "trailhead" else WORKTREES / head
    return str(root / rest) if rest else str(root)


def surface(spec) -> dict:
    """('terminal', 'trailhead') or ('file', 'trailhead/src/forecast.ts')."""
    kind, where = spec
    if kind == "terminal":
        return {"kind": "terminal", "cwd": path_of(where)}
    if kind == "file":
        return {"kind": "file", "path": path_of(where)}
    if kind in ("review", "history"):
        return {"kind": kind, "path": path_of(where)} | ({"scope": {"unstaged": {}}} if kind == "review" else {})
    raise ValueError(kind)


def tab(spec) -> dict:
    """A surface spec, or {'split': 'h'|'v', 'panes': [surface, …], 'ratios': […]}
    (h: side by side), optionally 'pinned': True."""
    if isinstance(spec, dict) and "panes" in spec:
        panes = [surface(p) for p in spec["panes"]]
        axis = "horizontal" if spec.get("split", "h") == "h" else "vertical"
        ratios = spec.get("ratios", [1] * len(panes))
        layout = spec.get("layout") or {
            "axis": axis, "children": [{"pane": i} for i in range(len(panes))], "ratios": ratios
        }
        return {"pinned": spec.get("pinned", False), "panes": panes, "layout": layout,
                "focused_pane": spec.get("focused", 0)}
    if isinstance(spec, dict):
        return {"pinned": spec.get("pinned", False), "panes": [surface(spec["pane"])]}
    return {"pinned": False, "panes": [surface(spec)]}


def session(workspaces: list, active: int = 0, sidebar: bool = True, sidebar_width: int = 260) -> dict:
    """workspaces: [{'root': 'trailhead', 'tabs': [...], 'active_tab': 0,
    'expanded': False, 'name': None, 'scratch': False}]"""
    out = []
    for ws in workspaces:
        entry = {
            "kind": "scratch" if ws.get("scratch") else "folder",
            "root": str(HOME) if ws.get("scratch") else path_of(ws["root"]),
            "tabs": [tab(t) for t in ws.get("tabs", [("terminal", ws.get("root", "trailhead"))])],
            "active_tab_index": ws.get("active_tab", 0),
        }
        if ws.get("expanded"):
            entry["expanded"] = True
        if ws.get("name"):
            entry["name"] = ws["name"]
        out.append(entry)
    return {
        "version": 2,
        "active_window_index": 0,
        "windows": [
            {"workspaces": out, "active_workspace_index": active, "sidebar_visible": sidebar,
             "sidebar_width": sidebar_width}
        ],
    }


def main(variant: str | None = None) -> None:
    if DEMO.exists():
        shutil.rmtree(DEMO)
    build_home()
    build_repo()
    if variant == "conflict":
        make_conflict()


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else None)
