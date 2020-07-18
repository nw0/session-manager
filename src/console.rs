use log::{debug, info};
use nix::pty::{openpty, Winsize};
use nix::unistd::setsid;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use termion::raw::RawTerminal;
use vte::{Parser, Perform};

nix::ioctl_write_ptr_bad!(win_resize, libc::TIOCSWINSZ, nix::pty::Winsize);
nix::ioctl_none_bad!(set_controlling, libc::TIOCSCTTY);

/// A console, which contains a display Grid and some state.
pub struct Console {
    pub child_pty: ChildPty,
}

impl Console {
    /// Initialise a new console.
    pub fn new(
        command: &str,
        size: Winsize,
        mut output_stream: RawTerminal<File>,
    ) -> Result<(Console, Receiver<bool>), ()> {
        let child_pty = ChildPty::new(command, size)?;
        let (sender, status) = channel();
        let mut pty_output = child_pty.file.try_clone().unwrap().bytes();
        let mut parser = Parser::new();
        let mut grid = Grid::new(size.ws_col, size.ws_row);

        thread::spawn(move || {
            while let Some(Ok(byte)) = pty_output.next() {
                parser.advance(&mut grid, byte);
                grid.draw(&mut output_stream);
            }
            sender.send(true).unwrap();
        });

        Ok((Console { child_pty }, status))
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
            'C' => {
                // CUF -- move cursor forward #
                let n = std::cmp::max(1, params[0]) as u16;
                self.cursor_x = std::cmp::min(self.width - 1, self.cursor_x + n);
            }
            'D' => {
                // CUB -- move cursor back #
                let n = std::cmp::max(1, params[0]) as u16;
                self.cursor_x = std::cmp::max(0, self.cursor_x - n);
            }
            'H' => {
                // CUP -- move cursor
                self.cursor_x = std::cmp::max(0, params[0] - 1) as u16;
                if params.len() > 1 {
                    self.cursor_y = std::cmp::max(0, params[1] - 1) as u16;
                } else {
                    self.cursor_y = 0;
                }
            }
            'J' => {
                // ED -- erase display
                match params[0] {
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
                }
            }
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
            "[csi_dispatch] (K) params={:?}, intermediates={:?}, ignore={:?}, char={:?}",
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

/// A pty.
pub struct ChildPty {
    fd: RawFd,
    pub file: File,
}

impl ChildPty {
    pub fn new(shell: &str, size: Winsize) -> Result<ChildPty, ()> {
        let pty = openpty(&size, None).unwrap();
        unsafe {
            Command::new(&shell)
                .stdin(Stdio::from_raw_fd(pty.slave))
                .stdout(Stdio::from_raw_fd(pty.slave))
                .stderr(Stdio::from_raw_fd(pty.slave))
                .pre_exec(|| {
                    setsid().unwrap();
                    set_controlling(0).unwrap();
                    Ok(())
                })
                .spawn()
                .map_err(|_| ())
                .and_then(|_| {
                    let child = ChildPty {
                        fd: pty.master,
                        file: File::from_raw_fd(pty.master),
                    };

                    child.resize(size)?;

                    Ok(child)
                })
        }
    }

    pub fn resize(&self, size: Winsize) -> Result<(), ()> {
        unsafe { win_resize(self.fd, &size) }
            .map(|_| ())
            .map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_child_pty() {
        use std::io::Read;
        use std::path::Path;
        use std::str;

        let mut child = ChildPty::new(
            "pwd",
            Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let mut buffer = [0; 1024];
        let count = child.file.read(&mut buffer).unwrap();
        let data = str::from_utf8(&buffer[..count]).unwrap().trim();
        assert_eq!(Path::new(&data), std::env::current_dir().unwrap());
    }
}
