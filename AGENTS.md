# Repository Guidelines

`CLAUDE.md` is the detailed guide to this repository; this file is the short version for any coding agent.

## Project Structure

Impulse is a Mac-first terminal IDE written in Swift (AppKit + SwiftUI), with the terminal emulation core in Rust.

- `impulse-macos/` — the Swift package. `Sources/ImpulseApp` is the app; `Sources/ImpulseKit` holds Foundation-only logic (headless-testable); `Sources/ImpulseGit` wraps a vendored libgit2 (reads) plus the `git` CLI (writes); `Sources/ImpulseLSP` is the language-server client. `web/` holds the Monaco editor glue (the review is native).
- `impulse-terminal/` — Rust terminal emulation on `alacritty_terminal` (PTY, grid, OSC 133/7/9/6973 scanning, command blocks, history).
- `impulse-ffi/` — the C FFI over `impulse-terminal`, linked into the app as a static library.
- `vendor/` — Monaco, fonts and highlight.js (refresh with `scripts/vendor-monaco.sh`; never hand-edit).
- `docs/superpowers/` — design specs and implementation plans. The current redesign is `specs/2026-10-05-terminal-ide-redesign-design.md` and `plans/2026-10-05-terminal-ide-redesign.md`.

## Build and Test

Use the scripts; don't replicate their steps by hand.

- `./impulse-macos/build.sh --dev` — build `dist/Impulse Dev.app` (runs alongside the release app).
- `impulse-macos/swiftw build` / `impulse-macos/swiftw test` — build or test the Swift package with the flags the local toolchain needs.
- `cargo test -p impulse-terminal`, `cargo fmt`, `cargo clippy` — Rust core.
- `./scripts/release.sh <version> [--push]` — the only release path.

## Conventions

- Swift types are `CamelCase`; keep new backend logic out of `Bridge/ImpulseCore.swift` and in the right library target.
- Golden fixtures under `impulse-macos/Tests/*/Fixtures` are the spec. Never regenerate them from Swift output; if behavior changes on purpose, edit the fixture in the same commit and say why. New git APIs use the real `git` CLI as the test oracle.
- Wrap new test files in `#if canImport(Testing)`.
- Commits are short, imperative, sentence case (`Add OSC 8 hyperlink support`); release commits are `Release vX.Y.Z`.
