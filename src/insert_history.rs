use std::io;
use std::io::Write;

use crossterm::cursor::MoveDown;
use crossterm::cursor::MoveTo;
use crossterm::cursor::MoveToColumn;
use crossterm::cursor::RestorePosition;
use crossterm::cursor::SavePosition;
use crossterm::queue;
use crossterm::style::Print;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use crossterm::terminal::ScrollUp;
use ratatui::backend::Backend;

use crate::custom_terminal::Terminal;

pub fn insert_history_text<B>(terminal: &mut Terminal<B>, text: &str) -> io::Result<()>
where
    B: Backend<Error = io::Error> + Write,
{
    let screen_size = terminal.size()?;
    let mut area = terminal.viewport_area();
    let lines = transcript_lines(text);
    let wrap_width = area.width.max(1) as usize;
    let rendered_rows = rendered_line_count(&lines, wrap_width);
    let max_top = screen_size
        .height
        .saturating_sub(area.height.min(screen_size.height.max(1)));

    if area.y == max_top && area.y > 0 {
        let history_bottom = area.y.saturating_sub(1);
        let start_row = area.y.saturating_sub(rendered_rows.min(area.y));
        let writer = terminal.backend_mut();

        queue!(
            writer,
            Print(set_scroll_region(0, history_bottom)),
            MoveTo(0, history_bottom),
            ScrollUp(rendered_rows),
            MoveTo(0, start_row)
        )?;

        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                queue!(writer, Print("\r\n"))?;
            }
            write_history_line(writer, line, wrap_width)?;
        }

        queue!(
            writer,
            Print(reset_scroll_region()),
            MoveTo(0, area.y),
            Clear(ClearType::UntilNewLine)
        )?;

        std::io::Write::flush(writer)?;
        terminal.invalidate_viewport();
        return Ok(());
    }

    {
        let writer = terminal.backend_mut();
        queue!(writer, MoveTo(0, area.y))?;

        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                queue!(writer, Print("\r\n"))?;
            }
            write_history_line(writer, line, wrap_width)?;
        }

        queue!(writer, Print("\r\n"), Clear(ClearType::UntilNewLine))?;

        std::io::Write::flush(writer)?;
    }

    area.y = area.y.saturating_add(rendered_rows).min(max_top);
    terminal.set_viewport_area(area);
    terminal.invalidate_viewport();
    Ok(())
}

fn set_scroll_region(top: u16, bottom: u16) -> String {
    format!(
        "\x1b[{};{}r",
        top.saturating_add(1),
        bottom.saturating_add(1)
    )
}

fn reset_scroll_region() -> &'static str {
    "\x1b[r"
}

pub(crate) fn transcript_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized
        .split('\n')
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if normalized.ends_with('\n') && lines.len() > 1 {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn rendered_line_count(lines: &[String], width: usize) -> u16 {
    lines
        .iter()
        .map(|line| visible_width(line).max(1).div_ceil(width))
        .sum::<usize>()
        .clamp(1, u16::MAX as usize) as u16
}

fn write_history_line<W: Write>(writer: &mut W, line: &str, wrap_width: usize) -> io::Result<()> {
    let physical_rows = visible_width(line).max(1).div_ceil(wrap_width) as u16;
    if physical_rows > 1 {
        queue!(writer, SavePosition)?;
        for _ in 1..physical_rows {
            queue!(
                writer,
                MoveDown(1),
                MoveToColumn(0),
                Clear(ClearType::UntilNewLine)
            )?;
        }
        queue!(writer, RestorePosition)?;
    }
    queue!(writer, Clear(ClearType::UntilNewLine))?;
    writer.write_all(line.as_bytes())?;
    Ok(())
}

fn visible_width(text: &str) -> usize {
    strip_ansi(text).chars().count()
}

fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            while let Some(next) = chars.next() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }

        result.push(ch);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::transcript_lines;

    #[test]
    fn preserves_real_blank_lines_but_not_trailing_line_terminator() {
        assert_eq!(transcript_lines("a\n\nb\n"), vec!["a", "", "b"]);
    }
}
