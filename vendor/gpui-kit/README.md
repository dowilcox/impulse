# GPUI Kit, vendored

`gpui-base` and `gpui-component` from [GPUI Kit](https://github.com/longbridge/gpui-kit)
0.8.0 (commit `1676826d9d49b90f4d7b6808b69924f58796f6f1`, `crates/base` and
`crates/component`), the version `impulse-app` pins. They are copied as
crates.io published them (without their `Cargo.lock`), and the workspace's
`Cargo.toml` puts them in place of the published crates with
`[patch.crates-io]`. The other GPUI Kit crates (`gpui-kit`,
`gpui-kit-assets`, `gpui-component-macros`) still come from crates.io.

They're here so the editor's gaps that can only be fixed inside GPUI Kit
can be fixed now. Keep each change small, mark it in the code with a
`// Impulse:` comment that says why, and list it below, so it can be
offered upstream and dropped when it lands there.

To move to a newer GPUI Kit: bump the pin in `impulse-app/Cargo.toml`,
copy the new release's two crates over these, apply the patches below
again (or drop the ones upstream now has), and update this file.

## Patches

Each is marked `// Impulse:` in the code (`; Impulse:` in a query).

### gpui-base

Monaco's editor options live in a new file, `input/base/editor_options.rs`
(enums, `EditorColors`, their setters, and folding from code), stored in the
state's `editor_options`; every option defaults to GPUI Kit's behavior. Colors
go through `set_editor_colors` rather than new `InputEditorStyle` fields,
because gpui-component builds that struct as a full literal.

- **Line number colors** (`input/base/element.rs`): the theme's
  `editor.line_number` and `editor.active_line_number` when the host sets them
  (Monaco's `editorLineNumber.*`), instead of `muted_foreground`.
- **One scroll axis per gesture** (`input/base/state.rs`,
  `EditorOptions::predominant_axis`): a trackpad gesture locks to its first
  movement's axis through its momentum, as VS Code's `scrollPredominantAxis`.
- **Selection in an inactive window** (`input/base/element.rs`): the selection
  and search matches stay, in the host's inactive color
  (`editor.inactiveSelectionBackground`); the caret stays hidden.
- **Fold chevrons on mouseover** (`input/base/element.rs`,
  `set_folding_controls`): Monaco's `showFoldingControls`.
- **Arrow over the gutter** (`input/base/element.rs`): not the text's I-beam.
  The one patch that isn't an option, since an I-beam there is a bug.
- **No crosshair under ⌥** (`input/base/state.rs`): the host can keep the
  I-beam, as Monaco does; ⌥-drag still selects a column.
- **Hover delay** (`input/editor/lsp/mod.rs`, `input/editor/lsp/hover.rs`,
  `set_hover_delay`): 150 ms by default.
- **A hover answered after the pointer left** (`input/editor/lsp/hover.rs`):
  is dropped. The editor hears mouse moves only while the pointer is over it,
  so nothing took such a hover's highlight away. Not an option: a bug.
- **Bounds of a range's visible part** (`input/base/state.rs`,
  `visible_range_bounds`): for the hover popover, whose range can run past
  the last visible line.
- **Folding from the keyboard** (`editor_options.rs`,
  `input/editor/display_map/display_map.rs`): fold and unfold at the cursor,
  fold and unfold all, over the ranges the display map exposes.
- **Caret styles and blinking** (`input/base/element.rs`,
  `input/base/blink_cursor.rs`): Monaco's `cursorStyle` (line, block,
  underline, line-thin, block-outline, underline-thin) and `cursorBlinking`
  (blink, smooth, phase, expand, solid), timed as Monaco's CSS.
- **Whitespace markers** (`input/base/element.rs`, `input/base/layout.rs`,
  `input/editor/display_map/text_wrapper.rs`): Monaco's `renderWhitespace`
  (none, all, boundary, selection, trailing).
- **Auto-closing brackets** (`input/base/state.rs`,
  `set_auto_close_brackets`): Monaco's `autoClosingBrackets`
  (languageDefined, beforeWhitespace, always); quotes keep the language's.
