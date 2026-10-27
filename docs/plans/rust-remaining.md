# Rust app: what's left

What's left before the Rust app (`impulse-app`, on GPUI + GPUI Kit) ships.
It replaced the Swift app, which was removed in October 2026 once parity was
signed off; the migration plan is in [rust-migration.md](rust-migration.md).
The code review of October 2026, with the bugs, security and performance
findings still to fix, is [rust-review.md](rust-review.md).

Where things stand: parity with the Swift app is signed off. Before the
Swift app went, every docs shot on Linux matched it (except where the Swift
shots were older than the code or the platforms differ), and every smoke
scenario passed (`scripts/smoke/run.py`, under Xvfb).

## 1. The Mac

The largest unknown, so it goes first.

- [x] **Build and run it on a Mac.** Done: it builds, links and runs on
  macOS 26 (Apple silicon), and `cargo test --workspace` passes there.
  The first runs turned up Mac-only bugs, fixed since: a launch from
  Finder opened a workspace at `/`, the editor's scroll bounce drew over
  the tab strip, and AppKit dragged the window from the whole titlebar
  (tabs included) until the app took the drag over
  (`app_owns_titlebar_drag`, `window_frame::drag_area`).
- [ ] **Let all of the titlebar's empty space move the window.** The drag
  areas are siblings of the controls (`window_frame::drag_area` in
  `titlebar()`), so the bands above and below the 24–28 pt controls and
  the 2 pt gaps between tabs don't move the window, where Swift's
  `TitlebarDragArea` sits behind everything. Full coverage would take one
  drag area behind the whole bar and `.block_mouse_except_scroll()` on
  every control (not `occlude`, so the tab strip still scrolls), and
  every control added later would need it too. Good enough for now.
- [ ] **Check the macOS-only paths by hand**: the menu bar (`menus.rs`), the
  Dock badge (`badge.rs`), Increase Contrast and Reduce Motion
  (`display_options.rs`), the window frame saved in AppKit's coordinates
  (`session.rs`: `frame_string`, `saved_bounds`), the Quick Terminal's
  global shortcut, notifications through `osascript`, `NSBeep` for the
  terminal bell, and ⌘Z bringing back a closed tab (`UndoClose`).
- [x] **Run the snapshot mode there** (parity signed off without it; the
  smoke checks and docs shots on the Mac are still worth a run before the
  release). Window images go through
  `screencapture -l` on macOS, which hasn't been tried. Then run
  `scripts/smoke/run.py` and `scripts/docs/capture.py` against the Rust
  binary and compare the shots with the Swift app's, side by side on the
  same OS.
- [x] **Compare against the Swift app in a key window** (moot: parity
  signed off, and the Swift app is gone). The Swift docs
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
  snapshot's `diff-inline` and `diff-reveal` actions drive it). Since
  then both layouts hide unchanged runs through the vendored gpui-base's
  hidden ranges and view zones, and side by side lines its sides up with
  hatched padding zones, as Monaco's diff editor does.
- [x] **Put conflict actions inline.** Done: each conflict's Accept
  Current, Accept Incoming and Accept Both, its labels and, with more
  than one, "1 of 2" with ↑ and ↓ float on its `<<<<<<<` line after the
  marker (`EditorTab::conflict_bars` in `editor_git.rs`) and act on that
  conflict. Since then the bar sits in a 24 px view zone above the
  conflict, as Monaco's, and the git peek in one that pushes the lines
  below it down.
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

Each needs a change in GPUI Kit, now vendored (`vendor/gpui-kit/`; its
README lists the patches made so far: caret styles and blinking, whitespace
modes, auto-closing modes, keyboard folding, view zones, inlay hints,
snippets, smooth scrolling, Home for every cursor and more).

| What | Why it waits | Setting |
|---|---|---|
| Announcing agent state to screen readers (not needed for the release) | GPUI has no live regions | |

Left of the editor work done here:

- [x] **Inlay hints' extras** (not needed for the release). No tooltip on a hint, no ⌘-click on a label
  part's location or command, and no `workspace/inlayHint/refresh`
  (advertising it needs a `ServerEvent` the workbench handles), so a
  server's hints that change without an edit (rust-analyzer after
  indexing) show after the next edit or scroll. TypeScript sends none
  with Impulse's settings, in either app: its `includeInlay…`
  preferences are off unless `lsp.json` turns them on
  (`initialization_options.preferences`).
- [x] **Nested snippet sessions** (not needed for the release). A snippet inserted inside another's
  placeholder replaces the session, where Monaco merges them; choices
  with mirrors update only the first place.

## 4. Known issues

- [x] **Find (Ctrl+F) in an editor said "Find isn't in the Rust app
  yet."** `Workbench::run_command` only handled Find for a terminal; an
  editor fell through to the catch-all for commands with no handler.
  Fixed: Find opens the editor's find bar (`editor_find.rs`, the same
  `FindBar` the terminal uses) in an editor,
  and does nothing in Review, History, images and tool tabs, as in Swift.
  Every command id in the catalog (palette, menus, keybindings) has a
  handler; this was the only one that reached the catch-all from a
  surface it should work on. Nothing scripted it: the smoke scenarios and
  docs shots drive the editor through `edit=`, not the command.
- [x] **First keystroke lost in the editor's search field** under Xvfb.
  Not on a real desktop: checked by hand on KDE Plasma under Wayland, with
  the find bar that replaced GPUI Kit's search panel. It was X11 key input
  without a window manager.
- [x] **Hover popover overlap** (no longer an issue). GPUI Kit's hover popover draws a code
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

## 5. The release

- [ ] **First run of `scripts/package-macos.sh --sign --notarize --dmg`** on
  a Mac: signing, notarizing and Gatekeeper on the Rust binary, and
  installing over the Swift app (the same bundle ID and data folder:
  settings, the session, trust lists and command history carry over).
- [ ] **First run of `.github/workflows/release-linux.yml`**, which can only
  run once it's on `main`: by hand with a throwaway tag first.
- [ ] **The `.deb`**, untested (built where `dpkg-deb` is installed).
- [ ] **The glibc floor**: the workflow builds on Ubuntu 24.04, so the
  Linux packages need glibc 2.39 (no Debian 12, Ubuntu 22.04 or RHEL 9);
  building on `ubuntu-22.04` would lower it to 2.35.
- [ ] **CI for the tests**: only the website has a workflow; nothing runs
  `cargo test --workspace` on a push.
- [ ] **The user docs** (`docs/`) still describe the Swift app in places
  (inline blame, which the Rust app dropped; Monaco), and their screenshots
  are the Swift app's: `scripts/docs/capture.py` regenerates them.

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
