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

Each is marked `// Impulse:` in the code.

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

### gpui-component

- **Hover popover placement** (`input/popovers/hover_popover.rs`): the room
  above a word is measured from the editor's top, not the window's, so a
  hover on the first lines opens below the word instead of over the tabs and
  titlebar. The diagnostic popover shares it.
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
- **Scrollbar gutter** (`input/editor.rs`, `input/input.rs`):
  `Editor::scrollbar_gutter(width)` keeps room between the text and the
  vertical scrollbar, for a minimap there as in Monaco (0 by default).
