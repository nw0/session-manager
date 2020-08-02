//! Console buffer implementation.
use log::{debug, info, trace, warn};
use std::cmp::{max, min};
use std::convert::{TryFrom, TryInto};
use std::fs::File;
use std::io::Write;
use termion::raw::RawTerminal;
use vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorStyle, Handler, LineClearMode, Mode, Rgb, StandardCharset,
    TabulationClearMode,
};

enum Displace {
    Absolute(i64),
    Relative(i64),
    ToStart,
    ToTabStop,
}

#[derive(Eq, PartialEq)]
enum Range {
    Full,
    FromCursor,
    ToCursor,
}

/// Zero-indexed cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CursorPos {
    /// The x-coordinate.
    col: u16,
    /// The y-coordinate.
    row: u16,
}

/// The display buffer of a console.
pub struct Grid {
    cursor: CursorPos,
    saved_cursor: CursorPos,
    scrolling_region: (u16, u16),
    width: u16,
    height: u16,
    buffer: Vec<Cell>,
}

impl Grid {
    /// Initialise an empty display buffer.
    pub fn new(width: u16, height: u16) -> Grid {
        let sz = width * height;
        let mut buffer = Vec::with_capacity(sz as usize);
        for _ in 0..sz {
            buffer.push(Cell::default());
        }
        Grid {
            cursor: Default::default(),
            saved_cursor: Default::default(),
            scrolling_region: (0, height),
            width,
            height,
            buffer,
        }
    }

    /// Draw this buffer to `term`.
    pub fn draw(&self, term: &mut RawTerminal<File>) {
        write!(
            term,
            "{}{}{}",
            termion::cursor::Goto(1, 1),
            self.buffer.iter().map(|c| c.c).collect::<String>(),
            termion::cursor::Goto(1 + self.cursor.col, 1 + self.cursor.row)
        )
        .unwrap();
    }

    pub fn update(&mut self, c: char) {
        self.buffer[(self.cursor.col + self.cursor.row * self.width) as usize].c = c;
        self.cursor.col += 1;
        if self.cursor.col == self.width {
            self.cursor.col = 0;
            self.cursor.row += 1;
            if self.cursor.row == self.height {
                self.cursor.row -= 1;
            }
        }
    }

    fn buffer_idx(&self, x: u16, y: u16) -> usize {
        // TODO: x >= width, y >= height?
        (x + y * self.width).into()
    }

    fn move_horizontal(&mut self, displacement: Displace) {
        self.cursor.col = match displacement {
            Displace::Absolute(offset) => max(0, min(self.width as i64 - 1, offset)),
            Displace::Relative(offset) => max(
                0,
                min(self.width as i64 - 1, self.cursor.col as i64 + offset),
            ),
            Displace::ToStart => 0,
            Displace::ToTabStop => ((self.cursor.col + 8) & !7).into(),
        }
        .try_into()
        .unwrap();
    }

    fn move_vertical(&mut self, displacement: Displace) {
        self.cursor.row = match displacement {
            Displace::Absolute(offset) => max(0, min(self.height as i64 - 1, offset)),
            Displace::Relative(offset) => max(
                0,
                min(self.height as i64 - 1, self.cursor.row as i64 + offset),
            ),
            Displace::ToStart => 0,
            Displace::ToTabStop => {
                warn!("unimpl: vertical tab");
                self.cursor.row.into()
            }
        }
        .try_into()
        .unwrap();
        // no scrolling
    }

    fn erase_display(&mut self, range: Range) {
        let start = if range == Range::FromCursor {
            self.buffer_idx(self.cursor.col, self.cursor.row)
        } else {
            0
        };
        let end = if range == Range::ToCursor {
            self.buffer_idx(self.cursor.col, self.cursor.row)
        } else {
            self.buffer.len()
        };
        for i in start..end {
            self.buffer[i] = Cell::default();
        }
    }

