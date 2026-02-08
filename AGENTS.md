# Repository Guidelines

## Project Structure & Module Organization
This repository is a Rust workspace with two crates:

- `impulse-core/`: platform-agnostic backend logic (PTY, shell integration, filesystem, search, git, LSP).
- `impulse-linux/`: GTK4/libadwaita Linux frontend (`src/main.rs` entrypoint, UI modules in `src/*.rs`).

Top-level files:

- `Cargo.toml`: workspace definition.
- `Cargo.lock`: locked dependency graph.
- `target/`: build output (ignored by git).

Keep dependency direction one-way: `impulse-linux` may depend on `impulse-core`, never the reverse.

## Build, Test, and Development Commands
Run commands from repository root:

- `cargo build`: build all workspace members.
- `cargo build -p impulse-core`: build backend crate only.
- `cargo build -p impulse-linux`: build GTK frontend only.
- `cargo run -p impulse-linux`: launch the app locally.
- `cargo check`: fast type-checking without full build.
- `cargo test`: run all tests in workspace.
- `cargo test -p impulse-core`: run tests for core crate only.
- `cargo fmt && cargo clippy`: format and lint before opening a PR.

Note: building `impulse-linux` requires GTK4/libadwaita/VTE/GtkSourceView development libraries installed on your system.

## Coding Style & Naming Conventions
- Use standard Rust formatting (`cargo fmt`) with 4-space indentation.
- Keep modules focused by feature (`pty.rs`, `sidebar.rs`, `terminal.rs`).
- Use `snake_case` for files, modules, and functions; `PascalCase` for structs/enums/traits.
- Prefer `Result<T, String>` and explicit error propagation in core APIs.
- Use small, targeted comments only where control flow or state sharing is non-obvious.

## Testing Guidelines
- Add unit tests next to the code they validate using `#[cfg(test)] mod tests`.
- Name tests by behavior, e.g. `parses_osc_133_sequences`.
- Prioritize coverage for core logic in `impulse-core`; UI behavior in `impulse-linux` should be validated with focused integration-style tests where practical.
- Run `cargo test` locally before pushing.

## Commit & Pull Request Guidelines
- Follow the existing history style: imperative, concise subject lines (e.g. `Fix LSP race conditions`).
- Keep commits scoped to one logical change.
- PRs should include:
- A short problem/solution description.
- Testing performed (`cargo test`, manual UI checks).
- Linked issue(s), if applicable.
- Screenshots or screen recordings for UI-visible changes.
