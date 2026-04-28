# Linux Frontend Plan: GTK4

## Direction

The Linux frontend is GTK4/libadwaita again, restored from `v0.14.0`, the most recent GitHub release that published Linux artifacts. The Qt/QML rewrite is abandoned for now, and new Linux UI work should happen in the Rust GTK modules under `impulse-linux/src`.

## Current Stack

- Application shell: GTK4/libadwaita
- Terminal: VTE4
- Editor: Monaco in WebKitGTK
- Settings UI: `adw::PreferencesWindow`
- Packaging: `.deb`, `.rpm`, and Arch packages from `scripts/release.sh`

## Working Rules

1. Keep platform-neutral behavior in `impulse-core` and `impulse-editor`.
2. Keep Linux-specific UI wiring in `impulse-linux/src`.
3. Do not add QML, CXX-Qt, Qt helper C++, or Qt package dependencies for the Linux frontend.
4. Reuse the existing GTK modules before adding new abstractions.
5. Verify Linux changes with `cargo check -p impulse-linux`, `cargo build -p impulse-linux`, and focused tests when available.