- **Whole-word and regex search** (`input/editor/search.rs`,
  `input/mod.rs`): `SearchOptions { case_sensitive, whole_word, regex }`
  for a custom find bar's toggles, kept in `SearchSession.options`;
  `InputBaseState::set_search_query_with` returns the match count, or an
  invalid regex's message (also in `SearchSession.error`), which leaves no
  matches instead of panicking. Whole words are bounded as Monaco bounds
  them; regexes use the `regex` crate with `^`/`$` at line ends and skip
  empty matches. `set_search_query` and the built-in panel keep their
  behavior (literal, case only).
- **Stepping from the cursor** (`input/editor/search.rs`,
  `search_match_from`): makes the match nearest an offset, forward or back
  and wrapping, the current one and reveals it, as Monaco's find widget
  moves from the selection; `next_search_match` still steps from the
  current match.
- **Scrollbar mode** (`editor_options.rs`, `input/base/element.rs`,
  `set_scrollbar_mode`): the editor's scrollbars can show always (or on
  hover) whatever the theme's `scrollbar_mode`, as Monaco's vertical slider
  stays shown. `None` (the default) follows the theme.

- **Placeholder color** (`input/base/state.rs`, `input/base/element.rs`):
  `InputState::placeholder_color`, for a field whose placeholder isn't
  `muted_foreground` (Impulse's search fields, as NSSearchField draws them).
- **Selectable text for a document** (`selectable_text.rs`, its export in
  `lib.rs`), for the Markdown preview: `SelectableText::runs` lays the text
  out in styled runs; `on_click(ranges, f)` makes links, with a pointing
  hand, that a press and release on the same link without dragging follows
  (a drag from one selects); `group(&SelectableTextGroup)` makes separate
  elements (a paragraph wrapped word by word) one participant, selected,
  double-clicked and copied as one text; `document(&SelectableTextDocument)`
  and `SelectableTextDocument::select_all` select a document's runs whole
  until the next press, as ⌘A. Each is off unless used. Two fixes that
  aren't options: a selection change notifies the view the text is in, so
  one under a cached view (`Entity::cached`) repaints its selection rather
  than replaying the frame before; and a run outside the selection no
  longer asks for the window's selected text twice a frame (a walk over
  every participant), which can't change unless the run is selected.
- **View zones** (`input/editor/display_map/view_zones.rs`,
  `input/base/view_zones.rs`; `display_map.rs`, `text_wrapper.rs`,
  `element.rs`, `layout.rs`, `state.rs`, `indent.rs`): Monaco's
  `changeViewZones`. `ViewZone::new(before_line)` (or `after_line`) with
  `.lines(n)` and/or `.px(h)` and an optional `.element(f)`;
  `EditorState::add_view_zone`, `remove_view_zone`, `replace_view_zones`
  and `view_zone_bounds`. A zone is a blank band between two lines (or
  above the first) that pushes the lines under it down; it isn't a display
  row, so it has no line number and the caret, selections and vertical
  movement step over it. The display map places zones after the fold
  projection (`DisplayMap::display_row_top`, `display_row_at_y`,
  `content_height`), and every row-to-pixel step goes through it: the
  visible range, the scroll height, carets and scroll-into-view, hit
  testing, selections and decorations, line numbers, the current line,
  fold chevrons and indent guides (a line's `LineLayout::zone_after` counts
  the zones under it). A zone under a hidden line hides with it; zones move
  with edits. An element is built each frame its zone is on screen, laid
  out over the zone (the gutter included; `ViewZoneContext::text_left` says
  where the text starts) and painted over it; a press in a zone doesn't
  move the caret, and the pointer is an arrow there.
- **Hidden lines from the host** (`input/editor/display_map/fold_map.rs`,
  `display_map.rs`, `input/base/view_zones.rs`, `state.rs`): Monaco's
  `setHiddenAreas`. `EditorState::set_hidden_ranges` hides whole buffer
  lines apart from the folds, so they last through the highlighter's
  reparses; they move with edits (an edit inside one shows it) and the
  caret steps over them (`clamp_offset_to_visible_*` now step over any
  hidden run, folds as before). A view zone above a range stands for it.
  The scroll height now counts the shown rows rather than every wrap row,
  so folded and hidden lines no longer leave room past the end.

