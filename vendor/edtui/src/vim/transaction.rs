use crate::EditorState;

/// Execute a Vim command boundary as one undo transaction.
///
/// This is intentionally small for now: the existing Vim integration still
/// dispatches many commands through generic actions. Centralizing transaction
/// ownership here gives the Vim layer a single place to grow command-oriented
/// undo semantics as commands migrate out of the keybinding table.
pub(crate) fn in_undo_transaction<T>(state: &mut EditorState, f: impl FnOnce(&mut EditorState) -> T) -> T {
    state.begin_undo_transaction();
    let result = f(state);
    state.end_undo_transaction();
    result
}
