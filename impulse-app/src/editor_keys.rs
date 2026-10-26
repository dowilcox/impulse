//! Monaco's editor keys that GPUI Kit's editor doesn't bind, on both
//! platforms (⌘ on macOS, Ctrl on Linux, as Monaco and VS Code have them).
//! GPUI Kit's own bindings (cursor movement, multi-cursor, indent, find,
//! code actions, undo) already match Monaco's. Ours are bound for GPUI
//! Kit's `Input` inside an editor tab (`ImpulseEditor > Input`): as deep as
//! GPUI Kit's own, so ours, registered later, win where both bind a key
//! (⌘↩ inserts a line, as in Monaco), and only in editor tabs. Keys that
//! act on the buffer rather than the field with the keyboard (Home, ⌘L)
//! leave out the find field inside the editor (`mode == editor`). On the
//! Mac, the Emacs keys every text view has (⌃F ⌃B ⌃N ⌃P ⌃D ⌃H ⌃K; GPUI
//! Kit binds ⌃A and ⌃E) name GPUI Kit's own moves and deletes. Folding
//! from the keyboard (Monaco's Fold, Unfold, Fold All and Unfold All) goes
//! through Impulse's GPUI Kit patch (`EditorState::fold_at_cursor` and the
//! rest).

use gpui_kit::component::input::{
    Backspace, Delete, DeleteToEndOfLine, GoToDefinition, MoveDown, MoveLeft, MoveRight, MoveUp,
    Redo, Replace, ShowCompletions,
};
use gpui_kit::{actions, App, KeyBinding};

use crate::commands::RunCommand;

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
        ToggleBlockComment,
        GoToBracket,
        NextProblem,
        PreviousProblem,
        ExpandSelection,
        ShrinkSelection,
        ShowParameterHints,
        CursorUndo,
        CursorsAtLineEnds,
        TrimTrailingWhitespace,
        CursorHome,
        ExpandLineSelection,
        Fold,
        Unfold,
        FoldAll,
        UnfoldAll,
    ]
);

/// The key context of an editor tab.
pub const CONTEXT: &str = "ImpulseEditor";

pub fn bind_keys(cx: &mut App) {
    let c = Some("ImpulseEditor > Input");
    let buffer = Some("ImpulseEditor > Input && mode == editor");
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("home", CursorHome, buffer),
            KeyBinding::new("cmd-left", CursorHome, buffer),
            KeyBinding::new("cmd-l", ExpandLineSelection, buffer),
            KeyBinding::new("ctrl-f", MoveRight, c),
            KeyBinding::new("ctrl-b", MoveLeft, c),
            KeyBinding::new("ctrl-n", MoveDown, c),
            KeyBinding::new("ctrl-p", MoveUp, c),
            KeyBinding::new("ctrl-d", Delete, c),
            KeyBinding::new("ctrl-h", Backspace, c),
            KeyBinding::new("ctrl-k", DeleteToEndOfLine, c),
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
            KeyBinding::new("cmd-shift-l", ChangeAllOccurrences, c),
            KeyBinding::new("shift-alt-a", ToggleBlockComment, c),
            KeyBinding::new("cmd-shift-\\", GoToBracket, c),
            KeyBinding::new("f8", NextProblem, c),
            KeyBinding::new("shift-f8", PreviousProblem, c),
            KeyBinding::new("ctrl-shift-cmd-right", ExpandSelection, c),
            KeyBinding::new("ctrl-shift-cmd-left", ShrinkSelection, c),
            KeyBinding::new("cmd-shift-space", ShowParameterHints, c),
            KeyBinding::new("cmd-u", CursorUndo, c),
            KeyBinding::new("shift-alt-i", CursorsAtLineEnds, c),
            KeyBinding::new("cmd-k cmd-x", TrimTrailingWhitespace, c),
            KeyBinding::new("alt-cmd-[", Fold, buffer),
            KeyBinding::new("alt-cmd-]", Unfold, buffer),
            KeyBinding::new("cmd-k cmd-0", FoldAll, buffer),
            KeyBinding::new("cmd-k cmd-j", UnfoldAll, buffer),
            KeyBinding::new("f1", RunCommand::new("command_palette"), c),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("home", CursorHome, buffer),
            KeyBinding::new("ctrl-l", ExpandLineSelection, buffer),
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
            KeyBinding::new("ctrl-shift-l", ChangeAllOccurrences, c),
            KeyBinding::new("ctrl-shift-a", ToggleBlockComment, c),
            KeyBinding::new("ctrl-shift-\\", GoToBracket, c),
            KeyBinding::new("f8", NextProblem, c),
            KeyBinding::new("shift-f8", PreviousProblem, c),
            KeyBinding::new("shift-alt-right", ExpandSelection, c),
            KeyBinding::new("shift-alt-left", ShrinkSelection, c),
            KeyBinding::new("ctrl-shift-space", ShowParameterHints, c),
            KeyBinding::new("ctrl-u", CursorUndo, c),
            KeyBinding::new("shift-alt-i", CursorsAtLineEnds, c),
            KeyBinding::new("ctrl-k ctrl-x", TrimTrailingWhitespace, c),
            KeyBinding::new("ctrl-shift-[", Fold, buffer),
            KeyBinding::new("ctrl-shift-]", Unfold, buffer),
            KeyBinding::new("ctrl-k ctrl-0", FoldAll, buffer),
            KeyBinding::new("ctrl-k ctrl-j", UnfoldAll, buffer),
            KeyBinding::new("f1", RunCommand::new("command_palette"), c),
        ]);
    }
}
