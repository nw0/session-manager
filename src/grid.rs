//! Console buffer implementation.

use std::{
    cmp::{max, min, Ord, Ordering, PartialOrd},
    collections::BTreeSet,
    convert::{TryFrom, TryInto},
    io::Write,
    iter::Iterator,
    marker::PhantomData,
    ops::{Index, IndexMut, Range},
};

use log::{debug, info, trace, warn};
use termion::cursor::Goto;
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

impl From<CursorPos> for Goto {
    fn from(p: CursorPos) -> Goto {
        Goto(1 + p.col, 1 + p.row)
    }
}

impl PartialOrd for CursorPos {
    fn partial_cmp(&self, other: &CursorPos) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CursorPos {
    fn cmp(&self, other: &CursorPos) -> Ordering {
        self.row.cmp(&other.row).then(self.col.cmp(&other.col))
    }
}

#[derive(Clone)]
struct Row<C: Clone + Copy> {
    buf: Vec<C>,
}

impl<C: Clone + Copy> Row<C> {
    pub fn new(cols: u16, fill: C) -> Row<C> {
        Row {
            buf: vec![fill; cols as usize],
        }
    }
}

struct GridBuffer<C: Clone + Copy> {
    rows: Vec<Row<C>>,
}

impl<C: Clone + Copy> GridBuffer<C> {
    pub fn new(cols: u16, rows: u16, fill: C) -> GridBuffer<C> {
        GridBuffer {
            rows: vec![Row::new(cols, fill); rows as usize],
        }
    }
}

impl<C: Clone + Copy> Index<CursorPos> for GridBuffer<C> {
    type Output = C;

    fn index(&self, pos: CursorPos) -> &Self::Output {
        &self.rows[pos.row as usize].buf[pos.col as usize]
    }
}

impl<C: Clone + Copy> IndexMut<CursorPos> for GridBuffer<C> {
    fn index_mut(&mut self, pos: CursorPos) -> &mut Self::Output {
        &mut self.rows[pos.row as usize].buf[pos.col as usize]
    }
}

/// The display buffer of a console.
pub struct Grid<W> {
    cursor: CursorPos,
    saved_cursor: CursorPos,
    scrolling_region: Range<u16>,
    width: u16,
    height: u16,
    buffer: GridBuffer<Cell>,
    dirty_rows: BTreeSet<u16>,
    _phantom: PhantomData<W>,
}

impl<W: Write> Grid<W> {
    /// Initialise an empty display buffer.
    pub fn new(width: u16, height: u16) -> Grid<W> {
        let dirty_rows = (0..height).collect();
        Grid {
            cursor: Default::default(),
            saved_cursor: Default::default(),
            scrolling_region: 0..height,
            width,
            height,
            buffer: GridBuffer::new(width, height, Cell::default()),
            dirty_rows,
            _phantom: Default::default(),
        }
    }

    /// Mark all rows as dirty.
    pub fn mark_all_dirty(&mut self) {
        self.dirty_rows.clear();
        self.dirty_rows.extend(0..self.height);
    }

    /// Draw this buffer to `term`.
    pub fn draw<T: Write>(&mut self, term: &mut T) {
        for row in self.dirty_rows.iter() {
            let start = CursorPos { row: *row, col: 0 };
            write!(
                term,
                "{}{}",
                Goto::from(start),
                self.buffer.rows[*row as usize]
                    .buf
                    .iter()
                    .map(|c| c.c)
                    .collect::<String>()
            )
            .unwrap();
        }
        write!(term, "{}", Goto::from(self.cursor)).unwrap();
        self.dirty_rows.clear();
    }

    /// Resize this grid (not its connected PTY).
    pub fn resize(&mut self, new_width: u16, new_height: u16) {
        // TODO: support re-flowing
        if new_height < self.height {
            let end = if self.cursor.col == 0 {
                self.cursor.row
            } else {
                1 + self.cursor.row
            };
            if end > new_height {
                self.scroll_up_in_region(0, end, end - new_height);
                self.cursor.row -= end - new_height;
            }
            self.scrolling_region.end = min(self.scrolling_region.end, new_height);
            self.saved_cursor.row = min(self.saved_cursor.row, new_height - 1);
        }
        if self.height < new_height {
            if self.scrolling_region.end == self.height {
                self.scrolling_region.end = new_height;
            }
        }
        self.height = new_height;
        self.buffer
            .rows
            .resize(self.height as usize, Row::new(self.width, Cell::default()));

        if new_width < self.width {
            self.cursor.row = min(self.cursor.row, new_width - 1);
            self.saved_cursor.row = min(self.saved_cursor.row, new_width - 1);
        }
        self.width = new_width;
        self.buffer
            .rows
            .iter_mut()
            .for_each(|row| row.buf.resize(new_width as usize, Cell::default()));

        self.mark_all_dirty();
    }

