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

None yet.
