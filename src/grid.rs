//! Console buffer implementation.

use std::{
    cmp::{max, min},
    convert::{TryFrom, TryInto},
    fs::File,
    io::Write,
    marker::PhantomData,
    ops::Range,
};

use log::{debug, info, trace, warn};
use termion::raw::RawTerminal;
use vte::ansi::{
    Attr, CharsetIndex, ClearMode, CursorStyle, Handler, LineClearMode, Mode, Rgb,
    StandardCharset, TabulationClearMode,
};

enum Displace {
    Absolute(i64),
    Relative(i64),
    ToStart,
    ToTabStop,
}

/// Zero-indexed cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CursorPos {
    /// The x-coordinate.
    col: u16,
    /// The y-coordinate.
    row: u16,
}

impl CursorPos {
    /// Note that this is col, row (x, y).
    fn at(col: u16, row: u16) -> CursorPos {
        CursorPos { col, row }
    }
}

/// The display buffer of a console.
pub struct Grid<W> {
    cursor: CursorPos,
    saved_cursor: CursorPos,
    scrolling_region: Range<u16>,
    width: u16,
    height: u16,
    buffer: Vec<Cell>,
    _phantom: PhantomData<W>,
}

impl<W: Write> Grid<W> {
    /// Initialise an empty display buffer.
    pub fn new(width: u16, height: u16) -> Grid<W> {
        let sz = width * height;
        let mut buffer = Vec::with_capacity(sz as usize);
        for _ in 0..sz {
            buffer.push(Cell::default());
        }
        Grid {
            cursor: Default::default(),
            saved_cursor: Default::default(),
            scrolling_region: 0..height,
            width,
            height,
            buffer,
            _phantom: Default::default(),
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

    fn buffer_idx(&self, pos: CursorPos) -> usize {
        // TODO: check row, col are in bounds
        (pos.col + pos.row * self.width).into()
    }

    fn cell_at(&self, pos: CursorPos) -> &Cell {
        &self.buffer[self.buffer_idx(pos)]
    }

    fn cell_at_mut(&mut self, pos: CursorPos) -> &mut Cell {
        let idx = self.buffer_idx(pos);
        &mut self.buffer[idx]
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

    fn scroll_up_in_region(&mut self, start: u16, lines: u16) {
        // Move text UP
        trace!(
            "scroll UP, region: ({:?}, lines: {})",
            self.scrolling_region,
            lines
        );
        if lines < 1 {
            return;
        }
        for row in start..self.scrolling_region.end {
            for col in 0..self.width {
                *self.cell_at_mut(CursorPos { col, row }) =
                    if row + lines < self.scrolling_region.end {
                        *self.cell_at(CursorPos::at(col, row + lines))
                    } else {
                        Cell::default()
                    };
            }
        }
    }

    fn scroll_down_in_region(&mut self, start: u16, lines: u16) {
        // Move text DOWN
        trace!(
            "scroll DOWN, region: ({:?}), lines: {}",
            self.scrolling_region,
            lines
        );
        if lines < 1 {
            return;
        }
        for row in (start..self.scrolling_region.end).rev() {
            for col in 0..self.width {
                *self.cell_at_mut(CursorPos { col, row }) = if row >= lines + start {
                    *self.cell_at(CursorPos::at(col, row - lines))
                } else {
                    Cell::default()
                };
            }
        }
    }
}

impl<W: Write> Handler<W> for Grid<W> {
    fn set_title(&mut self, title: Option<&str>) {
        // TODO
        info!("set title: {:?}", title);
    }

    fn set_cursor_style(&mut self, _: Option<CursorStyle>) {
        // TODO
    }

    fn input(&mut self, c: char) {
        // TODO: handle c.width() != 1
        self.cell_at_mut(self.cursor).c = c;
        self.cursor.col += 1;
        if self.cursor.col == self.width {
            // FIXME: want to change this to self.linefeed, but it breaks tmux
            // I suspect we only want to linefeed if there is actual input on the next row
            // TODO: test case for this scenario
            self.cursor.row += 1;
            if self.cursor.row == self.scrolling_region.end {
                self.cursor.row -= 1;
            }
            self.carriage_return();
        }
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
        let cols = u16::try_from(cols).unwrap();
        if cols < 1 {
            return;
        }
        for col in (self.cursor.col..self.width).rev() {
            *self.cell_at_mut(CursorPos::at(col, self.cursor.row)) =
                if col >= cols + self.cursor.col {
                    *self.cell_at(CursorPos::at(col - cols, self.cursor.row))
                } else {
                    Cell::default()
                };
        }
    }

    fn move_up(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(-i64::try_from(rows).unwrap()));
    }

    fn move_down(&mut self, rows: usize) {
        self.move_vertical(Displace::Relative(i64::try_from(rows).unwrap()));
    }

    fn identify_terminal(&mut self, _: &mut W, _intermediate: Option<char>) {
        // TODO
    }

    fn device_status(&mut self, file: &mut W, param: usize) {
        match param {
            5 => {
                let buf = [0x1b, b'[', b'0', b'n'];
                file.write_all(&buf).unwrap();
            }
            6 => {
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
        if self.cursor.row + 1 == self.scrolling_region.end {
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
        self.scroll_up_in_region(
            self.scrolling_region.start,
            u16::try_from(rows).unwrap(),
        );
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll_down_in_region(
            self.scrolling_region.start,
            u16::try_from(rows).unwrap(),
        );
    }

    fn insert_blank_lines(&mut self, rows: usize) {
        trace!("IL: {}", rows);
        if !self.scrolling_region.contains(&self.cursor.row) {
            return;
        }
        self.scroll_down_in_region(self.cursor.row, u16::try_from(rows).unwrap());
    }

    fn delete_lines(&mut self, rows: usize) {
        trace!("DL: {}", rows);
        let rows = u16::try_from(rows).unwrap();
        if !self.scrolling_region.contains(&self.cursor.row) {
            return;
        }
        self.scroll_up_in_region(self.cursor.row, rows);
    }

    fn erase_chars(&mut self, cols: usize) {
        let cols = u16::try_from(cols).unwrap();
        for x1 in 0..cols {
            let col = self.cursor.col + x1;
            if col < self.width {
                *self.cell_at_mut(CursorPos::at(col, self.cursor.row)) =
                    Cell::default();
            }
        }
    }

    fn delete_chars(&mut self, cols: usize) {
        let cols = u16::try_from(cols).unwrap();
        for col in self.cursor.col..self.width {
            *self.cell_at_mut(CursorPos::at(col, self.cursor.row)) =
                if col + cols < self.width {
                    *self.cell_at(CursorPos::at(col + cols, self.cursor.row))
                } else {
                    Cell::default()
                };
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
        self.saved_cursor = self.cursor;
    }

    fn restore_cursor_position(&mut self) {
        self.cursor = self.saved_cursor;
    }

    fn clear_line(&mut self, mode: LineClearMode) {
        let range = match mode {
            LineClearMode::All => {
                self.buffer_idx(CursorPos {
                    col: 0,
                    row: self.cursor.row,
                })..self.buffer_idx(CursorPos {
                    col: self.width,
                    row: self.cursor.row,
                })
            }
            LineClearMode::Left => {
                self.buffer_idx(CursorPos {
                    col: 0,
                    row: self.cursor.row,
                })..self.buffer_idx(self.cursor)
            }
            LineClearMode::Right => {
                self.buffer_idx(self.cursor)..self.buffer_idx(CursorPos {
                    col: self.width,
                    row: self.cursor.row,
                })
            }
        };
        self.buffer[range]
            .iter_mut()
            .for_each(|i| *i = Cell::default());
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        let range = match mode {
            ClearMode::All | ClearMode::Saved => 0..self.buffer.len(),
            ClearMode::Above => 0..self.buffer_idx(self.cursor),
            ClearMode::Below => self.buffer_idx(self.cursor)..self.buffer.len(),
        };
        self.buffer[range]
            .iter_mut()
            .for_each(|i| *i = Cell::default());
    }

    fn clear_tabs(&mut self, _mode: TabulationClearMode) {
        // TODO
    }

    fn reset_state(&mut self) {
        // TODO
    }

    fn reverse_index(&mut self) {
        trace!("RI");
        if self.cursor.row == self.scrolling_region.start {
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
        self.scrolling_region = u16::try_from(top - 1).unwrap()
            ..min(u16::try_from(bottom).unwrap(), self.height);
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

    fn dynamic_color_sequence(&mut self, _: &mut W, _: u8, _: usize, _: &str) {
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

#[derive(Clone, Copy)]
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

    use std::io::{Read, Sink};
    use std::str;
    use tempfile::NamedTempFile;

    #[test]
    fn goto() {
        let mut grid = Grid::<Sink>::new(4, 4);
        grid.goto(1, 1);
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.move_up_and_cr(1);
        assert_eq!(grid.cursor, CursorPos { col: 0, row: 0 });
        grid.move_down(6);
        assert_eq!(grid.cursor, CursorPos { col: 0, row: 3 });
    }

    #[test]
    fn linefeed() {
        let mut grid = Grid::<Sink>::new(4, 2);
        grid.goto(0, 1);
        grid.linefeed();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.linefeed();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 1 });
        grid.reverse_index();
        assert_eq!(grid.cursor, CursorPos { col: 1, row: 0 });
    }

    #[test]
    fn cursor_save() {
        let mut grid = Grid::<Sink>::new(4, 4);
        let original = grid.cursor;
        grid.save_cursor_position();
        grid.linefeed();
        grid.input('c');
        grid.restore_cursor_position();
        assert_eq!(grid.cursor, original);
    }

    #[test]
    fn report() {
        let mut sink = NamedTempFile::new().unwrap();
        let mut source = sink.reopen().unwrap();
        let mut grid = Grid::new(4, 4);
        let mut buf = Vec::new();

        grid.device_status(&mut sink, 12); // invalid
        source.read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), 0);

        grid.device_status(&mut sink, 5);
        source.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), "\x1b[0n"); // Terminal OK

        buf.clear();
        grid.goto(2, 3);
        grid.device_status(&mut sink, 6);
        source.read_to_end(&mut buf).unwrap();
        assert_eq!(str::from_utf8(&buf).unwrap(), "\x1b[3;4R"); // 1-indexed cursor pos
    }

    // TODO: test input/draw
}
