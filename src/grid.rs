//! Console buffer implementation.
use log::{debug, info, warn};
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

/// The display buffer of a console.
pub struct Grid {
    cursor_x: u16,
    cursor_y: u16,
    saved_cursor: (u16, u16),
    width: u16,
    height: u16,
    buffer: Vec<Cell>,
    pty_file: File,
}

impl Grid {
    /// Initialise an empty display buffer.
    pub fn new(width: u16, height: u16, pty_file: File) -> Grid {
        let sz = width * height;
        let mut buffer = Vec::with_capacity(sz as usize);
        for _ in 0..sz {
            buffer.push(Cell::default());
        }
        Grid {
            cursor_x: 0,
            cursor_y: 0,
            saved_cursor: (0, 0),
            width,
            height,
            buffer,
            pty_file,
        }
    }

    /// Draw this buffer to `term`.
    pub fn draw(&self, term: &mut RawTerminal<File>) {
        write!(
            term,
            "{}{}{}",
            termion::cursor::Goto(1, 1),
            self.buffer.iter().map(|c| c.c).collect::<String>(),
            termion::cursor::Goto(1 + self.cursor_x, 1 + self.cursor_y)
        )
        .unwrap();
    }

    pub fn update(&mut self, c: char) {
        self.buffer[(self.cursor_x + self.cursor_y * self.width) as usize].c = c;
        self.cursor_x += 1;
        if self.cursor_x == self.width {
            self.cursor_x = 0;
            self.cursor_y += 1;
            if self.cursor_y == self.height {
                self.cursor_y -= 1;
            }
        }
    }

    fn buffer_idx(&self, x: u16, y: u16) -> usize {
        // TODO: x >= width, y >= height?
        (x + y * self.width).into()
    }

    fn move_horizontal(&mut self, displacement: Displace) {
        self.cursor_x = match displacement {
            Displace::Absolute(offset) => max(0, min(self.width as i64 - 1, offset)),
            Displace::Relative(offset) => {
                max(0, min(self.width as i64 - 1, self.cursor_x as i64 + offset))
            }
            Displace::ToStart => 0,
            Displace::ToTabStop => ((self.cursor_x + 8) & !7).into(),
        }
        .try_into()
        .unwrap();
    }

    fn move_vertical(&mut self, displacement: Displace) {
        self.cursor_y = match displacement {
            Displace::Absolute(offset) => max(0, min(self.height as i64 - 1, offset)),
            Displace::Relative(offset) => max(
                0,
                min(self.height as i64 - 1, self.cursor_y as i64 + offset),
            ),
            Displace::ToStart => 0,
            Displace::ToTabStop => {
                warn!("unimpl: vertical tab");
                self.cursor_y.into()
            }
        }
        .try_into()
        .unwrap();
        // no scrolling
    }

    fn erase_display(&mut self, range: Range) {
        let start = if range == Range::FromCursor {
            self.buffer_idx(self.cursor_x, self.cursor_y)
        } else {
            0
        };
        let end = if range == Range::ToCursor {
            self.buffer_idx(self.cursor_x, self.cursor_y)
        } else {
            self.buffer.len()
        };
        for i in start..end {
            self.buffer[i] = Cell::default();
        }
    }

    fn erase_line(&mut self, range: Range) {
        let start = if range == Range::FromCursor {
            self.buffer_idx(self.cursor_x, self.cursor_y)
        } else {
            self.buffer_idx(0, self.cursor_y)
        };
        let end = if range == Range::ToCursor {
            self.buffer_idx(self.cursor_x, self.cursor_y)
        } else {
            self.buffer_idx(self.width - 1, self.cursor_y)
        };
        for i in start..end {
            self.buffer[i] = Cell::default();
        }
    }

    fn cursor_save(&mut self) {
        self.saved_cursor = (self.cursor_x, self.cursor_y);
    }

    fn cursor_restore(&mut self) {
        self.move_horizontal(Displace::Absolute(self.saved_cursor.0 as i64));
        self.move_vertical(Displace::Absolute(self.saved_cursor.1 as i64));
    }

    pub fn set_current(&mut self, c: char) {
        self.set_cell(c, self.cursor_x, self.cursor_y);
    }

