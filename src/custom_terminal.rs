use std::io;
use std::io::Stdout;
use std::io::Write;

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::Colors;
use crossterm::style::Print;
use crossterm::style::SetAttribute;
use crossterm::style::SetBackgroundColor;
use crossterm::style::SetColors;
use crossterm::style::SetForegroundColor;
use crossterm::terminal::Clear;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::buffer::Cell;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::layout::Size;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::widgets::StatefulWidget;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

pub type DefaultTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Frame<'a> {
    cursor_position: Option<Position>,
    viewport_area: Rect,
    buffer: &'a mut Buffer,
}

impl Frame<'_> {
    pub const fn area(&self) -> Rect {
        self.viewport_area
    }

    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    pub fn render_stateful_widget<W>(&mut self, widget: W, area: Rect, state: &mut W::State)
    where
        W: StatefulWidget,
    {
        widget.render(area, self.buffer, state);
    }

    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }
}

#[derive(Debug)]
pub struct Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    backend: B,
    buffers: [Buffer; 2],
    current: usize,
    hidden_cursor: bool,
    viewport_area: Rect,
    last_known_screen_size: Size,
    last_known_cursor_pos: Position,
    visible_history_rows: u16,
}

impl<B> Drop for Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    fn drop(&mut self) {
        let _ = self.show_cursor();
    }
}

impl<B> Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    pub fn with_options(mut backend: B) -> io::Result<Self> {
        let screen_size = backend.size()?;
        let cursor_pos = backend
            .get_cursor_position()
            .unwrap_or(Position { x: 0, y: 0 });
        let viewport_area = Rect::new(0, cursor_pos.y, screen_size.width, 0);
        Ok(Self {
            backend,
            buffers: [Buffer::empty(viewport_area), Buffer::empty(viewport_area)],
            current: 0,
            hidden_cursor: false,
            viewport_area,
            last_known_screen_size: screen_size,
            last_known_cursor_pos: cursor_pos,
            visible_history_rows: 0,
        })
    }

    pub const fn viewport_area(&self) -> Rect {
        self.viewport_area
    }

    pub fn set_viewport_area(&mut self, area: Rect) {
        self.current_buffer_mut().resize(area);
        self.previous_buffer_mut().resize(area);
        self.viewport_area = area;
        self.visible_history_rows = self.visible_history_rows.min(area.top());
    }

    pub fn invalidate_viewport(&mut self) {
        self.previous_buffer_mut().reset();
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn size(&mut self) -> io::Result<Size> {
        let size = self.backend.size()?;
        self.last_known_screen_size = size;
        Ok(size)
    }

    pub fn draw<F>(&mut self, render_callback: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        let mut frame = Frame {
            cursor_position: None,
            viewport_area: self.viewport_area,
            buffer: self.current_buffer_mut(),
        };
        render_callback(&mut frame);
        let cursor_position = frame.cursor_position;

        self.flush()?;
        match cursor_position {
            Some(position) => {
                self.show_cursor()?;
                self.set_cursor_position(position)?;
            }
            None => self.hide_cursor()?,
        }
        self.swap_buffers();
        Backend::flush(&mut self.backend)?;
        Ok(())
    }

    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        self.backend.set_cursor_position(position)?;
        self.last_known_cursor_pos = position;
        Ok(())
    }

    pub fn show_cursor(&mut self) -> io::Result<()> {
        self.backend.show_cursor()?;
        self.hidden_cursor = false;
        Ok(())
    }

    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.hide_cursor()?;
        self.hidden_cursor = true;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        let updates = diff_buffers(self.previous_buffer(), self.current_buffer());
        if let Some(DrawCommand::Put { x, y, cell }) = updates
            .iter()
            .rev()
            .find(|command| matches!(command, DrawCommand::Put { .. }))
        {
            let width = u16::try_from(display_width(cell.symbol())).unwrap_or(1);
            self.last_known_cursor_pos = Position {
                x: x.saturating_add(width.saturating_sub(1)),
                y: *y,
            };
        }
        draw(&mut self.backend, updates.into_iter())
    }

    fn swap_buffers(&mut self) {
        self.previous_buffer_mut().reset();
        self.current = 1 - self.current;
    }

    fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    fn previous_buffer(&self) -> &Buffer {
        &self.buffers[1 - self.current]
    }

    fn previous_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[1 - self.current]
    }
}

