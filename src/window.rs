//! Structures and functions to manage windows.
use anyhow::Result;
use nix::pty::Winsize;
use std::fs::File;
use std::sync::mpsc::Receiver;
use termion::raw::RawTerminal;

use crate::console::Console;

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pub console: Console,
    pub status: Receiver<bool>,
}

impl Window {
    pub fn new(
        command: &str,
        size: Winsize,
        output_stream: RawTerminal<File>,
    ) -> Result<Window, ()> {
        let (console, status) = Console::new(command, size, output_stream)?;
        Ok(Window { console, status })
    }

    pub fn get_file(&self) -> &File {
        &self.console.child_pty.file
    }
}
