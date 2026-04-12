mod text;

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
pub(crate) use text::transcript_lines;
use text::{rendered_line_count, visible_width};

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
        insert_bottom_pinned_history(terminal, &lines, wrap_width, rendered_rows)?;
        terminal.invalidate_viewport();
        return Ok(());
    }

    write_history_at_cursor(terminal.backend_mut(), area.y, &lines, wrap_width)?;

    area.y = area.y.saturating_add(rendered_rows).min(max_top);
    terminal.set_viewport_area(area);
    terminal.invalidate_viewport();
    Ok(())
}

fn insert_bottom_pinned_history<B>(
    terminal: &mut Terminal<B>,
    lines: &[String],
    wrap_width: usize,
    rendered_rows: u16,
) -> io::Result<()>
where
    B: Backend<Error = io::Error> + Write,
{
    let area = terminal.viewport_area();
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
    write_history_lines(writer, lines, wrap_width)?;
    queue!(
        writer,
        Print(reset_scroll_region()),
        MoveTo(0, area.y),
        Clear(ClearType::UntilNewLine)
    )?;
    std::io::Write::flush(writer)?;
    Ok(())
}

fn write_history_at_cursor<W: Write>(
    writer: &mut W,
    start_row: u16,
    lines: &[String],
    wrap_width: usize,
) -> io::Result<()> {
    queue!(writer, MoveTo(0, start_row))?;
    write_history_lines(writer, lines, wrap_width)?;
    queue!(writer, Print("\r\n"), Clear(ClearType::UntilNewLine))?;
    std::io::Write::flush(writer)?;
    Ok(())
}

fn write_history_lines<W: Write>(
    writer: &mut W,
    lines: &[String],
    wrap_width: usize,
) -> io::Result<()> {
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            queue!(writer, Print("\r\n"))?;
        }
        write_history_line(writer, line, wrap_width)?;
    }
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
