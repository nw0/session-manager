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

mod ioctl {
    nix::ioctl_none_bad!(set_controlling, libc::TIOCSCTTY);
    nix::ioctl_write_ptr_bad!(win_resize, libc::TIOCSWINSZ, nix::pty::Winsize);
}

/// Initialise a new process and grid.
pub fn spawn_pty(
    command: &str,
    size: Winsize,
) -> Result<(ChildPty, Grid<File>, Receiver<u8>), ()> {
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
    Ok((child_pty, grid, pty_update))
}

/// A pseudoterminal.
pub struct ChildPty {
    fd: RawFd,
    /// The File used by this PTY.
    pub file: File,
}

impl ChildPty {
    /// Spawn a process in a new pty.
    pub fn new(shell: &str, size: Winsize) -> Result<ChildPty, ()> {
        let pty = openpty(&size, None).unwrap();
        unsafe {
            Command::new(&shell)
                .stdin(Stdio::from_raw_fd(pty.slave))
                .stdout(Stdio::from_raw_fd(pty.slave))
                .stderr(Stdio::from_raw_fd(pty.slave))
                .pre_exec(|| {
                    setsid().unwrap();
                    ioctl::set_controlling(0).unwrap();
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

    /// Send a resize to the process running in this PTY.
    pub fn resize(&self, size: Winsize) -> Result<(), ()> {
        unsafe { ioctl::win_resize(self.fd, &size) }
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
