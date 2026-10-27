# Images in the terminal: plan

Plan, proposed 26 Oct 2026. Not user documentation: this folder isn't published to the website (`scripts/build-site.py` only builds `docs/*.md`).

## Status

Proposals 1 to 5 are built (26 Oct 2026): `impulse-terminal/src/graphics/` (command parsing, `ImageStore`, placeholders, decoding), the scanner's APC and CSI events, `OutputPipeline::graphics` and `erase_display` in `backend.rs`, `impulse-app/src/terminal_images.rs` and the hold-⌘ preview in `terminal_view.rs`, the `terminal_images` setting, docs, and the `images` smoke scenario. Where it differs from the plan below:

- Graphics APCs still reach vte, which discards them; leaving them out through `OscSkip` saved nothing worth the bookkeeping.
- A zlib-compressed PNG is inflated on the reader thread (its size is in its header); raw formats inflate on the decoding thread with the rest.
- Two limits more: 4096 placements per terminal and 2000 cells a side per placement.
- Erasing with `CSI 0 J` or `CSI 1 J` leaves images alone; only `CSI 2 J` (the screen) and `CSI 3 J` (the scrollback) remove them.
- Agents' block output leaves images out entirely (no `[image]` line).
- Named files (`t=f`) are read after all, PNGs only and never answered (`read_named_png`): `kitten icat` takes a working `t=t` to mean files work and then sends a PNG by its path, which it couldn't show otherwise.
- Impulse answers primary device attributes (`CSI c`) itself, `ESC [ ? 62 ; 22 c` as Ghostty does, keeping the request from alacritty: kitten never accepts alacritty's `ESC [ ? 6 c` and waited forever (found trying `kitten icat`).

Tried in the app (26 Oct 2026, Linux, XWayland): direct placements, placeholders and `clear` with a script; `kitten icat` detects support (`files`) and shows images, the cursor ending below them.

Tried again (27 Oct 2026, Linux, Xvfb with software Vulkan, through the smoke runner's headless mode), which found three bugs, now fixed:

- `chafa -f kitty` showed nothing: chafa encodes each chunk on its own and leaves the unused bits of the character before the padding set, which the `base64` crate refuses by default. Kitty decodes each chunk on its own and doesn't check those bits; now neither does Impulse (`BASE64` in `store.rs`).
- Placeholders through tmux showed nothing: tmux redraws a cell in color 1 as `SGR 31`, which alacritty keeps as a named color, and `color_id` read only indexed and 24-bit ones. The 16 named colors now count as their indexes, as in kitty.
- `kitten icat --place` cut its image off: icat places it by screen position with `C=1` and leaves the cursor at its top left, so the rows shown at the prompt ended at the last output and the rest of the image wasn't drawn. Rows under an image hanging below the last output now count as content (`layout_rows`), so it shows whole above the input bar. The shell's next prompt still prints over the image's first rows, as in kitty.

Still to do:

- [ ] yazi, not tried yet: the newest yazi on crates.io doesn't build from its package, and an older one wasn't tried.
- [x] `kitten icat --place`: see above.
- [x] The `images` smoke scenario passes under Xvfb (27 Oct), as do `terminal-*` and `session`, and on the Mac (27 Oct, macOS 26), where the docs shot `terminal-images.png` shows `kitten icat`'s image too.
- [ ] The test matrix by hand (below). Done on Linux: chafa 1.14 and timg 1.5.2 draw with `-f kitty` and `-pk`, but pick text by themselves: they choose from known terminals' environment variables and ask nothing (timg asks only the background color). In tmux 3.4 both show images by placeholders once `allow-passthrough` is on, and timg's (a random 32-bit image id, sent as a 24-bit color) only once tmux knows the terminal has RGB (`terminal-features ',xterm-256color:RGB'`); without it tmux rounds the color to the nearest of 256. Both are in `docs/terminal.md`. Not done: yazi, image.nvim, snacks.nvim, matplotlib's kitty backends, Pi, grok, `kitten icat` itself (kitty isn't installable here), ssh, and anything on the Mac.
- [x] The cursor after a placement, at the right edge and bottom too, checked against kitty's source (0.48.2: `handle_put_command` in `graphics.c`, `screen_handle_graphics_command` in `screen.c`): right by the columns and down by the rows less one, wrapping to the next line at the right edge and scrolling at the bottom margin, as `ImageStore::place` does.
- [ ] A docs screenshot of an image in a block: the `terminal-images` shot is defined (`scripts/docs/shots.py`, with a stand-in `kitten` in the demo home, `scripts/docs/kitten.sh`, and a chart `make_demo.py` draws). Capture it on the Mac (`scripts/docs/capture.py terminal-images`), as the other shots are, then show it in `docs/terminal.md`'s Images section.
- [x] `scripts/check-macos.sh` passes.

