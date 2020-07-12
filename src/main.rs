//! Session manager
//!
//! A would-be terminal multiplexer.

use nix::pty::{openpty, Winsize};
use nix::unistd::setsid;
use signal_hook::{iterator::Signals, SIGWINCH};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use termion::get_tty;
use termion::raw::IntoRawMode;

nix::ioctl_write_ptr_bad!(win_resize, libc::TIOCSWINSZ, nix::pty::Winsize);
nix::ioctl_none_bad!(set_controlling, libc::TIOCSCTTY);

fn main() {
    let signal = Signals::new(&[SIGWINCH]).unwrap();

    let mut tty_output = get_tty().unwrap().into_raw_mode().unwrap();
    let mut tty_input = tty_output.try_clone().unwrap();

    let child = Child::spawn(&get_shell(), get_term_size()).unwrap();
    let mut pty_output = child.file.try_clone().unwrap();
    let mut pty_input = child.file.try_clone().unwrap();

    let handle = thread::spawn(move || loop {
        let mut packet = [0; 4096];
        let count = pty_input.read(&mut packet).unwrap(); // TODO: handle connection drop
        let read = &packet[..count];
        tty_output.write_all(&read).unwrap();
        tty_output.flush().unwrap();
    });

    thread::spawn(move || loop {
        let mut packet = [0; 4096];
        let count = tty_input.read(&mut packet).unwrap();
        let read = &packet[..count];
        if read.len() == 1 && read[0] == 0x18 {
            // capture C-x for control
            // TODO
        } else {
            pty_output.write_all(&read).unwrap();
            pty_output.flush().unwrap();
        }
    });

    thread::spawn(move || loop {
        for _ in signal.forever() {
            child.resize(get_term_size()).unwrap();
        }
    });

    handle.join().unwrap();
}

struct Child {
    fd: RawFd,
    pub file: File,
}

impl Child {
    fn spawn(shell: &str, size: Winsize) -> Result<Child, ()> {
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
                    let child = Child {
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

pub fn get_term_size() -> Winsize {
    let (cols, rows) = termion::terminal_size().unwrap();
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

/// Return the path to the shell executable.
pub fn get_shell() -> String {
    // TODO: something reasonable
    "/bin/sh".to_string()
}