#[derive(Debug)]
enum DrawCommand {
    Put { x: u16, y: u16, cell: Cell },
    ClearToEnd { x: u16, y: u16, bg: Color },
}

fn diff_buffers(previous: &Buffer, next: &Buffer) -> Vec<DrawCommand> {
    let mut updates = Vec::new();
    let mut last_nonblank_columns = vec![0; next.area.height as usize];

    for y in 0..next.area.height {
        let row_start = y as usize * next.area.width as usize;
        let row_end = row_start + next.area.width as usize;
        let row = &next.content[row_start..row_end];
        let bg = row.last().map(|cell| cell.bg).unwrap_or(Color::Reset);

        let mut last_nonblank_column = 0usize;
        let mut column = 0usize;
        while column < row.len() {
            let cell = &row[column];
            let width = display_width(cell.symbol());
            if cell.symbol() != " " || cell.bg != bg || cell.modifier != Modifier::empty() {
                last_nonblank_column = column + (width.saturating_sub(1));
            }
            column += width.max(1);
        }

        if last_nonblank_column + 1 < row.len() {
            let x = next.area.x + last_nonblank_column as u16 + 1;
            let y = next.area.y + y;
            updates.push(DrawCommand::ClearToEnd { x, y, bg });
        }

        last_nonblank_columns[y as usize] = last_nonblank_column as u16;
    }

    let mut invalidated = 0usize;
    let mut to_skip = 0usize;
    for (index, (current, previous)) in next.content.iter().zip(previous.content.iter()).enumerate()
    {
        if !current.skip && (current != previous || invalidated > 0) && to_skip == 0 {
            let width = next.area.width as usize;
            let x = next.area.x + (index % width) as u16;
            let y = next.area.y + (index / width) as u16;
            let row = index / width;
            if (index % width) as u16 <= last_nonblank_columns[row] {
                updates.push(DrawCommand::Put {
                    x,
                    y,
                    cell: current.clone(),
                });
            }
        }

        to_skip = display_width(current.symbol()).saturating_sub(1);
        let affected_width = display_width(current.symbol()).max(display_width(previous.symbol()));
        invalidated = affected_width.max(invalidated).saturating_sub(1);
    }

    updates
}

fn draw<I>(writer: &mut impl Write, commands: I) -> io::Result<()>
where
    I: Iterator<Item = DrawCommand>,
{
    let mut fg = Color::Reset;
    let mut bg = Color::Reset;
    let mut modifier = Modifier::empty();
    let mut last_pos: Option<Position> = None;

    for command in commands {
        let (x, y) = match &command {
            DrawCommand::Put { x, y, .. } => (*x, *y),
            DrawCommand::ClearToEnd { x, y, .. } => (*x, *y),
        };

        if !matches!(last_pos, Some(pos) if pos.x + 1 == x && pos.y == y) {
            queue!(writer, MoveTo(x, y))?;
        }
        last_pos = Some(Position { x, y });

        match command {
            DrawCommand::Put { cell, .. } => {
                if cell.modifier != modifier {
                    ModifierDiff {
                        from: modifier,
                        to: cell.modifier,
                    }
                    .queue(writer)?;
                    modifier = cell.modifier;
                }
                if cell.fg != fg || cell.bg != bg {
                    queue!(
                        writer,
                        SetColors(Colors::new(
                            to_crossterm_color(cell.fg),
                            to_crossterm_color(cell.bg)
                        ))
                    )?;
                    fg = cell.fg;
                    bg = cell.bg;
                }
                queue!(writer, Print(cell.symbol()))?;
            }
            DrawCommand::ClearToEnd { bg: clear_bg, .. } => {
                queue!(writer, SetAttribute(crossterm::style::Attribute::Reset))?;
                modifier = Modifier::empty();
                queue!(writer, SetBackgroundColor(to_crossterm_color(clear_bg)))?;
                bg = clear_bg;
                queue!(writer, Clear(crossterm::terminal::ClearType::UntilNewLine))?;
            }
        }
    }

    queue!(
        writer,
        SetForegroundColor(crossterm::style::Color::Reset),
        SetBackgroundColor(crossterm::style::Color::Reset),
        SetAttribute(crossterm::style::Attribute::Reset),
    )?;
    Ok(())
}