## The problem

Programs that show pictures in a terminal print nothing in Impulse. A plotting script, `kitten icat`, yazi's previews, image.nvim and snacks.nvim in Neovim, and agents that preview a pasted screenshot (Pi, grok) all speak the [kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/). It is APC escape sequences (`ESC _ G <control data> ; <base64 payload> ESC \`), which vte reads into its `SosPmApcString` state and discards, so alacritty_terminal never sees them and neither do we. Kitty, Ghostty and WezTerm draw them; so does Kero (on libghostty), which competes with us directly.

What Kero's changelog shows users run into, and what this plan has to get right from the start:

- **Kitty's protocol first.** It's what Kero shipped (2.0.6) and what the programs above speak. iTerm2's `OSC 1337;File=` and sixel can come later, or never ([Later](#later)).
- **Unicode placeholders aren't optional.** yazi showed "a box of placeholder characters" until Kero drew them (2.1.15). It's the only mode that works through tmux and over SSH.
- **Images go when the screen is cleared.** Pi's `/new` left its images on screen (fixed in 2.1.22). An image belongs to the grid, not to a layer over it.
- **Hovering an image's path previews it.** Cheap, and it works with any program, `ls` included.

Images already go the other way: pasting one into a terminal saves it and pastes its path (`TerminalView`, `terminal_view.rs:1790`), and so does the input bar (`terminal_input_bar.rs:252`).

## How the protocol puts an image on screen

