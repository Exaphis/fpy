use crate::{
    events::key::input::{self, KeyInput},
    state::selection::set_selection_with_lines,
    EditorMode, EditorState,
};

use super::{
    motion as vim_motion, operator as vim_operator, operator::Operator, range::TextRange,
    state::VimCommandState, text_object as vim_text_object, visual as vim_visual,
};

pub(crate) struct VimCommandContext<'a> {
    pub(crate) state: &'a mut VimCommandState,
    pub(crate) lookup: &'a mut Vec<KeyInput>,
}

impl VimCommandContext<'_> {
    pub(crate) fn handle_count_prefix(
        &mut self,
        key_input: KeyInput,
        editor: &mut EditorState,
    ) -> bool {
        match key_input.key {
            input::KeyCode::Char(digit)
                if digit.is_ascii_digit()
                    && key_input.modifiers == input::Modifiers::NONE
                    && (self.state.has_pending_count() || digit != '0') =>
            {
                self.state.push_count_digit(digit);
                true
            }
            input::KeyCode::Char('G')
                if key_input.modifiers == input::Modifiers::SHIFT
                    && self.state.has_pending_count() =>
            {
                let count = self.state.take_count();
                self.state.clear();
                if let Some(target_row) = count.and_then(|n| n.checked_sub(1)) {
                    move_to_row(editor, target_row);
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn handle_normal_key(
        &mut self,
        key_input: KeyInput,
        editor: &mut EditorState,
    ) -> bool {
        self.handle_visual_line_key(key_input, editor)
            || self.handle_substitute_key(key_input, editor)
            || self.handle_char_motion_key(key_input, editor)
            || self.handle_operator_key(key_input, editor)
    }

    fn handle_visual_line_key(&mut self, key_input: KeyInput, editor: &mut EditorState) -> bool {
        if key_input.key == input::KeyCode::Char('V')
            && key_input.modifiers == input::Modifiers::SHIFT
        {
            vim_visual::select_current_line(editor);
            self.lookup.clear();
            self.state.clear();
            return true;
        }
        false
    }

    pub(crate) fn handle_visual_key(
        &mut self,
        key_input: KeyInput,
        editor: &mut EditorState,
    ) -> bool {
        self.handle_visual_text_object_key(key_input, editor)
            || handle_visual_operator_key(key_input, editor)
    }

    pub(crate) fn clear_if_idle(&mut self) {
        self.state.clear();
    }

    pub(crate) fn capture_operator_count_if_needed(&mut self) {
        self.state.capture_operator_count_if_needed(self.lookup);
    }

    pub(crate) fn take_command_count(&mut self) -> usize {
        self.state.take_command_count()
    }

    fn handle_substitute_key(&mut self, key_input: KeyInput, editor: &mut EditorState) -> bool {
        use input::KeyCode::Char;
        if key_input.key != Char('s') || key_input.modifiers != input::Modifiers::NONE {
            return false;
        }
        let count = self.take_command_count();
        if let Some(range) = vim_motion::char_span_range(editor, count) {
            vim_operator::apply_operator(editor, Operator::Change, range);
        } else {
            editor.mode = EditorMode::Insert;
        }
        self.lookup.clear();
        self.state.clear();
        true
    }

    fn handle_char_motion_key(&mut self, key_input: KeyInput, editor: &mut EditorState) -> bool {
        use input::KeyCode::Char;

        if self.lookup.is_empty() {
            if matches!(key_input.key, Char('f' | 't' | 'F' | 'T'))
                && matches!(
                    key_input.modifiers,
                    input::Modifiers::NONE | input::Modifiers::SHIFT
                )
            {
                self.lookup.push(key_input);
                return true;
            }
            return false;
        }

        let Char(motion @ ('f' | 't' | 'F' | 'T')) = self.lookup[0].key else {
            return false;
        };
        let Char(target) = key_input.key else {
            self.lookup.clear();
            self.state.clear();
            return false;
        };

        let count = self.take_command_count();
        if let Some(range) = char_motion_range(motion, target, count, editor) {
            editor.cursor = if matches!(motion, 'F' | 'T') {
                range.start
            } else {
                range.end
            };
        }
        self.lookup.clear();
        self.state.clear();
        true
    }

    fn handle_visual_text_object_key(
        &mut self,
        key_input: KeyInput,
        editor: &mut EditorState,
    ) -> bool {
        use input::KeyCode::Char;

        if self.lookup.is_empty() {
            if matches!(key_input.key, Char('i' | 'a'))
                && key_input.modifiers == input::Modifiers::NONE
            {
                self.lookup.push(key_input);
                return true;
            }
            return false;
        }

        let Char(prefix @ ('i' | 'a')) = self.lookup[0].key else {
            return false;
        };
        if let Some(range) = text_object_range(prefix, key_input, editor) {
            vim_visual::apply_range_as_selection(editor, range);
            self.lookup.clear();
            self.state.clear();
            return true;
        }
        self.lookup.clear();
        self.state.clear();
        false
    }

    fn handle_operator_key(&mut self, key_input: KeyInput, editor: &mut EditorState) -> bool {
        use input::KeyCode::Char;

        if self.lookup.is_empty() {
            if matches!(key_input.key, Char('d' | 'c' | 'y'))
                && key_input.modifiers == input::Modifiers::NONE
            {
                self.lookup.push(key_input);
                self.capture_operator_count_if_needed();
                return true;
            }
            return false;
        }

        let Char(op @ ('d' | 'c' | 'y')) = self.lookup[0].key else {
            return false;
        };

        if self.lookup.len() == 1 {
            if matches!(key_input.key, Char('i' | 'a' | 'g' | 'f' | 't' | 'F' | 'T'))
                && matches!(
                    key_input.modifiers,
                    input::Modifiers::NONE | input::Modifiers::SHIFT
                )
            {
                self.lookup.push(key_input);
                return true;
            }
            let count = self.take_command_count();
            if execute_operator_motion(op, key_input, count, editor) {
                self.lookup.clear();
                self.state.clear();
                return true;
            }
            self.lookup.clear();
            self.state.clear();
            return false;
        }

        if self.lookup.len() == 2 {
            let Char(prefix @ ('i' | 'a' | 'g' | 'f' | 't' | 'F' | 'T')) = self.lookup[1].key
            else {
                self.lookup.clear();
                self.state.clear();
                return false;
            };
            if matches!(prefix, 'f' | 't' | 'F' | 'T') {
                if let Char(target) = key_input.key {
                    if let Some(range) =
                        char_motion_range(prefix, target, self.take_command_count(), editor)
                    {
                        apply_operator(op, editor, range);
                    }
                    self.lookup.clear();
                    self.state.clear();
                    return true;
                }
            } else if prefix == 'g' {
                if key_input.key == Char('g') && key_input.modifiers == input::Modifiers::NONE {
                    if let Some(range) = vim_motion::to_first_line_range(editor) {
                        apply_operator(op, editor, range);
                    }
                    self.lookup.clear();
                    self.state.clear();
                    return true;
                }
            } else if let Some(range) = text_object_range(prefix, key_input, editor) {
                apply_operator(op, editor, range);
                self.lookup.clear();
                self.state.clear();
                return true;
            }
            self.lookup.clear();
            self.state.clear();
            return false;
        }

        self.lookup.clear();
        self.state.clear();
        false
    }
}

fn char_motion_range(
    motion: char,
    target: char,
    count: usize,
    editor: &EditorState,
) -> Option<TextRange> {
    match motion {
        'f' => vim_motion::char_forward_range(editor, target, count, false),
        't' => vim_motion::char_forward_range(editor, target, count, true),
        'F' => vim_motion::char_backward_range(editor, target, count, false),
        'T' => vim_motion::char_backward_range(editor, target, count, true),
        _ => None,
    }
}

fn handle_visual_operator_key(key_input: KeyInput, editor: &mut EditorState) -> bool {
    use input::KeyCode::Char;
    if key_input.modifiers != input::Modifiers::NONE {
        return false;
    }
    let op = match key_input.key {
        Char('d' | 'x') => 'd',
        Char('c') => 'c',
        Char('y') => 'y',
        _ => return false,
    };
    let Some(range) = vim_visual::selection_range(editor) else {
        return false;
    };
    editor.selection = None;
    apply_operator(op, editor, range);
    if op != 'c' {
        editor.mode = EditorMode::Normal;
    }
    true
}

fn execute_operator_motion(
    op: char,
    key_input: KeyInput,
    count: usize,
    editor: &mut EditorState,
) -> bool {
    use input::KeyCode::Char;
    let shifted = key_input.modifiers == input::Modifiers::SHIFT;
    let plain = key_input.modifiers == input::Modifiers::NONE;

    if plain && key_input.key == Char(op) {
        let range = TextRange::linewise(
            editor.cursor.row,
            editor.cursor.row.saturating_add(count.saturating_sub(1)),
        );
        apply_operator(op, editor, range);
        return true;
    }

    let range = match (key_input.key, plain, shifted) {
        (Char('w'), true, _) => counted_range(count, editor, vim_motion::word_forward_range),
        (Char('e'), true, _) => counted_range(count, editor, vim_motion::word_end_range),
        (Char('b'), true, _) => counted_range(count, editor, vim_motion::word_backward_range),
        (Char('W'), _, true) => counted_range(count, editor, vim_motion::big_word_forward_range),
        (Char('E'), _, true) => counted_range(count, editor, vim_motion::big_word_end_range),
        (Char('B'), _, true) => counted_range(count, editor, vim_motion::big_word_backward_range),
        (Char('0'), true, _) => vim_motion::line_start_range(editor),
        (Char('$'), _, _) => vim_motion::line_end_range(editor),
        (Char('j'), true, _) => vim_motion::line_down_range(editor, count),
        (Char('k'), true, _) => vim_motion::line_up_range(editor, count),
        (Char('G'), _, true) => vim_motion::to_last_line_range(editor),
        _ => None,
    };

    if let Some(range) = range {
        apply_operator(op, editor, range);
        true
    } else {
        false
    }
}

fn counted_range(
    count: usize,
    editor: &EditorState,
    motion: fn(&EditorState) -> Option<TextRange>,
) -> Option<TextRange> {
    let mut scratch = editor.clone();
    let mut combined: Option<TextRange> = None;
    for _ in 0..count {
        let range = motion(&scratch)?;
        scratch.cursor = range.end;
        combined = Some(match combined {
            Some(mut combined_range) => {
                combined_range.end = range.end;
                combined_range
            }
            None => range,
        });
    }
    combined
}

fn apply_operator(op: char, editor: &mut EditorState, range: TextRange) {
    let operator = match op {
        'd' => Operator::Delete,
        'c' => Operator::Change,
        'y' => Operator::Yank,
        _ => return,
    };
    vim_operator::apply_operator(editor, operator, range);
}

fn text_object_range(prefix: char, key_input: KeyInput, editor: &EditorState) -> Option<TextRange> {
    use input::KeyCode::Char;
    let Char(ch) = key_input.key else {
        return None;
    };
    match (prefix, ch, key_input.modifiers == input::Modifiers::SHIFT) {
        ('i', 'w', false) => vim_text_object::inner_word_range(editor),
        ('a', 'w', false) => vim_text_object::around_word_range(editor),
        ('i', 'W', true) => vim_text_object::inner_big_word_range(editor),
        ('a', 'W', true) => vim_text_object::around_big_word_range(editor),
        _ => delimiter_text_object_range(prefix, ch, editor),
    }
}

fn delimiter_text_object_range(prefix: char, ch: char, editor: &EditorState) -> Option<TextRange> {
    let (open, close) = match ch {
        '"' => ('"', '"'),
        '\'' => ('\'', '\''),
        '(' | ')' => ('(', ')'),
        '[' | ']' => ('[', ']'),
        '{' | '}' => ('{', '}'),
        _ => return None,
    };
    match prefix {
        'i' => vim_text_object::inner_between_range(editor, open, close),
        'a' => vim_text_object::around_between_range(editor, open, close),
        _ => None,
    }
}

fn move_to_row(editor: &mut EditorState, target_row: usize) {
    let current_row = editor.cursor.row;
    if target_row == current_row {
        return;
    }
    editor.cursor.row = target_row.min(editor.lines.len().saturating_sub(1));
    editor.clamp_column();
    if editor.mode == EditorMode::Visual {
        set_selection_with_lines(&mut editor.selection, editor.cursor, &editor.lines);
    }
}
