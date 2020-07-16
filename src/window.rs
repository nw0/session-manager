//! Structures and functions to manage windows.

use anyhow::Result;
use nix::pty::{openpty, Winsize};
use nix::unistd::setsid;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread::{spawn, JoinHandle};
use termion::raw::RawTerminal;

nix::ioctl_write_ptr_bad!(win_resize, libc::TIOCSWINSZ, nix::pty::Winsize);
nix::ioctl_none_bad!(set_controlling, libc::TIOCSCTTY);

/// Window: a buffer and a pty.
pub struct Window {
    pub child_pty: ChildPty,
    pub update_thread: JoinHandle<Result<()>>,
}

impl Window {
    pub fn new(
        command: &str,
        size: Winsize,
        mut output_stream: RawTerminal<File>,
    ) -> Result<Window, ()> {
        let child_pty = ChildPty::new(command, size)?;
        let mut grid = Grid::new(size.ws_col, size.ws_row);
        let mut child_input = child_pty.file.try_clone().unwrap().bytes();
        let update_thread = spawn(move || {
            while let Some(Ok(byte)) = child_input.next() {
                grid.update(byte as char);
                grid.draw(&mut output_stream);
            }
            Ok(())
        });
        Ok(Window {
            child_pty,
            update_thread,
        })
    }

    pub fn get_file(&self) -> &File {
        &self.child_pty.file
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

struct Grid {
    cursor_x: u16,
    cursor_y: u16,
    width: u16,
    height: u16,
    buffer: Vec<Cell>,
}

impl Grid {
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

    pub fn draw(&self, term: &mut RawTerminal<File>) {
        for row in 0..self.height {
            for col in 0..self.width {
                write!(
                    term,
                    "{}{}",
                    termion::cursor::Goto(1 + col, 1 + row),
                    self.buffer[(col + row * self.width) as usize].c
                );
            }
        }
    }

    pub fn update(&mut self, c: char) {
        self.buffer[(self.cursor_x + self.cursor_y * self.width) as usize].c = c;
        self.cursor_x += 1;
        if self.cursor_x == self.width {
            self.cursor_x = 0;
            self.cursor_y += 1;
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