Two steps: the program **transmits** image data under an id (`a=t`), then **places** it (`a=p`); `a=T` does both. Data comes in the escape sequence (`t=d`, base64, chunked with `m=1` in pieces of at most 4096 bytes), from a file (`t=f`), a temporary file the terminal deletes (`t=t`) or shared memory (`t=s`). Formats are raw RGB (`f=24`), RGBA (`f=32`) or PNG (`f=100`), optionally zlib-compressed (`o=z`). The terminal answers each command (`ESC _ G i=<id>;OK ESC \`, or an error such as `ENOENT:…`) unless told to keep quiet (`q=1`, `q=2`); programs detect support by sending a query (`a=q`) followed by a device-attributes request and seeing which answer comes first.

A placement is one of two kinds, and they need different machinery:

- **Direct placements.** The image is drawn at the cursor, over `c`×`r` cells (or as many as its pixels need), at a z-index (`z<0` under the text), with optional source rectangle (`x,y,w,h`) and pixel offsets inside the first cell (`X,Y`). The cursor then moves past the image unless `C=1`. Text printed over it doesn't erase it; delete commands (`a=d`, by id, placement, cell, column, row, z-index or all) and clearing the screen do.
- **Virtual placements with Unicode placeholders** (`U=1`). The terminal draws nothing at the cursor. The program prints U+10EEEE cells itself, each one's foreground color holding the image id, its underline color the placement id, and its combining marks (from kitty's table of 297 diacritics) the row and column of the image it shows; cells can leave the marks out and continue from the cell to their left. The terminal draws the matching piece of the image in each cell. Because the image *is* text, it scrolls, wraps, survives tmux and SSH, and is cleared by anything that clears text.

## Proposals

Sizes are rough: **S** is a day or two, **M** about a week. 1 comes first; 2 and 3 each need it; 4 needs nothing and can ship first.

### 1. The protocol in impulse-terminal

**Size M.** Gives: images stored and answered for; nothing drawn yet. All of it is UI-free and tested headless.

- **Finding the sequences.** `OscScanner` already follows vte's state machine so it agrees with alacritty on where each OSC ends; it learns APC the same way (new `ApcBody` and `ApcEscape` states, `ESC _` from `Escape`), passing on APCs that start with `G` as a new event, `OscEvent::Graphics(Vec<u8>)`, with offsets like the others. Graphics APCs are left out of the parser's input through the existing `OscSkip` spans (`abort: false`), since vte would only discard them, and other APCs pass through untouched. A single APC is capped (4 MiB, as `MAX_CLIPBOARD_OSC_LEN` is for OSC 52); one over it is dropped, as an oversized OSC is.
- **Handling them where the cursor is.** `OutputPipeline::process` parses up to each event's offset before handling it, so the cursor is where the image goes. Graphics events get the same treatment, in a new `graphics` module (`impulse-terminal/src/graphics/`: `command.rs` parsing the control data, `store.rs` the images and placements, `placeholder.rs` the diacritics table and cell decoding).
- **Transmission.** `t=d` with chunking, and `t=t` under kitty's own rules: the file must be a regular file in the system's temporary folder with `tty-graphics-protocol` in its name, and is deleted after reading. `t=f` and `t=s` answer `EINVAL` for now: `t=f` lets any output Impulse prints (`cat` of a hostile file, a remote server over SSH) make it read local files, and the OK-or-ENOENT answer tells the sender whether a path exists. `kitten icat` and the other clients fall back to `t=d` when a query says the faster media don't work.
- **Decoding.** PNG with the `png` crate (already in the tree through GPUI), zlib with `flate2` or `miniz_oxide`. A PNG's size comes from its header, at once, so placements and answers don't wait. The full decode runs off the reader thread, so a huge image can't stall the terminal's output, and sends a new `TerminalEvent::ImagesChanged` when done so the view redraws. Images are kept as RGBA in an `Arc<[u8]>` with a generation number.
- **Answers.** Through the same `TerminalEvent::PtyWrite` channel alacritty's own replies use (`backend.rs:107`), sent as the command is handled. A query's answer then reaches the PTY before the device-attributes reply that follows it in the stream, which is what detection checks. `q=1` and `q=2` are respected.
- **Limits.** Each image at most 10000 px a side and 64 MiB decoded; each terminal at most 256 MiB of images, freeing the oldest unplaced ones first, then the oldest, as kitty does with its quota. Numbers are proposals; kitty's own are the reference.
- **Deletes.** Every `d=` target, lowercase removing placements and uppercase also freeing the data no placement uses.
- **Pixel sizes that match.** Programs size images from the cell's size in pixels, from `TIOCGWINSZ`, `CSI 14 t` (which alacritty answers from the same `WindowSize`) or `CSI 16 t` (which it doesn't). Today `TerminalView::fit` reports the cell in logical pixels, truncated (`f32::from(m.width) as u16`), so on a 1.6× display an image a program sizes for 10 cells is drawn 1.6 times too large or scaled down blurry. Report device pixels, rounded (the cell's size times the window's scale factor), work out an image's natural cell span from the same numbers, and answer `CSI 16 t` from the scanner. Nothing else reads these values.
- **The setting.** `terminal_images` (on by default), one line in `impulse_kit::settings`'s `settings!` list plus its `SettingItem`. Off, queries answer an error so programs fall back to text, and other commands are ignored.

**Tests.** Byte streams fed through `TerminalBackend` headless, as the existing `PtyWrite` tests do (`backend.rs:2662`): chunked and one-piece transmissions, each format and compression, the answers and quiet modes, detection's query-then-DA1 order, the limits, every delete target, an APC split across reads, and an APC inside an OSC and the reverse. There are no golden fixtures for this (v0.29.0 had no images); kitty itself is the oracle, checked by hand with `kitten icat` and the protocol's examples, and anything we copy from it says so in a comment.

### 2. Unicode placeholders

**Size S–M.** Gives: yazi, image.nvim and snacks.nvim, through tmux and over SSH. Smaller than direct placements, and the grid does most of the work.

- **The snapshot carries what's needed.** The grid buffer already sends combining marks (`EXTRA_ZERO_WIDTH`) and underline colors (`EXTRA_UNDERLINE_COLOR`) per cell. The image id, though, is the cell's foreground as the program set it, a 256-color index or an RGB value, and the buffer only has the theme's resolved color. So `write_grid_to_buffer` decodes placeholder cells itself (the raw foreground and underline colors, the diacritics, continuation from the left) and sends a new extras kind, `EXTRA_IMAGE_CELL`: image id, placement id, row and column in the image. `Grid::read` (`terminal_view.rs:330`) reads it next to the others.
- **Painting.** `ShapedRow` skips placeholder cells, so no glyph is shaped. Each run of cells in a row showing consecutive columns of one placement is one `window.paint_image` call: `bounds` the run's cells, `image_bounds` the whole placement's rectangle (the image fitted into its `c`×`r` cells, centered, as kitty draws it) shifted so the run's row and column line up. GPUI clips to the intersection. A cell whose image isn't there (yet, or any more) is drawn blank.
- **GPUI images.** `TerminalView` keeps one `Arc<RenderImage>` per image and generation, converted from RGBA to GPUI's BGRA, and gives each back with `window.drop_image` when the backend frees it, so the GPU atlas doesn't keep it.
- **Text paths.** Placeholder characters are left out wherever the grid becomes text: copying a selection, the transcripts session restore saves (`transcript.rs`), search, hints, and block output for agents (`impulse blocks`, `get_block_output`).

### 3. Direct placements

**Size M.** Gives: `kitten icat`, plotting libraries, agents' previews, and every program that doesn't use placeholders.

The choice here is between a layer of placements beside the grid, as kitty and Ghostty keep, and writing placeholder cells into the grid ourselves, which would reuse 2 for everything. The cells win on scrolling and clearing but break TUIs: yazi places an image over cells it believes hold spaces, and when it deletes the image it expects whatever was under it back, which overwritten cells can't give. So a layer, kept the way command-block rows already are.

- **Anchoring.** Each placement keeps its screen (primary or alternate), its absolute row on the same numbering as `CommandBlockTracker`'s (`absolute_cursor_row`, with the base bumped in `OutputPipeline::trim` when scrollback is evicted, which also drops placements that left it), its column, cell span, source rectangle, offsets and z-index. On a resize that reflows, its row moves with the text it was on, through the same mapping `resize_term` gives block rows (`row_positions`).
- **The cursor.** After a placement without `C=1` the cursor moves past the image as the spec says, through `Term`'s own `Handler` methods (`linefeed`, which scrolls at the bottom, and `move_forward`), so the program's colors and attributes aren't touched. Kitty's exact cursor position afterwards, especially at the screen's right edge and bottom, is checked against kitty by hand and pinned in a test.
- **Clearing.** The scanner already walks every escape; it learns enough of CSI to report erase-display (`CSI 2 J`, `CSI 3 J`), full reset (`ESC c`) and the alternate screen switching (`CSI ? 1049 h/l`, `47`, `1047`) as events at their offsets. Erasing the display removes the screen's placements, as kitty and Ghostty do (confirm the details against both, notably `CSI J` with 0 or 1); leaving the alternate screen removes its placements; the primary screen's are kept while the alternate one shows. Text printed over a placement leaves it, as in kitty.
- **What it doesn't do.** Placements move with whole-screen scrolling, not inside a scroll region (`DECSTBM`). Kitty moves them there; programs that scroll regions (Neovim, tmux) delete and place again, and image.nvim uses placeholders anyway. Animation (`a=f`, `a=a`, `a=c`) answers an error, and the first frame is shown.
- **Painting.** `TerminalBackend::image_placements` returns the viewport's placements, cached by a generation number as the grid is cached by damage. The view maps rows through `RowLayout` (whose collapsing of blank padding must count rows under a placement as content), then paints `z<0` placements after the cell backgrounds and before the text, the rest after the text's decorations, clipped to the terminal. The sticky block header covers them as it covers text.

### 4. Hold Cmd to preview an image's path

**Size S.** Independent of the protocol; can ship first.

The terminal already finds paths under the pointer and checks them off the main thread (`hover_link`, `FileChecks`). While Cmd (Ctrl on Linux, as for opening links) is held over a path that `image_view::is_image_file` accepts, a flyout beside it shows the image through GPUI's `img()`, which decodes off the main thread and reads every format the image tab reads. Scaled down to fit 320×240, never up; files over a size cap (say 20 MB) aren't opened. A click still opens the image tab, as today.

### 5. Finishing

**Size S.**

- **Smoke check.** A scenario in `scripts/smoke/scenarios.py` that prints a PNG through the protocol in zsh, once directly and once with placeholders, clears, and checks the dumped placement counts and the window image.
- **Docs.** `docs/terminal.md` (which programs work, the setting, tmux's `allow-passthrough on`), a docs shot of a plot in a block, the CHANGELOG, and CLAUDE.md's architecture tree (impulse-terminal's line lists the sequences it scans) and its Terminal key pattern.
- **A test matrix by hand**, local and over SSH, with and without tmux: `kitten icat`, yazi, chafa, timg, image.nvim, snacks.nvim, matplotlib's kitty backends, Pi and grok. Several pick their method by `TERM_PROGRAM` or `XTVERSION` before they query, and we send `TERM_PROGRAM=Impulse` (`shell_launch.rs:103`), which none of them knows; note which fall back to text and why.

## Later

- **iTerm2 images** (`OSC 1337;File=`, used by `imgcat` and WezTerm's). They need a larger OSC limit for that key and become direct placements, so they come almost free after 3.
- **Sixel.** A DCS sequence that vte hands to alacritty, which ignores it. It would need its own decoder and intercepting DCS in the scanner; worth it only if users ask.
- **Images in restored terminals.** Session restore saves text. Restored output comes back without its images, the placeholders already stripped (2). Keeping them would mean saving image data beside the scrollback files.
- **`t=f` and shared memory.** Only for speed, and only with an answer to the file-reading question above (local programs only, never through SSH, which Impulse can't tell apart).

## Open questions

- Report device pixels for every terminal (proposed), or only once a program uses graphics?
- On by default (proposed), or off until the test matrix passes?
- Should agents get images back as text, a `[image 640×480]` line in block output, rather than nothing?
