//! Structures to manage a pseudoterminal.

use std::{
    fs::File,
    io::Read,
    os::unix::io::{FromRawFd, RawFd},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
};

use futures::channel::mpsc::{self, Receiver};
use nix::{
    pty::{openpty, Winsize},
    unistd::setsid,
};

use crate::grid::Grid;

nix::ioctl_write_ptr_bad!(win_resize, libc::TIOCSWINSZ, nix::pty::Winsize);
nix::ioctl_none_bad!(set_controlling, libc::TIOCSCTTY);

/// A pty and state.
///
/// A `Console` should:
/// - Maintain a child pty, to connect to the underlying application
/// - Maintain a buffer (`Grid`) to display the child pty
/// - Given access to the tty, draw the buffer
/// - Notify if the child closes its pty
/// - Be capable of terminating the child
pub struct Console {
    pub child_pty: ChildPty,
    pub grid: Grid<File>,
}

impl Console {
    /// Initialise a new console.
    ///
    /// The returned `Receiver` signals when the process running in the child
    /// pty has terminated.
    pub fn new(command: &str, size: Winsize) -> Result<(Console, Receiver<u8>), ()> {
        let child_pty = ChildPty::new(command, size)?;
        let mut pty_output = child_pty.file.try_clone().unwrap().bytes();
        let (mut send, pty_update) = mpsc::channel(0x1000);
        thread::spawn(move || {
            while let Some(Ok(byte)) = pty_output.next() {
                send.try_send(byte).unwrap();
            }
            send.disconnect();
        });
        let grid = Grid::new(size.ws_col, size.ws_row);
        Ok((Console { child_pty, grid }, pty_update))
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