    fn erase_line(&mut self, range: Range) {
        let start = if range == Range::FromCursor {
            self.buffer_idx(self.cursor.col, self.cursor.row)
        } else {
            self.buffer_idx(0, self.cursor.row)
        };
        let end = if range == Range::ToCursor {
            self.buffer_idx(self.cursor.col, self.cursor.row)
        } else {
            self.buffer_idx(self.width - 1, self.cursor.row)
        };
        for i in start..end {
            self.buffer[i] = Cell::default();
        }
    }

    fn cursor_save(&mut self) {
        self.saved_cursor = self.cursor;
    }

    fn cursor_restore(&mut self) {
        self.cursor = self.saved_cursor;
    }

    pub fn set_cell(&mut self, c: char, x: u16, y: u16) {
        // TODO: check x < width, y < height
        self.buffer[(x + y * self.width) as usize].c = c;
    }

    pub fn get_cell(&self, x: u16, y: u16) -> char {
        self.buffer[(x + y * self.width) as usize].c
    }

    fn scroll_up_in_region(&mut self, lines: u16) {
        // Move text UP
        trace!(
            "scroll UP, region: ({:?}, lines: {})",
            self.scrolling_region, lines
        );
        if lines < 1 {
            return;
        }
        for y in self.scrolling_region.0..self.scrolling_region.1 {
            for x in 0..self.width {
                if y + lines < self.scrolling_region.1 {
                    self.set_cell(self.get_cell(x, y + lines), x, y);
                } else {
                    self.set_cell('.', x, y);
                }
            }
        }
    }

    fn scroll_down_in_region(&mut self, lines: u16) {
        // Move text DOWN
        trace!(
            "scroll DOWN, region: ({:?}), lines: {}",
            self.scrolling_region, lines
        );
        if lines < 1 {
            return;
        }
        for y in (self.scrolling_region.0..self.scrolling_region.1).rev() {
            for x in 0..self.width {
                if y >= lines + self.scrolling_region.0 {
                    self.set_cell(self.get_cell(x, y - lines), x, y);
                } else {
                    self.set_cell('.', x, y);
                }
            }
        }
    }

    fn insert_col(&mut self, cols: u16) {
        if cols < 1 {
            return;
        }
        for x in (self.cursor.col..self.width).rev() {
            if x >= cols + self.cursor.col {
                self.set_cell(self.get_cell(x - cols, self.cursor.row), x, self.cursor.row);
            } else {
                self.set_cell('.', x, self.cursor.row);
            }
        }
    }

    fn insert_line(&mut self, lines: u16) {
        trace!(
            "INSERT LINES {}, from {} in {:?}",
            lines,
            self.cursor.row,
            self.scrolling_region
        );
        // Move this line down...
        if lines < 1
            || self.cursor.row < self.scrolling_region.0
            || self.cursor.row >= self.scrolling_region.1
        {
            return;
        }
        for y in (self.cursor.row..self.scrolling_region.1).rev() {
            for x in 0..self.width {
                if y >= lines + self.cursor.row {
                    self.set_cell(self.get_cell(x, y - lines), x, y);
                } else {
                    self.set_cell('.', x, y);
                }
            }
        }
    }

    fn report_status(&mut self, file: &mut File) {
        const ESC: u8 = 0x1b;
        let buf = [ESC, b'[', b'0', b'n'];
        file.write_all(&buf).unwrap();
    }

    fn report_cursor(&mut self, file: &mut File) {
        trace!(
            "cursor at ({} + 1, {} + 1)",
            self.cursor.col,
            self.cursor.row
        );
        file.write_fmt(format_args!(
            "\x1b[{};{}R",
            self.cursor.row + 1,
            self.cursor.col + 1
        ))
        .unwrap();
    }
}

impl Handler<File> for Grid {
    fn set_title(&mut self, title: Option<&str>) {
        // TODO
        info!("set title: {:?}", title);
    }

    fn set_cursor_style(&mut self, _: Option<CursorStyle>) {
        // TODO
    }