struct ModifierDiff {
    from: Modifier,
    to: Modifier,
}

impl ModifierDiff {
    fn queue<W: Write>(self, writer: &mut W) -> io::Result<()> {
        use crossterm::style::Attribute;

        let removed = self.from - self.to;
        if removed.contains(Modifier::REVERSED) {
            queue!(writer, SetAttribute(Attribute::NoReverse))?;
        }
        if removed.contains(Modifier::BOLD) {
            queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
            if self.to.contains(Modifier::DIM) {
                queue!(writer, SetAttribute(Attribute::Dim))?;
            }
        }
        if removed.contains(Modifier::ITALIC) {
            queue!(writer, SetAttribute(Attribute::NoItalic))?;
        }
        if removed.contains(Modifier::UNDERLINED) {
            queue!(writer, SetAttribute(Attribute::NoUnderline))?;
        }
        if removed.contains(Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
        }
        if removed.contains(Modifier::CROSSED_OUT) {
            queue!(writer, SetAttribute(Attribute::NotCrossedOut))?;
        }
        if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
            queue!(writer, SetAttribute(Attribute::NoBlink))?;
        }

        let added = self.to - self.from;
        if added.contains(Modifier::REVERSED) {
            queue!(writer, SetAttribute(Attribute::Reverse))?;
        }
        if added.contains(Modifier::BOLD) {
            queue!(writer, SetAttribute(Attribute::Bold))?;
        }
        if added.contains(Modifier::ITALIC) {
            queue!(writer, SetAttribute(Attribute::Italic))?;
        }
        if added.contains(Modifier::UNDERLINED) {
            queue!(writer, SetAttribute(Attribute::Underlined))?;
        }
        if added.contains(Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::Dim))?;
        }
        if added.contains(Modifier::CROSSED_OUT) {
            queue!(writer, SetAttribute(Attribute::CrossedOut))?;
        }
        if added.contains(Modifier::SLOW_BLINK) {
            queue!(writer, SetAttribute(Attribute::SlowBlink))?;
        }
        if added.contains(Modifier::RAPID_BLINK) {
            queue!(writer, SetAttribute(Attribute::RapidBlink))?;
        }
        Ok(())
    }
}

fn display_width(text: &str) -> usize {
    text.width().max(1)
}

fn to_crossterm_color(color: Color) -> crossterm::style::Color {
    match color {
        Color::Reset => crossterm::style::Color::Reset,
        Color::Black => crossterm::style::Color::Black,
        Color::Red => crossterm::style::Color::DarkRed,
        Color::Green => crossterm::style::Color::DarkGreen,
        Color::Yellow => crossterm::style::Color::DarkYellow,
        Color::Blue => crossterm::style::Color::DarkBlue,
        Color::Magenta => crossterm::style::Color::DarkMagenta,
        Color::Cyan => crossterm::style::Color::DarkCyan,
        Color::Gray => crossterm::style::Color::Grey,
        Color::DarkGray => crossterm::style::Color::DarkGrey,
        Color::LightRed => crossterm::style::Color::Red,
        Color::LightGreen => crossterm::style::Color::Green,
        Color::LightYellow => crossterm::style::Color::Yellow,
        Color::LightBlue => crossterm::style::Color::Blue,
        Color::LightMagenta => crossterm::style::Color::Magenta,
        Color::LightCyan => crossterm::style::Color::Cyan,
        Color::White => crossterm::style::Color::White,
        Color::Rgb(r, g, b) => crossterm::style::Color::Rgb { r, g, b },
        Color::Indexed(index) => crossterm::style::Color::AnsiValue(index),
    }
}
