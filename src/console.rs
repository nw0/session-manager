use log::{debug, info};
use std::fs::File;
use std::io::Write;
use termion::raw::RawTerminal;
use vte::{Parser, Perform};

/// A console, which contains a display Grid and some state.
pub struct Console {
    pub grid: Grid,
    parser: Parser,
}

impl Console {
    /// Initialise a new console.
    pub fn new(width: u16, height: u16) -> Console {
        Console {
            grid: Grid::new(width, height),
            parser: Parser::new(),
        }
    }

    /// Read input from the pty.
    ///
    /// This method intercepts control codes and updates the `Console`'s internal state.
    pub fn update(&mut self, input: u8) {
        self.parser.advance(&mut self.grid, input)
    }
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        self.update(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'' => {
                // BEL
                debug!("BEL");
            }
            b'' => {
                // BS
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            0x09 => {
                // HT -- tab stop
                self.cursor_x = std::cmp::min(self.width - 1, (self.cursor_x + 8) & !7);
            }
            0x0a | b'' | b'' => {
                // LF | VT | FF
                self.cursor_y += 1;
            }
            b'\r' => {
                // CR
                self.cursor_x = 0;
            }
            b'' => {
                // SO -- activate G1
            }
            b'' => {
                // SI -- activate G0
            }
            b'' => {
                // CAN -- abort
            }
            b'' => {
                // SUB -- abort
            }
            0x7f => {
                // DEL
            }
            _ => {
                debug!("[execute] {:02x}", byte);
            }
        }
    }

    fn hook(&mut self, params: &[i64], intermediates: &[u8], ignore: bool, c: char) {
        debug!(
            "[hook] params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, c
        );
    }

    fn put(&mut self, byte: u8) {
        debug!("[put] {:02x}", byte);
    }

    fn unhook(&mut self) {
        debug!("[unhook]");
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        debug!(
            "[osc_dispatch] params={:?} bell_terminated={}",
            params, bell_terminated
        );
    }

    fn csi_dispatch(&mut self, params: &[i64], intermediates: &[u8], ignore: bool, action: char) {
        match action {
            'K' => {
                // EL (K -- to end; 1 K -- from start; 2 K -- whole line)
                match params[0] {
                    0 => {
                        for x in self.cursor_x..self.width {
                            self.set_cell('_', x, self.cursor_y);
                        }
                    }
                    1 => {
                        for x in 0..self.cursor_x {
                            self.set_cell('_', x, self.cursor_y);
                        }
                    }
                    2 => {
                        for x in 0..self.width {
                            self.set_cell('_', x, self.cursor_y);
                        }
                    }
                    _ => {
                        debug!(
            "[csi_dispatch] params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, action
        );
                    }
                }
            }
            _ => {
                debug!(
                    "[csi_dispatch] params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
                    params, intermediates, ignore, action
                );
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        debug!(
            "[esc_dispatch] intermediates={:?}, ignore={:?}, byte={:02x}",
            intermediates, ignore, byte
        );
    }
}

/// The display buffer of a console.
pub struct Grid {
    cursor_x: u16,
    cursor_y: u16,
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
            cursor_x: 0,
            cursor_y: 0,
            width,
            height,
            buffer,
        }
    }

    /// Draw this buffer to `term`.
    pub fn draw(&self, term: &mut RawTerminal<File>) {
        for row in 0..self.height {
            let row_start = (row * self.width) as usize;
            let row_end = ((row + 1) * self.width) as usize;
            let row_chars = self.buffer[row_start..row_end].iter().map(|c| c.c);
            write!(
                term,
                "{}{}",
                termion::cursor::Goto(1, 1 + row),
                row_chars.collect::<String>()
            )
            .unwrap();
        }
        write!(
            term,
            "{}",
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
        }
    }

    pub fn set_current(&mut self, c: char) {
        self.set_cell(c, self.cursor_x, self.cursor_y);
    }

    pub fn set_cell(&mut self, c: char, x: u16, y: u16) {
        // TODO: check x < width, y < height
        self.buffer[(x + y * self.width) as usize].c = c;
    }
}

struct Cell {
    pub c: char,
}

impl Cell {
    pub fn default() -> Cell {
        Cell { c: '.' }
    }
}
