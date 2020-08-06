//! Structures and functions to manage windows.

use std::{fs::File, io::Write};

use anyhow::Result;
use futures::channel::mpsc::Receiver;
use nix::pty::Winsize;
use termion::raw::RawTerminal;
use vte::ansi::Processor;

use crate::{
    console::{self, ChildPty},
    grid::Grid,
    util,
};

/// A collection of `Window`s.
pub struct Session {
    next_window: usize,
    windows: Vec<Window>,
    processors: Vec<Processor>,
}

impl Session {
    /// Construct a new `Session`.
    pub fn new() -> Session {
        Session {
            next_window: 0,
            windows: Vec::new(),
            processors: Vec::new(),
        }
    }

    /// Initialise a new window within this `Session`.
    pub fn new_window(&mut self) -> Result<Receiver<u8>> {
        let (child, update) =
            Window::new(&util::get_shell(), util::get_term_size()?).unwrap();
        self.windows.push(child);
        self.processors.push(Processor::new());
        self.next_window += 1;
        Ok(update)
    }

    /// Send stdin to this `Window`.
    pub fn stdin_to_window(&self, idx: usize, data: &[u8]) -> Result<(), ()> {
        let mut file = self.windows.get(idx).expect("no such window").get_file();
        file.write(data).unwrap();
        file.flush().unwrap();
        Ok(())
    }

    /// Get a `Window` by index.
    pub fn get_window(&self, idx: usize) -> Option<&Window> {
        self.windows.get(idx)
    }

    /// Update grid with PTY output.
    pub fn pty_to_grid(
        &mut self,
        idx: usize,
        byte: u8,
        input: &mut File,
        tty_output: &mut RawTerminal<File>,
    ) {
        let window = self.windows.get_mut(idx).unwrap();
        self.processors
            .get_mut(idx)
            .unwrap()
            .advance(&mut window.grid, byte, input);
        window.grid.draw(tty_output);
    }

    /// Get a `Window` by index.
    pub fn get_window_mut(&mut self, idx: usize) -> Option<&mut Window> {
        self.windows.get_mut(idx)
    }

    pub fn resize_pty(&self, idx: usize) {
        let sz = util::get_term_size().unwrap();
        self.windows.get(idx).unwrap().pty.resize(sz).unwrap();
    }
}

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
