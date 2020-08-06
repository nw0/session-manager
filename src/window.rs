//! Structures and functions to manage windows.

use std::fs::File;

use anyhow::Result;
use futures::channel::mpsc::Receiver;
use nix::pty::Winsize;

use crate::console::Console;

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pub console: Console,
}

impl Window {
    pub fn new(command: &str, size: Winsize) -> Result<(Window, Receiver<u8>), ()> {
        let (console, pty_update) = Console::new(command, size)?;
        Ok((Window { console }, pty_update))
    }

    pub fn get_file(&self) -> &File {
        &self.console.child_pty.file
    }
}
