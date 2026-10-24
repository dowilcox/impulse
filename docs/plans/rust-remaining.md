# Rust app: what's left

What still stands between the Rust app (`impulse-app`, on GPUI + GPUI Kit)
and the Swift app it replaces. The migration plan and what's already done
are in [rust-migration.md](rust-migration.md).

Where things stand: on Linux every docs shot matches the Swift app, except
where the Swift shots are older than the code or the platforms differ, and
every smoke scenario passes (`scripts/smoke/run.py` with
`IMPULSE_SNAPSHOT_APP` on the Rust app, under Xvfb).

## 1. The Mac

The largest unknown, so it goes first.

- [ ] **Build and run it on a Mac.** It has never been linked or run on
  macOS. `scripts/check-macos.sh` type-checks the code behind
  `cfg(target_os = "macos")` for `aarch64-apple-darwin` from Linux, but
  linking, GPUI's Metal renderer and how the app looks there are untested.
- [ ] **Check the macOS-only paths by hand**: the menu bar (`menus.rs`), the
  Dock badge (`badge.rs`), Increase Contrast and Reduce Motion
  (`display_options.rs`), the window frame saved in AppKit's coordinates
  (`session.rs`: `frame_string`, `saved_bounds`), the Quick Terminal's
  global shortcut, notifications through `osascript`, `NSBeep` for the
  terminal bell, and ⌘Z bringing back a closed tab (`UndoClose`).
- [ ] **Run the snapshot mode there.** Window images go through
  `screencapture -l` on macOS, which hasn't been tried. Then run
  `scripts/smoke/run.py` and `scripts/docs/capture.py` against the Rust
  binary and compare the shots with the Swift app's, side by side on the
  same OS.
- [ ] **Compare against the Swift app in a key window.** The Swift docs
  shots come from inactive windows, where macOS draws native switches and
  checkboxes without the accent color, so they read grey there and accent
  in the Rust shots. Settle which one the Rust app should copy once both
  run on the same Mac.

## 2. Ours to build

Work that doesn't depend on anyone else.

- [ ] **Collapse unchanged regions in the Diff View** (`editor_diff.rs`,
  `impulse_kit::git::diff_view`), as Monaco's diff editor folds long runs
  of unchanged lines.
- [ ] **Put conflict actions inline.** Accept Current, Accept Incoming and
  Accept Both show in a bar today (`editor_git.rs`); Monaco puts them
  above each conflict.
- [ ] **Draw the cursor in the agent toolbelt's terminal.** The terminal
  under the toolbelt (`agent_toolbelt.rs`) doesn't paint its cursor.
- [ ] **SQL highlighting.** Its tree-sitter grammar needs an older `cc`
  than the workspace uses; either pin a compatible grammar or build it
  another way.
- [ ] **Remove the line on the editor's right side.** A vertical line shows
  along the right of an editor tab, which the Swift app doesn't have. Not
  found yet: it isn't the minimap (`minimap_enabled` is off by default),
  and our code draws nothing there apart from the overview ruler's change
  marks (`editor_git.rs`), which are short bars. So it's likely GPUI Kit's
  scrollbar track or an editor border. Find what draws it and hide it.
- [ ] **Show preview tabs in italics.** A file opened with a single click
  in the file tree is a preview until it's double-clicked or edited, and
  the Swift app shows its tab title in italics. The Rust app sets
  `.italic()` on the title (`workbench_chrome.rs`, `tab.preview`), but
  `vendor/fonts/inter/` has no italic face (only Regular, Medium,
  SemiBold and Bold), and GPUI doesn't slant a face itself, so the title
  draws upright. You can't tell a preview from a kept tab. Bundle Inter's
  italic faces (and add them to `scripts/package-linux.sh`'s fonts) or
  synthesize the slant.
- [ ] **Even out the space around terminal blocks**
  ([screenshot](images/rust-terminal-block-spacing.png)). The separator
  sits right on top of a block's prompt row, but below a block's last
  output is the blank line the shell prints before its next prompt, so
  each block has about a line of space at the bottom and almost none at
  the top, and the bottom looks too loose. The Swift renderer collapses
  those blank prompt-padding rows (`TerminalRenderer.collapsedRows`, up to
  3 rows between blocks) down to a 6 pt gap (`blockPadPixels`) and puts
  the separator in the middle of it (`blockBorderY`, `blockBottomY`), so
  every block has 3 pt above and below. `terminal_view.rs` draws the
  separator at `block.start_row` and collapses nothing.

## 3. Waiting on GPUI Kit

Each needs a change in GPUI Kit (upstream, or a fork if it doesn't come).
The settings named here exist and are saved, but the Rust app doesn't act
on them yet.

| What | Why it waits | Setting |
|---|---|---|
| Inlay hints | the editor can't insert text into a line's layout | `editor_inlay_hints` |
| Caret style and blinking | the caret has no styles, and only the primary caret's place is public | `editor_cursor_style`, `editor_cursor_blinking` |
| Whitespace markers for `boundary`, `selection` and `trailing` | markers are drawn everywhere or nowhere | `render_whitespace` |
| Auto-closing `always` and `beforeWhitespace` | one global rule, Monaco's `languageDefined` | `editor_auto_closing_brackets` |
| Snippets in completions | snippet handling is off until it's checked; completions insert plain text | |
| Padding lines in the Diff View | no view zones, so one side holds still through the other's insertion | |
| Announcing agent state to screen readers | GPUI has no live regions | |
| Smooth scrolling | not supported | `smooth_scrolling` |

## 4. Known issues

- [ ] **First keystroke lost in the editor's search field** under Xvfb.
  Focus moves there at once in GPUI Kit's code, so this looks like X11
  key input without a window manager. Check on a real Linux desktop and on
  the Mac.
- [ ] **Hover popover overlap.** GPUI Kit's hover popover draws a code
  block's language label on top of the code
  ([screenshot](images/rust-editor-hover.png)).

## Not differences

These came up when the Rust shots were compared with the Swift ones and
need no work:

- Commands the Swift palette shots don't show (Clear Recent Workspaces,
  Clear Command History, Install Agent Hooks… under `>split`): the Swift
  shots are older than those commands; the Rust palette matches the Swift
  code.
- The file tree open further in some Swift shots: the Swift app keeps open
  folders in UserDefaults, so folders opened in one shot stay open in the
  next. Each Rust shot starts fresh.
- "Undo" without "⌘Z" in the close toast on Linux, where that shortcut
  doesn't exist.
