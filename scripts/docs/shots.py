"""The documentation screenshots: what each one shows and how to get there.

Each shot:
  name      docs/images/<name>.png
  session   make_demo.session(...): the window's workspaces and tabs
  actions   named debug actions (MainWindowController+Debug.swift), run in
            order 0.6 s apart from about delay/2; "wait" is a no-op spacer
  delay     seconds (default 8); the capture happens at delay + 0.6/action
  size      window size in points (default 1280x800)
  crop      (x, y, w, h) in points from the window's top-left, or "overlay"
            (the first floating window: palette, sheet, popover, menu panel,
            plus `pad` points of the window around it)
  style     "window" (rounded, shadowed; the default without a crop) or
            "crop" (rounded edge only; the default with one)
  overlays  class names of floating windows to composite (default: all)
  base      class name of the window to use instead of the main one
  variant   make_demo variant ("conflict")
  settings  keys merged into the demo's settings.json; settings_text
            replaces the file
  files     extra input files, {name: text or JSON}, referenced in actions as
            {files}/<name>
"""

from make_demo import session

W, H = 1280, 800
SIDEBAR_W = 260
TITLE_H = 40
STATUS_H = 24

SIDEBAR = (0, TITLE_H, SIDEBAR_W, H - TITLE_H - STATUS_H)
TITLEBAR = (0, 0, W, TITLE_H)
STATUSBAR = (0, H - STATUS_H, W, STATUS_H)
CONTENT = (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, H - TITLE_H - STATUS_H)
CONTENT_NO_SIDEBAR = (0, TITLE_H, W, H - TITLE_H - STATUS_H)
# Where a terminal's output ends: the input bar sits below.
GRID_BOTTOM = 700
# The Nord card in the Component Gallery's dark themes.
NORD_CARD = (10, 376, 574, 280)


def wait(n: int = 1) -> list[str]:
    return ["wait"] * n


# Workspaces.
def trailhead(*tabs, active_tab=0, expanded=False):
    return {"root": "trailhead", "tabs": list(tabs) or [("terminal", "trailhead")], "active_tab": active_tab,
            "expanded": expanded}


FIX_ELEVATION = {"root": "fix-elevation", "tabs": [("terminal", "fix-elevation")]}
TRAIL_PHOTOS = {"root": "add-trail-photos", "tabs": [("terminal", "add-trail-photos")]}
SCRATCH = {"scratch": True, "tabs": [("terminal", "trailhead")]}

ONE = session([trailhead()])
TASKS = session([trailhead(), FIX_ELEVATION, TRAIL_PHOTOS])

# An agent waiting in fix-elevation and one working in add-trail-photos.
START_AGENTS = [
    "select-workspace=fix-elevation", "run=claude waiting",
    "select-workspace=add-trail-photos", "run=claude working",
    "select-workspace=trailhead",
]
# Three finished blocks: ok, failed, ok.
THREE_BLOCKS = ["run=git status -sb", *wait(2), "run=npm test --silent", *wait(3), "run=npm run lint --silent", *wait(3)]
DEV_SERVER_BG = "run=npm run dev > /dev/null 2>&1 &"

PROBLEMS = [
    {"path": "src/forecast.ts", "line": 23, "column": 25, "severity": "error",
     "message": "Property 'hi' does not exist on type 'ForecastResponse'. Did you mean 'high'?",
     "source": "ts", "code": "2551"},
    {"path": "src/forecast.ts", "line": 2, "column": 31, "severity": "warning",
     "message": "'round' is declared but its value is never read.", "source": "ts", "code": "6133"},
    {"path": "src/lib/cache.ts", "line": 21, "column": 7, "severity": "warning",
     "message": "Unexpected console statement.", "source": "eslint", "code": "no-console"},
    {"path": "test/metrics.test.ts", "line": 9, "column": 16, "severity": "error",
     "message": "Argument of type 'number' is not assignable to parameter of type 'string'.",
     "source": "ts", "code": "2345"},
    {"path": "test/metrics.test.ts", "line": 3, "column": 1, "severity": "warning",
     "message": "'forecastFor' is defined but never used.", "source": "eslint", "code": "no-unused-vars"},
    {"path": "src/lib/units.ts", "line": 13, "column": 17, "severity": "info",
     "message": "This may be converted to an arrow function.", "source": "ts", "code": "80002"},
]