    fn cell_at(&self, pos: CursorPos) -> &Cell {
        &self.buffer[pos]
    }

    fn cell_at_mut(&mut self, pos: CursorPos) -> &mut Cell {
        self.dirty_rows.insert(pos.row);
        &mut self.buffer[pos]
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

    fn scroll_up_in_region(&mut self, start: u16, end: u16, lines: u16) {
        // Move text UP
        trace!("SU ({}), rows: ({}, {})", lines, start, end);
        if lines < 1 {
            return;
        }
        for row in start..end {
            for col in 0..self.width {
                *self.cell_at_mut(CursorPos { col, row }) = if row + lines < end {
                    *self.cell_at(CursorPos::at(col, row + lines))
                } else {
                    Cell::default()
                };
            }
        }
    }

    fn scroll_down_in_region(&mut self, start: u16, end: u16, lines: u16) {
        // Move text DOWN
        trace!("SD ({}), rows ({}, {})", lines, start, end);
        if lines < 1 {
            return;
        }
        for row in (start..end).rev() {
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
        if self.cursor == CursorPos::at(0, self.scrolling_region.end) {
            self.scroll_up(1);
            self.cursor.row -= 1;
        }
        self.cell_at_mut(self.cursor).c = c;
        self.cursor.col += 1;
        if self.cursor.col == self.width {
            self.cursor.row += 1;
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
            debug!(
                "LF: can't scroll ({}, {}, {})",
                self.cursor.row + 1,
                self.scrolling_region.end,
                self.height
            );
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
            self.scrolling_region.end,
            u16::try_from(rows).unwrap(),
        );
    }

    fn scroll_down(&mut self, rows: usize) {
        self.scroll_down_in_region(
            self.scrolling_region.start,
            self.scrolling_region.end,
            u16::try_from(rows).unwrap(),
        );
    }

    fn insert_blank_lines(&mut self, rows: usize) {
        trace!("IL: {}", rows);
        if !self.scrolling_region.contains(&self.cursor.row) {
            return;
        }
        self.scroll_down_in_region(
            self.cursor.row,
            self.scrolling_region.end,
            u16::try_from(rows).unwrap(),
        );
    }

    fn delete_lines(&mut self, rows: usize) {
        trace!("DL: {}", rows);
        let rows = u16::try_from(rows).unwrap();
        if !self.scrolling_region.contains(&self.cursor.row) {
            return;
        }
        self.scroll_up_in_region(self.cursor.row, self.scrolling_region.end, rows);
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
            LineClearMode::All => 0..(self.width as usize),
            LineClearMode::Left => 0..(self.cursor.col as usize),
            LineClearMode::Right => (self.cursor.col as usize)..(self.width as usize),
        };
        self.dirty_rows.insert(self.cursor.row);
        self.buffer.rows[self.cursor.row as usize].buf[range]
            .iter_mut()
            .for_each(|i| *i = Cell::default());
    }

    fn clear_screen(&mut self, mode: ClearMode) {
        let range = match mode {
            ClearMode::All | ClearMode::Saved => {
                CursorPos::at(0, 0)..CursorPos::at(0, self.height)
            }
            ClearMode::Above => CursorPos::at(0, 0)..self.cursor,
            ClearMode::Below => self.cursor..CursorPos::at(0, self.height),
        };
        // TODO: only mark cleared rows
        self.mark_all_dirty();
        self.buffer
            .rows
            .iter_mut()
            .enumerate()
            .flat_map(|(ri, row)| row.buf.iter_mut().map(move |c| (ri, c)))
            .enumerate()
            .for_each(|(col, (row, cell))| {
                if range.contains(&CursorPos::at(col as u16, row as u16)) {
                    *cell = Cell::default();
                }
            });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    macro_rules! input_str {
        ($grid:expr, $str:expr) => {
            $str.to_string().chars().for_each(|c| $grid.input(c))
        };
    }

    macro_rules! check_char {
        ($grid:expr, $col:expr, $row:expr, $char:expr) => {
            assert_eq!($grid.buffer[CursorPos::at($col, $row)].c, $char)
        };
    }

    macro_rules! check_cur {
        ($grid:expr, $col:expr, $row:expr) => {
            assert_eq!($grid.cursor, CursorPos::at($col, $row))
        };
    }

    #[test]
    fn goto() {
        let mut grid = Grid::<Sink>::new(4, 4);
        grid.goto(1, 1);
        check_cur!(grid, 1, 1);
        grid.move_up_and_cr(1);
        check_cur!(grid, 0, 0);
        grid.move_down(6);
        check_cur!(grid, 0, 3);
    }

    #[test]
    fn linefeed_reverse_idx() {
        let mut grid = Grid::<Sink>::new(8, 3);
        grid.goto(1, 0); // row, col
        input_str!(grid, "Hello");
        grid.goto(2, 1);
        input_str!(grid, "World");
        grid.linefeed();
        check_cur!(grid, 6, 2);
        check_char!(grid, 1, 1, 'W');
        grid.reverse_index();
        check_cur!(grid, 6, 1);
        check_char!(grid, 2, 1, 'o');
        grid.reverse_index();
        grid.reverse_index();
        check_cur!(grid, 6, 0);
        check_char!(grid, 1, 1, 'e');
        check_char!(grid, 3, 2, 'r');
        grid.linefeed();
        grid.linefeed();
        check_cur!(grid, 6, 2);
        grid.linefeed();
        grid.linefeed();
        check_char!(grid, 4, 0, 'l');
        check_cur!(grid, 6, 2);
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

    #[test]
    fn input_scroll() {
        let mut grid = Grid::<Sink>::new(4, 2);
        input_str!(grid, "Hello ");
        check_char!(grid, 0, 0, 'H');
        check_char!(grid, 0, 1, 'o');
        assert_eq!(grid.buffer[CursorPos::at(2, 1)], Cell::default());
        input_str!(grid, "World!");
        check_char!(grid, 0, 1, 'r');
        check_char!(grid, 3, 1, '!');
        check_char!(grid, 0, 0, 'o');
        check_char!(grid, 2, 0, 'W');
    }

    #[test]
    fn resize_scroll_up() {
        let mut grid = Grid::<Sink>::new(4, 4);
        input_str!(grid, "Hello World");
        check_char!(grid, 0, 0, 'H');
        check_char!(grid, 2, 1, 'W');
        check_char!(grid, 0, 2, 'r');
        check_cur!(grid, 3, 2);
        grid.resize(4, 3);
        check_char!(grid, 0, 0, 'H');
        check_char!(grid, 2, 1, 'W');
        check_char!(grid, 0, 2, 'r');
        check_cur!(grid, 3, 2);
        grid.resize(4, 2);
        check_char!(grid, 0, 0, 'o');
        check_char!(grid, 1, 0, ' ');
        check_char!(grid, 1, 1, 'l');
        check_cur!(grid, 3, 1);
        assert_eq!(grid.height, 2);
    }

    #[test]
    fn resize_scroll_up_newline() {
        // Slightly trickier: cursor is at the start of a new line.
        let mut grid = Grid::<Sink>::new(4, 4);
        input_str!(grid, "Hello World!");
        check_char!(grid, 0, 0, 'H');
        check_char!(grid, 2, 1, 'W');
        check_char!(grid, 0, 2, 'r');
        check_cur!(grid, 0, 3);
        grid.resize(4, 3);
        check_char!(grid, 0, 0, 'H');
        check_char!(grid, 2, 1, 'W');
        check_char!(grid, 0, 2, 'r');
        check_cur!(grid, 0, 3);
        grid.resize(4, 2);
        check_char!(grid, 0, 0, 'o');
        check_char!(grid, 1, 0, ' ');
        check_char!(grid, 1, 1, 'l');
        check_cur!(grid, 0, 2);
        assert_eq!(grid.height, 2);
    }
}
