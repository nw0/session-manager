//! Structures and functions to manage windows.
use anyhow::Result;
use nix::pty::Winsize;
use std::fs::File;

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
    pub fn new(command: &str, size: Winsize) -> Result<Window, ()> {
        let console = Console::new(command, size)?;
        Ok(Window { console })
    }

    pub fn get_file(&self) -> &File {
        &self.console.child_pty.file
    }
}