PR_THREADS = {"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": [{
    "id": "PRRT_1", "path": "src/forecast.ts", "isResolved": False, "isOutdated": False,
    "diffSide": "RIGHT", "line": 16, "startLine": None,
    "comments": {"nodes": [
        {"author": {"login": "samortiz"}, "createdAt": "2026-10-06T16:20:00Z",
         "url": "https://github.com/trailhead/trailhead/pull/12#discussion_r1",
         "body": "Could the TTL come from config instead? Ops wants to tune it per environment."},
        {"author": {"login": "mayachen"}, "createdAt": "2026-10-06T17:02:00Z",
         "url": "https://github.com/trailhead/trailhead/pull/12#discussion_r2",
         "body": "Good call, reading FORECAST_TTL_MS now (defaults to ten minutes)."},
    ]},
}]}}}}}

SHOTS = [
    # ------------------------------------------------------------ getting started
    {"name": "getting-started-window-tour",
     "session": session([trailhead(("terminal", "trailhead"), ("file", "trailhead/src/forecast.ts")), FIX_ELEVATION, SCRATCH]),
     "actions": ["select-workspace=fix-elevation", "run=claude waiting", "select-workspace=trailhead",
                 DEV_SERVER_BG, *wait(), *THREE_BLOCKS],
     "delay": 12},
    {"name": "getting-started-trust-prompt", "session": ONE, "actions": ["trust-prompt"], "crop": "overlay", "pad": 0},
    {"name": "getting-started-restricted-status", "session": ONE, "actions": ["trust-on"],
     "crop": (0, H - STATUS_H, 520, STATUS_H)},

    # ------------------------------------------------------------ workspaces and tabs
    {"name": "workspaces-and-tabs-sidebar",
     "session": session([trailhead(("terminal", "trailhead"), ("file", "trailhead/src/forecast.ts"), ("review", "trailhead")),
                         FIX_ELEVATION, TRAIL_PHOTOS, SCRATCH]),
     "actions": [*START_AGENTS, DEV_SERVER_BG], "delay": 12, "crop": (0, TITLE_H, SIDEBAR_W, 230)},
    {"name": "workspaces-and-tabs-expanded-tabs",
     "session": session([trailhead(("terminal", "trailhead"), ("file", "trailhead/src/forecast.ts"), ("review", "trailhead"),
                                   expanded=True), FIX_ELEVATION, SCRATCH]),
     "crop": (0, TITLE_H, SIDEBAR_W, 230)},
    {"name": "workspaces-and-tabs-tab-strip",
     "session": session([trailhead({"pinned": True, "pane": ("terminal", "trailhead")}, ("terminal", "trailhead"),
                                   ("file", "trailhead/src/forecast.ts"),
                                   {"panes": [("terminal", "trailhead"), ("file", "trailhead/src/server.ts")]},
                                   active_tab=2)]),
     "actions": ["workspace-edit=README.md", "preview=src/lib/units.ts"], "crop": (SIDEBAR_W - 10, 0, 760, TITLE_H)},
    {"name": "workspaces-and-tabs-split-panes",
     "session": session([trailhead({"panes": [("terminal", "trailhead"), ("file", "trailhead/src/server.ts"),
                                              ("terminal", "trailhead")],
                                    "layout": {"axis": "horizontal", "ratios": [0.5, 0.5], "children": [
                                        {"pane": 0},
                                        {"axis": "vertical", "ratios": [0.6, 0.4], "children": [{"pane": 1}, {"pane": 2}]}]}})]),
     "actions": ["run=npm run dev", *wait(3), "focus-pane=2"], "delay": 10},
    {"name": "workspaces-and-tabs-files-panel",
     "session": session([trailhead(("file", "trailhead/src/forecast.ts"))]),
     "actions": ["tree-expand=src", "tree-expand=src/lib", "tree-expand=test"], "crop": (0, 80, SIDEBAR_W, 420)},
    {"name": "workspaces-and-tabs-search-panel", "session": ONE, "actions": ["search=forecastCache"],
     "crop": (0, 80, SIDEBAR_W, 420)},
    {"name": "workspaces-and-tabs-undo-close-toast",
     "session": session([trailhead(("terminal", "trailhead"), ("file", "trailhead/src/forecast.ts"), active_tab=1)]),
     "actions": [*THREE_BLOCKS, "close-pane"], "delay": 8, "overlays": ["ToastPanel"], "crop": "overlay",
     "pad": 40},
    {"name": "workspaces-and-tabs-status-bar", "session": ONE, "actions": [DEV_SERVER_BG], "delay": 12,
     "crop": STATUSBAR},

    # ------------------------------------------------------------ command palette
    {"name": "command-palette-commands", "session": ONE, "actions": ["palette=>split"], "crop": "overlay"},
    {"name": "command-palette-files", "session": ONE, "actions": ["palette=fcst"], "crop": "overlay"},
    {"name": "command-palette-workspaces", "session": session([trailhead(), FIX_ELEVATION, SCRATCH]),
     "defaults": {"recentWorkspaceFolders": ["~/Code/trail-ui", "~/Code/trailhead-infra", "~/Code/trailhead"]},
     "actions": ["palette=w:"], "crop": "overlay"},
    {"name": "command-palette-branches", "session": ONE, "actions": ["palette=b:"], "crop": "overlay"},
    {"name": "command-palette-history", "session": ONE,
     "actions": ["run=npm test --silent", *wait(3), "run=npm run lint --silent", *wait(2), "run=npm test --silent",
                 *wait(3), "run=git status -sb", *wait(), "palette=h:npm"],
     "delay": 8, "crop": "overlay"},
    {"name": "command-palette-help", "session": ONE, "actions": ["palette=?"], "crop": "overlay"},

    # ------------------------------------------------------------ terminal
    {"name": "terminal-overview", "session": ONE, "actions": THREE_BLOCKS, "delay": 8},
    {"name": "terminal-blocks", "session": ONE, "actions": [*THREE_BLOCKS, "block=toggle_block_bookmark"],
     "crop": (SIDEBAR_W, GRID_BOTTOM - 430, W - SIDEBAR_W, 430)},
    {"name": "terminal-block-selection", "session": ONE, "actions": [*THREE_BLOCKS, "select-blocks=2"],
     "crop": (SIDEBAR_W, GRID_BOTTOM - 430, W - SIDEBAR_W, 430)},
    {"name": "terminal-input-bar", "session": ONE,
     "actions": ["run=npm run lint --silent", *wait(3), "draft=npm run l"],
     "crop": (SIDEBAR_W, H - STATUS_H - 80, W - SIDEBAR_W, 80)},
    {"name": "terminal-completions", "session": ONE, "actions": ["draft=git switch f", "complete", *wait()],
     "crop": (SIDEBAR_W, H - STATUS_H - 150, W - SIDEBAR_W, 150)},
    {"name": "terminal-running", "session": ONE, "actions": ["run=npm run dev"], "delay": 14,
     "crop": (0, H - 200, W, 200)},
    {"name": "terminal-find", "session": ONE,
     "actions": ["run=git log --oneline -12", *wait(2), "run=git log --stat -3 --format=%h%x20%s", *wait(2),
                 "find=forecast"],
     "crop": (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, GRID_BOTTOM - TITLE_H)},
    {"name": "terminal-hints", "session": ONE,
     "actions": ["run=git log --oneline -4", *wait(2), "run=npm test --silent", *wait(3),
                 "run=echo Dev server: http://localhost:3000", *wait(2), "block=terminal_hints"],
     "crop": (SIDEBAR_W, GRID_BOTTOM - 480, W - SIDEBAR_W, 480)},

    # ------------------------------------------------------------ tasks
    {"name": "tasks-new-task-sheet", "session": ONE, "actions": ["task-sheet=Add trail difficulty filter|claude|main", *wait(2)],
     "crop": "overlay", "pad": 0},
    {"name": "tasks-sidebar-group", "session": session([trailhead(), FIX_ELEVATION, TRAIL_PHOTOS], active=1),
     "actions": ["run=claude turn", "select-workspace=add-trail-photos", "run=claude working",
                 "select-workspace=fix-elevation"],
     "delay": 14, "crop": (0, TITLE_H, SIDEBAR_W, 124)},
    {"name": "tasks-archive-confirm", "session": session([trailhead(), FIX_ELEVATION], active=1),
     "actions": ["run=claude turn", *wait(8), "command=archive_task", *wait()], "delay": 8,
     "crop": "overlay", "pad": 0},

    # ------------------------------------------------------------ agents
    {"name": "agents-tab-states", "session": ONE,
     "actions": ["run=claude working", "newtab", "run=codex waiting", "newtab", "run=gemini done", *wait(4)],
     "delay": 10, "crop": TITLEBAR},
    {"name": "agents-inbox", "session": session([trailhead(), FIX_ELEVATION]),
     "actions": ["run=claude working", "newtab", "run=codex waiting", "select-workspace=fix-elevation",
                 "run=claude turn", *wait(8), "select-workspace=trailhead", "click=905:20", *wait(2)],
     "delay": 10, "crop": "overlay"},
    {"name": "agents-toolbelt", "session": session([FIX_ELEVATION]), "actions": ["run=claude turn", *wait(8)],
     "delay": 8, "size": (1180, 520), "crop": (SIDEBAR_W, TITLE_H, 1180 - SIDEBAR_W, 520 - TITLE_H - STATUS_H)},
    {"name": "agents-composer", "session": session([TRAIL_PHOTOS]),
     "actions": ["run=claude working", *wait(6), "composer=Also return each photo's photographer from @src/ph"],
     "delay": 8, "size": (1180, 560), "crop": (SIDEBAR_W, TITLE_H, 1180 - SIDEBAR_W, 560 - TITLE_H - STATUS_H)},
    {"name": "agents-hooks-sheet", "session": ONE, "actions": ["command=agent_hooks", *wait(2)], "crop": "overlay",
     "pad": 0},

    # ------------------------------------------------------------ project config, CLI
    {"name": "project-config-trust-prompt", "session": ONE, "actions": ["project-trust", *wait()], "crop": "overlay",
     "pad": 0},
    {"name": "project-config-actions-palette", "session": ONE, "actions": ["palette=a:"], "crop": "overlay"},
    {"name": "cli-editor-setting", "session": ONE, "actions": ["no-sidebar", "settings=$EDITOR"],
     "crop": (0, TITLE_H, W, 300)},
    {"name": "cli-help", "session": ONE, "actions": ["run=impulse help"],
     "crop": (SIDEBAR_W, GRID_BOTTOM - 320, W - SIDEBAR_W, 320)},

    # ------------------------------------------------------------ git
    {"name": "git-titlebar", "session": ONE, "actions": ["no-sidebar"], "crop": TITLEBAR},
    {"name": "git-changes-panel", "session": session([trailhead()], sidebar_width=340), "actions": ["changes"],
     "crop": (0, TITLE_H, 340, H - TITLE_H - STATUS_H)},
    {"name": "git-branch-switcher", "session": ONE, "actions": ["branches"], "crop": "overlay"},
    {"name": "git-manage-branches", "session": ONE, "actions": ["manage-branches", *wait(2)], "crop": "overlay", "pad": 0},
    {"name": "git-operation-banner", "session": session([trailhead()], sidebar_width=400), "variant": "conflict",
     "actions": ["changes"], "crop": (0, TITLE_H, 400, 300)},
    {"name": "git-create-tag", "session": ONE, "actions": ["tag-sheet", *wait()], "crop": "overlay", "pad": 0},

    # ------------------------------------------------------------ review
    {"name": "review-overview", "session": ONE,
     "actions": ["review", *wait(2), "review-scope=uncommitted", *wait(2), "review-reveal=src/lib/cache.ts", *wait(),
                 "review-keys=n", "review-comment=Expired entries count as misses here. Is that what the metrics should report?"],
     "delay": 8},
    {"name": "review-navigator", "session": ONE,
     "actions": ["review", *wait(2), "review-scope=uncommitted", *wait(2), "review-reveal=src/server.ts", "review-keys=v",
                 "review-reveal=src/lib/cache.ts", "review-keys=n", "review-comment=Expired entries count as misses here."],
     "delay": 8, "crop": (SIDEBAR_W, TITLE_H, 300, 420)},
    {"name": "review-split", "session": ONE,
     "actions": ["no-sidebar", "review", *wait(2), "review-split", *wait(), "review-reveal=src/forecast.ts"],
     "delay": 8, "crop": (0, TITLE_H, W, 470)},
    {"name": "review-line-selection", "session": ONE,
     "actions": ["no-sidebar", "review", *wait(2), "review-reveal=src/forecast.ts", *wait(),
                 "review-select=src/forecast.ts:0:6-7"],
     "delay": 8, "crop": (0, TITLE_H, W, 420)},
    {"name": "review-comment-composer", "session": ONE,
     "actions": ["no-sidebar", "review", *wait(2), "review-scope=uncommitted", *wait(2),
                 "review-reveal=src/lib/cache.ts", *wait(), "review-keys=n", "review-composer"],
     "delay": 8, "crop": (0, TITLE_H, W, 560)},
    {"name": "review-pr-threads", "session": ONE, "files": {"threads.json": PR_THREADS},
     "actions": ["no-sidebar", "review", *wait(2), "review-scope=branch:origin/main", *wait(2),
                 "pr-threads={files}/threads.json", *wait(), "review-reveal=src/forecast.ts"],
     "delay": 8, "crop": (0, TITLE_H, W, 560)},

    # ------------------------------------------------------------ history
    {"name": "history-overview", "session": ONE, "actions": ["history", *wait(2)], "delay": 8},
    {"name": "history-all-branches", "session": ONE, "actions": ["history", *wait(), "history-all", *wait(2)],
     "delay": 8, "crop": (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, 400)},
    {"name": "history-filter", "session": ONE, "actions": ["history-filter=author:maya since:30d", *wait(2)],
     "delay": 8, "crop": (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, 300)},

    # ------------------------------------------------------------ editor
    {"name": "editor-overview", "session": session([trailhead(("file", "trailhead/src/forecast.ts"))]),
     "actions": ["tree-expand=src", "tree-expand=src/lib"], "delay": 10},
    {"name": "editor-language-servers", "session": ONE, "actions": ["settings-category=Language Servers", *wait(2)],
     "crop": CONTENT},
    {"name": "editor-problems", "session": ONE, "files": {"problems.json": PROBLEMS},
     "actions": ["problems={files}/problems.json"], "crop": (SIDEBAR_W, 0, W - SIDEBAR_W, 360)},
    {"name": "editor-search-replace", "session": ONE, "actions": ["replace=forecastCache:trailForecasts"],
     "crop": (0, 80, SIDEBAR_W, 560)},
    {"name": "editor-markdown-preview", "session": session([trailhead(("file", "trailhead/README.md"))]),
     "actions": ["preview-beside", *wait(3)], "delay": 10, "crop": (SIDEBAR_W, 0, W - SIDEBAR_W, H)},
    {"name": "editor-git-peek", "session": session([trailhead(("file", "trailhead/src/forecast.ts"))]),
     "actions": [*wait(4), "git-peek=15", *wait(2)], "delay": 10, "crop": (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, 460)},
    {"name": "editor-diff-view", "session": session([trailhead(("file", "trailhead/src/forecast.ts"))]),
     "actions": [*wait(3), "diff-view", *wait(3)], "delay": 10, "crop": (SIDEBAR_W, 0, W - SIDEBAR_W, 560)},
    {"name": "editor-conflict", "variant": "conflict", "session": session([trailhead(("file", "trailhead/src/lib/units.ts"))]),
     "actions": [*wait(4)], "delay": 10, "crop": (SIDEBAR_W, TITLE_H, W - SIDEBAR_W, 460)},

    # ------------------------------------------------------------ settings, themes, shortcuts
    {"name": "settings-and-themes-settings-tab", "session": ONE,
     "actions": ["no-sidebar", "setting-on=minimap_enabled", "settings-category=Editor"], "crop": CONTENT_NO_SIDEBAR},
    {"name": "settings-and-themes-search", "session": ONE, "actions": ["no-sidebar", "settings=font"],
     "crop": CONTENT_NO_SIDEBAR},
    {"name": "settings-and-themes-settings-json", "session": ONE,
     "settings": {"git_pull_mode": "rebase", "terminal_editor_integration": True,
                  "keybinding_overrides": {"next_pane": "Cmd+]"}},
     "actions": ["no-sidebar", "command=open_settings_json", *wait(10)], "delay": 8, "crop": (0, 0, W, 460)},
    {"name": "settings-and-themes-automation", "session": ONE,
     "settings": {
         "commands_on_save": [{"name": "Lint", "command": "npx", "args": ["eslint", "--fix", "."],
                               "file_pattern": "*.ts", "reload_file": True}],
         "file_type_overrides": [{"pattern": "*.md", "tab_width": 2, "use_spaces": True,
                                  "format_on_save": {"command": "npx", "args": ["prettier", "--write", "."]}}]},
     "actions": ["no-sidebar", "settings-category=Automation"], "crop": CONTENT_NO_SIDEBAR},
    {"name": "settings-and-themes-language-servers", "session": ONE,
     "actions": ["no-sidebar", "settings-category=Language Servers", *wait(2)], "crop": CONTENT_NO_SIDEBAR},
    # The developer Component Gallery's theme cards (without its header).
    {"name": "settings-and-themes-theme-gallery", "session": ONE, "actions": ["gallery=dark", *wait(2)],
     "base": "NSWindow", "overlays": [], "crop": (8, 90, 1164, 750)},
    {"name": "settings-and-themes-harbor", "session": session([trailhead(("file", "trailhead/src/forecast.ts"))]),
     "settings": {"color_scheme": "harbor"}, "delay": 10},
    {"name": "settings-and-themes-keyboard-shortcuts", "session": ONE,
     "settings": {"keybinding_overrides": {"split_down": "Cmd+D"}}, "actions": ["no-sidebar", "keybindings"],
     "crop": CONTENT_NO_SIDEBAR},
    {"name": "keyboard-shortcuts-palette", "session": ONE, "actions": ["palette=>"], "crop": "overlay"},
    # The same theme card with Increase Contrast off and on, side by side.
    {"name": "_contrast-off", "session": ONE, "actions": ["gallery=dark", *wait(2)],
     "base": "NSWindow", "overlays": [], "crop": NORD_CARD},
    {"name": "_contrast-on", "session": ONE, "actions": ["gallery=dark+contrast", *wait(2)],
     "base": "NSWindow", "overlays": [], "crop": NORD_CARD},
    {"name": "accessibility-increase-contrast", "combine": ["_contrast-off", "_contrast-on"], "gap": 24},
]