- **Line decorations width** (`editor_options.rs`, `element.rs`,
  `set_line_decorations_width`): Monaco's `lineDecorationsWidth` with
  `folding: false`, as its diff editor's gutter has them: the numbers end
  that far before the text, in place of GPUI Kit's margin and fold chevron
  lane; no chevrons show (folds stay). `None` (the default) keeps GPUI
  Kit's.
- **Line backgrounds** (`editor_options.rs`, `element.rs`,
  `set_line_backgrounds`): whole-line backgrounds (Monaco's `isWholeLine`
  decorations), buffer lines and colors painted in order under the text,
  the selection and range fills, from the text column across the editor,
  each line's wrapped rows and not the view zones under it. The host sets
  them again after edits.

- **Inlay hints** (`input/editor/inlay_hints.rs`; `display_map/inline_line.rs`,
  `display_map/text_wrapper.rs`, `element.rs`, `editor/mod.rs`, `state.rs`):
  Monaco's `inlayHints`. `EditorState::set_inlay_hints(Vec<InlayHint>)`
  (`InlayHint::new(offset, label)` with `.padding_left` and
  `.padding_right`), `inlay_hints` and `set_inlay_hints_visible`. A line
  with hints is laid out in fragments (`InlineFragment::inlay`, an empty
  range at the hint's offset), so the text after a hint moves right, the
  caret at its offset sits after it (Monaco's cursor stop), a click on it
  lands at its offset, the caret steps over it and a selection across it
  covers it. Labels are shaped in the editor font in
  `EditorColors::inlay_hint` over `inlay_hint_background`, with a third of
  the font size for each padding. Soft wrapping counts their widths (the
  wrap map's inline metrics; a line with hints still breaks at word
  boundaries), the scroll width counts them, and they move with edits (a
  type after a name moves on with text typed at it, a parameter name
  stays before an argument; one inside replaced text goes).
- **Snippets** (`input/editor/snippet.rs`; `editor/lsp/overlay.rs`,
  `kind.rs`, `editor/mod.rs`, `indent.rs`, `state.rs`, `element.rs`): a
  completion with `insertTextFormat: 2` goes in as Monaco's snippet
  controller puts it, its `additionalTextEdits` beside it in one undo step.
  LSP's snippet syntax is parsed (tab stops, placeholders, nested ones,
  choices, variables with defaults, transforms, escapes); each line after
  the first takes the line's indentation, a leading tab the editor's
  indent. Its stops become a session: a stop's places selected together,
  Tab and Shift+Tab to the next and previous (`InputModeKind::snippet_jump`),
  Escape (`leave_snippet`), reaching `$0` or a cursor leaving every stop
  ends it; placeholders are tinted in `EditorColors::snippet_tabstop`, and a
  choice offers its options in the completion menu. Public:
  `insert_snippet`, `jump_in_snippet`, `leave_snippet`, `in_snippet`,
  `set_snippet_variables` (the host's `TM_FILENAME` and the like; the editor
  resolves the selection, line, word, date, random and UUID ones).
- **Smooth scrolling** (`editor_options.rs`, `state.rs`, `element.rs`,
  `set_smooth_scrolling`, `animate_scroll_offset`): Monaco's
  `smoothScrolling`: scrolls the editor makes itself (`scroll_to`, so keys
  that move the cursor off screen, Page Up and Down, revealing) ease out
  over 125 ms, stepped each animation frame as the overscroll bounce is.
  Wheels and trackpads stay direct and take over from an animation;
  `set_scroll_offset` stays a jump. Off by default, and with Reduce Motion.
- **Home for every cursor** (`editor_options.rs`, `movement.rs`;
  `select_all_cursors_to` made `pub(super)`): `EditorState::smart_home`
  (and `smart_home(true)` for Shift+Home) takes each cursor to its own
  line's first non-blank character, then to column 0, on a wrapped line's
  later rows to the row's start first; `selected_ranges` reads every
  selection. GPUI Kit's `MoveHome` is unchanged.

### gpui-component

- **Hover text at the editor's size** (`input/popovers/hover_popover.rs`,
  `input/popovers/mod.rs`): hovers, diagnostics and their code blocks use the
  theme's `mono_font_size` (the host sets it to the editor's font size), not
  `text_xs` and an 11px code block.

