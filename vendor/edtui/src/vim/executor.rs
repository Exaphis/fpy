use crate::{
    actions::{Action, Execute},
    EditorMode, EditorState,
};

use super::transaction;

/// Transitional Vim command executor.
///
/// Phase 1 keeps the existing parser/keybinding paths, but routes completed Vim
/// mutations through this executor so undo boundaries are owned by the Vim layer
/// instead of by tests or individual generic actions.
pub(crate) struct VimCommandExecutor;

impl VimCommandExecutor {
    pub(crate) fn execute_normal_action(action: &mut Action, state: &mut EditorState) -> bool {
        let mode_before = state.mode;
        state.begin_undo_transaction();
        transaction::in_undo_transaction(state, |state| action.execute(state));
        let entering_insert = matches!(mode_before, EditorMode::Normal | EditorMode::Visual)
            && state.mode == EditorMode::Insert;
        if !entering_insert {
            state.end_undo_transaction();
        }
        entering_insert
    }

    pub(crate) fn end_insert_session(state: &mut EditorState) {
        state.end_undo_transaction();
    }

    pub(crate) fn execute_handled(
        state: &mut EditorState,
        f: impl FnOnce(&mut EditorState) -> bool,
    ) -> bool {
        transaction::in_undo_transaction(state, f)
    }
}
