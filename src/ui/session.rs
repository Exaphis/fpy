use std::io::{Write, stdout};

use anyhow::Result;
use crossterm::{
    cursor::{MoveToColumn, SetCursorStyle, Show},
    event::{
        DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    style::ResetColor,
    terminal::{EnableLineWrap, disable_raw_mode, enable_raw_mode},
};

pub(super) struct TerminalSession {
    restored: bool,
}

impl TerminalSession {
    pub(super) fn start() -> Result<Self> {
        enable_raw_mode()?;
        let _ = execute!(
            stdout(),
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            )
        );
        Ok(Self { restored: false })
    }

    pub(super) fn is_restored(&self) -> bool {
        self.restored
    }

    pub(super) fn shutdown_at_current_cursor(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        let mut handle = stdout();
        let _ = execute!(handle, DisableBracketedPaste, PopKeyboardEnhancementFlags);
        write!(handle, "\x1b7\x1b[r\x1b8\x1b[0m")?;
        execute!(
            handle,
            Show,
            SetCursorStyle::DefaultUserShape,
            ResetColor,
            EnableLineWrap,
            MoveToColumn(0)
        )?;
        handle.flush()?;
        disable_raw_mode()?;
        execute!(
            handle,
            ResetColor,
            EnableLineWrap,
            Show,
            SetCursorStyle::DefaultUserShape,
            MoveToColumn(0)
        )?;
        handle.flush()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if !self.restored {
            let _ = execute!(
                stdout(),
                DisableBracketedPaste,
                PopKeyboardEnhancementFlags,
                ResetColor,
                EnableLineWrap,
                Show,
                SetCursorStyle::DefaultUserShape,
                MoveToColumn(0)
            );
            let _ = disable_raw_mode();
        }
    }
}