- **Hover popover placement** (`input/popovers/hover_popover.rs`): the room
  above a word is measured from the editor's top, not the window's, so a
  hover on the first lines opens below the word instead of over the tabs and
  titlebar. The diagnostic popover shares it.
- **Hovers past the visible lines** (`input/popovers/hover_popover.rs`): a
  hover is anchored on its range's visible part (`visible_range_bounds`).
  `range_to_bounds` has nothing for a range that ends below the last visible
  line (a YAML key's hover covers the key's whole block), so such a hover
  painted its highlight over the whole block with no popover, and without
  the popover nothing took the highlight away. Not an option: a bug.
- **No language label on popover code blocks** (`input/popovers/mod.rs`,
  `render_markdown`): the label drew over the code, since the popovers cancel
  the padding it needs; Monaco's hovers show none. The fence's language still
  picks the syntax colors.
- **Completion kind icons** (`input/popovers/completion_menu.rs`):
  `CompletionMenuSettings { show_icons }` on the theme (off by default), a
  Lucide icon per LSP `CompletionItemKind` in Monaco's colors, and
  `CompletionIconAssets`, an asset source with just those icons for the app
  to compose with its own.
- **Caret popups flip above** (`input/popovers/caret_anchored.rs`): the
  completion and code-action menus open above the caret when they don't fit
  below it, inside the editor and the window, and there's more room above.
- **Editor popover background** (`highlighter/registry.rs`,
  `highlighter/wasm_stub.rs`, `input/popovers/mod.rs`): a highlight-theme key,
  `editor.popover.background` (Monaco's `editorHoverWidget.background` and
  `editorSuggestWidget.background`), for hovers, completions and the
  completion docs; the theme's `popover` color when it's unset.
- **First parse colors the file** (`highlighter/input_adapter.rs`): while
  the highlighter has no tree yet, the synchronous parse gets 25 ms instead
  of 2 ms and the background parse starts without its 150 ms debounce, so
  a file opens colored rather than gaining its colors a moment later.
  Later edits keep the 2 ms budget and the debounce.
- **Captures for a host** (`highlighter/highlighter.rs`,
  `SyntaxHighlighter::captures`): the capture names over a range, injected
  languages included, in precedence order, for a host that colors them its
  own way (Review imitates highlight.js's classes from them and the text
  around them).
- **Conflict markers left out of the parse** (`highlighter/highlighter.rs`,
  `conflict_free_ranges`; `highlighter/input_adapter.rs`): the lines of a
  complete merge conflict's markers (`<<<<<<<`, `|||||||`, `=======`,
  `>>>>>>>`) are excluded with `Parser::set_included_ranges`, in the
  synchronous and the background parse, so both sides parse as code and keep
  their colors, as Monaco's line tokenizer does. A marker inside a function
  otherwise turns the rest of it into an error node.
- **Markdown tables and fences** (`highlighter/languages/markdown/highlights.scm`):
  a code fence and its info string are `@string`, the table header and
  delimiter rows and the other rows' pipes `@keyword`, as Monaco's Markdown
  tokenizer colors them.
- **View zone exports** (`input/mod.rs`): `ViewZone`, `ViewZoneContext`,
  `ViewZoneId` and `ViewZoneRenderer` from gpui-base's input.
- **Scrollbar gutter** (`input/editor.rs`, `input/input.rs`):
  `Editor::scrollbar_gutter(width)` keeps room between the text and the
  vertical scrollbar, for a minimap there as in Monaco (0 by default).