    fn input(&mut self, c: char) {
        // TODO: handle c.width() != 1
        self.update(c);
    }

    fn goto(&mut self, row: usize, col: usize) {
        // TODO: change Displace type
        self.move_horizontal(Displace::Absolute((col).try_into().unwrap()));
        self.move_vertical(Displace::Absolute((row).try_into().unwrap()));
    }

    fn goto_line(&mut self, row: usize) {
        self.move_vertical(Displace::Absolute(row.try_into().unwrap()));
    }

    fn goto_col(&mut self, col: usize) {
        self.move_horizontal(Displace::Absolute(col.try_into().unwrap()));
    }

    fn insert_blank(&mut self, cols: usize) {
        self.insert_col(u16::try_from(cols).unwrap());
    }

    fn move_up(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(-i64::try_from(rows).unwrap()));
    }

    fn move_down(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(i64::try_from(rows).unwrap()));
    }

    fn identify_terminal(&mut self, _: &mut File, _intermediate: Option<char>) {
        // TODO
    }

    fn device_status(&mut self, file: &mut File, param: usize) {
        match param {
            5 => self.report_status(file),
            6 => self.report_cursor(file),
            _ => debug!("invalid device status report {}", param),
        }
    }

    fn move_forward(&mut self, cols: usize) {
        self.move_horizontal(Displace::Relative(i64::try_from(cols).unwrap()));
    }

    fn move_backward(&mut self, cols: usize) {
        self.move_horizontal(Displace::Relative(-i64::try_from(cols).unwrap()));
    }

    fn move_down_and_cr(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(i64::try_from(rows).unwrap()));
        self.move_horizontal(Displace::ToStart);
    }

    fn move_up_and_cr(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(-i64::try_from(rows).unwrap()));
        self.move_horizontal(Displace::ToStart);
    }

    fn put_tab(&mut self, count: i64) {
        // FIXME
        for _ in 0..count {
            self.move_horizontal(Displace::ToTabStop);
        }
    }

    fn backspace(&mut self) {
        self.move_horizontal(Displace::Relative(-1));
    }

    fn carriage_return(&mut self) {
        self.move_horizontal(Displace::ToStart);
    }

    fn linefeed(&mut self) {
        if self.cursor.row + 1 == self.scrolling_region.1 {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.height {
            self.cursor.row += 1;
        } else {
            debug!("tried to scroll past end of grid");
        }
    }

    fn bell(&mut self) {
        info!("BEL");
    }

    fn substitute(&mut self) {}

    fn newline(&mut self) {
        self.linefeed();
    }

    fn set_horizontal_tabstop(&mut self) {
        // TODO
    }

    fn scroll_up(&mut self, rows: usize) {
        self.scroll_up_in_region(u16::try_from(rows).unwrap());
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll_down_in_region(u16::try_from(rows).unwrap());
    }

    fn insert_blank_lines(&mut self, rows: usize) {
        trace!("IL: {}", rows);
        self.insert_line(u16::try_from(rows).unwrap());
    }

    fn delete_lines(&mut self, rows: usize) {
        trace!("DL: {}", rows);
        let rows = u16::try_from(rows).unwrap();
        if rows < 1 {
            return;
        }
        for y in self.cursor.row..self.height {
            for x in 0..self.width {
                if y < self.cursor.row + rows {
                    self.set_cell(self.get_cell(x, y + rows), x, y);
                } else {
                    self.set_cell('.', x, y);
                }
            }
        }
    }

    fn erase_chars(&mut self, cols: usize) {
        let cols = u16::try_from(cols).unwrap();
        for x1 in 0..cols {
            let x = self.cursor.col + x1;
            if x < self.width {
                self.set_cell('.', x, self.cursor.row);
            }
        }
    }

    fn delete_chars(&mut self, cols: usize) {
        let cols = u16::try_from(cols).unwrap();
        for x in self.cursor.col..self.width {
            if x + cols < self.width {
                self.set_cell(self.get_cell(x + cols, self.cursor.row), x, self.cursor.row);
            } else {
                self.set_cell('.', x, self.cursor.row);
            }
        }
    }

    fn move_backward_tabs(&mut self, _count: i64) {
        // TODO
    }

    fn move_forward_tabs(&mut self, count: i64) {
        for _ in 0..count {
            self.move_horizontal(Displace::ToTabStop);
        }
    }

    fn save_cursor_position(&mut self) {
        self.cursor_save();
    }

    fn restore_cursor_position(&mut self) {
        self.cursor_restore();
    }

    fn clear_line(&mut self, mode: LineClearMode) {
        match mode {
            LineClearMode::All => self.erase_line(Range::Full),
            LineClearMode::Left => self.erase_line(Range::ToCursor),
            LineClearMode::Right => self.erase_line(Range::FromCursor),
        }
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        match mode {
            ClearMode::All | ClearMode::Saved => self.erase_display(Range::Full),
            ClearMode::Above => self.erase_display(Range::ToCursor),
            ClearMode::Below => self.erase_display(Range::FromCursor),
        }
    }

    fn clear_tabs(&mut self, _mode: TabulationClearMode) {
        // TODO
    }

    fn reset_state(&mut self) {
        // TODO
    }

    fn reverse_index(&mut self) {
        trace!("RI");
        if self.cursor.row == self.scrolling_region.0 {
            self.scroll_down(1);
        } else {
            self.cursor.row -= 1;
        }
    }

    fn terminal_attribute(&mut self, _attr: Attr) {
        // TODO
    }

    fn set_mode(&mut self, mode: Mode) {
        // TODO
        debug!("set mode: {:?}", mode);
    }

    fn unset_mode(&mut self, mode: Mode) {
        // TODO
        debug!("unset mode: {:?}", mode);
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        // set scrolling region to [Pt, Pb] (1-indexed).
        debug!("set scroll region: {:?} - {:?}", top, bottom);

        let bottom = bottom.unwrap_or(self.height as usize);
        self.scrolling_region = (
            u16::try_from(top - 1).unwrap(),
            min(u16::try_from(bottom).unwrap(), self.height),
        );
        self.goto(0, 0);
    }

    fn set_keypad_application_mode(&mut self) {
        debug!("set keypad");
    }

    fn unset_keypad_application_mode(&mut self) {
        debug!("unset keypad");
    }

    fn set_active_charset(&mut self, _: CharsetIndex) {
        debug!("set charset");
    }

    fn configure_charset(&mut self, _: CharsetIndex, _: StandardCharset) {
        debug!("config charset");
    }

    fn set_color(&mut self, _: usize, _: Rgb) {
        debug!("set color");
    }

    fn dynamic_color_sequence(&mut self, _: &mut File, _: u8, _: usize, _: &str) {
        debug!("write color seq");
    }

    fn reset_color(&mut self, _: usize) {
        debug!("reset color");
    }

    fn clipboard_store(&mut self, _: u8, _: &[u8]) {}

    fn clipboard_load(&mut self, _: u8, _: &str) {}

    fn decaln(&mut self) {}

    fn push_title(&mut self) {}

    fn pop_title(&mut self) {}
}

struct Cell {
    pub c: char,
}

impl Cell {
    pub fn default() -> Cell {
        Cell { c: '.' }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto() {
        let mut grid = Grid::new(4, 4);
        grid.goto(1, 1);
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.move_up_and_cr(1);
        assert_eq!(grid.cursor, CursorPos { col: 0, row: 0 });
        grid.move_down(6);
        assert_eq!(grid.cursor, CursorPos { col: 0, row: 3 });
    }

    #[test]
    fn linefeed() {
        let mut grid = Grid::new(4, 2);
        grid.goto(0, 1);
        grid.linefeed();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.linefeed();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.reverse_index();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 0 });
    }
}
