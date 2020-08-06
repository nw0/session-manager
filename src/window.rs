//! Structures and functions to manage windows.

use std::fs::File;

use anyhow::Result;
use futures::channel::mpsc::Receiver;
use nix::pty::Winsize;

use crate::console::{self, ChildPty};
use crate::grid::Grid;

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pub pty: ChildPty,
    pub grid: Grid<File>,
}

impl Window {
    pub fn new(command: &str, size: Winsize) -> Result<(Window, Receiver<u8>), ()> {
        let (pty, grid, pty_update) = console::spawn_pty(command, size)?;
        Ok((Window { pty, grid }, pty_update))
    }

    pub fn get_file(&self) -> &File {
        &self.pty.file
    }
}
