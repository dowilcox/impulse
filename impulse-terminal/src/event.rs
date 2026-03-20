//! Terminal events emitted to the frontend.

use serde::Serialize;

/// Events emitted by the terminal backend to the frontend.
///
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
    /// Child process exited with an error code.
    ChildExited(i32),
    /// Request to store text in the clipboard.
    ClipboardStore(String),
    /// Request to read text from the clipboard.
    ClipboardLoad,
    /// Cursor blinking state has changed.
    CursorBlinkingChange,
    /// Terminal requested exit.
    Exit,
}
