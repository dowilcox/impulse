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
    /// Working directory changed (OSC 7).
    CwdChanged(String),
    /// Shell prompt started (OSC 133;A).
    PromptStart,
    /// Command execution started (OSC 133;C).
    CommandStart,
    /// Command execution ended with exit code (OSC 133;D).
    CommandEnd(i32),
    /// Terminal requested user attention (iTerm2 OSC 1337;RequestAttention).
    AttentionRequest(String),
    /// Terminal requested a user notification (OSC 9 or OSC 777 notify).
    Notification { title: String, body: String },
    /// Internal: Term sends PtyWrite for device query responses (e.g., DA1).
    /// Filtered out in poll_events() and forwarded back to the PTY as input.
    PtyWrite(String),
}
