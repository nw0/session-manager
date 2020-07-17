use log::{debug, info};
use std::fs::File;
use std::io::Write;
use termion::raw::RawTerminal;

enum ConsoleState {
    Normal,
    Esc,
    Csi,
}

/// A console, which contains a display Grid and some state.
pub struct Console {
    pub grid: Grid,
    state: ConsoleState,
}

impl Console {
    /// Initialise a new console.
    pub fn new(width: u16, height: u16) -> Console {
        Console {
            grid: Grid::new(width, height),
            state: ConsoleState::Normal,
        }
    }

    /// Read input from the pty.
    ///
    /// This method intercepts control codes and updates the `Console`'s internal state.
    pub fn update(&mut self, input: u8) {
        match self.state {
            ConsoleState::Normal => {
                match input {
                    b'' => {
                        // BEL
                        debug!("BEL");
                    }
                    b'' => {
                        // BS
                        if self.grid.cursor_x > 0 {
                            self.grid.cursor_x -= 1;
                            self.grid.set_current('_');
                        }
                    }
                    0x09 => {
                        // HT -- tab stop
                        info!("unhandled tab stop");
                    }
                    0x0a | b'' | b'' => {
                        // LF | VT | FF
                        self.grid.cursor_y += 1;
                    }
                    b'\r' => {
                        // CR
                        self.grid.cursor_x = 0;
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
                    b'' => {
                        // ESC
                        self.state = ConsoleState::Esc;
                    }
                    0x7f => {
                        // DEL
                    }
                    0x9b => {
                        // CSI
                        self.state = ConsoleState::Csi;
                    }
                    ch if ch.is_ascii() => {
                        self.grid.update(ch as char);
                    }
                    _ => {
                        debug!("unrecognised: {:2x}", input);
                    }
                }
            }
            ConsoleState::Esc => match input {
                b'[' => {
                    self.state = ConsoleState::Csi;
                }
                _ => {
                    debug!("ESC unrecognised: {:2x}", input);
                }
            },
            ConsoleState::Csi => {
                match input {
                    b'K' => {
                        // EL
                        // TODO: handle 1, 2
                        for x in self.grid.cursor_x..self.grid.width {
                            self.grid.set_cell('-', x, self.grid.cursor_y);
                        }
                        self.state = ConsoleState::Normal;
                    }
                    _ => {
                        debug!("CSI unrecognised: {:2x}", input);
                    }
                }
            }
        }
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
