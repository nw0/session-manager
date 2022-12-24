//! Structures to manage a pseudoterminal.

use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    os::unix::io::{FromRawFd, RawFd},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
};

use futures::{
    channel::mpsc::{self, Receiver},
    executor, future,
};
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
pub fn spawn_pty<I, S>(
    command: &str,
    args: I,
    size: Winsize,
) -> Result<(ChildPty, Grid<File>), ()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let child_pty = ChildPty::new(command, args, size)?;
    let mut pty_output = child_pty.file.try_clone().unwrap();
    let grid = Grid::new(size.ws_col, size.ws_row);
    Ok((child_pty, grid))
}

/// An update from a PTY.
#[derive(Debug, PartialEq, Eq)]
pub enum PtyUpdate {
    /// The PTY has closed the file.
    Exited,
    /// PTY sends byte.
    Byte(u8),
}

/// A pseudoterminal.
pub struct ChildPty {
    fd: RawFd,
    /// The File used by this PTY.
    pub file: File,
}

impl ChildPty {
    /// Spawn a process in a new pty.
    pub fn new<I, S>(command: &str, args: I, size: Winsize) -> Result<ChildPty, ()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let pty = openpty(&size, None).unwrap();
        unsafe {
            Command::new(&command)
                .args(args)
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
    use crate::tests::WINSZ;

    use std::path::{Path, PathBuf};

    use futures::stream::StreamExt;

    #[test]
    fn open_child_pty() {
        use std::io::Read;
        use std::str;

        let args: [&str; 0] = [];
        let mut child = ChildPty::new("pwd", &args, WINSZ).unwrap();
        let mut buffer = [0; 1024];
        let count = child.file.read(&mut buffer).unwrap();
        let data = str::from_utf8(&buffer[..count]).unwrap().trim();
        assert_eq!(Path::new(&data), std::env::current_dir().unwrap());
    }

    #[test]
    fn regulate_pty_update() {
        let mut dir = PathBuf::from(std::env!("CARGO_MANIFEST_DIR"));
        dir.push("Cargo.lock"); // a suitably long file
        let (_, _, mut recv) =
            spawn_pty("cat", &[&dir.into_os_string()], WINSZ).unwrap();
        while let Some(msg) = executor::block_on(recv.next()) {
            if msg == PtyUpdate::Exited {
                return;
            }
        }
        assert!(false, "update thread did not exit cleanly")
    }
}