    pub fn set_cell(&mut self, c: char, x: u16, y: u16) {
        // TODO: check x < width, y < height
        self.buffer[(x + y * self.width) as usize].c = c;
    }

    pub fn get_cell(&self, x: u16, y: u16) -> char {
        self.buffer[(x + y * self.width) as usize].c
    }

    fn scroll_up(&mut self, lines: u16) {
        if lines < 1 {
            return;
        }
        for y in self.height..0 {
            for x in 0..self.width {
                if y > lines {
                    self.set_cell(self.get_cell(x, y - lines - 1), x, y - 1);
                } else {
                    self.set_cell('.', x, y - 1);
                }
            }
        }
    }

    // Move viewport down (text up)
    fn scroll_down(&mut self, lines: u16) {
        if lines < 1 {
            return;
        }
        for y in 0..self.height {
            for x in 0..self.width {
                if y + lines < self.height {
                    self.set_cell(self.get_cell(x, y + lines), x, y);
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
        for x in self.width..self.cursor_x {
            if x > cols + self.cursor_x {
                self.set_cell(
                    self.get_cell(x - cols - 1, self.cursor_y),
                    x - 1,
                    self.cursor_y,
                );
            } else {
                self.set_cell('.', x - 1, self.cursor_y);
            }
        }
    }

    fn insert_line(&mut self, lines: u16) {
        // Move this line down...
        if lines < 1 {
            return;
        }
        for y in self.height..self.cursor_y {
            for x in 0..self.width {
                if y > lines + self.cursor_y {
                    self.set_cell(self.get_cell(x, y - lines - 1), x, y - 1);
                } else {
                    self.set_cell('.', x, y - 1);
                }
            }
        }
    }

    fn report_status(&mut self) {
        const ESC: u8 = 0x1b;
        let buf = [ESC, b'[', b'0', b'n'];
        self.pty_file.write_all(&buf).unwrap();
    }

    fn report_cursor(&mut self) {
        self.pty_file
            .write_fmt(format_args!(
                "\x1b[{};{}R",
                self.cursor_y + 1,
                self.cursor_x + 1
            ))
            .unwrap();
    }
}

impl Handler<()> for Grid {
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
        self.move_vertical(Displace::Absolute((row - 1).try_into().unwrap()));
    }

    fn goto_col(&mut self, col: usize) {
        self.move_horizontal(Displace::Absolute((col - 1).try_into().unwrap()));
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

    fn identify_terminal(&mut self, _: &mut (), _intermediate: Option<char>) {
        // TODO
    }

    fn device_status(&mut self, _: &mut (), param: usize) {
        match param {
            5 => self.report_status(),
            6 => self.report_cursor(),
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
        self.cursor_y += 1;
        if self.cursor_y == self.height {
            self.scroll_down(1);
            self.cursor_y -= 1;
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
        self.scroll_up(u16::try_from(rows).unwrap());
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll_down(u16::try_from(rows).unwrap());
    }

    fn insert_blank_lines(&mut self, rows: usize) {
        self.insert_line(u16::try_from(rows).unwrap());
    }

    fn delete_lines(&mut self, rows: usize) {
        let rows = u16::try_from(rows).unwrap();
        if rows < 1 {
            return;
        }
        for y in self.cursor_y..self.height {
            for x in 0..self.width {
                if y < self.cursor_y + rows {
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
            let x = self.cursor_x + x1;
            if x < self.width {
                self.set_cell('.', x, self.cursor_y);
            }
        }
    }

    fn delete_chars(&mut self, cols: usize) {
        let cols = u16::try_from(cols).unwrap();
        for x in self.cursor_x..self.width {
            if x + cols < self.width {
                self.set_cell(self.get_cell(x + cols, self.cursor_y), x, self.cursor_y);
            } else {
                self.set_cell('.', x, self.cursor_y);
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
        if self.cursor_y == 0 {
            self.scroll_up(1);
        } else {
            self.cursor_y -= 1;
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
        // TODO
        debug!("set scroll region: {:?} - {:?}", top, bottom)
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

    fn dynamic_color_sequence(&mut self, _: &mut (), _: u8, _: usize, _: &str) {
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
