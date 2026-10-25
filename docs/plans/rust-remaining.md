# Rust app: what's left

What still stands between the Rust app (`impulse-app`, on GPUI + GPUI Kit)
and the Swift app it replaces. The migration plan and what's already done
are in [rust-migration.md](rust-migration.md).
The code review of October 2026, with the bugs, security and performance
findings still to fix, is [rust-review.md](rust-review.md).

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

- [x] **Collapse unchanged regions in the Diff View** (`editor_diff.rs`,
  `impulse_kit::git::diff_view`), as Monaco's diff editor folds long runs
  of unchanged lines. Done inline: `diff_view::hide_unchanged` keeps
  three lines of context beside each change and puts a run of three or
  more others behind one "18 hidden lines" row, which a click opens
  (Monaco's `hideUnchangedRegions` as the Swift app sets it; the
  snapshot's `diff-inline` and `diff-reveal` actions drive it). Side by
  side still shows every line: that waits on GPUI Kit (below).
- [x] **Put conflict actions inline.** Done: each conflict's Accept
  Current, Accept Incoming and Accept Both, its labels and, with more
  than one, "1 of 2" with ↑ and ↓ float on its `<<<<<<<` line after the
  marker (`EditorTab::conflict_bars` in `editor_git.rs`) and act on that
  conflict. Monaco puts the same bar in a 24 px view zone above the
  conflict; without view zones in GPUI Kit, it can't push the lines down.
- [x] **Draw the cursor in the agent toolbelt's terminal.** Done: the
  cursor was there but blinking, and the shot caught it off. The
  terminal now blinks only in the active window, as Swift's
  `shouldBlink` does, so it stays solid elsewhere (and in snapshots).
- [x] **SQL highlighting.** Done: GPUI Kit's `tree-sitter-sql` feature
  is on. Its grammar (`tree-sitter-sequel`) asks for `cc ~1.2`, and
  nothing else in the workspace needs a newer one, so `Cargo.lock` holds
  `cc` at 1.2.67, and `cargo update` keeps it on 1.2 while the grammar
  asks for that.
- [x] **Remove the line on the editor's right side.** A vertical line shows
  along the right of an editor tab, which the Swift app doesn't have.
  Settled: it was the right margin, on in the dev build's own
  settings.json (`~/.local/share/impulse-dev/`, not the v0.29 app's
  `~/.config/impulse/`, where it was already off); turned off there, and
  the default stays on as in the Swift app.
  Found: on a window wide enough, it's the right margin at column 120
  (`show_right_margin` and `right_margin_position`, on by default in both
  apps, painted in `editor_git.rs`). The Swift app draws the same ruler
  (Monaco's `rulers`) at the same column: the Swift docs shot
  `settings-and-themes-settings-json` shows it at x≈1076, as the Rust one
  does. The Rust app drew it in the theme's border color and now uses
  Monaco's `editorRuler.foreground` (#5A5A5A dark, #D3D3D3 light), as
  Swift does. Nothing else draws a line there under Xvfb. To check: if the
  line meant here is somewhere else (on the Mac, in a split, or at the
  far right edge), note where; otherwise turn the setting off to hide it.
- [x] **Show preview tabs in italics.** A file opened with a single click
  in the file tree is a preview until it's double-clicked or edited, and
  the Swift app shows its tab title in italics. Done: Inter 4.1's Italic,
  Medium Italic and SemiBold Italic faces are bundled in
  `vendor/fonts/inter/` (the same release as the upright faces) and
  registered at startup with JetBrains Mono's Italic and Bold Italic
  (`bundled_fonts` in `main.rs`), so terminal italics no longer fall back
  to a system font either. `scripts/package-linux.sh` copies the whole
  fonts folder, so packages carry them.
- [x] **Even out the space around terminal blocks**
  ([screenshot](images/rust-terminal-block-spacing.png)). Done:
  `RowLayout` in `terminal_view.rs` collapses the blank padding between
  two blocks (up to 3 rows) to one 6 pt gap and draws the separator in
  its middle, so every block has 3 pt above and below; washes run
  separator to separator, and the pointer, the hover toolbar, chips and
  input method windows follow the moved rows. Swift's `collapsedRows`
  only collapses rows between a block's end mark and the next prompt
  mark, which with Impulse's integration never happens in fish (both
  marks come from one `fish_prompt` handler). The screenshot's blank row
  comes from the user's own `fish_prompt` handler running before
  Impulse's, which puts it inside the block above; a prompt starting with
  a newline (Starship's default) puts it inside the block below. The Rust
  app counts both as padding, so the spacing is even either way.

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
| Hidden unchanged lines side by side in the Diff View | folds can be offered (`apply_highlighter_fold_candidates`) but only a gutter click folds one, and the highlighter replaces them on each reparse, so the live buffer can't hide lines from outside | |
| Announcing agent state to screen readers | GPUI has no live regions | |
| Smooth scrolling | not supported | `smooth_scrolling` |

## 4. Known issues

- [x] **Find (Ctrl+F) in an editor said "Find isn't in the Rust app
  yet."** `Workbench::run_command` only handled Find for a terminal; an
  editor fell through to the catch-all for commands with no handler.
  Fixed: Find opens GPUI Kit's search panel (`open_search`) in an editor,
  and does nothing in Review, History, images and tool tabs, as in Swift.
  Every command id in the catalog (palette, menus, keybindings) has a
  handler; this was the only one that reached the catch-all from a
  surface it should work on. Nothing scripted it: the smoke scenarios and
  docs shots drive the editor through `edit=`, not the command.
- [ ] **First keystroke lost in the editor's search field** under Xvfb.
  Focus moves there at once in GPUI Kit's code, so this looks like X11
  key input without a window manager. Check on a real Linux desktop and on
  the Mac.
- [ ] **Hover popover overlap.** GPUI Kit's hover popover draws a code
  block's language label on top of the code
  ([screenshot](images/rust-editor-hover.png)). The cause, in GPUI Kit
  0.8.0: a code block puts its language label 7 px from the top and pads
  its top by 28 px to clear it (`gpui-base`'s `text/node.rs`), but the
  editor popovers style code blocks with `p_0()`
  (`gpui-component`'s `input/popovers/mod.rs`, `render_markdown`), which
  cancels that padding. The fix belongs there (hide the label in
  popovers, as Monaco shows none, or keep the top padding). Working
  around it here would mean dropping the fence's language, and with it
  the hover's syntax colors.

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
- No inline blame in the editor: dropped on purpose (it was clunky).
  Show Commit for This Line stays, blaming the cursor's line when asked.
