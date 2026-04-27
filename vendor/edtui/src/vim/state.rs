use crate::events::key::input::KeyCode;
use crate::events::KeyInput;

#[derive(Clone, Debug, Default)]
pub(crate) struct VimCommandState {
    pending_count: String,
    operator_count: Option<usize>,
}

impl VimCommandState {
    pub(crate) fn push_count_digit(&mut self, digit: char) {
        self.pending_count.push(digit);
    }

    pub(crate) fn has_pending_count(&self) -> bool {
        !self.pending_count.is_empty()
    }

    pub(crate) fn take_count(&mut self) -> Option<usize> {
        if self.pending_count.is_empty() {
            None
        } else {
            let count = self.pending_count.parse::<usize>().ok();
            if count.is_some() {
                self.pending_count.clear();
            }
            count
        }
    }

    pub(crate) fn take_command_count(&mut self) -> usize {
        let motion_count = self.take_count().unwrap_or(1);
        let operator_count = self.operator_count.take().unwrap_or(1);
        operator_count.saturating_mul(motion_count).max(1)
    }

    pub(crate) fn clear(&mut self) {
        self.pending_count.clear();
        self.operator_count = None;
    }

    pub(crate) fn capture_operator_count_if_needed(&mut self, lookup: &[KeyInput]) {
        if self.operator_count.is_none()
            && lookup.len() == 1
            && matches!(lookup[0].key, KeyCode::Char('d' | 'c' | 'y'))
        {
            self.operator_count = self.take_count();
        }
    }
}
