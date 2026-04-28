# Repository Guidelines

## Project Structure & Module Organization

Impulse is a Rust-first workspace with platform frontends. Shared backend code lives in `impulse-core/src`, editor protocol and bundled web assets live in `impulse-editor/`, terminal emulation code lives in `impulse-terminal/`, and the C FFI layer for macOS lives in `impulse-ffi/src`. The Linux app is a GTK4/libadwaita frontend in `impulse-linux/src`, including VTE terminal, WebKitGTK editor, sidebar, settings, and window modules. The macOS app is a separate Swift package under `impulse-macos/Sources`. Static assets, screenshots, and icons are under `assets/`; release and maintenance scripts are under `scripts/`.

## Build, Test, and Development Commands

Use the existing scripts and Cargo targets instead of ad hoc build steps.

- `cargo build` — build the Rust workspace on Linux.
- `cargo run -p impulse-linux -- --dev` — run the Linux app in dev mode.
- `cargo check` — fast type-check before a full build.
- `cargo test` or `cargo test -p impulse-core` — run workspace or crate-specific tests.
- `cargo fmt` and `cargo clippy` — required formatting and lint passes.
- `./impulse-macos/build.sh --dev` — build the macOS app bundle.
- `./scripts/install-lsp-servers.sh` — install managed LSP servers.
- `./scripts/release.sh <version>` — the only supported release path.

## Coding Style & Naming Conventions

Follow `cargo fmt` defaults for Rust and keep modules focused by feature (`git.rs`, `search.rs`, `theme.rs`). Use `snake_case` for Rust files, functions, and modules; `CamelCase` for Swift types. Prefer putting cross-platform behavior in `impulse-core` or `impulse-editor`; keep frontend-specific UI wiring in `impulse-linux` or `impulse-macos`. Do not hand-edit vendored Monaco assets in `impulse-editor/vendor`; use `scripts/vendor-monaco.sh` when updating them.

## Testing Guidelines

Most tests are inline `#[cfg(test)]` module tests within the Rust crates rather than a top-level `tests/` tree. Add tests next to the code you change, especially in `impulse-core`, `impulse-editor`, and `impulse-terminal`. Before opening a PR, run `cargo test`, then run the most relevant app build for the platform you touched.

## Commit & Pull Request Guidelines

Recent history uses short, imperative, sentence-case commits such as `Add OSC 8 hyperlink support`; release commits use `Release vX.Y.Z`. Keep commits narrowly scoped. PRs should explain the user-visible change, list the commands you ran, link related issues, and include screenshots for Linux GTK or macOS UI changes. If a feature touches both frontends, update both, or document the gap explicitly.
