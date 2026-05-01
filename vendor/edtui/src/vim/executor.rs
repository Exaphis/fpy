use crate::{actions::{Action, Execute}, EditorState};

use super::transaction;

/// Transitional Vim command executor.
///
/// Phase 1 keeps the existing parser/keybinding paths, but routes completed Vim
/// mutations through this executor so undo boundaries are owned by the Vim layer
/// instead of by tests or individual generic actions.
pub(crate) struct VimCommandExecutor;

impl VimCommandExecutor {
    pub(crate) fn execute_action(action: &mut Action, state: &mut EditorState) {
        transaction::in_undo_transaction(state, |state| action.execute(state));
    }

    pub(crate) fn execute_handled(state: &mut EditorState, f: impl FnOnce(&mut EditorState) -> bool) -> bool {
        transaction::in_undo_transaction(state, f)
    }
}
