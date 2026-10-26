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
