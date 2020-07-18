//! Console buffer implementation.
use log::{debug, info, warn};
use std::cmp::{max, min};
use std::convert::TryInto;
use std::fs::File;
use std::io::Write;
use termion::raw::RawTerminal;
use vte::Perform;

enum Displace {
    Absolute(i64),
    Relative(i64),
    ToStart,
    ToTabStop,
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
            if self.cursor_y == self.height {
                self.scroll(1);
                self.cursor_y -= 1;
            }
        }
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

    /// Scroll up (sorry, no scrolling down yet).
    fn scroll(&mut self, lines: u16) {
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
}

struct Cell {
    pub c: char,
}

impl Cell {
    pub fn default() -> Cell {
        Cell { c: '.' }
    }
}

/// Control character constants.
mod cc {
    pub const BEL: u8 = 0x07;
    pub const BS: u8 = 0x08;
    pub const HT: u8 = 0x09;
    pub const LF: u8 = 0x0a;
    pub const VT: u8 = 0x0b;
    pub const FF: u8 = 0x0c;
    pub const CR: u8 = 0x0d;
    pub const SO: u8 = 0x0e;
    pub const SI: u8 = 0x0f;

    pub const CAN: u8 = 0x18;
    pub const SUB: u8 = 0x1a;
    pub const DEL: u8 = 0x7f;
}

/// CSI sequences.
///
/// `char` used for compatibility with `csi_dispatch`.
// from ECMA-48, via `man console_codes` -- some missing?
mod csi {
    pub const ICH: char = '@';
    pub const CUU: char = 'A';
    pub const CUD: char = 'B';
    pub const CUF: char = 'C';
    pub const CUB: char = 'D';
    pub const CNL: char = 'E';
    pub const CPL: char = 'F';
    pub const CHA: char = 'G';
    pub const CUP: char = 'H';
    pub const ED: char = 'J';
    pub const EL: char = 'K';
    pub const IL: char = 'L';
    pub const DL: char = 'M';
    pub const DCH: char = 'P';
    pub const ECH: char = 'X';
    pub const HPR: char = 'a';
    pub const DA: char = 'c';
    pub const VPA: char = 'd';
    pub const VPR: char = 'e';
    pub const HVP: char = 'f';
    pub const TBC: char = 'g';
    pub const SM: char = 'h';
    pub const RM: char = 'l';
    pub const SGR: char = 'm';
    pub const DSR: char = 'n';
    pub const DECLL: char = 'q';
    pub const DECSTBM: char = 'r';
    pub const SAVEC: char = 's'; // not official name
    pub const RESTC: char = 'u'; // not official name
    pub const HPA: char = '`';
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        self.update(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            cc::BEL => info!("BEL"),
            cc::BS => self.move_horizontal(Displace::Relative(-1)),
            cc::HT => self.move_horizontal(Displace::ToTabStop),
            cc::LF | cc::VT | cc::FF => {
                self.cursor_y += 1;
                if self.cursor_y == self.height {
                    self.scroll(1);
                    self.cursor_y -= 1;
                }
            }
            cc::CR => self.move_horizontal(Displace::ToStart),
            cc::SO => info!("unimpl: exec SO"),
            cc::SI => info!("unimpl: exec SI"),
            cc::CAN => debug!("unimpl: exec CAN"),
            cc::SUB => debug!("unimpl: exec SUB"),
            cc::DEL => debug!("DEL"),
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
        if !params.is_empty() {
            match params[0] {
                b"0" | b"2" => {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        info!("[osc] set title: \"{}\"", title)
                    }
                }
                _ => {
                    debug!(
                        "[osc_dispatch] params={:?} bell_terminated={}",
                        params, bell_terminated
                    );
                }
            }
        } else {
            debug!("empty OSC sequence");
        }
    }

    fn csi_dispatch(&mut self, params: &[i64], intermediates: &[u8], ignore: bool, action: char) {
        match action {
            csi::CUU => {
                let n = std::cmp::max(1, params[0]) as u16;
                self.cursor_y = std::cmp::max(0, self.cursor_y - n);
            }
            csi::CUD => {
                let n = std::cmp::max(1, params[0]) as u16;
                self.cursor_y = std::cmp::min(self.height - 1, self.cursor_y - n);
            }
            csi::CUF => {
                let n = std::cmp::max(1, params[0]);
                self.move_horizontal(Displace::Relative(n));
            }
            csi::CUB => {
                let n = std::cmp::max(1, params[0]);
                self.move_horizontal(Displace::Relative(-n))
            }
            csi::CUP => {
                self.move_vertical(Displace::Absolute(params[0] - 1));
                if params.len() > 1 {
                    self.move_horizontal(Displace::Absolute(params[1] - 1));
                } else {
                    self.move_horizontal(Displace::Absolute(0));
                }
            }
            csi::ED => match params[0] {
                0 => {
                    let cur_pos = (self.cursor_x + (self.width * self.cursor_y)) as usize;
                    for i in cur_pos..(self.buffer.len()) {
                        self.buffer[i].c = '.';
                    }
                }
                1 => {
                    let cur_pos = self.cursor_x + (self.width * self.cursor_y);
                    for i in 0..cur_pos {
                        self.buffer[i as usize].c = '.';
                    }
                }
                2 | 3 => {
                    for i in &mut self.buffer {
                        i.c = '.';
                    }
                }
                _ => {
                    debug!(
            "[csi_dispatch] (J) params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, action
        );
                }
            },
            csi::EL => match params[0] {
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
            "[csi_dispatch] (K) params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
            params, intermediates, ignore, action
        );
                }
            },
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
