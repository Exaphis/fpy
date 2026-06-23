use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GlobalKey {
    OpenHistorySearch,
    OpenPalette,
    InterruptOrClear,
    ExitOrConfirm,
    ClearScreen,
    HistoryUp,
    HistoryDown,
    InsertIndent,
    AcceptGhostSuggestion,
    InsertLineBreak,
    Submit,
    Editor,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistorySearchKey {
    Close,
    Up,
    Down,
    Load,
    Backspace,
    Cycle,
    Insert(char),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaletteKey {
    Close,
    Up,
    Down,
    Select,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GlobalKeyContext {
    pub editor_enabled: bool,
    pub awaiting_input: bool,
    pub editor_empty: bool,
    pub editor_single_line: bool,
    pub editor_has_history: bool,
    pub editor_insert_mode: bool,
    pub submit_ready: bool,
    pub ghost_suggestion_available: bool,
}

pub(super) fn normalize_paste_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn history_search_paste_text(text: &str) -> String {
    text.replace('\n', " ")
}

pub(super) fn classify_global_key(key: KeyEvent, context: GlobalKeyContext) -> GlobalKey {
    match key {
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => GlobalKey::OpenHistorySearch,
        KeyEvent {
            code: KeyCode::Char('p'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => GlobalKey::OpenPalette,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => GlobalKey::InterruptOrClear,
        KeyEvent {
            code: KeyCode::Char('d'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && context.editor_enabled
            && !context.awaiting_input
            && context.editor_empty =>
        {
            GlobalKey::ExitOrConfirm
        }
        KeyEvent {
            code: KeyCode::Char('l'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => GlobalKey::ClearScreen,
        KeyEvent {
            code: KeyCode::Char('k'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && context.editor_enabled
            && context.editor_has_history =>
        {
            GlobalKey::HistoryUp
        }
        KeyEvent {
            code: KeyCode::Char('j'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL)
            && context.editor_enabled
            && context.editor_has_history =>
        {
            GlobalKey::HistoryDown
        }
        KeyEvent {
            code: KeyCode::Tab, ..
        } if context.editor_enabled && context.editor_insert_mode => GlobalKey::InsertIndent,
        KeyEvent {
            code: KeyCode::Right,
            modifiers,
            ..
        } if context.editor_enabled
            && modifiers.is_empty()
            && context.ghost_suggestion_available =>
        {
            GlobalKey::AcceptGhostSuggestion
        }
        KeyEvent {
            code: KeyCode::Char('f'),
            modifiers,
            ..
        } if context.editor_enabled
            && modifiers.contains(KeyModifiers::CONTROL)
            && context.ghost_suggestion_available =>
        {
            GlobalKey::AcceptGhostSuggestion
        }
        KeyEvent {
            code: KeyCode::Enter,
            modifiers,
            ..
        } if context.editor_enabled && modifiers.contains(KeyModifiers::SHIFT) => {
            GlobalKey::InsertLineBreak
        }
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } if context.submit_ready => GlobalKey::Submit,
        KeyEvent {
            code: KeyCode::Up, ..
        } if context.editor_enabled && context.editor_single_line && context.editor_has_history => {
            GlobalKey::HistoryUp
        }
        KeyEvent {
            code: KeyCode::Down,
            ..
        } if context.editor_enabled && context.editor_single_line && context.editor_has_history => {
            GlobalKey::HistoryDown
        }
        _ if context.editor_enabled => GlobalKey::Editor,
        _ => GlobalKey::Ignored,
    }
}

pub(super) fn is_exit_confirmation_key(key: KeyEvent, context: GlobalKeyContext) -> bool {
    classify_global_key(key, context) == GlobalKey::ExitOrConfirm
}

pub(super) fn classify_history_search_key(key: KeyEvent) -> HistorySearchKey {
    match key {
        KeyEvent {
            code: KeyCode::Esc, ..
        } => HistorySearchKey::Close,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => HistorySearchKey::Close,
        KeyEvent {
            code: KeyCode::Up, ..
        } => HistorySearchKey::Up,
        KeyEvent {
            code: KeyCode::Down,
            ..
        } => HistorySearchKey::Down,
        KeyEvent {
            code: KeyCode::Enter,
            ..
        } => HistorySearchKey::Load,
        KeyEvent {
            code: KeyCode::Backspace,
            ..
        } => HistorySearchKey::Backspace,
        KeyEvent {
            code: KeyCode::Char('r'),
            modifiers,
            ..
        } if modifiers.contains(KeyModifiers::CONTROL) => HistorySearchKey::Cycle,
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        } if !modifiers.contains(KeyModifiers::CONTROL)
            && !modifiers.contains(KeyModifiers::ALT) =>
        {
            HistorySearchKey::Insert(ch)
        }
        _ => HistorySearchKey::Ignored,
    }
}

pub(super) fn classify_palette_key(key: KeyEvent) -> PaletteKey {
    match key.code {
        KeyCode::Esc => PaletteKey::Close,
        KeyCode::Up => PaletteKey::Up,
        KeyCode::Down => PaletteKey::Down,
        KeyCode::Enter => PaletteKey::Select,
        _ => PaletteKey::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        GlobalKey, GlobalKeyContext, HistorySearchKey, PaletteKey, classify_global_key,
        classify_history_search_key, classify_palette_key, history_search_paste_text,
        is_exit_confirmation_key, normalize_paste_text,
    };

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn context() -> GlobalKeyContext {
        GlobalKeyContext {
            editor_enabled: true,
            awaiting_input: false,
            editor_empty: true,
            editor_single_line: true,
            editor_has_history: true,
            editor_insert_mode: true,
            submit_ready: true,
            ghost_suggestion_available: true,
        }
    }

    #[test]
    fn paste_normalization_converts_crlf_and_cr_to_lf() {
        assert_eq!(normalize_paste_text("a\r\nb\rc"), "a\nb\nc");
    }

    #[test]
    fn history_search_paste_flattens_lines_to_spaces() {
        let normalized = normalize_paste_text("a\r\nb\rc");
        assert_eq!(history_search_paste_text(&normalized), "a b c");
    }

    #[test]
    fn classifies_global_control_keys() {
        assert_eq!(
            classify_global_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL), context()),
            GlobalKey::OpenPalette
        );
        assert_eq!(
            classify_global_key(key(KeyCode::Char('l'), KeyModifiers::CONTROL), context()),
            GlobalKey::ClearScreen
        );
        assert!(is_exit_confirmation_key(
            key(KeyCode::Char('d'), KeyModifiers::CONTROL),
            context()
        ));
    }

    #[test]
    fn classifies_global_editor_keys_from_context() {
        assert_eq!(
            classify_global_key(key(KeyCode::Tab, KeyModifiers::empty()), context()),
            GlobalKey::InsertIndent
        );
        assert_eq!(
            classify_global_key(key(KeyCode::Right, KeyModifiers::empty()), context()),
            GlobalKey::AcceptGhostSuggestion
        );
        assert_eq!(
            classify_global_key(key(KeyCode::Enter, KeyModifiers::empty()), context()),
            GlobalKey::Submit
        );
    }

    #[test]
    fn classifies_history_search_keys() {
        assert_eq!(
            classify_history_search_key(key(KeyCode::Char('x'), KeyModifiers::empty())),
            HistorySearchKey::Insert('x')
        );
        assert_eq!(
            classify_history_search_key(key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            HistorySearchKey::Cycle
        );
        assert_eq!(
            classify_history_search_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            HistorySearchKey::Close
        );
    }

    #[test]
    fn classifies_palette_keys() {
        assert_eq!(
            classify_palette_key(key(KeyCode::Enter, KeyModifiers::empty())),
            PaletteKey::Select
        );
    }
}
