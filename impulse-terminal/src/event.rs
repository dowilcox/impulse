//! Terminal events emitted to the frontend.

use serde::Serialize;

/// Events emitted by the terminal backend.
/// Frontends poll these via `TerminalBackend::poll_events()`.
#[derive(Clone, Debug, Serialize)]
pub enum TerminalEvent {
    /// Terminal output changed — frontend should re-render.
    Wakeup,
    /// Terminal title changed (OSC 0/2).
    TitleChanged(String),
    /// Title was reset to default.
    ResetTitle,
    /// Bell character received.
    Bell,
    /// Child process exited.
    ChildExited(i32),
    /// Request to store text in the clipboard (OSC 52).
    ClipboardStore(String),
    /// Request to read text from the clipboard (OSC 52).
    ClipboardLoad,
    /// Cursor blinking state has changed.
    CursorBlinkingChange,
    /// Terminal requested exit.
    Exit,
}
