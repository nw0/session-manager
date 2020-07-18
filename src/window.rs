//! Structures and functions to manage windows.

use anyhow::Result;
use nix::pty::Winsize;
use std::fs::File;
use std::sync::mpsc::Receiver;
use termion::raw::RawTerminal;

use crate::console::Console;

/// Window: a buffer and a pty.
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
