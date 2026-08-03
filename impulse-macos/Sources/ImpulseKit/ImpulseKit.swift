// ImpulseKit — pure logic ported from the Rust backend (impulse-core /
// impulse-editor) as part of the Mac-first Swift rewrite.
//
// Rules for this module:
// - Foundation only. No AppKit, no WebKit, no CImpulseFFI.
// - Behavior parity with the Rust implementation is verified against the
//   golden fixtures in Tests/ImpulseKitTests/Fixtures, which were generated
//   by `cargo run -p impulse-editor --example dump_fixtures` while the Rust
//   code was still the source of truth.

import Foundation
