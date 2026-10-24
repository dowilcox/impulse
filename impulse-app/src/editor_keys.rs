//! Monaco's editor keys that GPUI Kit's editor doesn't bind, on both
//! platforms (⌘ on macOS, Ctrl on Linux, as Monaco and VS Code have them).
//! GPUI Kit's own bindings (cursor movement, multi-cursor, indent, find,
//! code actions, undo) already match Monaco's. Ours are bound for GPUI
//! Kit's `Input` inside an editor tab (`ImpulseEditor > Input`): as deep as
//! GPUI Kit's own, so ours, registered later, win where both bind a key
//! (⌘↩ inserts a line, as in Monaco), and only in editor tabs.

use gpui_kit::component::input::{GoToDefinition, Redo, Replace, ShowCompletions};
use gpui_kit::{actions, App, KeyBinding};

actions!(
    impulse_editor,
    [
        ToggleLineComment,
        MoveLinesUp,
        MoveLinesDown,
        CopyLinesUp,
        CopyLinesDown,
        DeleteLines,
        InsertLineBelow,
        InsertLineAbove,
        AddNextOccurrence,
        ChangeAllOccurrences,
        GoToTypeDefinition,
        GoToImplementations,
        GoToDeclaration,
        PeekDefinition,
        ShowLineCommit,
        NextConflict,
        PreviousConflict,
    ]
);

/// The key context of an editor tab.
pub const CONTEXT: &str = "ImpulseEditor";

pub fn bind_keys(cx: &mut App) {
    let c = Some("ImpulseEditor > Input");
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-/", ToggleLineComment, c),
            KeyBinding::new("alt-up", MoveLinesUp, c),
            KeyBinding::new("alt-down", MoveLinesDown, c),
            KeyBinding::new("shift-alt-up", CopyLinesUp, c),
            KeyBinding::new("shift-alt-down", CopyLinesDown, c),
            KeyBinding::new("cmd-shift-k", DeleteLines, c),
            KeyBinding::new("cmd-enter", InsertLineBelow, c),
            KeyBinding::new("cmd-shift-enter", InsertLineAbove, c),
            KeyBinding::new("cmd-d", AddNextOccurrence, c),
            KeyBinding::new("f12", GoToDefinition, c),
            KeyBinding::new("cmd-f12", GoToImplementations, c),
            KeyBinding::new("alt-f12", PeekDefinition, c),
            KeyBinding::new("cmd-f2", ChangeAllOccurrences, c),
            KeyBinding::new("cmd-i", ShowCompletions, c),
            KeyBinding::new("ctrl-space", ShowCompletions, c),
            KeyBinding::new("alt-cmd-f", Replace, c),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-/", ToggleLineComment, c),
            KeyBinding::new("alt-up", MoveLinesUp, c),
            KeyBinding::new("alt-down", MoveLinesDown, c),
            KeyBinding::new("ctrl-shift-alt-up", CopyLinesUp, c),
            KeyBinding::new("ctrl-shift-alt-down", CopyLinesDown, c),
            KeyBinding::new("ctrl-shift-k", DeleteLines, c),
            KeyBinding::new("ctrl-enter", InsertLineBelow, c),
            KeyBinding::new("ctrl-shift-enter", InsertLineAbove, c),
            KeyBinding::new("ctrl-d", AddNextOccurrence, c),
            KeyBinding::new("f12", GoToDefinition, c),
            KeyBinding::new("ctrl-f12", GoToImplementations, c),
            KeyBinding::new("ctrl-shift-f10", PeekDefinition, c),
            KeyBinding::new("ctrl-f2", ChangeAllOccurrences, c),
            KeyBinding::new("ctrl-space", ShowCompletions, c),
            KeyBinding::new("ctrl-shift-z", Redo, c),
        ]);
    }
}
